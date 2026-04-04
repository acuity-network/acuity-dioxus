use crate::{INDEXER_URL, IPFS_API_URL};
use crate::indexer_api::{
    GetEventsPayload, IndexerCustomKey, IndexerDecodedEvent, IndexerEnvelope,
    IndexerErrorPayload, IndexerEventsData, IndexerKey, IndexerRequest, IndexerScalarValue,
};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, GenericImageView};
use parity_scale_codec::Encode;
use prost::Message;
use reqwest::{multipart, Client};
use serde::Deserialize;
use sp_core::{crypto::AccountId32, crypto::Ss58Codec, hashing::blake2_256};
use std::{fs, io::Cursor, path::Path};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

pub use crate::indexer_api::IndexerStoredEvent;

// ── Formatting utilities ──────────────────────────────────────────────────────

/// Abbreviates a long hex string to `first10...last8` for display.
pub fn short_hex(value: &str) -> String {
    if value.len() <= 18 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..10], &value[value.len() - 8..])
    }
}

// ── Mixin ID constants ────────────────────────────────────────────────────────

pub const LANGUAGE_MIXIN_ID: u32 = 0x9bc7_a0e6;
pub const TITLE_MIXIN_ID: u32 = 0x344f_4812;
pub const BODY_TEXT_MIXIN_ID: u32 = 0x2d38_2044;
pub const IMAGE_MIXIN_ID: u32 = 0x045e_ee8c;
pub const DEFAULT_LANGUAGE_TAG: &str = "en";
pub const ITEM_ID_NAMESPACE: u32 = 1000;
pub const JPEG_QUALITY: u8 = 82;

// ── Protobuf message types ────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Message)]
pub struct ItemMessage {
    #[prost(message, repeated, tag = "1")]
    pub mixin_payload: Vec<MixinPayloadMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub struct MixinPayloadMessage {
    #[prost(fixed32, tag = "1")]
    pub mixin_id: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TitleMixinMessage {
    #[prost(string, tag = "1")]
    pub title: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct BodyTextMixinMessage {
    #[prost(string, tag = "1")]
    pub body_text: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct LanguageMixinMessage {
    #[prost(string, tag = "1")]
    pub language_tag: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ImageMixinMessage {
    #[prost(string, tag = "1")]
    pub filename: String,
    #[prost(uint64, tag = "2")]
    pub filesize: u64,
    #[prost(bytes = "vec", tag = "3")]
    pub ipfs_hash: Vec<u8>,
    #[prost(uint32, tag = "4")]
    pub width: u32,
    #[prost(uint32, tag = "5")]
    pub height: u32,
    #[prost(message, repeated, tag = "6")]
    pub mipmap_level: Vec<MipmapLevelMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub struct MipmapLevelMessage {
    #[prost(uint64, tag = "1")]
    pub filesize: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub ipfs_hash: Vec<u8>,
}

// ── Shared data types ─────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Default)]
pub struct SelectedImage {
    pub path: String,
    pub file_name: String,
    pub preview_data_url: Option<String>,
}

pub struct BuiltImageMixin {
    pub payload: Vec<u8>,
    pub preview_data_url: String,
}

// ── Revision types ────────────────────────────────────────────────────────────

/// A single revision entry for an item, resolved from an indexed
/// `Content::PublishRevision` event.
#[derive(Clone, PartialEq)]
pub struct RevisionEntry {
    /// On-chain revision counter (starts at 0, increments per revision).
    pub revision_id: u32,
    /// IPFS content hash in `0x`-prefixed hex (sha2-256 digest).
    pub ipfs_hash_hex: String,
}

#[derive(Deserialize)]
pub struct IpfsAddResponse {
    #[serde(rename = "Hash")]
    pub hash: String,
}

// ── IPFS helpers ──────────────────────────────────────────────────────────────

pub async fn upload_ipfs_digest(bytes: &[u8]) -> Result<String, String> {
    let form = multipart::Form::new().part(
        "file",
        multipart::Part::bytes(bytes.to_vec()).file_name("content.bin"),
    );

    let response = Client::new()
        .post(format!("{IPFS_API_URL}/api/v0/add"))
        .query(&[("pin", "true"), ("quieter", "true")])
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("Failed to upload payload to IPFS: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("IPFS rejected the payload upload: {error}"))?;

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

pub async fn upload_raw_ipfs_cid(bytes: &[u8], file_name: &str) -> Result<String, String> {
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
        .map_err(|error| format!("Failed to upload image to IPFS: {error}"))?;

    let response = response
        .error_for_status()
        .map_err(|error| format!("IPFS rejected the image upload: {error}"))?;

    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to decode the image upload response: {error}"))?;
    let last_line = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .next_back()
        .ok_or_else(|| "IPFS add returned an empty image upload response.".to_string())?;
    let payload = serde_json::from_str::<IpfsAddResponse>(last_line)
        .map_err(|error| format!("Failed to decode the image CID returned by IPFS: {error}"))?;

    Ok(payload.hash)
}

pub async fn fetch_ipfs_digest_bytes(ipfs_hash_hex: &str) -> Result<Vec<u8>, String> {
    let cid = digest_hex_to_cid(ipfs_hash_hex)?;
    fetch_ipfs_bytes_by_cid(&cid).await
}

pub async fn fetch_ipfs_bytes_by_cid(cid: &str) -> Result<Vec<u8>, String> {
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

// ── Image helpers ─────────────────────────────────────────────────────────────

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

pub async fn preview_data_url_for_image_mixin(payload: &[u8]) -> Result<Option<String>, String> {
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

pub async fn build_image_mixin(selected_image: &SelectedImage) -> Result<BuiltImageMixin, String> {
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

pub fn encode_as_jpeg(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    JpegEncoder::new_with_quality(&mut cursor, JPEG_QUALITY)
        .encode_image(image)
        .map_err(|error| format!("Failed to encode JPEG preview: {error}"))?;
    Ok(bytes)
}

// ── Item ID derivation ────────────────────────────────────────────────────────

pub fn derive_item_id(account_id: AccountId32, nonce: [u8; 32]) -> [u8; 32] {
    let payload = [
        account_id.encode(),
        nonce.encode(),
        parity_scale_codec::Encode::encode(&ITEM_ID_NAMESPACE),
    ]
    .concat();
    blake2_256(&payload)
}

// ── Account helpers ───────────────────────────────────────────────────────────

pub fn account_id_from_ss58(address: &str) -> Result<AccountId32, String> {
    AccountId32::from_ss58check(address)
        .map_err(|error| format!("Failed to decode account address {address}: {error}"))
}

// ── Hex / CID helpers ─────────────────────────────────────────────────────────

pub fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(bytes))
}

pub fn hex_to_bytes32(hex_value: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(hex_value.trim_start_matches("0x"))
        .map_err(|error| format!("Invalid hex value {hex_value}: {error}"))?;
    raw.try_into()
        .map_err(|_| format!("Expected 32 bytes for {hex_value}."))
}

pub fn digest_hex_to_cid(hex_value: &str) -> Result<String, String> {
    let digest = hex_to_bytes32(hex_value)?;
    let mut multihash = Vec::with_capacity(34);
    multihash.push(0x12);
    multihash.push(0x20);
    multihash.extend_from_slice(&digest);
    Ok(bs58::encode(multihash).into_string())
}

pub fn cid_to_digest_hex(cid: &str) -> Result<String, String> {
    let multihash = bs58::decode(cid)
        .into_vec()
        .map_err(|error| format!("Failed to decode CID {cid}: {error}"))?;
    if multihash.len() != 34 || multihash[0] != 0x12 || multihash[1] != 0x20 {
        return Err(format!("CID {cid} is not a sha2-256 CIDv0 multihash."));
    }
    Ok(format!("0x{}", hex::encode(&multihash[2..])))
}

// ── Protobuf decode helpers ───────────────────────────────────────────────────

pub fn decode_single_mixin<M>(item: &ItemMessage, mixin_id: u32) -> Option<M>
where
    M: Message + Default,
{
    item.mixin_payload
        .iter()
        .find(|mixin| mixin.mixin_id == mixin_id)
        .and_then(|mixin| M::decode(mixin.payload.as_slice()).ok())
}

// ── Indexer helpers ───────────────────────────────────────────────────────────

/// Fetches all decoded events from the indexer for a given `item_id`.
///
/// Returns the full list of `IndexerDecodedEvent` entries in reverse
/// chronological order (newest first), up to the indexer's per-query limit.
pub async fn fetch_events_for_item(
    item_id_hex: String,
) -> Result<Vec<IndexerDecodedEvent>, String> {
    let (mut stream, _) = connect_async(INDEXER_URL)
        .await
        .map_err(|error| format!("Failed to connect to {INDEXER_URL}: {error}"))?;

    let request = IndexerRequest {
        id: 1,
        message_type: "GetEvents",
        payload: GetEventsPayload {
            key: IndexerKey::Custom(IndexerCustomKey {
                name: "item_id".to_string(),
                kind: "bytes32".to_string(),
                value: IndexerScalarValue::String(item_id_hex),
            }),
            limit: None,
            before: None,
        },
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

        match parse_indexer_events_envelope(envelope, request.id)? {
            Some(decoded_events) => return Ok(decoded_events),
            None => continue,
        }
    }

    Err("The indexer closed the websocket before returning events.".to_string())
}

fn parse_indexer_events_envelope(
    envelope: IndexerEnvelope,
    request_id: u64,
) -> Result<Option<Vec<IndexerDecodedEvent>>, String> {
    match envelope.message_type.as_str() {
        "events" if envelope.id == Some(request_id) => {
            let response = serde_json::from_value::<IndexerEventsData>(
                envelope.data.ok_or_else(|| {
                    "Indexer events response did not include any event data.".to_string()
                })?,
            )
            .map_err(|error| format!("Failed to decode indexer events payload: {error}"))?;

            Ok(Some(response.decoded_events))
        }
        "error" if envelope.id == Some(request_id) => {
            let error = serde_json::from_value::<IndexerErrorPayload>(envelope.data.ok_or_else(
                || "Indexer error response did not include any error data.".to_string(),
            )?)
            .map_err(|decode_error| {
                format!("Failed to decode indexer error payload: {decode_error}")
            })?;
            Err(format!(
                "Indexer rejected GetEvents with {}: {}",
                error.code, error.message
            ))
        }
        _ => Ok(None),
    }
}

pub async fn fetch_latest_revision_hash(item_id_hex: String) -> Result<String, String> {
    let history = fetch_revision_history(item_id_hex).await?;
    history
        .into_iter()
        .next()
        .map(|entry| entry.ipfs_hash_hex)
        .ok_or_else(|| {
            "No indexed Content::PublishRevision event was found for this item.".to_string()
        })
}

/// Fetches the full revision history for an item from the indexer, returning
/// all `Content::PublishRevision` events sorted by `revision_id` **descending**
/// (newest first).
pub async fn fetch_revision_history(item_id_hex: String) -> Result<Vec<RevisionEntry>, String> {
    let decoded_events = fetch_events_for_item(item_id_hex).await?;

    if decoded_events.is_empty() {
        return Err(
            "The indexer did not return stored events. Enable event storage in the indexer so the dapp can resolve revisions."
                .to_string(),
        );
    }

    let mut entries: Vec<RevisionEntry> = Vec::new();

    for decoded_event in decoded_events {
        let event = serde_json::from_value::<IndexerStoredEvent>(decoded_event.event)
            .map_err(|error| format!("Failed to decode indexed event payload: {error}"))?;

        if event.pallet_name != "Content" || event.event_name != "PublishRevision" {
            continue;
        }

        let ipfs_hash_hex = event
            .fields
            .get("ipfs_hash")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "PublishRevision event was missing an ipfs_hash field.".to_string())?
            .to_string();

        let revision_id = event
            .fields
            .get("revision_id")
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
            .ok_or_else(|| "PublishRevision event was missing a revision_id field.".to_string())?
            as u32;

        entries.push(RevisionEntry {
            revision_id,
            ipfs_hash_hex,
        });
    }

    if entries.is_empty() {
        return Err(
            "No indexed Content::PublishRevision event was found for this item.".to_string(),
        );
    }

    // Sort newest first.
    entries.sort_by(|a, b| b.revision_id.cmp(&a.revision_id));

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{env, fs, path::PathBuf, process};

    fn unique_test_dir(label: &str) -> PathBuf {
        let unique = format!(
            "acuity-dioxus-content-{label}-{}-{}",
            process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default()
        );
        env::temp_dir().join(unique)
    }

    fn write_test_image(path: &Path, format: ImageFormat) {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            2,
            2,
            Rgba([12, 34, 56, 255]),
        ));
        image.save_with_format(path, format).unwrap();
    }

    #[test]
    fn preview_data_url_for_path_uses_expected_mime_types() {
        let test_dir = unique_test_dir("preview-mime");
        fs::create_dir_all(&test_dir).unwrap();

        let cases = [
            ("image.png", ImageFormat::Png, "data:image/png;base64,"),
            ("image.gif", ImageFormat::Gif, "data:image/gif;base64,"),
            ("image.bmp", ImageFormat::Bmp, "data:image/bmp;base64,"),
            ("image.webp", ImageFormat::WebP, "data:image/webp;base64,"),
            ("image.tiff", ImageFormat::Tiff, "data:image/tiff;base64,"),
        ];

        for (file_name, format, expected_prefix) in cases {
            let path = test_dir.join(file_name);
            write_test_image(&path, format);
            let data_url = preview_data_url_for_path(&path).unwrap();
            assert!(data_url.starts_with(expected_prefix), "{file_name}: {data_url}");
        }

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn preview_data_url_for_path_defaults_to_jpeg_for_unknown_bytes() {
        let test_dir = unique_test_dir("preview-fallback");
        fs::create_dir_all(&test_dir).unwrap();
        let path = test_dir.join("unknown.bin");
        fs::write(&path, b"not an image format").unwrap();

        let data_url = preview_data_url_for_path(&path).unwrap();

        assert!(data_url.starts_with("data:image/jpeg;base64,"));

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn preview_data_url_for_path_reports_read_errors() {
        let path = unique_test_dir("missing-image").join("missing.png");

        let error = preview_data_url_for_path(&path).unwrap_err();

        assert!(error.contains("Failed to read image"));
        assert!(error.contains("missing.png"));
    }

    #[test]
    fn encode_as_jpeg_returns_decodable_jpeg_bytes() {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            4,
            3,
            Rgba([100, 150, 200, 255]),
        ));

        let jpeg = encode_as_jpeg(&image).unwrap();
        let format = image::guess_format(&jpeg).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();

        assert_eq!(format, ImageFormat::Jpeg);
        assert_eq!(decoded.dimensions(), (4, 3));
    }

    #[test]
    fn derive_item_id_is_deterministic_and_nonce_sensitive() {
        let account_id = AccountId32::new([7; 32]);
        let nonce_a = [1; 32];
        let nonce_b = [2; 32];

        let first = derive_item_id(account_id.clone(), nonce_a);
        let second = derive_item_id(account_id.clone(), nonce_a);
        let third = derive_item_id(account_id, nonce_b);

        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn account_id_from_ss58_round_trips_and_reports_invalid_input() {
        let original = AccountId32::new([9; 32]);
        let address = original.to_ss58check();

        assert_eq!(account_id_from_ss58(&address).unwrap(), original);

        let error = account_id_from_ss58("not-a-valid-address").unwrap_err();
        assert!(error.contains("Failed to decode account address not-a-valid-address"));
    }

    #[test]
    fn bytes32_hex_helpers_round_trip_and_validate_lengths() {
        let bytes = [0xab; 32];
        let encoded = bytes32_to_hex(&bytes);

        assert_eq!(encoded, format!("0x{}", "ab".repeat(32)));
        assert_eq!(hex_to_bytes32(&encoded).unwrap(), bytes);
        assert_eq!(hex_to_bytes32(&encoded[2..]).unwrap(), bytes);

        let invalid_hex = hex_to_bytes32("0xzz").unwrap_err();
        assert!(invalid_hex.contains("Invalid hex value 0xzz"));

        let wrong_length = hex_to_bytes32("0x1234").unwrap_err();
        assert_eq!(wrong_length, "Expected 32 bytes for 0x1234.");
    }

    #[test]
    fn digest_hex_and_cid_helpers_round_trip() {
        let digest_hex = format!("0x{}", "11".repeat(32));

        let cid = digest_hex_to_cid(&digest_hex).unwrap();
        let round_trip = cid_to_digest_hex(&cid).unwrap();

        assert_eq!(round_trip, digest_hex);
    }

    #[test]
    fn cid_to_digest_hex_rejects_invalid_multihash_shape() {
        let not_sha2_256 = bs58::encode([0x13_u8, 0x20].into_iter().chain([0_u8; 32]).collect::<Vec<_>>())
            .into_string();
        let error = cid_to_digest_hex(&not_sha2_256).unwrap_err();

        assert!(error.contains("is not a sha2-256 CIDv0 multihash"));
    }

    #[test]
    fn parse_indexer_events_envelope_returns_matching_events_response() {
        let envelope = IndexerEnvelope {
            id: Some(7),
            message_type: "events".to_string(),
            data: Some(json!({
                "decodedEvents": [
                    {
                        "event": {
                            "palletName": "Content",
                            "eventName": "PublishRevision",
                            "fields": {
                                "revision_id": 1,
                                "ipfs_hash": "0x11"
                            }
                        }
                    }
                ]
            })),
        };

        let decoded_events = parse_indexer_events_envelope(envelope, 7)
            .unwrap()
            .unwrap();

        assert_eq!(decoded_events.len(), 1);
        assert_eq!(decoded_events[0].event["palletName"], "Content");
    }

    #[test]
    fn parse_indexer_events_envelope_ignores_other_request_ids() {
        let envelope = IndexerEnvelope {
            id: Some(8),
            message_type: "events".to_string(),
            data: Some(json!({ "decodedEvents": [] })),
        };

        let result = parse_indexer_events_envelope(envelope, 7).unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn parse_indexer_events_envelope_surfaces_error_response() {
        let envelope = IndexerEnvelope {
            id: Some(7),
            message_type: "error".to_string(),
            data: Some(json!({
                "code": "invalid_request",
                "message": "missing field `id`"
            })),
        };

        let error = parse_indexer_events_envelope(envelope, 7).unwrap_err();

        assert!(error.contains("invalid_request"));
        assert!(error.contains("missing field `id`"));
    }
}
