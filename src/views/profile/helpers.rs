use crate::content::SelectedImage;
use crate::profile::{LoadedProfile, ProfileDraft};
use dioxus::prelude::{Signal, WritableExt};

pub fn account_type_label(account_type: u32) -> &'static str {
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

/// Applies the fields of a freshly-loaded [`LoadedProfile`] back into the
/// edit form's signals.  Used both on initial load and after a successful save.
pub fn apply_loaded_profile(
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
