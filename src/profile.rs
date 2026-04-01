use crate::{acuity_runtime::api, runtime_client::connect as connect_acuity_client};
use prost::Message;
use rand::RngCore;
use subxt::events::DecodeAsEvent;

use crate::accounts::AccountStore;
use crate::content::{
    self, account_id_from_ss58, build_image_mixin, bytes32_to_hex, decode_single_mixin,
    fetch_ipfs_digest_bytes, fetch_latest_revision_hash, hex_to_bytes32,
    preview_data_url_for_image_mixin, upload_ipfs_digest, BodyTextMixinMessage, ItemMessage,
    LanguageMixinMessage, MixinPayloadMessage, SelectedImage, TitleMixinMessage,
    BODY_TEXT_MIXIN_ID, DEFAULT_LANGUAGE_TAG, IMAGE_MIXIN_ID, LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID,
};

pub const PROFILE_MIXIN_ID: u32 = 0xbeef_2144;
const PROFILE_ITEM_FLAGS: u8 = 0x01;

#[derive(Clone, PartialEq, Default)]
pub struct ProfileDraft {
    pub name: String,
    pub bio: String,
    pub location: String,
    pub account_type: u32,
}

#[derive(Clone, PartialEq, Default)]
pub struct LoadedProfile {
    pub exists: bool,
    pub item_id: Option<[u8; 32]>,
    pub item_id_hex: Option<String>,
    pub revision_ipfs_hash_hex: Option<String>,
    pub draft: ProfileDraft,
    pub image_preview_data_url: Option<String>,
    pub existing_image_payload: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct SaveProfileRequest {
    pub draft: ProfileDraft,
    pub existing_item_id: Option<[u8; 32]>,
    pub existing_image_payload: Option<Vec<u8>>,
    pub selected_image: Option<SelectedImage>,
}

#[derive(Clone, PartialEq, Message)]
struct ProfileMixinMessage {
    #[prost(enumeration = "AccountType", tag = "1")]
    account_type: i32,
    #[prost(string, tag = "2")]
    location: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
enum AccountType {
    Anon = 0,
    Person = 1,
    Project = 2,
    Organization = 3,
    Proxy = 4,
    Parody = 5,
    Bot = 6,
    Shill = 7,
    Test = 8,
}

pub async fn load_profile_for_account(address: &str) -> Result<LoadedProfile, String> {
    let account_id = account_id_from_ss58(address)?;
    let item_id = fetch_profile_item_id(account_id).await?;

    let Some(item_id) = item_id else {
        return Ok(LoadedProfile::default());
    };

    let item_id_hex = bytes32_to_hex(&item_id);
    let revision_ipfs_hash = fetch_latest_revision_hash(item_id_hex.clone()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_ipfs_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode profile payload: {error}"))?;

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|mixin| mixin.title)
        .unwrap_or_default();
    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|mixin| mixin.body_text)
        .unwrap_or_default();
    let profile_mixin = decode_single_mixin::<ProfileMixinMessage>(&item, PROFILE_MIXIN_ID);
    let image_mixin_payload = item
        .mixin_payload
        .iter()
        .find(|mixin| mixin.mixin_id == IMAGE_MIXIN_ID)
        .map(|mixin| mixin.payload.clone());

    let mut draft = ProfileDraft {
        name: title,
        bio: body_text,
        ..ProfileDraft::default()
    };

    if let Some(profile_mixin) = profile_mixin {
        draft.account_type = u32::try_from(profile_mixin.account_type).unwrap_or_default();
        draft.location = profile_mixin.location;
    }

    let image_preview_data_url = if let Some(payload) = image_mixin_payload.as_ref() {
        preview_data_url_for_image_mixin(payload).await?
    } else {
        None
    };

    Ok(LoadedProfile {
        exists: true,
        item_id: Some(item_id),
        item_id_hex: Some(item_id_hex),
        revision_ipfs_hash_hex: Some(revision_ipfs_hash),
        draft,
        image_preview_data_url,
        existing_image_payload: image_mixin_payload,
    })
}

pub async fn save_profile(
    account_store: &AccountStore,
    request: SaveProfileRequest,
) -> Result<LoadedProfile, String> {
    let active_account = account_store
        .active_account()
        .ok_or_else(|| "Select an account before saving a profile.".to_string())?
        .clone();
    let signer = account_store
        .active_account_id
        .as_deref()
        .and_then(|id| account_store.unlocked_signers.get(id))
        .cloned()
        .ok_or_else(|| "Unlock the active account before saving a profile.".to_string())?;
    let account_id = account_id_from_ss58(&active_account.address)?;

    let had_selected_image = request.selected_image.is_some();
    let (image_payload, image_preview_data_url) = match request.selected_image.clone() {
        Some(selected_image) => {
            let built = build_image_mixin(&selected_image).await?;
            (Some(built.payload), Some(built.preview_data_url))
        }
        None => (request.existing_image_payload.clone(), None),
    };

    let item_payload = encode_profile_item(&request.draft, image_payload.clone());
    let revision_ipfs_hash = upload_ipfs_digest(&item_payload).await?;
    let revision_ipfs_hash_bytes = hex_to_bytes32(&revision_ipfs_hash)?;

    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access the latest block for transactions: {error}"))?;
    let mut tx_client = at_block.tx();

    let item_id = if let Some(existing_item_id) = request.existing_item_id {
        let publish_revision_tx = api::tx().content().publish_revision(
            api::runtime_types::pallet_content::pallet::ItemId(existing_item_id),
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            api::runtime_types::pallet_content::pallet::IpfsHash(revision_ipfs_hash_bytes),
        );

        tx_client
            .sign_and_submit_then_watch_default(&publish_revision_tx, &signer)
            .await
            .map_err(|error| format!("Failed to submit profile revision: {error}"))?
            .wait_for_finalized_success()
            .await
            .map_err(|error| format!("Profile revision failed: {error}"))?;

        existing_item_id
    } else {
        let mut nonce = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        let item_id = content::derive_item_id(account_id, nonce);

        let publish_item_call = api::Call::Content(
            api::runtime_types::pallet_content::pallet::Call::publish_item {
                nonce: api::runtime_types::pallet_content::Nonce(nonce),
                parents: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                flags: PROFILE_ITEM_FLAGS,
                links: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                mentions: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                ipfs_hash: api::runtime_types::pallet_content::pallet::IpfsHash(
                    revision_ipfs_hash_bytes,
                ),
            },
        );

        let set_profile_call = api::Call::AccountProfile(
            api::runtime_types::pallet_account_profile::pallet::Call::set_profile {
                item_id: api::runtime_types::pallet_content::pallet::ItemId(item_id),
            },
        );

        let first_profile_batch_tx = api::tx()
            .utility()
            .batch_all(vec![publish_item_call, set_profile_call]);

        let batch_events = tx_client
            .sign_and_submit_then_watch_default(&first_profile_batch_tx, &signer)
            .await
            .map_err(|error| format!("Failed to submit first profile batch: {error}"))?
            .wait_for_finalized_success()
            .await
            .map_err(|error| format!("First profile creation failed: {error}"))?;

        let mut saw_batch_completed = false;
        let mut saw_profile_set = false;
        for event in batch_events.iter() {
            let event = event
                .map_err(|error| format!("Failed to decode first profile batch event: {error}"))?;

            if api::utility::events::BatchCompleted::is_event(event.pallet_name(), event.event_name()) {
                saw_batch_completed = true;
                continue;
            }

            if api::utility::events::BatchInterrupted::is_event(event.pallet_name(), event.event_name()) {
                return Err(
                    "First profile batch was interrupted before all calls completed."
                        .to_string(),
                );
            }

            if api::account_profile::events::ProfileSet::is_event(event.pallet_name(), event.event_name()) {
                saw_profile_set = true;
            }
        }

        if !saw_batch_completed {
            return Err(
                "First profile batch finalized without a Utility::BatchCompleted event."
                    .to_string(),
            );
        }

        let published_item_id = batch_events
            .find_first::<api::content::events::PublishItem>()
            .transpose()
            .map_err(|error| format!("Failed to decode Content::PublishItem event: {error}"))?
            .ok_or_else(|| {
                "First profile batch completed without emitting Content::PublishItem.".to_string()
            })?
            .item_id
            .0;

        if published_item_id != item_id {
            return Err(
                format!(
                    "First profile batch created item {} but the dapp expected {}.",
                    bytes32_to_hex(&published_item_id),
                    bytes32_to_hex(&item_id),
                ),
            );
        }

        if !saw_profile_set {
            return Err(
                "First profile batch completed without emitting AccountProfile::ProfileSet."
                    .to_string(),
            );
        }

        item_id
    };

    let image_preview_data_url = if had_selected_image {
        image_preview_data_url
    } else {
        if let Some(payload) = image_payload.as_ref() {
            preview_data_url_for_image_mixin(payload).await?
        } else {
            None
        }
    };

    Ok(LoadedProfile {
        exists: true,
        item_id: Some(item_id),
        item_id_hex: Some(bytes32_to_hex(&item_id)),
        revision_ipfs_hash_hex: Some(revision_ipfs_hash),
        draft: request.draft,
        image_preview_data_url,
        existing_image_payload: image_payload,
    })
}

fn encode_profile_item(draft: &ProfileDraft, image_payload: Option<Vec<u8>>) -> Vec<u8> {
    let mut item = ItemMessage {
        mixin_payload: vec![
            MixinPayloadMessage {
                mixin_id: PROFILE_MIXIN_ID,
                payload: ProfileMixinMessage {
                    account_type: i32::try_from(draft.account_type)
                        .unwrap_or(AccountType::Anon as i32),
                    location: draft.location.clone(),
                }
                .encode_to_vec(),
            },
            MixinPayloadMessage {
                mixin_id: LANGUAGE_MIXIN_ID,
                payload: LanguageMixinMessage {
                    language_tag: DEFAULT_LANGUAGE_TAG.to_string(),
                }
                .encode_to_vec(),
            },
            MixinPayloadMessage {
                mixin_id: TITLE_MIXIN_ID,
                payload: TitleMixinMessage {
                    title: draft.name.clone(),
                }
                .encode_to_vec(),
            },
            MixinPayloadMessage {
                mixin_id: BODY_TEXT_MIXIN_ID,
                payload: BodyTextMixinMessage {
                    body_text: draft.bio.clone(),
                }
                .encode_to_vec(),
            },
        ],
    };

    if let Some(image_payload) = image_payload {
        item.mixin_payload.push(MixinPayloadMessage {
            mixin_id: IMAGE_MIXIN_ID,
            payload: image_payload,
        });
    }

    item.encode_to_vec()
}

async fn fetch_profile_item_id(account_id: sp_core::crypto::AccountId32) -> Result<Option<[u8; 32]>, String> {
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access the latest block for storage: {error}"))?;
    let storage_address = api::storage().account_profile().account_profile();
    let storage_account_id = subxt::utils::AccountId32(account_id.into());
    let maybe_item_id = at_block
        .storage()
        .try_fetch(&storage_address, (storage_account_id,))
        .await
        .map_err(|error| format!("Failed to fetch account profile pointer: {error}"))?;

    let Some(item_id) = maybe_item_id else {
        return Ok(None);
    };

    let item_id = item_id
        .decode()
        .map_err(|error| format!("Failed to decode account profile pointer: {error}"))?;

    Ok(Some(item_id.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{decode_single_mixin, ImageMixinMessage};

    fn decode_item(bytes: &[u8]) -> ItemMessage {
        ItemMessage::decode(bytes).unwrap()
    }

    #[test]
    fn encode_profile_item_includes_required_mixins_without_image() {
        let draft = ProfileDraft {
            name: "Alice".to_string(),
            bio: "Hello".to_string(),
            location: "Earth".to_string(),
            account_type: AccountType::Project as u32,
        };

        let item = decode_item(&encode_profile_item(&draft, None));

        assert_eq!(item.mixin_payload.len(), 4);
        assert_eq!(
            decode_single_mixin::<ProfileMixinMessage>(&item, PROFILE_MIXIN_ID),
            Some(ProfileMixinMessage {
                account_type: AccountType::Project as i32,
                location: "Earth".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID),
            Some(LanguageMixinMessage {
                language_tag: "en".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID),
            Some(TitleMixinMessage {
                title: "Alice".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID),
            Some(BodyTextMixinMessage {
                body_text: "Hello".to_string(),
            })
        );
        assert_eq!(decode_single_mixin::<ImageMixinMessage>(&item, IMAGE_MIXIN_ID), None);
    }

    #[test]
    fn encode_profile_item_includes_optional_image_payload() {
        let draft = ProfileDraft::default();
        let image_payload = vec![1, 2, 3, 4];

        let item = decode_item(&encode_profile_item(&draft, Some(image_payload.clone())));

        let image_mixin = item
            .mixin_payload
            .iter()
            .find(|mixin| mixin.mixin_id == IMAGE_MIXIN_ID)
            .unwrap();
        assert_eq!(image_mixin.payload, image_payload);
    }

    #[test]
    fn encode_profile_item_falls_back_to_anon_for_out_of_range_account_type() {
        let draft = ProfileDraft {
            account_type: u32::MAX,
            ..ProfileDraft::default()
        };

        let item = decode_item(&encode_profile_item(&draft, None));
        let profile = decode_single_mixin::<ProfileMixinMessage>(&item, PROFILE_MIXIN_ID).unwrap();

        assert_eq!(profile.account_type, AccountType::Anon as i32);
    }
}
