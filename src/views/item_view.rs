use crate::{
    acuity_runtime::api,
    accounts::AccountStore,
    content::{
        build_image_mixin, bytes32_to_hex, decode_single_mixin, fetch_events_for_item,
        fetch_ipfs_digest_bytes, fetch_latest_revision_hash, hex_to_bytes32,
        preview_data_url_for_image_mixin, preview_data_url_for_path, upload_ipfs_digest,
        BodyTextMixinMessage, IndexerStoredEvent, ItemMessage, LanguageMixinMessage,
        MixinPayloadMessage, SelectedImage, TitleMixinMessage, BODY_TEXT_MIXIN_ID, IMAGE_MIXIN_ID,
        LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID, DEFAULT_LANGUAGE_TAG,
    },
    feed::FEED_TYPE_MIXIN_ID,
    profile::PROFILE_MIXIN_ID,
    runtime_client::connect as connect_acuity_client,
    Route,
};
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use prost::Message;
use rfd::FileDialog;
use sp_core::crypto::Ss58Codec;

const ITEM_VIEW_CSS: Asset = asset!("/assets/styling/item_view.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

fn short_hex(value: &str) -> String {
    if value.len() <= 18 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..10], &value[value.len() - 8..])
    }
}

fn content_type_label(item: &ItemMessage) -> &'static str {
    for mixin in &item.mixin_payload {
        if mixin.mixin_id == FEED_TYPE_MIXIN_ID {
            return "Feed";
        }
        if mixin.mixin_id == PROFILE_MIXIN_ID {
            return "Profile";
        }
    }
    "Content"
}

#[derive(Clone, PartialEq, Default)]
struct LoadedItem {
    encoded_item_id: String,
    item_id: [u8; 32],
    item_id_hex: String,
    revision_ipfs_hash_hex: String,
    content_type: String,
    title: String,
    body_text: String,
    language: String,
    image_preview_data_url: Option<String>,
    /// Raw image mixin payload bytes from IPFS — retained so the edit form can
    /// keep the existing image when no new image is selected.
    existing_image_payload: Option<Vec<u8>>,
    parents: Vec<ParentSummary>,
    /// SS58 address of the on-chain item owner (from Content.ItemState storage).
    owner_address: String,
}

#[derive(Clone, PartialEq)]
struct ParentSummary {
    encoded_item_id: String,
    title: String,
    content_type: String,
}

#[derive(Clone, PartialEq)]
struct FeedPost {
    encoded_item_id: String,
    title: String,
    body_preview: String,
    image_preview_data_url: Option<String>,
}

#[derive(Clone, PartialEq)]
enum ActiveTab {
    View,
    Edit,
}

#[derive(Clone, PartialEq, Default)]
struct ItemDraft {
    title: String,
    body: String,
}

async fn load_item(encoded_item_id: &str) -> Result<LoadedItem, String> {
    let item_id_bytes = bs58::decode(encoded_item_id)
        .into_vec()
        .map_err(|error| format!("Invalid item ID encoding: {error}"))?;

    if item_id_bytes.len() != 32 {
        return Err(format!(
            "Item ID must be 32 bytes, got {}.",
            item_id_bytes.len()
        ));
    }

    let item_id: [u8; 32] = item_id_bytes
        .try_into()
        .map_err(|_| "Failed to convert item ID bytes.".to_string())?;

    let item_id_hex = bytes32_to_hex(&item_id);
    let revision_ipfs_hash = fetch_latest_revision_hash(item_id_hex.clone()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_ipfs_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode item payload: {error}"))?;

    let content_type = content_type_label(&item).to_string();

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|m| m.body_text)
        .unwrap_or_default();

    let language = decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID)
        .map(|m| m.language_tag)
        .unwrap_or_default();

    let existing_image_payload = item
        .mixin_payload
        .iter()
        .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
        .map(|m| m.payload.clone());

    let image_preview_data_url = if let Some(ref payload) = existing_image_payload {
        preview_data_url_for_image_mixin(payload).await?
    } else {
        None
    };

    // Load parent summaries from the item's own PublishItem indexer event.
    let parents = load_parent_summaries(&item_id_hex).await.unwrap_or_default();

    // ── Query item owner from on-chain Content.ItemState storage ──────────
    let owner_address = fetch_item_owner(item_id).await.unwrap_or_default();

    Ok(LoadedItem {
        encoded_item_id: encoded_item_id.to_string(),
        item_id,
        item_id_hex,
        revision_ipfs_hash_hex: revision_ipfs_hash,
        content_type,
        title,
        body_text,
        language,
        image_preview_data_url,
        existing_image_payload,
        parents,
        owner_address,
    })
}

/// Queries `Content.ItemState` on-chain and returns the owner as an SS58 address.
async fn fetch_item_owner(item_id: [u8; 32]) -> Result<String, String> {
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access latest block for item state: {error}"))?;

    let storage_address = api::storage().content().item_state();
    let maybe_state = at_block
        .storage()
        .try_fetch(
            &storage_address,
            (api::runtime_types::pallet_content::pallet::ItemId(item_id),),
        )
        .await
        .map_err(|error| format!("Failed to fetch item state: {error}"))?;

    let owner_address = maybe_state
        .and_then(|encoded| encoded.decode().ok())
        .map(|state| {
            let sp_account = sp_core::crypto::AccountId32::from(state.owner.0);
            sp_account.to_ss58check()
        })
        .unwrap_or_default();

    Ok(owner_address)
}

/// Finds this item's own `Content::PublishItem` event in the indexer and
/// returns a lightweight summary for each declared parent.  Parents that
/// fail to load are silently skipped.
async fn load_parent_summaries(item_id_hex: &str) -> Result<Vec<ParentSummary>, String> {
    let decoded_events = fetch_events_for_item(item_id_hex.to_string()).await?;

    // Find the PublishItem event whose item_id matches this item (not a child).
    let mut parent_hex_ids: Vec<String> = Vec::new();
    for decoded_event in &decoded_events {
        let event = serde_json::from_value::<IndexerStoredEvent>(decoded_event.event.clone())
            .unwrap_or_else(|_| IndexerStoredEvent {
                pallet_name: String::new(),
                event_name: String::new(),
                fields: serde_json::Value::Null,
            });

        if event.pallet_name != "Content" || event.event_name != "PublishItem" {
            continue;
        }

        let event_item_id = event
            .fields
            .get("item_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        // Only process the event that belongs to *this* item, not child items.
        if event_item_id != item_id_hex {
            continue;
        }

        // Extract the parents array — the indexer stores it as a JSON array
        // of hex strings or objects with an inner value field.
        if let Some(parents_val) = event.fields.get("parents") {
            if let Some(arr) = parents_val.as_array() {
                for entry in arr {
                    // Try plain string first, then {"0": "0xabc..."} or nested.
                    let hex = if let Some(s) = entry.as_str() {
                        s.to_string()
                    } else if let Some(s) = entry.get("0").and_then(|v| v.as_str()) {
                        s.to_string()
                    } else {
                        continue;
                    };
                    if !hex.is_empty() {
                        parent_hex_ids.push(hex);
                    }
                }
            }
        }

        // There is only one PublishItem event for this item; stop after it.
        break;
    }

    let mut summaries = Vec::new();
    for parent_hex in parent_hex_ids {
        match load_parent_summary(&parent_hex).await {
            Ok(summary) => summaries.push(summary),
            Err(_) => continue,
        }
    }

    Ok(summaries)
}

async fn load_parent_summary(item_id_hex: &str) -> Result<ParentSummary, String> {
    let revision_hash = fetch_latest_revision_hash(item_id_hex.to_string()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode parent item payload: {error}"))?;

    let content_type = content_type_label(&item).to_string();
    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    // Use shortened hex as fallback display name when there is no title.
    let display_title = if title.trim().is_empty() {
        short_hex(item_id_hex)
    } else {
        title
    };

    let item_id_bytes = hex_to_bytes32(item_id_hex)?;
    let encoded_item_id = bs58::encode(item_id_bytes).into_string();

    Ok(ParentSummary {
        encoded_item_id,
        title: display_title,
        content_type,
    })
}

/// Loads child posts for a feed by querying the indexer for all events
/// keyed by the feed's item_id, then filtering for `Content::PublishItem`
/// events where this feed appears as a parent.
async fn load_feed_posts(item_id_hex: &str) -> Result<Vec<FeedPost>, String> {
    let decoded_events = fetch_events_for_item(item_id_hex.to_string()).await?;

    // Collect child item IDs from PublishItem events where this feed is a parent
    let mut child_item_ids: Vec<String> = Vec::new();
    for decoded_event in &decoded_events {
        let event = serde_json::from_value::<IndexerStoredEvent>(decoded_event.event.clone())
            .unwrap_or_else(|_| IndexerStoredEvent {
                pallet_name: String::new(),
                event_name: String::new(),
                fields: serde_json::Value::Null,
            });

        if event.pallet_name != "Content" || event.event_name != "PublishItem" {
            continue;
        }

        // The event's item_id field is the child item being published.
        // We only want events where the child's parents include our feed.
        // Since the indexer indexes each parent with multi=true, querying by
        // the feed's item_id returns PublishItem events for children that
        // declared this feed as a parent. But it also returns the feed's own
        // PublishItem event. Skip the feed's own event by checking item_id.
        let child_item_id = event
            .fields
            .get("item_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if child_item_id.is_empty() || child_item_id == item_id_hex {
            continue;
        }

        child_item_ids.push(child_item_id.to_string());
    }

    // Load each child post's content from IPFS
    let mut posts = Vec::new();
    for child_id_hex in child_item_ids {
        let post = load_single_post(&child_id_hex).await;
        match post {
            Ok(p) => posts.push(p),
            Err(_) => continue, // Skip posts that fail to load
        }
    }

    Ok(posts)
}

async fn load_single_post(item_id_hex: &str) -> Result<FeedPost, String> {
    let revision_hash = fetch_latest_revision_hash(item_id_hex.to_string()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode post payload: {error}"))?;

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|m| m.body_text)
        .unwrap_or_default();

    // Truncate body for preview
    let body_preview = if body_text.len() > 200 {
        format!("{}...", &body_text[..200])
    } else {
        body_text
    };

    let image_mixin_payload = item
        .mixin_payload
        .iter()
        .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
        .map(|m| m.payload.clone());

    let image_preview_data_url = if let Some(ref payload) = image_mixin_payload {
        preview_data_url_for_image_mixin(payload).await.unwrap_or(None)
    } else {
        None
    };

    // Convert hex item_id to base58 for the URL
    let item_id_bytes = hex_to_bytes32(item_id_hex)?;
    let encoded_item_id = bs58::encode(item_id_bytes).into_string();

    Ok(FeedPost {
        encoded_item_id,
        title,
        body_preview,
        image_preview_data_url,
    })
}

/// Builds a revised protobuf item payload, preserving the type-marker mixin
/// (Feed or Profile) from the original content type, and replacing title,
/// body, and optionally image with the draft values.
fn encode_revised_item(
    content_type: &str,
    draft: &ItemDraft,
    image_payload: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut mixins: Vec<MixinPayloadMessage> = Vec::new();

    // Preserve the type-marker mixin at the front if this is a Feed or Profile.
    if content_type == "Feed" {
        mixins.push(MixinPayloadMessage {
            mixin_id: FEED_TYPE_MIXIN_ID,
            payload: vec![],
        });
    } else if content_type == "Profile" {
        // For profile items we keep an empty profile mixin to preserve the
        // type marker; account_type / location edits are out of scope here.
        mixins.push(MixinPayloadMessage {
            mixin_id: PROFILE_MIXIN_ID,
            payload: vec![],
        });
    }

    mixins.push(MixinPayloadMessage {
        mixin_id: LANGUAGE_MIXIN_ID,
        payload: LanguageMixinMessage {
            language_tag: DEFAULT_LANGUAGE_TAG.to_string(),
        }
        .encode_to_vec(),
    });

    mixins.push(MixinPayloadMessage {
        mixin_id: TITLE_MIXIN_ID,
        payload: TitleMixinMessage {
            title: draft.title.clone(),
        }
        .encode_to_vec(),
    });

    mixins.push(MixinPayloadMessage {
        mixin_id: BODY_TEXT_MIXIN_ID,
        payload: BodyTextMixinMessage {
            body_text: draft.body.clone(),
        }
        .encode_to_vec(),
    });

    if let Some(image_payload) = image_payload {
        mixins.push(MixinPayloadMessage {
            mixin_id: IMAGE_MIXIN_ID,
            payload: image_payload,
        });
    }

    ItemMessage {
        mixin_payload: mixins,
    }
    .encode_to_vec()
}

#[component]
pub fn ItemView(encoded_item_id: String) -> Element {
    let account_store = use_context::<Signal<AccountStore>>();

    let mut loaded: Signal<Option<LoadedItem>> = use_signal(|| None);
    let mut is_loading = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);

    // Feed posts state (only used when the item is a Feed)
    let mut feed_posts: Signal<Vec<FeedPost>> = use_signal(Vec::new);
    let mut posts_loading = use_signal(|| false);
    let mut posts_error: Signal<Option<String>> = use_signal(|| None);

    // ── Edit tab state ──────────────────────────────────────────────────────
    let mut active_tab = use_signal(|| ActiveTab::View);
    let mut draft = use_signal(ItemDraft::default);
    let mut selected_image = use_signal(|| None::<SelectedImage>);
    let mut drag_over = use_signal(|| false);
    let mut is_saving = use_signal(|| false);
    let mut save_error: Signal<Option<String>> = use_signal(|| None);
    let mut save_notice: Signal<Option<String>> = use_signal(|| None);
    // Incrementing this signal re-triggers the load effect after a save.
    let mut reload_tick = use_signal(|| 0_u64);

    let encoded_id = use_memo({
        let encoded_item_id = encoded_item_id.clone();
        move || encoded_item_id.clone()
    });

    use_effect(move || {
        let id = encoded_id();
        let _tick = reload_tick();
        spawn(async move {
            error_message.set(None);
            posts_error.set(None);
            feed_posts.set(Vec::new());
            is_loading.set(true);
            match load_item(&id).await {
                Ok(item) => {
                    let is_feed = item.content_type == "Feed";
                    let item_id_hex = item.item_id_hex.clone();
                    // Pre-populate edit draft from loaded content.
                    draft.set(ItemDraft {
                        title: item.title.clone(),
                        body: item.body_text.clone(),
                    });
                    selected_image.set(None);
                    active_tab.set(ActiveTab::View);
                    loaded.set(Some(item));
                    is_loading.set(false);

                    // If this is a feed, load its child posts
                    if is_feed {
                        posts_loading.set(true);
                        match load_feed_posts(&item_id_hex).await {
                            Ok(posts) => feed_posts.set(posts),
                            Err(err) => posts_error.set(Some(err)),
                        }
                        posts_loading.set(false);
                    }
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
                                            "{short_hex(&item.item_id_hex)}"
                                        }
                                    }
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Latest revision" }
                                        code { class: "meta-code",
                                            "{short_hex(&item.revision_ipfs_hash_hex)}"
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
                            div {
                                class: if drag_over() {
                                    "drop-zone drop-zone-active"
                                } else if selected_image()
                                    .and_then(|img| img.preview_data_url.clone())
                                    .or_else(|| item.image_preview_data_url.clone())
                                    .is_some()
                                {
                                    "drop-zone drop-zone-has-image"
                                } else {
                                    "drop-zone"
                                },
                                onclick: move |_| {
                                    if is_saving() { return; }
                                    if let Some(path) = FileDialog::new()
                                        .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                                        .pick_file()
                                    {
                                        let preview = preview_data_url_for_path(&path).ok();
                                        let file_name = path
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("image")
                                            .to_string();
                                        selected_image.set(Some(SelectedImage {
                                            path: path.display().to_string(),
                                            file_name,
                                            preview_data_url: preview,
                                        }));
                                    }
                                },
                                ondragover: move |e: DragEvent| {
                                    e.prevent_default();
                                    drag_over.set(true);
                                },
                                ondragleave: move |_| drag_over.set(false),
                                ondrop: move |e: DragEvent| {
                                    e.prevent_default();
                                    drag_over.set(false);
                                    let file_list = e.files();
                                    if let Some(first) = file_list.first() {
                                        let path = first.path();
                                        let preview = preview_data_url_for_path(&path).ok();
                                        let file_name = first.name();
                                        selected_image.set(Some(SelectedImage {
                                            path: path.display().to_string(),
                                            file_name,
                                            preview_data_url: preview,
                                        }));
                                    }
                                },

                                if let Some(preview_url) = selected_image()
                                    .and_then(|img| img.preview_data_url.clone())
                                    .or_else(|| item.image_preview_data_url.clone())
                                {
                                    img {
                                        class: "drop-zone-preview",
                                        src: preview_url,
                                        alt: "Image preview",
                                    }
                                    if selected_image().is_some() {
                                        button {
                                            class: "drop-zone-clear",
                                            title: "Remove staged image",
                                            onclick: move |e| {
                                                e.stop_propagation();
                                                selected_image.set(None);
                                            },
                                            "x"
                                        }
                                    }
                                } else {
                                    div { class: "drop-zone-hint",
                                        span { class: "drop-zone-icon", "I" }
                                        span { "Drop an image here or click to choose" }
                                    }
                                }
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
                                disabled: is_saving(),
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

                                            // Get signer
                                            let signer = account_store_snap
                                                .active_account_id
                                                .as_deref()
                                                .and_then(|id| account_store_snap.unlocked_signers.get(id))
                                                .cloned();
                                            let Some(signer) = signer else {
                                                save_error.set(Some(
                                                    "Unlock the active account before saving.".to_string(),
                                                ));
                                                is_saving.set(false);
                                                return;
                                            };

                                            // Build image payload
                                            let image_payload = match selected_img {
                                                Some(ref img) => {
                                                    match build_image_mixin(img).await {
                                                        Ok(built) => Some(built.payload),
                                                        Err(err) => {
                                                            save_error.set(Some(err));
                                                            is_saving.set(false);
                                                            return;
                                                        }
                                                    }
                                                }
                                                None => existing_image_payload,
                                            };

                                            // Encode revised payload
                                            let item_payload =
                                                encode_revised_item(&content_type, &draft_snap, image_payload);

                                            // Upload to IPFS
                                            let revision_ipfs_hash =
                                                match upload_ipfs_digest(&item_payload).await {
                                                    Ok(h) => h,
                                                    Err(err) => {
                                                        save_error.set(Some(err));
                                                        is_saving.set(false);
                                                        return;
                                                    }
                                                };
                                            let revision_ipfs_hash_bytes =
                                                match hex_to_bytes32(&revision_ipfs_hash) {
                                                    Ok(b) => b,
                                                    Err(err) => {
                                                        save_error.set(Some(err));
                                                        is_saving.set(false);
                                                        return;
                                                    }
                                                };

                                            // Submit publish_revision extrinsic
                                            let client = match connect_acuity_client().await {
                                                Ok(c) => c,
                                                Err(err) => {
                                                    save_error.set(Some(err));
                                                    is_saving.set(false);
                                                    return;
                                                }
                                            };
                                            let at_block = match client.at_current_block().await {
                                                Ok(b) => b,
                                                Err(err) => {
                                                    save_error.set(Some(format!(
                                                        "Failed to access latest block: {err}"
                                                    )));
                                                    is_saving.set(false);
                                                    return;
                                                }
                                            };

                                            let publish_revision_tx = api::tx().content().publish_revision(
                                                api::runtime_types::pallet_content::pallet::ItemId(item_id),
                                                api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                                                api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                                                api::runtime_types::pallet_content::pallet::IpfsHash(
                                                    revision_ipfs_hash_bytes,
                                                ),
                                            );

                                            match at_block
                                                .tx()
                                                .sign_and_submit_then_watch_default(
                                                    &publish_revision_tx,
                                                    &signer,
                                                )
                                                .await
                                            {
                                                Err(err) => {
                                                    save_error.set(Some(format!(
                                                        "Failed to submit revision: {err}"
                                                    )));
                                                    is_saving.set(false);
                                                    return;
                                                }
                                                Ok(progress) => {
                                                    if let Err(err) =
                                                        progress.wait_for_finalized_success().await
                                                    {
                                                        save_error.set(Some(format!(
                                                            "Revision transaction failed: {err}"
                                                        )));
                                                        is_saving.set(false);
                                                        return;
                                                    }
                                                }
                                            }

                                            is_saving.set(false);
                                            save_notice.set(Some(
                                                "Revision published successfully.".to_string(),
                                            ));
                                            // Reload the item to reflect the new revision.
                                            reload_tick.with_mut(|t| *t += 1);
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
                    }
                }

            } else if !is_loading() && error_message().is_none() {
                div { class: "empty-state panel-surface",
                    p { class: "empty-state-title", "Item not found" }
                    p { class: "empty-state-body",
                        "The item could not be loaded. It may not have been indexed yet."
                    }
                    Link {
                        class: "btn-secondary",
                        to: Route::Home {},
                        "Go to Dashboard"
                    }
                }
            }
        }
    }
}
