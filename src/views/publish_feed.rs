use crate::{
    accounts::AccountStore,
    content::SelectedImage,
    feed::{publish_feed, FeedDraft, PublishFeedRequest},
    Route,
};
use dioxus::prelude::*;

use super::components::{EmptyState, ImageDropZone};

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
    let selected_image = use_signal(|| None::<SelectedImage>);

    let mut is_saving = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);
    let mut notice_message: Signal<Option<String>> = use_signal(|| None);

    let has_active_account = active_account.is_some();
    let title_empty = draft().title.trim().is_empty();

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
                EmptyState {
                    title: "No account selected".to_string(),
                    body: "Select or create an account on the Dashboard, then come back here to publish a feed.".to_string(),
                    cta_label: Some("Go to Dashboard".to_string()),
                    cta_route: Some(Route::Home {}),
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

                    // Image
                    div { class: "field",
                        span { "Image (optional)" }
                        ImageDropZone {
                            selected_image,
                            existing_preview_url: None,
                            disabled: is_saving() || !has_active_account,
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
