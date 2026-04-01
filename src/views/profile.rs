use crate::{
    accounts::AccountStore,
    content::{preview_data_url_for_path, SelectedImage},
    profile::{
        load_profile_for_account, save_profile, LoadedProfile, ProfileDraft, SaveProfileRequest,
    },
    Route,
};
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use rfd::FileDialog;

const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

// ── Shared helpers ─────────────────────────────────────────────────────────────

fn account_type_label(account_type: u32) -> &'static str {
    match account_type {
        1 => "Person",
        2 => "Project",
        3 => "Organization",
        4 => "Proxy",
        5 => "Parody",
        6 => "Bot",
        7 => "Shill",
        8 => "Test",
        _ => "Anon",
    }
}

fn short_hex(value: &str) -> String {
    if value.len() <= 18 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..10], &value[value.len() - 8..])
    }
}

// ── ProfileView (read-only) ────────────────────────────────────────────────────

#[component]
pub fn ProfileView() -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let account_snapshot = account_store();
    let active_account = account_snapshot.active_account().cloned();
    let is_unlocked = account_snapshot.is_active_unlocked();

    let mut profile: Signal<Option<LoadedProfile>> = use_signal(|| None);
    let mut is_loading = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);

    let active_address = use_memo(move || {
        account_store()
            .active_account()
            .map(|a| a.address.clone())
    });

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
                div {
                    class: "empty-state panel-surface",
                    p { class: "empty-state-title", "No account selected" }
                    p { class: "empty-state-body",
                        "Select or create an account on the Dashboard, then come back here to view its profile."
                    }
                    Link {
                        class: "btn-secondary",
                        to: Route::Home {},
                        "Go to Dashboard"
                    }
                }
            } else if is_loading() {
                // placeholder skeleton while loading
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
                div { class: "empty-state panel-surface",
                    p { class: "empty-state-title", "No profile found" }
                    p { class: "empty-state-body",
                        "This account does not have a published profile yet. Edit the profile to create one."
                    }
                    Link {
                        class: "btn-primary",
                        to: Route::ProfileEdit {},
                        "Edit profile"
                    }
                }
            }
        }
    }
}

// ── ProfileEdit (edit form) ────────────────────────────────────────────────────

#[component]
pub fn ProfileEdit() -> Element {
    let navigator = use_navigator();
    let account_store = use_context::<Signal<AccountStore>>();
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

    // Track whether we're dragging a file over the drop zone
    let mut drag_over = use_signal(|| false);

    let has_active_account = active_account.is_some();

    let active_address = use_memo(move || {
        account_store()
            .active_account()
            .map(|a| a.address.clone())
    });

    use_effect(move || {
        let address = active_address();
        let _tick = reload_tick();

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

            is_loading.set(true);
            match load_profile_for_account(&address).await {
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
                            option { value: "0", "Anon" }
                            option { value: "1", "Person" }
                            option { value: "2", "Project" }
                            option { value: "3", "Organization" }
                            option { value: "4", "Proxy" }
                            option { value: "5", "Parody" }
                            option { value: "6", "Bot" }
                            option { value: "7", "Shill" }
                            option { value: "8", "Test" }
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
                        div {
                            class: if drag_over() {
                                "drop-zone drop-zone-active"
                            } else if displayed_image_preview.is_some() {
                                "drop-zone drop-zone-has-image"
                            } else {
                                "drop-zone"
                            },
                            // Click to open file dialog
                            onclick: move |_| {
                                if is_loading() || !has_active_account { return; }
                                if let Some(path) = FileDialog::new()
                                    .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                                    .pick_file()
                                {
                                    let preview = preview_data_url_for_path(&path).ok();
                                    let file_name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("profile-image")
                                        .to_string();
                                    selected_image.set(Some(SelectedImage {
                                        path: path.display().to_string(),
                                        file_name: file_name.clone(),
                                        preview_data_url: preview,
                                    }));
                                    notice_message.set(Some(format!("Selected {file_name}. Save to publish it.")));
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
                                // DragData implements HasFileData; files() returns Vec<FileData>
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
                                    notice_message.set(Some(format!("Selected {file_name}. Save to publish it.")));
                                    error_message.set(None);
                                }
                            },

                            // Image preview or placeholder content inside the zone
                            if let Some(ref img_url) = displayed_image_preview {
                                img {
                                    class: "drop-zone-preview",
                                    src: img_url.clone(),
                                    alt: "Profile image preview",
                                }
                                // × clear button (only shown when a new image is staged)
                                if selected_image().is_some() {
                                    button {
                                        class: "drop-zone-clear",
                                        title: "Remove staged image",
                                        // stop click bubbling to the zone (which would re-open the dialog)
                                        onclick: move |e| {
                                            e.stop_propagation();
                                            selected_image.set(None);
                                            notice_message.set(Some("Restored the currently indexed profile image.".to_string()));
                                        },
                                        "×"
                                    }
                                }
                            } else {
                                div { class: "drop-zone-hint",
                                    span { class: "drop-zone-icon", "📷" }
                                    span { "Drop an image here or click to choose" }
                                }
                            }
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
                            disabled: is_loading() || is_saving() || !has_active_account || !is_unlocked,
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

// ── Private helpers ─────────────────────────────────────────────────────────────

fn apply_loaded_profile(
    profile: LoadedProfile,
    draft: &mut Signal<ProfileDraft>,
    current_item_id: &mut Signal<Option<[u8; 32]>>,
    current_item_id_hex: &mut Signal<Option<String>>,
    current_revision_hash: &mut Signal<Option<String>>,
    existing_image_payload: &mut Signal<Option<Vec<u8>>>,
    stored_image_preview: &mut Signal<Option<String>>,
    selected_image: &mut Signal<Option<SelectedImage>>,
) {
    draft.set(profile.draft);
    current_item_id.set(profile.item_id);
    current_item_id_hex.set(profile.item_id_hex);
    current_revision_hash.set(profile.revision_ipfs_hash_hex);
    existing_image_payload.set(profile.existing_image_payload);
    stored_image_preview.set(profile.image_preview_data_url);
    selected_image.set(None);
}
