use crate::{
    accounts::AccountStore,
    content::short_hex,
    feed::{fetch_account_content_items, LoadedFeedSummary},
    profile::load_profile_for_account,
    Route,
};
use dioxus::prelude::*;

use super::helpers::account_type_label;
use crate::views::components::EmptyState;

const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

#[component]
pub fn ProfileView() -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let account_snapshot = account_store();
    let active_account = account_snapshot.active_account().cloned();
    let is_unlocked = account_snapshot.is_active_unlocked();

    let mut profile = use_signal(|| None);
    let mut is_loading = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);

    // ── Pinned content items state ─────────────────────────────────────────
    let mut content_items: Signal<Vec<LoadedFeedSummary>> = use_signal(Vec::new);
    let mut content_loading = use_signal(|| false);
    let mut content_error: Signal<Option<String>> = use_signal(|| None);

    let active_address = use_memo(move || {
        account_store()
            .active_account()
            .map(|a| a.address.clone())
    });

    // Load profile when address changes
    use_effect(move || {
        let address = active_address();
        spawn(async move {
            error_message.set(None);
            let Some(address) = address else {
                profile.set(None);
                return;
            };
            is_loading.set(true);
            match load_profile_for_account(&address).await {
                Ok(loaded) => profile.set(Some(loaded)),
                Err(err) => {
                    profile.set(None);
                    error_message.set(Some(err));
                }
            }
            is_loading.set(false);
        });
    });

    // Load pinned content items when address changes
    use_effect(move || {
        let address = active_address();
        spawn(async move {
            content_error.set(None);
            content_items.set(Vec::new());
            let Some(address) = address else {
                return;
            };
            content_loading.set(true);
            match fetch_account_content_items(&address).await {
                Ok(items) => content_items.set(items),
                Err(err) => content_error.set(Some(err)),
            }
            content_loading.set(false);
        });
    });

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }

        div {
            class: "profile-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    p { class: "page-eyebrow", "On-chain identity" }
                    h1 { class: "page-title", "Profile" }
                }
                if active_account.is_some() {
                    Link {
                        class: "btn-primary",
                        to: Route::ProfileEdit {},
                        "Edit profile"
                    }
                }
            }

            // ── Status bar ─────────────────────────────────────────────────
            if let Some(err) = error_message() {
                div { class: "status-bar error", "{err}" }
            } else if is_loading() {
                div { class: "status-bar loading", "Loading profile from the indexer and IPFS..." }
            }

            // ── No account selected ────────────────────────────────────────
            if active_account.is_none() {
                EmptyState {
                    title: "No account selected".to_string(),
                    body: "Select or create an account on the Dashboard, then come back here to view its profile.".to_string(),
                    cta_label: Some("Go to Dashboard".to_string()),
                    cta_route: Some(Route::Home {}),
                }
            } else if is_loading() {
                // Placeholder skeleton while loading
                div { class: "profile-view-grid",
                    div { class: "panel-surface profile-view-main skeleton-block" }
                    div { class: "panel-surface profile-view-side skeleton-block" }
                }
            } else if let Some(loaded) = profile() {
                div {
                    class: "profile-view-grid",

                    // ── Left: profile card ─────────────────────────────────
                    section {
                        class: "panel-surface profile-view-main",

                        // Avatar
                        if let Some(img_url) = loaded.image_preview_data_url.clone() {
                            img {
                                class: "pv-avatar",
                                src: img_url,
                                alt: "Profile avatar",
                            }
                        } else {
                            div { class: "pv-avatar-placeholder", "No image" }
                        }

                        // Name + type pill
                        div { class: "pv-identity",
                            h2 { class: "pv-name",
                                if loaded.draft.name.trim().is_empty() {
                                    "Unnamed profile"
                                } else {
                                    "{loaded.draft.name}"
                                }
                            }
                            span { class: "pv-type-pill",
                                "{account_type_label(loaded.draft.account_type)}"
                            }
                        }

                        // Location
                        if !loaded.draft.location.trim().is_empty() {
                            p { class: "pv-location", "{loaded.draft.location}" }
                        }

                        // Bio
                        if !loaded.draft.bio.trim().is_empty() {
                            p { class: "pv-bio", "{loaded.draft.bio}" }
                        }

                        // No profile yet notice
                        if !loaded.exists {
                            p { class: "pv-notice",
                                "No profile has been published for this account yet."
                            }
                        }
                    }

                    // ── Right: account info + on-chain metadata ────────────
                    aside {
                        class: "panel-surface profile-view-side",

                        // Account info
                        if let Some(ref account) = active_account {
                            div { class: "pv-account-card",
                                p { class: "pv-section-label", "Active account" }
                                p { class: "pv-account-name", "{account.name}" }
                                p { class: "pv-account-addr", "{account.address}" }
                                span {
                                    class: if is_unlocked { "status-pill unlocked" } else { "status-pill locked" },
                                    if is_unlocked { "Unlocked" } else { "Locked" }
                                }
                            }
                        }

                        // On-chain metadata
                        div { class: "pv-meta-section",
                            p { class: "pv-section-label", "On-chain data" }
                            div { class: "metadata-list",
                                div { class: "metadata-row",
                                    span { class: "meta-label", "Profile item" }
                                    code { class: "meta-code",
                                        if let Some(ref hex) = loaded.item_id_hex {
                                            "{short_hex(hex)}"
                                        } else {
                                            "Not created yet"
                                        }
                                    }
                                }
                                div { class: "metadata-row",
                                    span { class: "meta-label", "Latest revision" }
                                    code { class: "meta-code",
                                        if let Some(ref hash) = loaded.revision_ipfs_hash_hex {
                                            "{short_hex(hash)}"
                                        } else {
                                            "Not indexed yet"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else if !is_loading() && active_account.is_some() && error_message().is_none() {
                EmptyState {
                    title: "No profile found".to_string(),
                    body: "This account does not have a published profile yet. Edit the profile to create one.".to_string(),
                    cta_label: Some("Edit profile".to_string()),
                    cta_route: Some(Route::ProfileEdit {}),
                }
            }

            // ── Pinned Content section ─────────────────────────────────────
            if active_account.is_some() {
                section { class: "pv-content-section",

                    h3 { class: "pv-content-heading", "Pinned Content" }

                    if let Some(ref err) = content_error() {
                        div { class: "status-bar error", "{err}" }
                    }

                    if content_loading() {
                        div { class: "status-bar loading", "Loading content items..." }
                        // Skeleton cards while loading
                        for _ in 0..3 {
                            div { class: "pv-content-card panel-surface skeleton-block pv-content-card-skeleton" }
                        }
                    } else if content_items().is_empty() && content_error().is_none() {
                        p { class: "pv-notice pv-content-empty",
                            "No content items have been pinned to this account yet."
                        }
                    } else {
                        for item in content_items() {
                            Link {
                                class: "pv-content-card panel-surface",
                                to: Route::ItemView {
                                    encoded_item_id: item.encoded_item_id.clone(),
                                },

                                if let Some(ref img_url) = item.image_preview_data_url {
                                    img {
                                        class: "pv-content-thumb",
                                        src: img_url.clone(),
                                        alt: "Content item thumbnail",
                                    }
                                }

                                div { class: "pv-content-text",
                                    div { class: "pv-content-meta",
                                        span { class: "pv-content-type-pill", "{item.content_type}" }
                                    }
                                    if !item.title.trim().is_empty() {
                                        h4 { class: "pv-content-title", "{item.title}" }
                                    } else {
                                        h4 { class: "pv-content-title pv-content-title-untitled", "Untitled item" }
                                    }
                                    if !item.description_preview.trim().is_empty() {
                                        p { class: "pv-content-preview", "{item.description_preview}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
