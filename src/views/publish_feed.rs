use crate::{
    accounts::AccountStore,
    acuity_runtime::api,
    content::SelectedImage,
    feed::{publish_feed, FeedDraft, PublishFeedRequest},
    runtime_client::estimate_fee,
    ChainConnection, Route,
};
use dioxus::prelude::*;

use super::components::{EmptyState, ImageDropZone, InsufficientFundsHint};

const PUBLISH_FEED_CSS: Asset = asset!("/assets/styling/publish_feed.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

#[component]
pub fn PublishFeed() -> Element {
    let navigator = use_navigator();
    let account_store = use_context::<Signal<AccountStore>>();
    let chain_connection = use_context::<Signal<ChainConnection>>();
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

    // Fee estimation: batch_all([publish_item, add_item]) with dummy data.
    let fee_estimate = use_resource(move || async move {
        let signer = account_store().active_signer().cloned()?;
        let dummy_nonce = [0u8; 32];
        let dummy_item_id = [0u8; 32];
        let dummy_ipfs_hash = [0u8; 32];
        let publish_call = api::Call::Content(
            api::runtime_types::pallet_content::pallet::Call::publish_item {
                nonce: api::runtime_types::pallet_content::Nonce(dummy_nonce),
                parents: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                flags: 0x01,
                links: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                mentions: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                ipfs_hash: api::runtime_types::pallet_content::pallet::IpfsHash(dummy_ipfs_hash),
            },
        );
        let add_item_call = api::Call::AccountContent(
            api::runtime_types::pallet_account_content::pallet::Call::add_item {
                item_id: api::runtime_types::pallet_content::pallet::ItemId(dummy_item_id),
            },
        );
        let batch_call = api::tx().utility().batch_all(vec![publish_call, add_item_call]);
        estimate_fee(&batch_call, &signer).await.ok()
    });

    let insufficient_funds = use_memo(move || {
        let balance = chain_connection().details.active_account_balance;
        let fee = fee_estimate().flatten();
        matches!((balance, fee), (Some(b), Some(f)) if b < f)
    });

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
                            disabled: is_saving() || !has_active_account || !is_unlocked || title_empty || insufficient_funds(),
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
                    if has_active_account && is_unlocked {
                        InsufficientFundsHint {
                            balance: chain_connection().details.active_account_balance,
                            fee: fee_estimate().flatten(),
                        }
                    }
                }
            }
        }
    }
}
