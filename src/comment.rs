use crate::{acuity_runtime::api, runtime_client::connect as connect_acuity_client};
use prost::Message;
use rand::RngCore;

use crate::accounts::AccountStore;
use crate::content::{
    account_id_from_ss58, bytes32_to_hex, decode_single_mixin, derive_item_id,
    fetch_events_for_item, fetch_ipfs_digest_bytes, fetch_latest_revision_hash,
    fetch_revision_history, hex_to_bytes32, upload_ipfs_digest, BodyTextMixinMessage,
    IndexerStoredEvent, ItemMessage, LanguageMixinMessage, MixinPayloadMessage,
    RevisionEntry, BODY_TEXT_MIXIN_ID, DEFAULT_LANGUAGE_TAG, LANGUAGE_MIXIN_ID,
};

/// Mixin ID that marks a content item as a comment. Matches the original
/// Ethereum implementation (`0x874aba65`).
pub const COMMENT_TYPE_MIXIN_ID: u32 = 0x874a_ba65;

/// Comment items are revisionable and retractable.
const COMMENT_ITEM_FLAGS: u8 = 0x03;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Default)]
pub struct CommentDraft {
    pub body: String,
}

#[derive(Clone)]
pub struct PublishCommentRequest {
    pub draft: CommentDraft,
    /// The item this comment is replying to (may itself be a comment).
    pub parent_item_id: [u8; 32],
}

#[derive(Clone, PartialEq)]
pub struct PublishedComment {
    pub item_id: [u8; 32],
    pub item_id_hex: String,
}

/// A fully-loaded comment, including its own nested child comments.
#[derive(Clone, PartialEq)]
pub struct LoadedComment {
    pub item_id: [u8; 32],
    pub item_id_hex: String,
    pub encoded_item_id: String,
    pub body_text: String,
    /// SS58 address of the account that published this comment.
    pub owner_address: String,
    /// Current on-chain revision counter from `Content.ItemState`.
    pub revision_id: u32,
    /// Whether the comment's `flags & 0x01` bit is set (revisionable).
    pub is_revisionable: bool,
    /// Recursively-loaded child comments on this comment.
    pub children: Vec<LoadedComment>,
}

// ── Publish ───────────────────────────────────────────────────────────────────

/// Publish a new comment as a child of `parent_item_id`.
///
/// The comment payload contains `COMMENT_TYPE_MIXIN_ID` (empty, as a type
/// marker), `LANGUAGE_MIXIN_ID`, and `BODY_TEXT_MIXIN_ID`. The on-chain call
/// is a plain `content.publish_item` with `parents = [parent_item_id]`.
pub async fn publish_comment(
    account_store: &AccountStore,
    request: PublishCommentRequest,
) -> Result<PublishedComment, String> {
    use crate::content::upload_ipfs_digest;

    // ── 1. Validate account ────────────────────────────────────────────────
    let active_account = account_store
        .active_account()
        .ok_or_else(|| "Select an account before posting a comment.".to_string())?
        .clone();
    let signer = account_store
        .active_account_id
        .as_deref()
        .and_then(|id| account_store.unlocked_signers.get(id))
        .cloned()
        .ok_or_else(|| "Unlock the active account before posting a comment.".to_string())?;
    let account_id = account_id_from_ss58(&active_account.address)?;

    // ── 2. Encode protobuf payload ─────────────────────────────────────────
    let item_payload = encode_comment_item(&request.draft);

    // ── 3. Upload to IPFS ──────────────────────────────────────────────────
    let revision_ipfs_hash = upload_ipfs_digest(&item_payload).await?;
    let revision_ipfs_hash_bytes = hex_to_bytes32(&revision_ipfs_hash)?;

    // ── 4. Derive item ID ──────────────────────────────────────────────────
    let mut nonce = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let item_id = derive_item_id(account_id, nonce);

    // ── 5. Submit content.publish_item ─────────────────────────────────────
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to access latest block for comment publish: {e}"))?;

    let publish_item_tx = api::tx().content().publish_item(
        api::runtime_types::pallet_content::Nonce(nonce),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![
            api::runtime_types::pallet_content::pallet::ItemId(request.parent_item_id),
        ]),
        COMMENT_ITEM_FLAGS,
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::pallet_content::pallet::IpfsHash(revision_ipfs_hash_bytes),
    );

    let events = at_block
        .tx()
        .sign_and_submit_then_watch_default(&publish_item_tx, &signer)
        .await
        .map_err(|e| format!("Failed to submit comment publish transaction: {e}"))?
        .wait_for_finalized_success()
        .await
        .map_err(|e| format!("Comment publish transaction failed: {e}"))?;

    // ── 6. Verify Content::PublishItem event ───────────────────────────────
    let published_item_id = events
        .find_first::<api::content::events::PublishItem>()
        .transpose()
        .map_err(|e| format!("Failed to decode Content::PublishItem event: {e}"))?
        .ok_or_else(|| {
            "Comment publish transaction completed without a Content::PublishItem event."
                .to_string()
        })?
        .item_id
        .0;

    if published_item_id != item_id {
        return Err(format!(
            "Comment publish created item {} but the dapp expected {}.",
            bytes32_to_hex(&published_item_id),
            bytes32_to_hex(&item_id),
        ));
    }

    Ok(PublishedComment {
        item_id,
        item_id_hex: bytes32_to_hex(&item_id),
    })
}

fn encode_comment_item(draft: &CommentDraft) -> Vec<u8> {
    ItemMessage {
        mixin_payload: vec![
            // Type-marker mixin (empty payload — presence is sufficient).
            MixinPayloadMessage {
                mixin_id: COMMENT_TYPE_MIXIN_ID,
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
                mixin_id: BODY_TEXT_MIXIN_ID,
                payload: BodyTextMixinMessage {
                    body_text: draft.body.clone(),
                }
                .encode_to_vec(),
            },
        ],
    }
    .encode_to_vec()
}

// ── Load ──────────────────────────────────────────────────────────────────────

/// Load all comments (direct children) of `item_id_hex`, recursively loading
/// their own children.
///
/// Steps:
/// 1. Query the indexer for all `Content::PublishItem` events keyed by
///    `item_id_hex`.
/// 2. For each child item ID (excluding the parent itself), fetch the IPFS
///    payload and check for `COMMENT_TYPE_MIXIN_ID`.
/// 3. Items that carry the mixin are comments; fetch their metadata and recurse.
///
/// The function is wrapped in `Box::pin` because it is mutually recursive with
/// `load_single_comment` (which calls back into this function for nested replies).
pub fn load_comments_for_item(
    item_id_hex: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<LoadedComment>, String>> + Send>>
{
    Box::pin(async move {
        let decoded_events = fetch_events_for_item(item_id_hex.clone()).await?;

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

        let mut comments = Vec::new();
        for child_id_hex in child_item_ids {
            match load_single_comment(child_id_hex).await {
                Ok(Some(comment)) => comments.push(comment),
                Ok(None) => {} // Not a comment item — skip.
                Err(_) => {}   // Skip items that fail to load.
            }
        }

        Ok(comments)
    })
}

/// Load a single comment item. Returns `None` if the item does not carry
/// `COMMENT_TYPE_MIXIN_ID` (i.e. it is a post or other content type).
///
/// Also wrapped in `Box::pin` because it calls `load_comments_for_item` which
/// in turn calls back here.
fn load_single_comment(
    item_id_hex: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<LoadedComment>, String>> + Send>>
{
    Box::pin(async move {
        // Fetch latest revision from IPFS.
        let revision_hash = fetch_latest_revision_hash(item_id_hex.clone()).await?;
        let item_bytes = fetch_ipfs_digest_bytes(&revision_hash).await?;
        let item = ItemMessage::decode(item_bytes.as_slice())
            .map_err(|e| format!("Failed to decode comment payload: {e}"))?;

        // Must have the comment type-marker mixin.
        let is_comment = item
            .mixin_payload
            .iter()
            .any(|m| m.mixin_id == COMMENT_TYPE_MIXIN_ID);
        if !is_comment {
            return Ok(None);
        }

        let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
            .map(|m| m.body_text)
            .unwrap_or_default();

        // Resolve owner address, revision_id, and revisionable flag from on-chain ItemState.
        let (owner_address, revision_id, is_revisionable) =
            fetch_comment_state(&item_id_hex).await.unwrap_or_default();

        let item_id_bytes = hex_to_bytes32(&item_id_hex)?;
        let encoded_item_id = bs58::encode(item_id_bytes).into_string();

        // Recurse: load replies to this comment.
        let children = load_comments_for_item(item_id_hex.clone())
            .await
            .unwrap_or_default();

        Ok(Some(LoadedComment {
            item_id: item_id_bytes,
            item_id_hex,
            encoded_item_id,
            body_text,
            owner_address,
            revision_id,
            is_revisionable,
            children,
        }))
    })
}

/// Fetch the SS58 owner address, current revision ID, and revisionable flag
/// for a comment item from the on-chain `Content.ItemState`.
///
/// Returns `(owner_address, revision_id, is_revisionable)`.
async fn fetch_comment_state(item_id_hex: &str) -> Result<(String, u32, bool), String> {
    let item_id_bytes = hex_to_bytes32(item_id_hex)?;
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to access block for comment state lookup: {e}"))?;

    let maybe_state = at_block
        .storage()
        .try_fetch(
            &api::storage().content().item_state(),
            (api::runtime_types::pallet_content::pallet::ItemId(item_id_bytes),),
        )
        .await
        .map_err(|e| format!("Failed to fetch comment ItemState: {e}"))?;

    let result = maybe_state
        .and_then(|encoded| encoded.decode().ok())
        .map(|state| {
            use sp_core::crypto::Ss58Codec;
            let account_id = sp_core::crypto::AccountId32::from(state.owner.0);
            let owner_address = account_id.to_ss58check();
            let revision_id = state.revision_id;
            let is_revisionable = state.flags & 0x01 != 0;
            (owner_address, revision_id, is_revisionable)
        })
        .unwrap_or_default();

    Ok(result)
}

// ── Revision ──────────────────────────────────────────────────────────────────

/// Load the full revision history for a comment item from the indexer.
///
/// This is a thin wrapper around `fetch_revision_history` from `content.rs`,
/// exposed here so that `CommentCard` can request it without importing
/// lower-level content helpers directly.
pub async fn load_comment_revision_history(
    item_id_hex: String,
) -> Result<Vec<RevisionEntry>, String> {
    fetch_revision_history(item_id_hex).await
}

/// Publish an edited revision of an existing comment.
///
/// Encodes a new IPFS payload with the updated `draft.body`, uploads it, and
/// submits `content.publish_revision(item_id, [], [], ipfs_hash)`.
pub async fn publish_comment_revision(
    account_store: &AccountStore,
    comment_item_id: [u8; 32],
    draft: CommentDraft,
) -> Result<(), String> {
    // ── 1. Validate account ────────────────────────────────────────────────
    let signer = account_store
        .active_account_id
        .as_deref()
        .and_then(|id| account_store.unlocked_signers.get(id))
        .cloned()
        .ok_or_else(|| "Unlock the active account before editing a comment.".to_string())?;

    // ── 2. Encode protobuf payload ─────────────────────────────────────────
    let item_payload = encode_comment_item(&draft);

    // ── 3. Upload to IPFS ──────────────────────────────────────────────────
    let revision_ipfs_hash = upload_ipfs_digest(&item_payload).await?;
    let revision_ipfs_hash_bytes = hex_to_bytes32(&revision_ipfs_hash)?;

    // ── 4. Submit content.publish_revision ────────────────────────────────
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to access latest block for comment revision: {e}"))?;

    let publish_revision_tx = api::tx().content().publish_revision(
        api::runtime_types::pallet_content::pallet::ItemId(comment_item_id),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::pallet_content::pallet::IpfsHash(revision_ipfs_hash_bytes),
    );

    at_block
        .tx()
        .sign_and_submit_then_watch_default(&publish_revision_tx, &signer)
        .await
        .map_err(|e| format!("Failed to submit comment revision transaction: {e}"))?
        .wait_for_finalized_success()
        .await
        .map_err(|e| format!("Comment revision transaction failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::decode_single_mixin;

    fn decode_item(bytes: &[u8]) -> ItemMessage {
        ItemMessage::decode(bytes).unwrap()
    }

    #[test]
    fn encode_comment_item_has_type_marker() {
        let draft = CommentDraft {
            body: "Hello!".to_string(),
        };
        let bytes = encode_comment_item(&draft);
        let item = decode_item(&bytes);

        assert_eq!(item.mixin_payload.len(), 3);
        assert!(item
            .mixin_payload
            .iter()
            .any(|m| m.mixin_id == COMMENT_TYPE_MIXIN_ID));
    }

    #[test]
    fn encode_comment_item_has_body_text() {
        let draft = CommentDraft {
            body: "This is my reply".to_string(),
        };
        let bytes = encode_comment_item(&draft);
        let item = decode_item(&bytes);

        let body = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
            .map(|m| m.body_text)
            .unwrap_or_default();
        assert_eq!(body, "This is my reply");
    }

    #[test]
    fn encode_comment_item_has_language() {
        let draft = CommentDraft::default();
        let bytes = encode_comment_item(&draft);
        let item = decode_item(&bytes);

        let lang = decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID)
            .map(|m| m.language_tag)
            .unwrap_or_default();
        assert_eq!(lang, DEFAULT_LANGUAGE_TAG);
    }

    #[test]
    fn encode_comment_item_no_title() {
        use crate::content::TitleMixinMessage;
        use crate::content::TITLE_MIXIN_ID;
        let draft = CommentDraft::default();
        let bytes = encode_comment_item(&draft);
        let item = decode_item(&bytes);

        let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID);
        assert!(title.is_none(), "Comments must not have a title mixin");
    }
}
