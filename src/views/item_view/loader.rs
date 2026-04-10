use acuity_index_api_rs::IndexerClient;
use crate::{
    acuity_runtime::api,
    content::{
        bytes32_to_hex, decode_single_mixin, event_string_field, fetch_events_for_item,
        fetch_ipfs_digest_bytes, fetch_latest_revision_hash, fetch_revision_history, hex_to_bytes32,
        is_content_event,
        preview_data_url_for_image_mixin, short_hex, BodyTextMixinMessage, ItemMessage,
        RevisionEntry, TitleMixinMessage, BODY_TEXT_MIXIN_ID, IMAGE_MIXIN_ID, LANGUAGE_MIXIN_ID,
        TITLE_MIXIN_ID,
    },
    runtime_client::connect as connect_acuity_client,
};
use prost::Message;
use serde_json::Value;
use sp_core::crypto::Ss58Codec;

use super::types::{content_type_label, LoadedItem, ParentSummary};

/// Loads an item, optionally loading a specific revision by its IPFS hash.
///
/// When `ipfs_hash_override` is `None` the function fetches the full revision
/// history from the indexer and uses the on-chain `Content.ItemState`
/// `revision_id` to select the canonical latest revision.  Pass a specific
/// `ipfs_hash_hex` to display an older revision instead.
///
/// Returns `(LoadedItem, revision_history, chain_latest_revision_id)`.
pub async fn load_item(
    client: &IndexerClient,
    encoded_item_id: &str,
    ipfs_hash_override: Option<String>,
) -> Result<(LoadedItem, Vec<RevisionEntry>, u32), String> {
    let item_id_bytes = bs58::decode(encoded_item_id)
        .into_vec()
        .map_err(|error| format!("Invalid item ID encoding: {error}"))?;

    if item_id_bytes.len() != 32 {
        return Err(format!(
            "Item ID must be 32 bytes, got {}.",
            item_id_bytes.len()
        ));
    }

    let item_id: [u8; 32] = item_id_bytes
        .try_into()
        .map_err(|_| "Failed to convert item ID bytes.".to_string())?;

    let item_id_hex = bytes32_to_hex(&item_id);

    // Fetch revision history and on-chain state concurrently.
    let (history_result, state_result) = tokio::join!(
        fetch_revision_history(client, item_id_hex.clone()),
        fetch_item_state(item_id),
    );

    let history = history_result?;
    let (owner_address, chain_latest_revision_id) = state_result.unwrap_or_default();

    // Resolve which IPFS hash and revision_id to display.
    let (revision_ipfs_hash, loaded_revision_id) = if let Some(hash) = ipfs_hash_override {
        // Find the matching revision_id in history for metadata; fall back to 0.
        let rid = history
            .iter()
            .find(|e| e.ipfs_hash_hex == hash)
            .map(|e| e.revision_id)
            .unwrap_or(0);
        (hash, rid)
    } else {
        // Use the chain-confirmed latest revision.
        let entry = history
            .iter()
            .find(|e| e.revision_id == chain_latest_revision_id)
            .or_else(|| history.first())
            .ok_or_else(|| "No revisions found for this item.".to_string())?;
        (entry.ipfs_hash_hex.clone(), entry.revision_id)
    };

    let item_bytes = fetch_ipfs_digest_bytes(&revision_ipfs_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode item payload: {error}"))?;

    let content_type = content_type_label(&item).to_string();

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|m| m.body_text)
        .unwrap_or_default();

    let language = decode_single_mixin::<crate::content::LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID)
        .map(|m| m.language_tag)
        .unwrap_or_default();

    let existing_image_payload = item
        .mixin_payload
        .iter()
        .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
        .map(|m| m.payload.clone());

    let image_preview_data_url = if let Some(ref payload) = existing_image_payload {
        preview_data_url_for_image_mixin(payload).await?
    } else {
        None
    };

    // Load parent summaries from the item's own PublishItem indexer event.
    let parents = load_parent_summaries(client, &item_id_hex).await.unwrap_or_default();

    Ok((
        LoadedItem {
            encoded_item_id: encoded_item_id.to_string(),
            item_id,
            item_id_hex,
            revision_ipfs_hash_hex: revision_ipfs_hash,
            content_type,
            title,
            body_text,
            language,
            image_preview_data_url,
            existing_image_payload,
            parents,
            owner_address,
            revision_id: loaded_revision_id,
        },
        history,
        chain_latest_revision_id,
    ))
}

/// Queries `Content.ItemState` on-chain and returns `(owner_ss58, revision_id)`.
pub async fn fetch_item_state(item_id: [u8; 32]) -> Result<(String, u32), String> {
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access latest block for item state: {error}"))?;

    let storage_address = api::storage().content().item_state();
    let maybe_state = at_block
        .storage()
        .try_fetch(
            &storage_address,
            (api::runtime_types::pallet_content::pallet::ItemId(item_id),),
        )
        .await
        .map_err(|error| format!("Failed to fetch item state: {error}"))?;

    let (owner_address, revision_id) = maybe_state
        .and_then(|encoded| encoded.decode().ok())
        .map(|state| {
            let sp_account = sp_core::crypto::AccountId32::from(state.owner.0);
            (sp_account.to_ss58check(), state.revision_id)
        })
        .unwrap_or_default();

    Ok((owner_address, revision_id))
}

/// Finds this item's own `Content::PublishItem` event in the indexer and
/// returns a lightweight summary for each declared parent.  Parents that
/// fail to load are silently skipped.
pub async fn load_parent_summaries(client: &IndexerClient, item_id_hex: &str) -> Result<Vec<ParentSummary>, String> {
    let decoded_events = fetch_events_for_item(client, item_id_hex.to_string()).await?;

    // Find the PublishItem event whose item_id matches this item (not a child).
    let mut parent_hex_ids: Vec<String> = Vec::new();
    for decoded_event in &decoded_events {
        if !is_content_event(decoded_event, "PublishItem") {
            continue;
        }

        let event_item_id = event_string_field(decoded_event, "item_id").unwrap_or_default();

        // Only process the event that belongs to *this* item, not child items.
        if event_item_id != item_id_hex {
            continue;
        }

        // Extract the parents array — the indexer stores it as a JSON array
        // of hex strings or objects with an inner value field.
        if let Some(parents_val) = decoded_event.field("parents") {
            if let Some(arr) = parents_val.as_array() {
                for entry in arr.iter() {
                    // Try plain string first, then {"0": "0xabc..."} or nested.
                    let hex = if let Some(s) = entry.as_str() {
                        s.to_string()
                    } else if let Some(object) = entry.as_object() {
                        object
                            .get("0")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    if hex.is_empty() {
                        continue;
                    }
                    parent_hex_ids.push(hex);
                }
            }
        }

        // There is only one PublishItem event for this item; stop after it.
        break;
    }

    let mut summaries = Vec::new();
    for parent_hex in parent_hex_ids {
        match load_parent_summary(client, &parent_hex).await {
            Ok(summary) => summaries.push(summary),
            Err(_) => continue,
        }
    }

    Ok(summaries)
}

pub async fn load_parent_summary(client: &IndexerClient, item_id_hex: &str) -> Result<ParentSummary, String> {
    let revision_hash = fetch_latest_revision_hash(client, item_id_hex.to_string()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode parent item payload: {error}"))?;

    let content_type = content_type_label(&item).to_string();
    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    // Use shortened hex as fallback display name when there is no title.
    let display_title = if title.trim().is_empty() {
        short_hex(item_id_hex)
    } else {
        title
    };

    let item_id_bytes = hex_to_bytes32(item_id_hex)?;
    let encoded_item_id = bs58::encode(item_id_bytes).into_string();

    Ok(ParentSummary {
        encoded_item_id,
        title: display_title,
        content_type,
    })
}
