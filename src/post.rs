use crate::{acuity_runtime::api, runtime_client::connect as connect_acuity_client};
use prost::Message;
use rand::RngCore;

use crate::accounts::AccountStore;
use crate::content::{
    account_id_from_ss58, build_image_mixin, bytes32_to_hex, derive_item_id, hex_to_bytes32,
    upload_ipfs_digest, BodyTextMixinMessage, ItemMessage, LanguageMixinMessage,
    MixinPayloadMessage, SelectedImage, TitleMixinMessage, BODY_TEXT_MIXIN_ID,
    DEFAULT_LANGUAGE_TAG, IMAGE_MIXIN_ID, LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID,
};

/// Post items are revisionable and retractable.
const POST_ITEM_FLAGS: u8 = 0x03;

#[derive(Clone, PartialEq, Default)]
pub struct PostDraft {
    pub title: String,
    pub body: String,
}

#[derive(Clone)]
pub struct PublishPostRequest {
    pub draft: PostDraft,
    pub feed_item_id: [u8; 32],
    pub selected_image: Option<SelectedImage>,
}

#[derive(Clone, PartialEq)]
pub struct PublishedPost {
    pub item_id: [u8; 32],
    pub item_id_hex: String,
}

pub async fn publish_post(
    account_store: &AccountStore,
    request: PublishPostRequest,
) -> Result<PublishedPost, String> {
    // ── 1. Validate account is unlocked ────────────────────────────────────
    let active_account = account_store
        .active_account()
        .ok_or_else(|| "Select an account before publishing a post.".to_string())?
        .clone();
    let signer = account_store
        .active_account_id
        .as_deref()
        .and_then(|id| account_store.unlocked_signers.get(id))
        .cloned()
        .ok_or_else(|| "Unlock the active account before publishing a post.".to_string())?;
    let account_id = account_id_from_ss58(&active_account.address)?;

    // ── 2. Build image mixin if an image was selected ──────────────────────
    let image_payload = match request.selected_image {
        Some(ref selected_image) => {
            let built = build_image_mixin(selected_image).await?;
            Some(built.payload)
        }
        None => None,
    };

    // ── 3. Encode post item payload (protobuf) ─────────────────────────────
    let item_payload = encode_post_item(&request.draft, image_payload);

    // ── 4. Upload to IPFS ──────────────────────────────────────────────────
    let revision_ipfs_hash = upload_ipfs_digest(&item_payload).await?;
    let revision_ipfs_hash_bytes = hex_to_bytes32(&revision_ipfs_hash)?;

    // ── 5. Derive item ID ──────────────────────────────────────────────────
    let mut nonce = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let item_id = derive_item_id(account_id, nonce);

    // ── 6. Submit content.publish_item with feed as parent ─────────────────
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access the latest block for transactions: {error}"))?;
    let mut tx_client = at_block.tx();

    let publish_item_tx = api::tx().content().publish_item(
        api::runtime_types::pallet_content::Nonce(nonce),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![
            api::runtime_types::pallet_content::pallet::ItemId(request.feed_item_id),
        ]),
        POST_ITEM_FLAGS,
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::pallet_content::pallet::IpfsHash(revision_ipfs_hash_bytes),
    );

    let events = tx_client
        .sign_and_submit_then_watch_default(&publish_item_tx, &signer)
        .await
        .map_err(|error| format!("Failed to submit post publish transaction: {error}"))?
        .wait_for_finalized_success()
        .await
        .map_err(|error| format!("Post publish transaction failed: {error}"))?;

    // ── 7. Verify Content::PublishItem event ───────────────────────────────
    let published_item_id = events
        .find_first::<api::content::events::PublishItem>()
        .transpose()
        .map_err(|error| format!("Failed to decode Content::PublishItem event: {error}"))?
        .ok_or_else(|| {
            "Post publish transaction completed without emitting Content::PublishItem.".to_string()
        })?
        .item_id
        .0;

    if published_item_id != item_id {
        return Err(format!(
            "Post publish transaction created item {} but the dapp expected {}.",
            bytes32_to_hex(&published_item_id),
            bytes32_to_hex(&item_id),
        ));
    }

    Ok(PublishedPost {
        item_id,
        item_id_hex: bytes32_to_hex(&item_id),
    })
}

fn encode_post_item(draft: &PostDraft, image_payload: Option<Vec<u8>>) -> Vec<u8> {
    let mut item = ItemMessage {
        mixin_payload: vec![
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
                    body_text: draft.body.clone(),
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
    fn encode_post_item_includes_content_mixins() {
        let draft = PostDraft {
            title: "My Post".to_string(),
            body: "Post body text".to_string(),
        };

        let item = decode_item(&encode_post_item(&draft, None));

        assert_eq!(item.mixin_payload.len(), 3);

        assert_eq!(
            decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID),
            Some(LanguageMixinMessage {
                language_tag: "en".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID),
            Some(TitleMixinMessage {
                title: "My Post".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID),
            Some(BodyTextMixinMessage {
                body_text: "Post body text".to_string(),
            })
        );
        assert_eq!(
            decode_single_mixin::<ImageMixinMessage>(&item, IMAGE_MIXIN_ID),
            None
        );
    }

    #[test]
    fn encode_post_item_includes_optional_image() {
        let draft = PostDraft::default();
        let image_payload = vec![10, 20, 30];

        let item = decode_item(&encode_post_item(&draft, Some(image_payload.clone())));

        assert_eq!(item.mixin_payload.len(), 4);
        let image_mixin = item
            .mixin_payload
            .iter()
            .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
            .unwrap();
        assert_eq!(image_mixin.payload, image_payload);
    }
}
