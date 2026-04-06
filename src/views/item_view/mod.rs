mod comment_card;
mod feed_posts;
mod loader;
mod reactions;
pub mod types;

use crate::{
    accounts::AccountStore,
    acuity_runtime::api,
    comment::load_comments_for_item,
    content::{RevisionEntry, SelectedImage},
    item::{publish_item_revision, ItemDraft, PublishRevisionRequest},
    runtime_client::estimate_fee,
    ChainConnection,
    Route,
};
use dioxus::prelude::*;

use comment_card::CommentCard;
use feed_posts::load_feed_posts;
use loader::load_item;
use reactions::Reactions;
use types::{ActiveTab, FeedPost, LoadedItem};

use super::components::{EmptyState, ImageDropZone, InsufficientFundsHint};

const ITEM_VIEW_CSS: Asset = asset!("/assets/styling/item_view.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

#[component]
pub fn ItemView(encoded_item_id: ReadSignal<String>) -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let chain_connection = use_context::<Signal<ChainConnection>>();

    let mut loaded: Signal<Option<LoadedItem>> = use_signal(|| None);
    let mut is_loading = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);

    // Feed posts state (only used when the item is a Feed)
    let mut feed_posts: Signal<Vec<FeedPost>> = use_signal(Vec::new);
    let mut posts_loading = use_signal(|| false);
    let mut posts_error: Signal<Option<String>> = use_signal(|| None);

    // ── Revision history state ──────────────────────────────────────────────
    // All revisions for this item (newest first) fetched from the indexer.
    let mut revision_history: Signal<Vec<RevisionEntry>> = use_signal(Vec::new);
    // The on-chain canonical latest revision_id (from Content.ItemState).
    let mut chain_latest_revision_id: Signal<u32> = use_signal(|| 0_u32);
    // Whether a switch-revision reload is in progress.
    let mut revision_switching = use_signal(|| false);

    // ── Edit tab state ──────────────────────────────────────────────────────
    let mut active_tab = use_signal(|| ActiveTab::View);
    let mut draft = use_signal(ItemDraft::default);
    let mut selected_image = use_signal(|| None::<SelectedImage>);
    let mut is_saving = use_signal(|| false);
    let mut save_error: Signal<Option<String>> = use_signal(|| None);
    let mut save_notice: Signal<Option<String>> = use_signal(|| None);
    // Incrementing this signal re-triggers the load effect after a save.
    let mut reload_tick = use_signal(|| 0_u64);

    // ── Comment state ───────────────────────────────────────────────────────
    let mut comments = use_signal(Vec::new);
    let mut comments_loading = use_signal(|| false);
    let mut comments_error: Signal<Option<String>> = use_signal(|| None);
    let mut top_level_reply_body: Signal<String> = use_signal(String::new);
    let mut top_level_submitting = use_signal(|| false);
    let mut top_level_submit_error: Signal<Option<String>> = use_signal(|| None);

    // Full load: fetch history + latest content, also load feed posts and comments.
    use_effect(move || {
        let id = encoded_item_id();
        let _tick = reload_tick();
        spawn(async move {
            error_message.set(None);
            posts_error.set(None);
            comments_error.set(None);
            feed_posts.set(Vec::new());
            comments.set(Vec::new());
            revision_history.set(Vec::new());
            is_loading.set(true);
            match load_item(&id, None).await {
                Ok((item, history, chain_latest)) => {
                    let is_feed = item.content_type == "Feed";
                    let item_id_hex = item.item_id_hex.clone();
                    // Pre-populate edit draft from loaded content.
                    draft.set(ItemDraft {
                        title: item.title.clone(),
                        body: item.body_text.clone(),
                    });
                    selected_image.set(None);
                    active_tab.set(ActiveTab::View);
                    chain_latest_revision_id.set(chain_latest);
                    revision_history.set(history);
                    loaded.set(Some(item));
                    is_loading.set(false);

                    // If this is a feed, load its child posts.
                    if is_feed {
                        posts_loading.set(true);
                        match load_feed_posts(&item_id_hex).await {
                            Ok(posts) => feed_posts.set(posts),
                            Err(err) => posts_error.set(Some(err)),
                        }
                        posts_loading.set(false);
                    }

                    // Load comments for all item types.
                    comments_loading.set(true);
                    match load_comments_for_item(item_id_hex.clone()).await {
                        Ok(c) => comments.set(c),
                        Err(err) => comments_error.set(Some(err)),
                    }
                    comments_loading.set(false);
                }
                Err(err) => {
                    loaded.set(None);
                    error_message.set(Some(err));
                    is_loading.set(false);
                }
            }
        });
    });

    // True when the active account is the on-chain owner of this item.
    let is_owner = use_memo(move || {
        let Some(ref item) = loaded() else {
            return false;
        };
        account_store()
            .active_account()
            .map(|a| a.address == item.owner_address)
            .unwrap_or(false)
    });

    // Fee estimation for "Save changes" (publish_revision).
    let edit_fee_estimate = use_resource(move || async move {
        let signer = account_store().active_signer().cloned()?;
        let item_id = loaded()?.item_id;
        let dummy_ipfs_hash = [0u8; 32];
        let call = api::tx().content().publish_revision(
            api::runtime_types::pallet_content::pallet::ItemId(item_id),
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            api::runtime_types::pallet_content::pallet::IpfsHash(dummy_ipfs_hash),
        );
        estimate_fee(&call, &signer).await.ok()
    });

    let edit_insufficient_funds = use_memo(move || {
        let balance = chain_connection().details.active_account_balance;
        let fee = edit_fee_estimate().flatten();
        match (balance, fee) {
            (Some(b), Some(f)) => b < f,
            _ => true, // block until both balance and fee are known
        }
    });

    // Fee estimation for "Post comment" (publish_item with parent).
    let comment_fee_estimate = use_resource(move || async move {
        let signer = account_store().active_signer().cloned()?;
        let parent_id = loaded()?.item_id;
        let dummy_nonce = [0u8; 32];
        let dummy_ipfs_hash = [0u8; 32];
        let call = api::tx().content().publish_item(
            api::runtime_types::pallet_content::Nonce(dummy_nonce),
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![
                api::runtime_types::pallet_content::pallet::ItemId(parent_id),
            ]),
            0x01,
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            api::runtime_types::pallet_content::pallet::IpfsHash(dummy_ipfs_hash),
        );
        estimate_fee(&call, &signer).await.ok()
    });

    let comment_insufficient_funds = use_memo(move || {
        let balance = chain_connection().details.active_account_balance;
        let fee = comment_fee_estimate().flatten();
        match (balance, fee) {
            (Some(b), Some(f)) => b < f,
            _ => true, // block until both balance and fee are known
        }
    });

    // Top-level comment submit handler (reply to the item itself).
    let submit_top_level_comment = {
        move |_| {
            let body = top_level_reply_body();
            if body.trim().is_empty() {
                return;
            }
            let store = account_store();
            let Some(item) = loaded() else { return };
            let parent_item_id = item.item_id;
            spawn(async move {
                use crate::comment::{publish_comment, CommentDraft, PublishCommentRequest};
                top_level_submit_error.set(None);
                top_level_submitting.set(true);
                let req = PublishCommentRequest {
                    draft: CommentDraft { body },
                    parent_item_id,
                };
                match publish_comment(&store, req).await {
                    Ok(_) => {
                        top_level_reply_body.set(String::new());
                        reload_tick.with_mut(|t| *t += 1);
                    }
                    Err(e) => top_level_submit_error.set(Some(e)),
                }
                top_level_submitting.set(false);
            });
        }
    };

    // Edit status bar: error > saving > notice
    let edit_status: Option<(&'static str, String)> = if let Some(ref err) = save_error() {
        Some(("error", err.clone()))
    } else if is_saving() {
        Some((
            "loading",
            "Publishing the updated revision to IPFS and the chain...".to_string(),
        ))
    } else {
        save_notice().map(|n| ("notice", n))
    };

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }
        document::Link { rel: "stylesheet", href: ITEM_VIEW_CSS }

        div {
            class: "item-view-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    p { class: "page-eyebrow", "Content item" }
                    h1 { class: "page-title",
                        if let Some(ref item) = loaded() {
                            if item.title.trim().is_empty() {
                                "Untitled item"
                            } else {
                                "{item.title}"
                            }
                        } else {
                            "Item"
                        }
                    }
                }
            }

            // ── Load status bar ────────────────────────────────────────────
            if let Some(err) = error_message() {
                div { class: "status-bar error", "{err}" }
            } else if is_loading() {
                div { class: "status-bar loading", "Loading item from the indexer and IPFS..." }
            }

            // ── Tab bar (only when item is loaded and owned by active account) ──
            if loaded().is_some() && is_owner() {
                div {
                    class: "iv-tab-bar",
                    button {
                        class: if active_tab() == ActiveTab::View { "iv-tab active" } else { "iv-tab" },
                        onclick: move |_| {
                            save_error.set(None);
                            save_notice.set(None);
                            active_tab.set(ActiveTab::View);
                        },
                        "View"
                    }
                    button {
                        class: if active_tab() == ActiveTab::Edit { "iv-tab active" } else { "iv-tab" },
                        onclick: move |_| {
                            save_error.set(None);
                            save_notice.set(None);
                            active_tab.set(ActiveTab::Edit);
                        },
                        "Edit"
                    }
                }
            }

            if is_loading() {
                div { class: "item-view-grid",
                    div { class: "panel-surface item-view-main skeleton-block" }
                    div { class: "panel-surface item-view-side skeleton-block" }
                }
            } else if let Some(item) = loaded() {

                // ── View tab ───────────────────────────────────────────────
                if active_tab() == ActiveTab::View {
                    div {
                        class: "item-view-grid",

                        // ── Left: content ──────────────────────────────────
                        section {
                            class: "panel-surface item-view-main",

                            // ── Older-revision banner ──────────────────────
                            if item.revision_id < chain_latest_revision_id() {
                                div {
                                    class: "iv-revision-banner",
                                    span {
                                        "Viewing revision {item.revision_id} \u{2014} latest is revision {chain_latest_revision_id()}"
                                    }
                                    button {
                                        class: "iv-revision-banner-btn",
                                        onclick: move |_| {
                                            let id = encoded_item_id();
                                            revision_switching.set(true);
                                            spawn(async move {
                                                match load_item(&id, None).await {
                                                    Ok((new_item, new_history, chain_latest)) => {
                                                        chain_latest_revision_id.set(chain_latest);
                                                        revision_history.set(new_history);
                                                        loaded.set(Some(new_item));
                                                    }
                                                    Err(err) => {
                                                        error_message.set(Some(err));
                                                    }
                                                }
                                                revision_switching.set(false);
                                            });
                                        },
                                        if revision_switching() { "Loading..." } else { "View latest" }
                                    }
                                }
                            }

                            // Parent links
                            if !item.parents.is_empty() {
                                div { class: "iv-parents",
                                    span { class: "iv-parents-label", "In" }
                                    for parent in item.parents.clone() {
                                        Link {
                                            class: "iv-parent-link",
                                            to: Route::ItemView {
                                                encoded_item_id: parent.encoded_item_id.clone(),
                                            },
                                            span { class: "iv-parent-type", "{parent.content_type}" }
                                            span { class: "iv-parent-title", "{parent.title}" }
                                        }
                                    }
                                }
                            }

                            // Image
                            if let Some(img_url) = item.image_preview_data_url.clone() {
                                img {
                                    class: "iv-image",
                                    src: img_url,
                                    alt: "Item image",
                                }
                            }

                            // Type pill
                            span { class: "iv-type-pill", "{item.content_type}" }

                            // Title
                            if !item.title.trim().is_empty() {
                                h2 { class: "iv-title", "{item.title}" }
                            }

                            // Body text
                            if !item.body_text.trim().is_empty() {
                                p { class: "iv-body", "{item.body_text}" }
                            }

                            // Empty content notice
                            if item.title.trim().is_empty() && item.body_text.trim().is_empty() {
                                p { class: "pv-notice",
                                    "This item has no title or body text."
                                }
                            }

                            // ── Reactions ──────────────────────────────────
                            Reactions {
                                item_id: item.item_id,
                                revision_id: item.revision_id,
                            }
                        }

                        // ── Right: metadata ────────────────────────────────
                        aside {
                            class: "panel-surface item-view-side",

                            div { class: "pv-meta-section",
                                p { class: "pv-section-label", "Item metadata" }
                                div { class: "metadata-list",
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Item ID" }
                                        code { class: "meta-code",
                                            {crate::content::short_hex(&item.item_id_hex)}
                                        }
                                    }
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Revision IPFS hash" }
                                        code { class: "meta-code",
                                            {crate::content::short_hex(&item.revision_ipfs_hash_hex)}
                                        }
                                    }
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Type" }
                                        span { class: "meta-copy", "{item.content_type}" }
                                    }
                                    if !item.language.is_empty() {
                                        div { class: "metadata-row",
                                            span { class: "meta-label", "Language" }
                                            span { class: "meta-copy", "{item.language}" }
                                        }
                                    }
                                }
                            }

                            // ── Revision selector ─────────────────────────
                            if revision_history().len() > 1 {
                                div { class: "pv-meta-section",
                                    p { class: "pv-section-label", "Revisions" }
                                    select {
                                        class: "iv-revision-select",
                                        disabled: revision_switching(),
                                        value: item.revision_id.to_string(),
                                        onchange: move |e| {
                                            let selected_rid: u32 = e.value().parse().unwrap_or(0);
                                            let history = revision_history();
                                            let Some(entry) = history
                                                .iter()
                                                .find(|r| r.revision_id == selected_rid)
                                            else {
                                                return;
                                            };
                                            let hash = entry.ipfs_hash_hex.clone();
                                            let is_latest = selected_rid == chain_latest_revision_id();
                                            let override_hash = if is_latest { None } else { Some(hash.clone()) };
                                            let id = encoded_item_id();
                                            revision_switching.set(true);
                                            spawn(async move {
                                                match load_item(&id, override_hash).await {
                                                    Ok((new_item, new_history, chain_latest)) => {
                                                        chain_latest_revision_id.set(chain_latest);
                                                        revision_history.set(new_history);
                                                        loaded.set(Some(new_item));
                                                    }
                                                    Err(err) => {
                                                        error_message.set(Some(err));
                                                    }
                                                }
                                                revision_switching.set(false);
                                            });
                                        },
                                        for rev in revision_history() {
                                            {
                                                let is_latest = rev.revision_id == chain_latest_revision_id();
                                                let label = if is_latest {
                                                    format!("Revision {} (latest)", rev.revision_id)
                                                } else {
                                                    format!("Revision {}", rev.revision_id)
                                                };
                                                let rid_str = rev.revision_id.to_string();
                                                let selected = rev.revision_id == item.revision_id;
                                                rsx! {
                                                    option {
                                                        value: rid_str,
                                                        selected,
                                                        "{label}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // ── Publish Post button (feeds only) ──────────
                            if item.content_type == "Feed" {
                                div { class: "pv-meta-section",
                                    Link {
                                        class: "btn-primary",
                                        to: Route::PublishPost {
                                            encoded_feed_id: item.encoded_item_id.clone(),
                                        },
                                        "Publish post"
                                    }
                                }
                            }
                        }
                    }

                    // ── Feed posts section (feeds only) ───────────────────
                    if item.content_type == "Feed" {
                        section {
                            class: "iv-feed-posts-section",

                            h3 { class: "iv-feed-posts-heading", "Posts" }

                            if posts_loading() {
                                div { class: "status-bar loading", "Loading posts..." }
                            }

                            if let Some(err) = posts_error() {
                                div { class: "status-bar error", "{err}" }
                            }

                            if !posts_loading() && feed_posts().is_empty() && posts_error().is_none() {
                                p { class: "pv-notice", "No posts in this feed yet." }
                            }

                            for post in feed_posts() {
                                Link {
                                    class: "iv-feed-post-card panel-surface",
                                    to: Route::ItemView {
                                        encoded_item_id: post.encoded_item_id.clone(),
                                    },

                                    if let Some(img_url) = post.image_preview_data_url.clone() {
                                        img {
                                            class: "iv-feed-post-thumb",
                                            src: img_url,
                                            alt: "Post image",
                                        }
                                    }

                                    div { class: "iv-feed-post-text",
                                        if !post.title.trim().is_empty() {
                                            h4 { class: "iv-feed-post-title", "{post.title}" }
                                        }
                                        if !post.body_preview.trim().is_empty() {
                                            p { class: "iv-feed-post-body", "{post.body_preview}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── Comments section ───────────────────────────────────
                    section {
                        class: "iv-comments-section",

                        h3 { class: "iv-comments-heading", "Comments" }

                        // Top-level reply form
                        div { class: "iv-top-comment-form",
                            if let Some(err) = top_level_submit_error() {
                                div { class: "status-bar error", "{err}" }
                            }
                            textarea {
                                class: "comment-textarea",
                                rows: "3",
                                placeholder: if account_store().is_active_unlocked() {
                                    "Leave a comment…"
                                } else {
                                    "Unlock your account to leave a comment."
                                },
                                disabled: top_level_submitting() || !account_store().is_active_unlocked(),
                                value: top_level_reply_body(),
                                oninput: move |e| top_level_reply_body.set(e.value()),
                            }
                            if account_store().is_active_unlocked() {
                                div { class: "comment-reply-actions",
                                    button {
                                        class: "btn-primary",
                                        disabled: top_level_submitting() || top_level_reply_body().trim().is_empty() || comment_insufficient_funds(),
                                        onclick: submit_top_level_comment,
                                        if top_level_submitting() { "Posting…" } else { "Post comment" }
                                    }
                                    InsufficientFundsHint {
                                        balance: chain_connection().details.active_account_balance,
                                        fee: comment_fee_estimate().flatten(),
                                        fee_state: comment_fee_estimate.state()(),
                                    }
                                }
                            }
                        }

                        // Comment thread
                        if comments_loading() {
                            div { class: "status-bar loading", "Loading comments..." }
                        }

                        if let Some(err) = comments_error() {
                            div { class: "status-bar error", "{err}" }
                        }

                        if !comments_loading() && comments().is_empty() && comments_error().is_none() {
                            p { class: "pv-notice", "No comments yet. Be the first!" }
                        }

                        for comment in comments() {
                            CommentCard {
                                comment,
                                depth: 0,
                                reload_tick,
                                account_store,
                            }
                        }
                    }

                // ── Edit tab ───────────────────────────────────────────────
                } else {
                    section {
                        class: "panel-surface iv-edit-editor",

                        // Edit status bar
                        if let Some((kind, msg)) = edit_status {
                            div { class: "status-bar {kind}", "{msg}" }
                        }

                        // ── Title ──────────────────────────────────────────
                        label { class: "field",
                            span { "Title" }
                            input {
                                value: draft().title,
                                placeholder: "Item title",
                                disabled: is_saving(),
                                oninput: move |e| draft.with_mut(|d| d.title = e.value()),
                            }
                        }

                        // ── Body ───────────────────────────────────────────
                        label { class: "field",
                            span { "Body" }
                            textarea {
                                value: draft().body,
                                rows: "10",
                                placeholder: "Item body text",
                                disabled: is_saving(),
                                oninput: move |e| draft.with_mut(|d| d.body = e.value()),
                            }
                        }

                        // ── Image ──────────────────────────────────────────
                        div { class: "field",
                            span { "Image (optional)" }
                            ImageDropZone {
                                selected_image,
                                existing_preview_url: item.image_preview_data_url.clone(),
                                disabled: is_saving(),
                            }
                            if let Some(ref img) = selected_image() {
                                p { class: "field-note", "Pending: {img.file_name}" }
                            } else if item.existing_image_payload.is_some() {
                                p { class: "field-note field-note-muted", "Using the currently published image." }
                            }
                        }

                        // ── Actions ────────────────────────────────────────
                        div { class: "form-actions",
                            button {
                                class: "btn-primary",
                                disabled: is_saving() || edit_insufficient_funds(),
                                onclick: {
                                    let item_id = item.item_id;
                                    let content_type = item.content_type.clone();
                                    let existing_image_payload = item.existing_image_payload.clone();
                                    move |_| {
                                        let draft_snap = draft();
                                        let selected_img = selected_image();
                                        let content_type = content_type.clone();
                                        let existing_image_payload = existing_image_payload.clone();
                                        let account_store_snap = account_store();
                                        spawn(async move {
                                            save_error.set(None);
                                            save_notice.set(None);
                                            is_saving.set(true);

                                            let req = PublishRevisionRequest {
                                                item_id,
                                                content_type,
                                                draft: draft_snap,
                                                selected_image: selected_img,
                                                existing_image_payload,
                                            };

                                            match publish_item_revision(&account_store_snap, req).await {
                                                Ok(()) => {
                                                    save_notice.set(Some(
                                                        "Revision published successfully.".to_string(),
                                                    ));
                                                    reload_tick.with_mut(|t| *t += 1);
                                                }
                                                Err(err) => save_error.set(Some(err)),
                                            }
                                            is_saving.set(false);
                                        });
                                    }
                                },
                                if is_saving() { "Saving..." } else { "Save changes" }
                            }
                            button {
                                class: "btn-ghost",
                                disabled: is_saving(),
                                onclick: move |_| {
                                    save_error.set(None);
                                    save_notice.set(None);
                                    active_tab.set(ActiveTab::View);
                                },
                                "Cancel"
                            }
                        }

                        if !account_store().is_active_unlocked() {
                            p { class: "save-locked-hint",
                                "Unlock the account from the sidebar to save."
                            }
                        }
                        if account_store().is_active_unlocked() {
                            InsufficientFundsHint {
                                balance: chain_connection().details.active_account_balance,
                                fee: edit_fee_estimate().flatten(),
                                fee_state: edit_fee_estimate.state()(),
                            }
                        }
                    }
                }

            } else if !is_loading() && error_message().is_none() {
                EmptyState {
                    title: "Item not found".to_string(),
                    body: "The item could not be loaded. It may not have been indexed yet.".to_string(),
                    cta_label: Some("Go to Dashboard".to_string()),
                    cta_route: Some(Route::Home {}),
                }
            }
        }
    }
}
