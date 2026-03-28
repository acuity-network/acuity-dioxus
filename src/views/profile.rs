use crate::{
    accounts::AccountStore,
    profile::{
        load_profile_for_account, preview_data_url_for_path, save_profile, LoadedProfile,
        ProfileDraft, SaveProfileRequest, SelectedImage,
    },
};
use dioxus::prelude::*;
use rfd::FileDialog;

const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

#[component]
pub fn Profile() -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let account_snapshot = account_store();
    let active_account = account_snapshot.active_account().cloned();
    let is_unlocked = account_snapshot.is_active_unlocked();

    let mut name = use_signal(String::new);
    let mut bio = use_signal(String::new);
    let mut location = use_signal(String::new);
    let mut account_type = use_signal(|| 0_u32);
    let mut current_item_id = use_signal(|| None::<[u8; 32]>);
    let mut current_item_id_hex = use_signal(|| None::<String>);
    let mut current_revision_hash = use_signal(|| None::<String>);
    let mut existing_image_payload = use_signal(|| None::<Vec<u8>>);
    let mut stored_image_preview = use_signal(|| None::<String>);
    let mut selected_image = use_signal(|| None::<SelectedImage>);
    let mut is_loading = use_signal(|| false);
    let mut is_saving = use_signal(|| false);
    let mut error_message = use_signal(|| None::<String>);
    let mut notice_message = use_signal(|| None::<String>);
    let mut reload_tick = use_signal(|| 0_u64);

    let active_address = use_memo(move || {
        account_store()
            .active_account()
            .map(|account| account.address.clone())
    });

    use_effect(move || {
        let address = active_address();
        let _reload_tick = reload_tick();

        spawn(async move {
            error_message.set(None);
            notice_message.set(None);

            let Some(address) = address else {
                reset_form(
                    &mut name,
                    &mut bio,
                    &mut location,
                    &mut account_type,
                    &mut current_item_id,
                    &mut current_item_id_hex,
                    &mut current_revision_hash,
                    &mut existing_image_payload,
                    &mut stored_image_preview,
                    &mut selected_image,
                );
                return;
            };

            is_loading.set(true);
            match load_profile_for_account(&address).await {
                Ok(profile) => {
                    apply_loaded_profile(
                        profile,
                        &mut name,
                        &mut bio,
                        &mut location,
                        &mut account_type,
                        &mut current_item_id,
                        &mut current_item_id_hex,
                        &mut current_revision_hash,
                        &mut existing_image_payload,
                        &mut stored_image_preview,
                        &mut selected_image,
                    );
                    if current_item_id().is_some() {
                        notice_message.set(Some(
                            "Loaded the latest indexed profile revision for the active account."
                                .to_string(),
                        ));
                    } else {
                        notice_message.set(Some(
                            "No profile exists yet. Fill out the form and save to publish one."
                                .to_string(),
                        ));
                    }
                }
                Err(error) => {
                    reset_form(
                        &mut name,
                        &mut bio,
                        &mut location,
                        &mut account_type,
                        &mut current_item_id,
                        &mut current_item_id_hex,
                        &mut current_revision_hash,
                        &mut existing_image_payload,
                        &mut stored_image_preview,
                        &mut selected_image,
                    );
                    error_message.set(Some(error));
                }
            }
            is_loading.set(false);
        });
    });

    let displayed_image_preview = selected_image()
        .and_then(|image| image.preview_data_url.clone())
        .or_else(|| stored_image_preview());

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }

        div {
            class: "profile-shell",
            section {
                class: "profile-hero panel-surface",
                div {
                    class: "hero-copy",
                    p { class: "eyebrow", "Account profile" }
                    h1 { "View and update your on-chain profile" }
                    p {
                        class: "hero-text",
                        "Profile revisions are published to IPFS, linked through the Content pallet, and resolved through the indexer so the newest revision stays discoverable."
                    }
                }
                div {
                    class: "hero-card",
                    p { class: "meta-label", "Active account" }
                    if let Some(active_account) = active_account.clone() {
                        h2 { "{active_account.name}" }
                        p { class: "meta-value", "{active_account.address}" }
                        span {
                            class: if is_unlocked { "status-pill unlocked" } else { "status-pill locked" },
                            if is_unlocked { "Unlocked for publishing" } else { "Unlock on Dashboard to publish" }
                        }
                    } else {
                        h2 { "No account selected" }
                        p { class: "meta-value", "Create or select an account on the Dashboard first." }
                    }
                }
            }

            if let Some(error_message) = error_message() {
                div {
                    class: "banner error",
                    "{error_message}"
                }
            }

            if let Some(notice_message) = notice_message() {
                div {
                    class: "banner notice",
                    "{notice_message}"
                }
            }

            if is_loading() {
                div {
                    class: "banner loading",
                    "Loading the latest profile revision from the indexer and IPFS..."
                }
            }

            if is_saving() {
                div {
                    class: "banner loading",
                    "Publishing the updated profile to IPFS and the chain..."
                }
            }

            div {
                class: "profile-grid",
                section {
                    class: "panel-surface profile-editor",
                    div {
                        class: "panel-heading",
                        div {
                            p { class: "panel-label", "Profile details" }
                            h2 { "Edit profile" }
                        }
                        button {
                            class: "ghost-action",
                            disabled: is_loading() || active_account.is_none(),
                            onclick: move |_| reload_tick += 1,
                            "Refresh"
                        }
                    }

                    label {
                        class: "field",
                        span { "Name" }
                        input {
                            value: name,
                            placeholder: "Jonathan Brown",
                            disabled: is_loading() || active_account.is_none(),
                            oninput: move |event| name.set(event.value()),
                        }
                    }

                    label {
                        class: "field",
                        span { "Account type" }
                        select {
                            value: format!("{}", account_type()),
                            disabled: is_loading() || active_account.is_none(),
                            onchange: move |event| {
                                if let Ok(value) = event.value().parse::<u32>() {
                                    account_type.set(value);
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

                    label {
                        class: "field",
                        span { "Location" }
                        input {
                            value: location,
                            placeholder: "York, England",
                            disabled: is_loading() || active_account.is_none(),
                            oninput: move |event| location.set(event.value()),
                        }
                    }

                    label {
                        class: "field",
                        span { "Bio" }
                        textarea {
                            value: bio,
                            rows: "6",
                            placeholder: "Describe the person, project, or organization behind this account.",
                            disabled: is_loading() || active_account.is_none(),
                            oninput: move |event| bio.set(event.value()),
                        }
                    }

                    div {
                        class: "field image-field",
                        span { "Image" }
                        div {
                            class: "image-actions",
                            button {
                                class: "secondary-action",
                                disabled: is_loading() || active_account.is_none(),
                                onclick: move |_| {
                                    if let Some(path) = FileDialog::new()
                                        .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                                        .pick_file()
                                    {
                                        let preview = preview_data_url_for_path(&path).ok();
                                        let file_name = path
                                            .file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("profile-image")
                                            .to_string();
                                        selected_image.set(Some(SelectedImage {
                                            path: path.display().to_string(),
                                            file_name: file_name.clone(),
                                            preview_data_url: preview,
                                        }));
                                        error_message.set(None);
                                        notice_message.set(Some(format!("Selected {file_name}. Save to publish it.")));
                                    }
                                },
                                "Choose image"
                            }

                            button {
                                class: "ghost-action",
                                disabled: selected_image().is_none(),
                                onclick: move |_| {
                                    selected_image.set(None);
                                    notice_message.set(Some("Restored the currently indexed profile image.".to_string()));
                                },
                                "Clear new image"
                            }
                        }

                        if let Some(selected_image) = selected_image() {
                            p { class: "field-note", "Pending upload: {selected_image.file_name}" }
                        } else if existing_image_payload().is_some() {
                            p { class: "field-note", "Using the currently indexed profile image." }
                        } else {
                            p { class: "field-note", "No image has been published for this profile yet." }
                        }
                    }

                    div {
                        class: "save-actions",
                        button {
                            class: "primary-action",
                            disabled: is_loading() || is_saving() || active_account.is_none() || !is_unlocked,
                            onclick: {
                                let account_store_snapshot = account_store();
                                let request = SaveProfileRequest {
                                    draft: ProfileDraft {
                                        name: name(),
                                        bio: bio(),
                                        location: location(),
                                        account_type: account_type(),
                                    },
                                    existing_item_id: current_item_id(),
                                    existing_image_payload: existing_image_payload(),
                                    selected_image: selected_image(),
                                };

                                move |_| {
                                    let account_store_snapshot = account_store_snapshot.clone();
                                    let request = request.clone();
                                    spawn(async move {
                                        is_saving.set(true);
                                        error_message.set(None);
                                        notice_message.set(None);

                                        match save_profile(&account_store_snapshot, request).await {
                                            Ok(profile) => {
                                                apply_loaded_profile(
                                                    profile,
                                                    &mut name,
                                                    &mut bio,
                                                    &mut location,
                                                    &mut account_type,
                                                    &mut current_item_id,
                                                    &mut current_item_id_hex,
                                                    &mut current_revision_hash,
                                                    &mut existing_image_payload,
                                                    &mut stored_image_preview,
                                                    &mut selected_image,
                                                );
                                                notice_message.set(Some(
                                                    "Profile saved. The chain is updated immediately; the indexer may take a moment to catch up with the new revision."
                                                        .to_string(),
                                                ));
                                            }
                                            Err(error) => error_message.set(Some(error)),
                                        }

                                        is_saving.set(false);
                                    });
                                }
                            },
                            if is_unlocked { "Save profile" } else { "Unlock the account on Dashboard to save" }
                        }
                    }
                }

                section {
                    class: "panel-surface profile-sidebar",
                    p { class: "panel-label", "Preview" }
                    h2 { "Current profile state" }

                    if let Some(image_preview) = displayed_image_preview {
                        img {
                            class: "profile-image",
                            src: image_preview,
                            alt: "Profile preview",
                        }
                    } else {
                        div {
                            class: "image-placeholder",
                            "No profile image"
                        }
                    }

                    div {
                        class: "profile-card",
                        h3 { if name().trim().is_empty() { "Unnamed profile" } else { "{name}" } }
                        p { class: "profile-type", "{account_type_label(account_type())}" }
                        if !location().trim().is_empty() {
                            p { class: "profile-location", "{location}" }
                        }
                        if !bio().trim().is_empty() {
                            p { class: "profile-bio", "{bio}" }
                        }
                    }

                    div {
                        class: "metadata-list",
                        div {
                            class: "metadata-row",
                            span { class: "meta-label", "Profile item" }
                            code {
                                class: "meta-code",
                                if let Some(item_id_hex) = current_item_id_hex() {
                                    "{short_hex(&item_id_hex)}"
                                } else {
                                    "Not created yet"
                                }
                            }
                        }
                        div {
                            class: "metadata-row",
                            span { class: "meta-label", "Latest revision" }
                            code {
                                class: "meta-code",
                                if let Some(revision_hash) = current_revision_hash() {
                                    "{short_hex(&revision_hash)}"
                                } else {
                                    "Not indexed yet"
                                }
                            }
                        }
                        div {
                            class: "metadata-row",
                            span { class: "meta-label", "Publishing" }
                            span { class: "meta-copy", if is_unlocked { "Ready" } else { "Locked" } }
                        }
                    }
                }
            }
        }
    }
}

fn apply_loaded_profile(
    profile: LoadedProfile,
    name: &mut Signal<String>,
    bio: &mut Signal<String>,
    location: &mut Signal<String>,
    account_type: &mut Signal<u32>,
    current_item_id: &mut Signal<Option<[u8; 32]>>,
    current_item_id_hex: &mut Signal<Option<String>>,
    current_revision_hash: &mut Signal<Option<String>>,
    existing_image_payload: &mut Signal<Option<Vec<u8>>>,
    stored_image_preview: &mut Signal<Option<String>>,
    selected_image: &mut Signal<Option<SelectedImage>>,
) {
    name.set(profile.draft.name);
    bio.set(profile.draft.bio);
    location.set(profile.draft.location);
    account_type.set(profile.draft.account_type);
    current_item_id.set(profile.item_id);
    current_item_id_hex.set(profile.item_id_hex);
    current_revision_hash.set(profile.revision_ipfs_hash_hex);
    existing_image_payload.set(profile.existing_image_payload);
    stored_image_preview.set(profile.image_preview_data_url);
    selected_image.set(None);
}

fn reset_form(
    name: &mut Signal<String>,
    bio: &mut Signal<String>,
    location: &mut Signal<String>,
    account_type: &mut Signal<u32>,
    current_item_id: &mut Signal<Option<[u8; 32]>>,
    current_item_id_hex: &mut Signal<Option<String>>,
    current_revision_hash: &mut Signal<Option<String>>,
    existing_image_payload: &mut Signal<Option<Vec<u8>>>,
    stored_image_preview: &mut Signal<Option<String>>,
    selected_image: &mut Signal<Option<SelectedImage>>,
) {
    name.set(String::new());
    bio.set(String::new());
    location.set(String::new());
    account_type.set(0);
    current_item_id.set(None);
    current_item_id_hex.set(None);
    current_revision_hash.set(None);
    existing_image_payload.set(None);
    stored_image_preview.set(None);
    selected_image.set(None);
}

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
