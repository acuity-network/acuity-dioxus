use crate::{acuity_runtime::api, runtime_client::connect as connect_acuity_client, INDEXER_URL, IPFS_API_URL};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, GenericImageView};
use parity_scale_codec::Encode;
use prost::Message;
use rand::RngCore;
use reqwest::{multipart, Client};
use serde::{Deserialize, Serialize};
use sp_core::{crypto::AccountId32, crypto::Ss58Codec, hashing::blake2_256};
use std::{fs, io::Cursor, path::Path};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use crate::accounts::AccountStore;

pub const PROFILE_MIXIN_ID: u32 = 0xbeef_2144;
pub const LANGUAGE_MIXIN_ID: u32 = 0x9bc7_a0e6;
pub const TITLE_MIXIN_ID: u32 = 0x344f_4812;
pub const BODY_TEXT_MIXIN_ID: u32 = 0x2d38_2044;
pub const IMAGE_MIXIN_ID: u32 = 0x045e_ee8c;
const DEFAULT_LANGUAGE_TAG: &str = "en";
const ITEM_ID_NAMESPACE: u32 = 1000;
const PROFILE_ITEM_FLAGS: u8 = 0x01;
const JPEG_QUALITY: u8 = 82;

#[derive(Clone, PartialEq, Default)]
pub struct ProfileDraft {
    pub name: String,
    pub bio: String,
    pub location: String,
    pub account_type: u32,
}

#[derive(Clone, PartialEq, Default)]
pub struct SelectedImage {
    pub path: String,
    pub file_name: String,
    pub preview_data_url: Option<String>,
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
struct ItemMessage {
    #[prost(message, repeated, tag = "1")]
    mixin_payload: Vec<MixinPayloadMessage>,
}

#[derive(Clone, PartialEq, Message)]
struct MixinPayloadMessage {
    #[prost(fixed32, tag = "1")]
    mixin_id: u32,
    #[prost(bytes = "vec", tag = "2")]
    payload: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct ProfileMixinMessage {
    #[prost(enumeration = "AccountType", tag = "1")]
    account_type: i32,
    #[prost(string, tag = "2")]
    location: String,
}

#[derive(Clone, PartialEq, Message)]
struct TitleMixinMessage {
    #[prost(string, tag = "1")]
    title: String,
}

#[derive(Clone, PartialEq, Message)]
struct BodyTextMixinMessage {
    #[prost(string, tag = "1")]
    body_text: String,
}

#[derive(Clone, PartialEq, Message)]
struct LanguageMixinMessage {
    #[prost(string, tag = "1")]
    language_tag: String,
}

#[derive(Clone, PartialEq, Message)]
struct ImageMixinMessage {
    #[prost(string, tag = "1")]
    filename: String,
    #[prost(uint64, tag = "2")]
    filesize: u64,
    #[prost(bytes = "vec", tag = "3")]
    ipfs_hash: Vec<u8>,
    #[prost(uint32, tag = "4")]
    width: u32,
    #[prost(uint32, tag = "5")]
    height: u32,
    #[prost(message, repeated, tag = "6")]
    mipmap_level: Vec<MipmapLevelMessage>,
}

#[derive(Clone, PartialEq, Message)]
struct MipmapLevelMessage {
    #[prost(uint64, tag = "1")]
    filesize: u64,
    #[prost(bytes = "vec", tag = "2")]
    ipfs_hash: Vec<u8>,
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

#[derive(Deserialize)]
struct IndexerEnvelope {
    #[serde(rename = "type")]
    message_type: String,
    data: Option<IndexerEventsResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexerEventsResponse {
    events: Vec<IndexerEventRef>,
    #[serde(default)]
    block_events: Vec<IndexerBlockEvents>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexerEventRef {
    block_number: u32,
    event_index: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexerBlockEvents {
    block_number: u32,
    events: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexerStoredEvent {
    pallet_name: String,
    event_name: String,
    fields: serde_json::Value,
}

#[derive(Deserialize)]
struct IpfsAddResponse {
    #[serde(rename = "Hash")]
    hash: String,
}

#[derive(Serialize)]
struct IndexerEventsRequest {
    #[serde(rename = "type")]
    message_type: &'static str,
    key: IndexerKey,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "value")]
enum IndexerKey {
    Custom(IndexerCustomKey),
}

#[derive(Serialize)]
struct IndexerCustomKey {
    name: &'static str,
    kind: &'static str,
    value: String,
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
        let item_id = derive_item_id(account_id, nonce);

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

            if event.pallet_name() == "Utility" {
                match event.event_name() {
                    "BatchCompleted" => saw_batch_completed = true,
                    "BatchInterrupted" => {
                        return Err(
                            "First profile batch was interrupted before all calls completed."
                                .to_string(),
                        );
                    }
                    _ => {}
                }
            }

            if event.pallet_name() == "AccountProfile" && event.event_name() == "ProfileSet" {
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

pub fn preview_data_url_for_path(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read image {}: {error}", path.display()))?;
    let format = image::guess_format(&bytes)
        .map(|format| match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Gif => "image/gif",
            image::ImageFormat::Bmp => "image/bmp",
            image::ImageFormat::WebP => "image/webp",
            image::ImageFormat::Tiff => "image/tiff",
            _ => "image/jpeg",
        })
        .unwrap_or("image/jpeg");

    Ok(format!(
        "data:{format};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
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

fn decode_single_mixin<M>(item: &ItemMessage, mixin_id: u32) -> Option<M>
where
    M: Message + Default,
{
    item.mixin_payload
        .iter()
        .find(|mixin| mixin.mixin_id == mixin_id)
        .and_then(|mixin| M::decode(mixin.payload.as_slice()).ok())
}

async fn fetch_profile_item_id(account_id: AccountId32) -> Result<Option<[u8; 32]>, String> {
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

async fn fetch_latest_revision_hash(item_id_hex: String) -> Result<String, String> {
    let (mut stream, _) = connect_async(INDEXER_URL)
        .await
        .map_err(|error| format!("Failed to connect to {INDEXER_URL}: {error}"))?;

    let request = IndexerEventsRequest {
        message_type: "GetEvents",
        key: IndexerKey::Custom(IndexerCustomKey {
            name: "item_id",
            kind: "bytes32",
            value: item_id_hex,
        }),
    };

    stream
        .send(WsMessage::Text(
            serde_json::to_string(&request)
                .map_err(|error| format!("Failed to encode indexer request: {error}"))?
                .into(),
        ))
        .await
        .map_err(|error| format!("Failed to query the indexer: {error}"))?;

    while let Some(message) = stream.next().await {
        let message =
            message.map_err(|error| format!("Failed to read indexer response: {error}"))?;

        let payload = match message {
            WsMessage::Text(text) => text.to_string(),
            WsMessage::Binary(bytes) => std::str::from_utf8(&bytes)
                .map_err(|error| format!("Indexer returned invalid UTF-8: {error}"))?
                .to_string(),
            WsMessage::Close(_) => break,
            WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_) => continue,
        };

        let envelope = serde_json::from_str::<IndexerEnvelope>(&payload)
            .map_err(|error| format!("Failed to decode indexer event response: {error}"))?;

        if envelope.message_type != "events" {
            continue;
        }

        let response = envelope
            .data
            .ok_or_else(|| "Indexer events response did not include any event data.".to_string())?;
        if response.block_events.is_empty() {
            return Err(
                "The indexer did not return stored block events. Enable event storage in the indexer so the dapp can resolve profile revisions."
                    .to_string(),
            );
        }

        for event_ref in response.events {
            let Some(block) = response
                .block_events
                .iter()
                .find(|block| block.block_number == event_ref.block_number)
            else {
                continue;
            };

            let Some(event_json) = block.events.iter().find(|event| {
                event.get("eventIndex").and_then(|value| value.as_u64())
                    == Some(u64::from(event_ref.event_index))
            }) else {
                continue;
            };

            let event = serde_json::from_value::<IndexerStoredEvent>(event_json.clone())
                .map_err(|error| format!("Failed to decode indexed event payload: {error}"))?;
            if event.pallet_name != "Content" || event.event_name != "PublishRevision" {
                continue;
            }

            let ipfs_hash = event
                .fields
                .get("ipfs_hash")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    "Latest profile revision was missing an ipfs_hash field.".to_string()
                })?;
            return Ok(ipfs_hash.to_string());
        }

        return Err(
            "No indexed Content::PublishRevision event was found for this profile item."
                .to_string(),
        );
    }

    Err("The indexer closed the websocket before returning profile events.".to_string())
}

async fn fetch_ipfs_digest_bytes(ipfs_hash_hex: &str) -> Result<Vec<u8>, String> {
    let cid = digest_hex_to_cid(ipfs_hash_hex)?;
    fetch_ipfs_bytes_by_cid(&cid).await
}

async fn fetch_ipfs_bytes_by_cid(cid: &str) -> Result<Vec<u8>, String> {
    let response = Client::new()
        .post(format!("{IPFS_API_URL}/api/v0/cat"))
        .query(&[("arg", cid)])
        .send()
        .await
        .map_err(|error| format!("Failed to read {cid} from IPFS: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("IPFS returned an error while reading {cid}: {error}"))?;

    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("Failed to decode IPFS bytes for {cid}: {error}"))
}

async fn upload_ipfs_digest(bytes: &[u8]) -> Result<String, String> {
    let form = multipart::Form::new().part(
        "file",
        multipart::Part::bytes(bytes.to_vec()).file_name("profile.bin"),
    );

    let response = Client::new()
        .post(format!("{IPFS_API_URL}/api/v0/add"))
        .query(&[("pin", "true"), ("quieter", "true")])
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("Failed to upload profile payload to IPFS: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("IPFS rejected the profile payload upload: {error}"))?;

    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to decode IPFS add response: {error}"))?;
    let last_line = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .next_back()
        .ok_or_else(|| "IPFS add returned an empty response.".to_string())?;
    let payload = serde_json::from_str::<IpfsAddResponse>(last_line)
        .map_err(|error| format!("Failed to decode the returned IPFS CID: {error}"))?;

    cid_to_digest_hex(&payload.hash)
}

async fn preview_data_url_for_image_mixin(payload: &[u8]) -> Result<Option<String>, String> {
    let image = ImageMixinMessage::decode(payload)
        .map_err(|error| format!("Failed to decode the stored image mixin: {error}"))?;

    let cid = if let Some(mipmap) = image.mipmap_level.first() {
        if mipmap.ipfs_hash.is_empty() {
            None
        } else {
            Some(bs58::encode(&mipmap.ipfs_hash).into_string())
        }
    } else if image.ipfs_hash.is_empty() {
        None
    } else {
        Some(bs58::encode(&image.ipfs_hash).into_string())
    };

    let Some(cid) = cid else {
        return Ok(None);
    };

    let bytes = fetch_ipfs_bytes_by_cid(&cid).await?;
    Ok(Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

async fn build_image_mixin(selected_image: &SelectedImage) -> Result<BuiltImageMixin, String> {
    let bytes = fs::read(&selected_image.path)
        .map_err(|error| format!("Failed to read {}: {error}", selected_image.path))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|error| format!("Failed to decode {}: {error}", selected_image.file_name))?;
    let (width, height) = image.dimensions();

    let mut mipmap_level = Vec::new();
    let mut preview_data_url = None;
    let mut level = 0_u32;

    loop {
        let scale = 2_u32.pow(level);
        let out_width = ((width as f32) / (scale as f32)).round().max(1.0) as u32;
        let out_height = ((height as f32) / (scale as f32)).round().max(1.0) as u32;
        let resized = if level == 0 {
            image.clone()
        } else {
            image.resize_exact(out_width, out_height, FilterType::Lanczos3)
        };

        let jpeg_bytes = encode_as_jpeg(&resized)?;
        if preview_data_url.is_none() {
            preview_data_url = Some(format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&jpeg_bytes)
            ));
        }

        let cid = upload_raw_ipfs_cid(&jpeg_bytes, &selected_image.file_name).await?;
        let multihash = bs58::decode(cid)
            .into_vec()
            .map_err(|error| format!("Failed to decode uploaded image CID: {error}"))?;

        mipmap_level.push(MipmapLevelMessage {
            filesize: jpeg_bytes.len() as u64,
            ipfs_hash: multihash,
        });

        if out_width <= 64 || out_height <= 64 {
            break;
        }
        level += 1;
    }

    let payload = ImageMixinMessage {
        filename: String::new(),
        filesize: 0,
        ipfs_hash: Vec::new(),
        width,
        height,
        mipmap_level,
    }
    .encode_to_vec();

    Ok(BuiltImageMixin {
        payload,
        preview_data_url: preview_data_url.unwrap_or_default(),
    })
}

async fn upload_raw_ipfs_cid(bytes: &[u8], file_name: &str) -> Result<String, String> {
    let form = multipart::Form::new().part(
        "file",
        multipart::Part::bytes(bytes.to_vec()).file_name(file_name.to_string()),
    );

    let response = Client::new()
        .post(format!("{IPFS_API_URL}/api/v0/add"))
        .query(&[("pin", "true"), ("quieter", "true")])
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("Failed to upload the profile image to IPFS: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("IPFS rejected the profile image upload: {error}"))?;

    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to decode the profile image upload response: {error}"))?;
    let last_line = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .next_back()
        .ok_or_else(|| "IPFS add returned an empty image upload response.".to_string())?;
    let payload = serde_json::from_str::<IpfsAddResponse>(last_line)
        .map_err(|error| format!("Failed to decode the image CID returned by IPFS: {error}"))?;

    Ok(payload.hash)
}

fn encode_as_jpeg(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    JpegEncoder::new_with_quality(&mut cursor, JPEG_QUALITY)
        .encode_image(image)
        .map_err(|error| format!("Failed to encode JPEG preview: {error}"))?;
    Ok(bytes)
}

fn derive_item_id(account_id: AccountId32, nonce: [u8; 32]) -> [u8; 32] {
    let payload = [
        account_id.encode(),
        nonce.encode(),
        parity_scale_codec::Encode::encode(&ITEM_ID_NAMESPACE),
    ]
    .concat();
    blake2_256(&payload)
}

fn account_id_from_ss58(address: &str) -> Result<AccountId32, String> {
    AccountId32::from_ss58check(address)
        .map_err(|error| format!("Failed to decode account address {address}: {error}"))
}

fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn hex_to_bytes32(hex_value: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(hex_value.trim_start_matches("0x"))
        .map_err(|error| format!("Invalid hex value {hex_value}: {error}"))?;
    raw.try_into()
        .map_err(|_| format!("Expected 32 bytes for {hex_value}."))
}

fn digest_hex_to_cid(hex_value: &str) -> Result<String, String> {
    let digest = hex_to_bytes32(hex_value)?;
    let mut multihash = Vec::with_capacity(34);
    multihash.push(0x12);
    multihash.push(0x20);
    multihash.extend_from_slice(&digest);
    Ok(bs58::encode(multihash).into_string())
}

fn cid_to_digest_hex(cid: &str) -> Result<String, String> {
    let multihash = bs58::decode(cid)
        .into_vec()
        .map_err(|error| format!("Failed to decode CID {cid}: {error}"))?;
    if multihash.len() != 34 || multihash[0] != 0x12 || multihash[1] != 0x20 {
        return Err(format!("CID {cid} is not a sha2-256 CIDv0 multihash."));
    }
    Ok(format!("0x{}", hex::encode(&multihash[2..])))
}

struct BuiltImageMixin {
    payload: Vec<u8>,
    preview_data_url: String,
}
