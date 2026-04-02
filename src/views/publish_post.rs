use crate::{
    accounts::AccountStore,
    content::{
        bytes32_to_hex, decode_single_mixin, fetch_ipfs_digest_bytes, fetch_latest_revision_hash,
        preview_data_url_for_path, ItemMessage, SelectedImage, TitleMixinMessage, TITLE_MIXIN_ID,
    },
    post::{publish_post, PostDraft, PublishPostRequest},
    Route,
};
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use prost::Message;
use rfd::FileDialog;

const PUBLISH_POST_CSS: Asset = asset!("/assets/styling/publish_post.css");
const PUBLISH_FEED_CSS: Asset = asset!("/assets/styling/publish_feed.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

/// Decodes the base58-encoded feed ID from the route, loads the feed title,
/// and returns `(feed_item_id_bytes, feed_title)`.
async fn load_feed_context(encoded_feed_id: &str) -> Result<([u8; 32], String), String> {
    let feed_id_bytes: [u8; 32] = bs58::decode(encoded_feed_id)
        .into_vec()
        .map_err(|error| format!("Invalid feed ID encoding: {error}"))?
        .try_into()
        .map_err(|_| "Feed ID must be 32 bytes.".to_string())?;

    let item_id_hex = bytes32_to_hex(&feed_id_bytes);
    let revision_hash = fetch_latest_revision_hash(item_id_hex).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode feed payload: {error}"))?;

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_else(|| "Untitled feed".to_string());

    Ok((feed_id_bytes, title))
}

#[component]
pub fn PublishPost(encoded_feed_id: String) -> Element {
    let navigator = use_navigator();
    let account_store = use_context::<Signal<AccountStore>>();
    let account_snapshot = account_store();
    let active_account = account_snapshot.active_account().cloned();
    let is_unlocked = account_snapshot.is_active_unlocked();

    let mut draft = use_signal(PostDraft::default);
    let mut selected_image = use_signal(|| None::<SelectedImage>);

    let mut is_saving = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);
    let mut notice_message: Signal<Option<String>> = use_signal(|| None);

    let mut drag_over = use_signal(|| false);

    // Load feed context (title + decoded item ID) once
    let mut feed_item_id: Signal<Option<[u8; 32]>> = use_signal(|| None);
    let mut feed_title: Signal<Option<String>> = use_signal(|| None);
    let mut feed_loading = use_signal(|| true);

    let encoded_id = use_memo({
        let encoded_feed_id = encoded_feed_id.clone();
        move || encoded_feed_id.clone()
    });

    use_effect(move || {
        let id = encoded_id();
        spawn(async move {
            feed_loading.set(true);
            match load_feed_context(&id).await {
                Ok((item_id, title)) => {
                    feed_item_id.set(Some(item_id));
                    feed_title.set(Some(title));
                }
                Err(err) => {
                    error_message.set(Some(format!("Failed to load feed: {err}")));
                }
            }
            feed_loading.set(false);
        });
    });

    let has_active_account = active_account.is_some();
    let title_empty = draft().title.trim().is_empty();
    let feed_ready = feed_item_id().is_some();

    let displayed_image_preview = selected_image().and_then(|img| img.preview_data_url.clone());

    // Single smart status bar: error > saving > notice
    let status: Option<(&'static str, String)> = if let Some(ref err) = error_message() {
        Some(("error", err.clone()))
    } else if is_saving() {
        Some((
            "loading",
            "Publishing the post to IPFS and the chain...".to_string(),
        ))
    } else if feed_loading() {
        Some(("loading", "Loading feed details...".to_string()))
    } else {
        notice_message().map(|n| ("notice", n))
    };

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }
        document::Link { rel: "stylesheet", href: PUBLISH_FEED_CSS }
        document::Link { rel: "stylesheet", href: PUBLISH_POST_CSS }

        div {
            class: "publish-post-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    p { class: "page-eyebrow", "Content" }
                    h1 { class: "page-title", "Publish post" }
                }
            }

            // ── Status bar ─────────────────────────────────────────────────
            if let Some((kind, message)) = status {
                div { class: "status-bar {kind}", "{message}" }
            }

            // ── No account selected ────────────────────────────────────────
            if active_account.is_none() {
                div {
                    class: "empty-state panel-surface",
                    p { class: "empty-state-title", "No account selected" }
                    p { class: "empty-state-body",
                        "Select or create an account on the Dashboard, then come back here to publish a post."
                    }
                    Link {
                        class: "btn-secondary",
                        to: Route::Home {},
                        "Go to Dashboard"
                    }
                }
            } else {
                // ── Publish form ───────────────────────────────────────────
                section {
                    class: "panel-surface publish-post-editor",

                    // Feed context
                    if let Some(title) = feed_title() {
                        div {
                            class: "publish-post-feed-context",
                            span { class: "publish-post-feed-label", "Feed:" }
                            span { class: "publish-post-feed-title", "{title}" }
                        }
                    }

                    // Title
                    label {
                        class: "field",
                        span { "Title" }
                        input {
                            value: draft().title,
                            placeholder: "Post title",
                            disabled: is_saving() || !has_active_account || !feed_ready,
                            oninput: move |e| draft.with_mut(|d| d.title = e.value()),
                        }
                    }

                    // Body
                    label {
                        class: "field",
                        span { "Body" }
                        textarea {
                            value: draft().body,
                            rows: "10",
                            placeholder: "Write your post content here.",
                            disabled: is_saving() || !has_active_account || !feed_ready,
                            oninput: move |e| draft.with_mut(|d| d.body = e.value()),
                        }
                    }

                    // Image -- drag-and-drop zone
                    div { class: "field",
                        span { "Image (optional)" }
                        div {
                            class: if drag_over() {
                                "drop-zone drop-zone-active"
                            } else if displayed_image_preview.is_some() {
                                "drop-zone drop-zone-has-image"
                            } else {
                                "drop-zone"
                            },
                            onclick: move |_| {
                                if is_saving() || !has_active_account || !feed_ready { return; }
                                if let Some(path) = FileDialog::new()
                                    .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                                    .pick_file()
                                {
                                    let preview = preview_data_url_for_path(&path).ok();
                                    let file_name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("post-image")
                                        .to_string();
                                    selected_image.set(Some(SelectedImage {
                                        path: path.display().to_string(),
                                        file_name: file_name.clone(),
                                        preview_data_url: preview,
                                    }));
                                    notice_message.set(Some(format!("Selected {file_name}.")));
                                    error_message.set(None);
                                }
                            },
                            ondragover: move |e| {
                                e.prevent_default();
                                drag_over.set(true);
                            },
                            ondragleave: move |_| drag_over.set(false),
                            ondrop: move |e| {
                                e.prevent_default();
                                drag_over.set(false);
                                let file_list = e.files();
                                if let Some(first) = file_list.first() {
                                    let path = first.path();
                                    let preview = preview_data_url_for_path(&path).ok();
                                    let file_name = first.name();
                                    selected_image.set(Some(SelectedImage {
                                        path: path.display().to_string(),
                                        file_name: file_name.clone(),
                                        preview_data_url: preview,
                                    }));
                                    notice_message.set(Some(format!("Selected {file_name}.")));
                                    error_message.set(None);
                                }
                            },

                            if let Some(ref img_url) = displayed_image_preview {
                                img {
                                    class: "drop-zone-preview",
                                    src: img_url.clone(),
                                    alt: "Post image preview",
                                }
                                button {
                                    class: "drop-zone-clear",
                                    title: "Remove image",
                                    onclick: move |e| {
                                        e.stop_propagation();
                                        selected_image.set(None);
                                        notice_message.set(None);
                                    },
                                    "x"
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
                        }
                    }

                    // ── Publish / Cancel actions ───────────────────────────
                    div { class: "form-actions",
                        button {
                            class: "btn-primary",
                            disabled: is_saving() || !has_active_account || !is_unlocked || title_empty || !feed_ready,
                            onclick: {
                                let store_snap = account_store();
                                let current_draft = draft();
                                let current_image = selected_image();
                                let current_feed_id = feed_item_id();
                                move |_| {
                                    let store_snap = store_snap.clone();
                                    let current_draft = current_draft.clone();
                                    let current_image = current_image.clone();
                                    let Some(fid) = current_feed_id else { return; };
                                    spawn(async move {
                                        is_saving.set(true);
                                        error_message.set(None);
                                        notice_message.set(None);
                                        let req = PublishPostRequest {
                                            draft: current_draft,
                                            feed_item_id: fid,
                                            selected_image: current_image,
                                        };
                                        match publish_post(&store_snap, req).await {
                                            Ok(published) => {
                                                let encoded = bs58::encode(&published.item_id).into_string();
                                                navigator.push(Route::ItemView {
                                                    encoded_item_id: encoded,
                                                });
                                            }
                                            Err(err) => error_message.set(Some(err)),
                                        }
                                        is_saving.set(false);
                                    });
                                }
                            },
                            if is_saving() { "Publishing..." } else { "Publish post" }
                        }
                        Link {
                            class: "btn-ghost",
                            to: Route::Home {},
                            "Cancel"
                        }
                    }

                    // Locked hint
                    if has_active_account && !is_unlocked {
                        p { class: "save-locked-hint",
                            "Unlock the account from the sidebar to publish."
                        }
                    }
                }
            }
        }
    }
}
