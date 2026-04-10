use acuity_index_api_rs::IndexerClient;
use crate::{
    accounts::AccountStore,
    acuity_runtime::api,
    content::{short_hex, SelectedImage},
    profile::{load_profile_for_account, save_profile, ProfileDraft, SaveProfileRequest},
    runtime_client::estimate_fee,
    ChainConnection,
    Route,
};
use dioxus::prelude::*;

use super::helpers::{account_type_label, apply_loaded_profile};
use crate::views::components::{ImageDropZone, InsufficientFundsHint};

const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

#[component]
pub fn ProfileEdit() -> Element {
    let navigator = use_navigator();
    let account_store = use_context::<Signal<AccountStore>>();
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let indexer_client = use_context::<Signal<Option<IndexerClient>>>();
    let account_snapshot = account_store();
    let active_account = account_snapshot.active_account().cloned();
    let is_unlocked = account_snapshot.is_active_unlocked();

    // All text fields as one signal
    let mut draft = use_signal(ProfileDraft::default);

    // Metadata signals (not part of the editable draft)
    let mut current_item_id = use_signal(|| None::<[u8; 32]>);
    let mut current_item_id_hex = use_signal(|| None::<String>);
    let mut current_revision_hash = use_signal(|| None::<String>);
    let mut existing_image_payload = use_signal(|| None::<Vec<u8>>);
    let mut stored_image_preview = use_signal(|| None::<String>);
    let mut selected_image = use_signal(|| None::<SelectedImage>);

    // UI state
    let mut is_loading = use_signal(|| false);
    let mut is_saving = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);
    let mut notice_message: Signal<Option<String>> = use_signal(|| None);
    let mut reload_tick = use_signal(|| 0_u64);

    let has_active_account = active_account.is_some();

    let active_address = use_memo(move || {
        account_store()
            .active_account()
            .map(|a| a.address.clone())
    });

    // Fee estimation — re-runs when the signer or the profile state changes
    // (first-time creation needs a batch call; updates use publish_revision).
    let fee_estimate = use_resource(move || async move {
        let signer = account_store().active_signer().cloned()?;
        let is_first_save = current_item_id().is_none();
        if is_first_save {
            // Estimate batch_all([publish_item, set_profile]) — use zero-filled
            // dummy data; the fee depends on call structure, not payload content.
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
            let set_profile_call = api::Call::AccountProfile(
                api::runtime_types::pallet_account_profile::pallet::Call::set_profile {
                    item_id: api::runtime_types::pallet_content::pallet::ItemId(dummy_item_id),
                },
            );
            let batch_call = api::tx().utility().batch_all(vec![publish_call, set_profile_call]);
            estimate_fee(&batch_call, &signer).await.ok()
        } else {
            // Estimate publish_revision with dummy item/hash.
            let dummy_item_id = current_item_id().unwrap_or([0u8; 32]);
            let dummy_ipfs_hash = [0u8; 32];
            let revision_call = api::tx().content().publish_revision(
                api::runtime_types::pallet_content::pallet::ItemId(dummy_item_id),
                api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                api::runtime_types::pallet_content::pallet::IpfsHash(dummy_ipfs_hash),
            );
            estimate_fee(&revision_call, &signer).await.ok()
        }
    });

    let insufficient_funds = use_memo(move || {
        let balance = chain_connection().details.active_account_balance;
        let fee = fee_estimate().flatten();
        match (balance, fee) {
            (Some(b), Some(f)) => b < f,
            _ => true, // block until both balance and fee are known
        }
    });

    use_effect(move || {
        let address = active_address();
        let _tick = reload_tick();
        let client = indexer_client().clone();

        spawn(async move {
            error_message.set(None);
            notice_message.set(None);

            let Some(address) = address else {
                draft.set(ProfileDraft::default());
                current_item_id.set(None);
                current_item_id_hex.set(None);
                current_revision_hash.set(None);
                existing_image_payload.set(None);
                stored_image_preview.set(None);
                selected_image.set(None);
                return;
            };

            let Some(client) = client else {
                return;
            };

            is_loading.set(true);
            match load_profile_for_account(&client, &address).await {
                Ok(profile) => {
                    apply_loaded_profile(
                        profile,
                        &mut draft,
                        &mut current_item_id,
                        &mut current_item_id_hex,
                        &mut current_revision_hash,
                        &mut existing_image_payload,
                        &mut stored_image_preview,
                        &mut selected_image,
                    );
                    if current_item_id().is_some() {
                        notice_message.set(Some(
                            "Loaded the latest indexed revision.".to_string(),
                        ));
                    }
                }
                Err(err) => error_message.set(Some(err)),
            }
            is_loading.set(false);
        });
    });

    let displayed_image_preview = selected_image()
        .and_then(|img| img.preview_data_url.clone())
        .or_else(|| stored_image_preview());

    // Single smart status bar: error > saving > loading > notice
    let status: Option<(&'static str, String)> = if let Some(ref err) = error_message() {
        Some(("error", err.clone()))
    } else if is_saving() {
        Some(("loading", "Publishing the updated profile to IPFS and the chain...".to_string()))
    } else if is_loading() {
        Some(("loading", "Loading the latest profile revision...".to_string()))
    } else {
        notice_message().map(|n| ("notice", n))
    };

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }

        div {
            class: "profile-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    Link {
                        class: "back-link",
                        to: Route::ProfileView {},
                        "← Profile"
                    }
                    h1 { class: "page-title", "Edit profile" }
                }
                button {
                    class: "btn-ghost",
                    disabled: is_loading() || !has_active_account,
                    onclick: move |_| reload_tick += 1,
                    "Refresh"
                }
            }

            // ── Status bar ─────────────────────────────────────────────────
            if let Some((kind, message)) = status {
                div { class: "status-bar {kind}", "{message}" }
            }

            // ── Edit form ──────────────────────────────────────────────────
            section {
                class: "panel-surface profile-editor",

                // Name
                label {
                    class: "field",
                    span { "Name" }
                    input {
                        value: draft().name,
                        placeholder: "Jonathan Brown",
                        disabled: is_loading() || !has_active_account,
                        oninput: move |e| draft.with_mut(|d| d.name = e.value()),
                    }
                }

                // Account type
                label {
                    class: "field",
                    span { "Account type" }
                    select {
                        value: format!("{}", draft().account_type),
                        disabled: is_loading() || !has_active_account,
                        onchange: move |e| {
                            if let Ok(v) = e.value().parse::<u32>() {
                                draft.with_mut(|d| d.account_type = v);
                            }
                        },
                        option { value: "0", selected: draft().account_type == 0, "{account_type_label(0)}" }
                        option { value: "1", selected: draft().account_type == 1, "{account_type_label(1)}" }
                        option { value: "2", selected: draft().account_type == 2, "{account_type_label(2)}" }
                        option { value: "3", selected: draft().account_type == 3, "{account_type_label(3)}" }
                        option { value: "4", selected: draft().account_type == 4, "{account_type_label(4)}" }
                        option { value: "5", selected: draft().account_type == 5, "{account_type_label(5)}" }
                        option { value: "6", selected: draft().account_type == 6, "{account_type_label(6)}" }
                        option { value: "7", selected: draft().account_type == 7, "{account_type_label(7)}" }
                        option { value: "8", selected: draft().account_type == 8, "{account_type_label(8)}" }
                    }
                }

                // Location
                label {
                    class: "field",
                    span { "Location" }
                    input {
                        value: draft().location,
                        placeholder: "York, England",
                        disabled: is_loading() || !has_active_account,
                        oninput: move |e| draft.with_mut(|d| d.location = e.value()),
                    }
                }

                // Bio
                label {
                    class: "field",
                    span { "Bio" }
                    textarea {
                        value: draft().bio,
                        rows: "6",
                        placeholder: "Describe the person, project, or organization behind this account.",
                        disabled: is_loading() || !has_active_account,
                        oninput: move |e| draft.with_mut(|d| d.bio = e.value()),
                    }
                }

                // Image — drag-and-drop zone
                div { class: "field",
                    span { "Image" }
                    ImageDropZone {
                        selected_image,
                        existing_preview_url: displayed_image_preview.clone(),
                        disabled: is_loading() || !has_active_account,
                    }

                    // Pending-upload note
                    if let Some(ref img) = selected_image() {
                        p { class: "field-note", "Pending: {img.file_name}" }
                    } else if existing_image_payload().is_some() {
                        p { class: "field-note", "Using the currently published image." }
                    } else {
                        p { class: "field-note field-note-muted", "No image published yet." }
                    }
                }

                // ── Save / Cancel actions ──────────────────────────────
                div { class: "form-actions",
                    button {
                        class: "btn-primary",
                        disabled: is_loading() || is_saving() || !has_active_account || !is_unlocked || insufficient_funds(),
                        onclick: {
                            let store_snap = account_store();
                            let req = SaveProfileRequest {
                                draft: draft(),
                                existing_item_id: current_item_id(),
                                existing_image_payload: existing_image_payload(),
                                selected_image: selected_image(),
                            };
                            move |_| {
                                let store_snap = store_snap.clone();
                                let req = req.clone();
                                spawn(async move {
                                    is_saving.set(true);
                                    error_message.set(None);
                                    notice_message.set(None);
                                    match save_profile(&store_snap, req).await {
                                        Ok(profile) => {
                                            apply_loaded_profile(
                                                profile,
                                                &mut draft,
                                                &mut current_item_id,
                                                &mut current_item_id_hex,
                                                &mut current_revision_hash,
                                                &mut existing_image_payload,
                                                &mut stored_image_preview,
                                                &mut selected_image,
                                            );
                                            notice_message.set(Some(
                                                "Saved. The chain updates immediately; the indexer may take a moment to catch up."
                                                    .to_string(),
                                            ));
                                            navigator.push(Route::ProfileView {});
                                        }
                                        Err(err) => error_message.set(Some(err)),
                                    }
                                    is_saving.set(false);
                                });
                            }
                        },
                        if is_saving() { "Saving..." } else { "Save profile" }
                    }
                    Link {
                        class: "btn-ghost",
                        to: Route::ProfileView {},
                        "Cancel"
                    }
                }

                // Locked hint below the save button
                if has_active_account && !is_unlocked {
                    p { class: "save-locked-hint",
                        "Unlock the account from the sidebar to save."
                    }
                }
                if has_active_account && is_unlocked {
                    InsufficientFundsHint {
                        balance: chain_connection().details.active_account_balance,
                        fee: fee_estimate().flatten(),
                        fee_state: fee_estimate.state()(),
                    }
                }

                // ── On-chain metadata ──────────────────────────────────
                div { class: "metadata-list editor-metadata",
                    div { class: "metadata-row",
                        span { class: "meta-label", "Profile item" }
                        code { class: "meta-code",
                            if let Some(ref hex) = current_item_id_hex() {
                                "{short_hex(hex)}"
                            } else {
                                "Not created yet"
                            }
                        }
                    }
                    div { class: "metadata-row",
                        span { class: "meta-label", "Latest revision" }
                        code { class: "meta-code",
                            if let Some(ref hash) = current_revision_hash() {
                                "{short_hex(hash)}"
                            } else {
                                "Not indexed yet"
                            }
                        }
                    }
                    div { class: "metadata-row",
                        span { class: "meta-label", "Publishing" }
                        span { class: "meta-copy",
                            if is_unlocked { "Ready" } else { "Locked" }
                        }
                    }
                }
            }
        }
    }
}
