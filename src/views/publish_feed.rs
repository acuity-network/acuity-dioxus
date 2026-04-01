use crate::{
    accounts::AccountStore,
    content::{preview_data_url_for_path, SelectedImage},
    feed::{publish_feed, FeedDraft, PublishFeedRequest},
    Route,
};
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use rfd::FileDialog;

const PUBLISH_FEED_CSS: Asset = asset!("/assets/styling/publish_feed.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

#[component]
pub fn PublishFeed() -> Element {
    let navigator = use_navigator();
    let account_store = use_context::<Signal<AccountStore>>();
    let account_snapshot = account_store();
    let active_account = account_snapshot.active_account().cloned();
    let is_unlocked = account_snapshot.is_active_unlocked();

    let mut draft = use_signal(FeedDraft::default);
    let mut selected_image = use_signal(|| None::<SelectedImage>);

    let mut is_saving = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);
    let mut notice_message: Signal<Option<String>> = use_signal(|| None);

    let mut drag_over = use_signal(|| false);

    let has_active_account = active_account.is_some();
    let title_empty = draft().title.trim().is_empty();

    let displayed_image_preview = selected_image().and_then(|img| img.preview_data_url.clone());

    // Single smart status bar: error > saving > notice
    let status: Option<(&'static str, String)> = if let Some(ref err) = error_message() {
        Some(("error", err.clone()))
    } else if is_saving() {
        Some((
            "loading",
            "Publishing the feed to IPFS and the chain...".to_string(),
        ))
    } else {
        notice_message().map(|n| ("notice", n))
    };

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }
        document::Link { rel: "stylesheet", href: PUBLISH_FEED_CSS }

        div {
            class: "publish-feed-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    p { class: "page-eyebrow", "Content" }
                    h1 { class: "page-title", "Publish feed" }
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
                        "Select or create an account on the Dashboard, then come back here to publish a feed."
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
                    class: "panel-surface publish-feed-editor",

                    // Title
                    label {
                        class: "field",
                        span { "Title" }
                        input {
                            value: draft().title,
                            placeholder: "Feed title",
                            disabled: is_saving() || !has_active_account,
                            oninput: move |e| draft.with_mut(|d| d.title = e.value()),
                        }
                    }

                    // Description
                    label {
                        class: "field",
                        span { "Description" }
                        textarea {
                            value: draft().description,
                            rows: "6",
                            placeholder: "Describe what this feed is about.",
                            disabled: is_saving() || !has_active_account,
                            oninput: move |e| draft.with_mut(|d| d.description = e.value()),
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
                                if is_saving() || !has_active_account { return; }
                                if let Some(path) = FileDialog::new()
                                    .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                                    .pick_file()
                                {
                                    let preview = preview_data_url_for_path(&path).ok();
                                    let file_name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("feed-image")
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
                                    alt: "Feed image preview",
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
                            disabled: is_saving() || !has_active_account || !is_unlocked || title_empty,
                            onclick: {
                                let store_snap = account_store();
                                let req = PublishFeedRequest {
                                    draft: draft(),
                                    selected_image: selected_image(),
                                };
                                move |_| {
                                    let store_snap = store_snap.clone();
                                    let req = req.clone();
                                    spawn(async move {
                                        is_saving.set(true);
                                        error_message.set(None);
                                        notice_message.set(None);
                                        match publish_feed(&store_snap, req).await {
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
                            if is_saving() { "Publishing..." } else { "Publish feed" }
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
