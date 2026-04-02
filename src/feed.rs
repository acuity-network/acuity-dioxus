use crate::{acuity_runtime::api, runtime_client::connect as connect_acuity_client};
use prost::Message;
use rand::RngCore;
use subxt::events::DecodeAsEvent;

use crate::accounts::AccountStore;
use crate::content::{
    account_id_from_ss58, build_image_mixin, bytes32_to_hex, decode_single_mixin, derive_item_id,
    fetch_ipfs_digest_bytes, fetch_latest_revision_hash, hex_to_bytes32,
    preview_data_url_for_image_mixin, upload_ipfs_digest, BodyTextMixinMessage, ItemMessage,
    LanguageMixinMessage, MixinPayloadMessage, SelectedImage, TitleMixinMessage,
    BODY_TEXT_MIXIN_ID, DEFAULT_LANGUAGE_TAG, IMAGE_MIXIN_ID, LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID,
};
use crate::profile::PROFILE_MIXIN_ID;

/// Mixin ID for the feed type marker (no payload).
pub const FEED_TYPE_MIXIN_ID: u32 = 0xbcec_8faa;

/// Feed items are revisionable and retractable.
const FEED_ITEM_FLAGS: u8 = 0x03;

#[derive(Clone, PartialEq, Default)]
pub struct FeedDraft {
    pub title: String,
    pub description: String,
}

#[derive(Clone)]
pub struct PublishFeedRequest {
    pub draft: FeedDraft,
    pub selected_image: Option<SelectedImage>,
}

#[derive(Clone, PartialEq)]
pub struct PublishedFeed {
    pub item_id: [u8; 32],
    pub item_id_hex: String,
}

/// A lightweight summary of a single content item registered in `pallet-account-content`.
#[derive(Clone, PartialEq)]
pub struct LoadedFeedSummary {
    pub item_id_hex: String,
    /// Base-58 encoded item ID, used as the `encoded_item_id` route parameter.
    pub encoded_item_id: String,
    pub title: String,
    pub description_preview: String,
    pub image_preview_data_url: Option<String>,
    /// Human-readable type label: "Feed", "Profile", or "Content".
    pub content_type: &'static str,
}

/// Fetches every item ID registered against `address` in `pallet-account-content`,
/// then resolves each one through the indexer and IPFS to produce a summary card.
///
/// Items that fail to load are silently skipped so that one bad item cannot
/// prevent the rest from appearing.
pub async fn fetch_account_content_items(
    address: &str,
) -> Result<Vec<LoadedFeedSummary>, String> {
    let account_id = account_id_from_ss58(address)?;
    let storage_account_id = subxt::utils::AccountId32(account_id.into());

    // ── 1. Read the on-chain list of item IDs ──────────────────────────────
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access the latest block for storage: {error}"))?;

    let storage_address = api::storage().account_content().account_item_ids();
    let maybe_ids = at_block
        .storage()
        .try_fetch(&storage_address, (storage_account_id,))
        .await
        .map_err(|error| format!("Failed to fetch account content item IDs: {error}"))?;

    let item_ids = match maybe_ids {
        Some(encoded) => {
            let bounded_vec = encoded
                .decode()
                .map_err(|error| format!("Failed to decode account content item IDs: {error}"))?;
            bounded_vec.0
        }
        None => return Ok(Vec::new()),
    };

    // ── 2. Resolve each item through the indexer and IPFS ─────────────────
    let mut summaries = Vec::new();
    for item_id_wrapper in item_ids {
        let item_id: [u8; 32] = item_id_wrapper.0;
        let item_id_hex = bytes32_to_hex(&item_id);

        let summary = resolve_item_summary(item_id, &item_id_hex).await;
        match summary {
            Ok(s) => summaries.push(s),
            Err(_) => continue, // skip items that fail to load
        }
    }

    Ok(summaries)
}

async fn resolve_item_summary(
    item_id: [u8; 32],
    item_id_hex: &str,
) -> Result<LoadedFeedSummary, String> {
    let revision_hash = fetch_latest_revision_hash(item_id_hex.to_string()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode item payload: {error}"))?;

    let content_type = item_content_type_label(&item);

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|m| m.body_text)
        .unwrap_or_default();

    let description_preview = if body_text.len() > 160 {
        format!("{}...", &body_text[..160])
    } else {
        body_text
    };

    let image_mixin_payload = item
        .mixin_payload
        .iter()
        .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
        .map(|m| m.payload.clone());

    let image_preview_data_url = if let Some(ref payload) = image_mixin_payload {
        preview_data_url_for_image_mixin(payload).await.unwrap_or(None)
    } else {
        None
    };

    let encoded_item_id = bs58::encode(item_id).into_string();

    Ok(LoadedFeedSummary {
        item_id_hex: item_id_hex.to_string(),
        encoded_item_id,
        title,
        description_preview,
        image_preview_data_url,
        content_type,
    })
}

fn item_content_type_label(item: &ItemMessage) -> &'static str {
    for mixin in &item.mixin_payload {
        if mixin.mixin_id == FEED_TYPE_MIXIN_ID {
            return "Feed";
        }
        if mixin.mixin_id == PROFILE_MIXIN_ID {
            return "Profile";
        }
    }
    "Content"
}

pub async fn publish_feed(
    account_store: &AccountStore,
    request: PublishFeedRequest,
) -> Result<PublishedFeed, String> {
    // ── 1. Validate account is unlocked ────────────────────────────────────
    let active_account = account_store
        .active_account()
        .ok_or_else(|| "Select an account before publishing a feed.".to_string())?
        .clone();
    let signer = account_store
        .active_account_id
        .as_deref()
        .and_then(|id| account_store.unlocked_signers.get(id))
        .cloned()
        .ok_or_else(|| "Unlock the active account before publishing a feed.".to_string())?;
    let account_id = account_id_from_ss58(&active_account.address)?;

    // ── 2. Build image mixin if an image was selected ──────────────────────
    let image_payload = match request.selected_image {
        Some(ref selected_image) => {
            let built = build_image_mixin(selected_image).await?;
            Some(built.payload)
        }
        None => None,
    };

    // ── 3. Encode feed item payload (protobuf, no compression) ─────────────
    let item_payload = encode_feed_item(&request.draft, image_payload);

    // ── 4. Upload to IPFS ──────────────────────────────────────────────────
    let revision_ipfs_hash = upload_ipfs_digest(&item_payload).await?;
    let revision_ipfs_hash_bytes = hex_to_bytes32(&revision_ipfs_hash)?;

    // ── 5. Derive item ID ──────────────────────────────────────────────────
    let mut nonce = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let item_id = derive_item_id(account_id, nonce);

    // ── 6. Submit batch: publish_item + account_content::add_item ──────────
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access the latest block for transactions: {error}"))?;
    let mut tx_client = at_block.tx();

    let publish_item_call = api::Call::Content(
        api::runtime_types::pallet_content::pallet::Call::publish_item {
            nonce: api::runtime_types::pallet_content::Nonce(nonce),
            parents: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            flags: FEED_ITEM_FLAGS,
            links: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            mentions: api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
            ipfs_hash: api::runtime_types::pallet_content::pallet::IpfsHash(
                revision_ipfs_hash_bytes,
            ),
        },
    );

    let add_item_call = api::Call::AccountContent(
        api::runtime_types::pallet_account_content::pallet::Call::add_item {
            item_id: api::runtime_types::pallet_content::pallet::ItemId(item_id),
        },
    );

    let batch_tx = api::tx()
        .utility()
        .batch_all(vec![publish_item_call, add_item_call]);

    let batch_events = tx_client
        .sign_and_submit_then_watch_default(&batch_tx, &signer)
        .await
        .map_err(|error| format!("Failed to submit feed publish batch: {error}"))?
        .wait_for_finalized_success()
        .await
        .map_err(|error| format!("Feed publish batch failed: {error}"))?;

    // ── 7. Verify events ───────────────────────────────────────────────────
    let mut saw_batch_completed = false;
    let mut saw_add_item = false;
    for event in batch_events.iter() {
        let event = event
            .map_err(|error| format!("Failed to decode feed publish batch event: {error}"))?;

        if api::utility::events::BatchCompleted::is_event(
            event.pallet_name(),
            event.event_name(),
        ) {
            saw_batch_completed = true;
            continue;
        }

        if api::utility::events::BatchInterrupted::is_event(
            event.pallet_name(),
            event.event_name(),
        ) {
            return Err(
                "Feed publish batch was interrupted before all calls completed.".to_string(),
            );
        }

        if api::account_content::events::AddItem::is_event(
            event.pallet_name(),
            event.event_name(),
        ) {
            saw_add_item = true;
        }
    }

    if !saw_batch_completed {
        return Err(
            "Feed publish batch finalized without a Utility::BatchCompleted event.".to_string(),
        );
    }

    let published_item_id = batch_events
        .find_first::<api::content::events::PublishItem>()
        .transpose()
        .map_err(|error| format!("Failed to decode Content::PublishItem event: {error}"))?
        .ok_or_else(|| {
            "Feed publish batch completed without emitting Content::PublishItem.".to_string()
        })?
        .item_id
        .0;

    if published_item_id != item_id {
        return Err(format!(
            "Feed publish batch created item {} but the dapp expected {}.",
            bytes32_to_hex(&published_item_id),
            bytes32_to_hex(&item_id),
        ));
    }

    if !saw_add_item {
        return Err(
            "Feed publish batch completed without emitting AccountContent::AddItem.".to_string(),
        );
    }

    Ok(PublishedFeed {
        item_id,
        item_id_hex: bytes32_to_hex(&item_id),
    })
}

fn encode_feed_item(draft: &FeedDraft, image_payload: Option<Vec<u8>>) -> Vec<u8> {
    let mut item = ItemMessage {
        mixin_payload: vec![
            // Feed type marker (no payload)
            MixinPayloadMessage {
                mixin_id: FEED_TYPE_MIXIN_ID,
                payload: vec![],
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
                    title: draft.title.clone(),
                }
                .encode_to_vec(),
            },
            MixinPayloadMessage {
                mixin_id: BODY_TEXT_MIXIN_ID,
                payload: BodyTextMixinMessage {
                    body_text: draft.description.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{decode_single_mixin, ImageMixinMessage};

    fn decode_item(bytes: &[u8]) -> ItemMessage {
        ItemMessage::decode(bytes).unwrap()
    }

    #[test]
    fn encode_feed_item_includes_type_marker_and_content_mixins() {
        let draft = FeedDraft {
            title: "My Feed".to_string(),
            description: "A test feed".to_string(),
        };

        let item = decode_item(&encode_feed_item(&draft, None));

        assert_eq!(item.mixin_payload.len(), 4);

        // Feed type marker has no payload
        let type_mixin = item
            .mixin_payload
            .iter()
            .find(|m| m.mixin_id == FEED_TYPE_MIXIN_ID)
            .unwrap();
        assert!(type_mixin.payload.is_empty());

        assert_eq!(
            decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID),
            Some(LanguageMixinMessage {
                language_tag: "en".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID),
            Some(TitleMixinMessage {
                title: "My Feed".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID),
            Some(BodyTextMixinMessage {
                body_text: "A test feed".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<ImageMixinMessage>(&item, IMAGE_MIXIN_ID),
            None
        );
    }

    #[test]
    fn encode_feed_item_includes_optional_image() {
        let draft = FeedDraft::default();
        let image_payload = vec![10, 20, 30];

        let item = decode_item(&encode_feed_item(&draft, Some(image_payload.clone())));

        assert_eq!(item.mixin_payload.len(), 5);
        let image_mixin = item
            .mixin_payload
            .iter()
            .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
            .unwrap();
        assert_eq!(image_mixin.payload, image_payload);
    }
}
