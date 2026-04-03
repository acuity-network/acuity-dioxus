use crate::content::{
    decode_single_mixin, fetch_events_for_item, fetch_ipfs_digest_bytes,
    fetch_latest_revision_hash, hex_to_bytes32, preview_data_url_for_image_mixin,
    BodyTextMixinMessage, IndexerStoredEvent, ItemMessage, TitleMixinMessage, BODY_TEXT_MIXIN_ID,
    IMAGE_MIXIN_ID, TITLE_MIXIN_ID,
};
use prost::Message;

use super::types::FeedPost;

/// Loads child posts for a feed by querying the indexer for all events
/// keyed by the feed's item_id, then filtering for `Content::PublishItem`
/// events where this feed appears as a parent.
pub async fn load_feed_posts(item_id_hex: &str) -> Result<Vec<FeedPost>, String> {
    let decoded_events = fetch_events_for_item(item_id_hex.to_string()).await?;

    // Collect child item IDs from PublishItem events where this feed is a parent.
    let mut child_item_ids: Vec<String> = Vec::new();
    for decoded_event in &decoded_events {
        let event = serde_json::from_value::<IndexerStoredEvent>(decoded_event.event.clone())
            .unwrap_or_else(|_| IndexerStoredEvent {
                pallet_name: String::new(),
                event_name: String::new(),
                fields: serde_json::Value::Null,
            });

        if event.pallet_name != "Content" || event.event_name != "PublishItem" {
            continue;
        }

        // The event's item_id field is the child item being published.
        // We only want events where the child's parents include our feed.
        // Since the indexer indexes each parent with multi=true, querying by
        // the feed's item_id returns PublishItem events for children that
        // declared this feed as a parent. But it also returns the feed's own
        // PublishItem event. Skip the feed's own event by checking item_id.
        let child_item_id = event
            .fields
            .get("item_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if child_item_id.is_empty() || child_item_id == item_id_hex {
            continue;
        }

        child_item_ids.push(child_item_id.to_string());
    }

    // Load each child post's content from IPFS.
    let mut posts = Vec::new();
    for child_id_hex in child_item_ids {
        match load_single_post(&child_id_hex).await {
            Ok(p) => posts.push(p),
            Err(_) => continue, // Skip posts that fail to load
        }
    }

    Ok(posts)
}

pub async fn load_single_post(item_id_hex: &str) -> Result<FeedPost, String> {
    let revision_hash = fetch_latest_revision_hash(item_id_hex.to_string()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode post payload: {error}"))?;

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|m| m.body_text)
        .unwrap_or_default();

    // Truncate body for preview.
    let body_preview = if body_text.len() > 200 {
        format!("{}...", &body_text[..200])
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

    // Convert hex item_id to base58 for the URL.
    let item_id_bytes = hex_to_bytes32(item_id_hex)?;
    let encoded_item_id = bs58::encode(item_id_bytes).into_string();

    Ok(FeedPost {
        encoded_item_id,
        title,
        body_preview,
        image_preview_data_url,
    })
}
