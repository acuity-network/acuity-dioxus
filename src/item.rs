use crate::{
    accounts::AccountStore,
    acuity_runtime::api,
    content::{
        build_image_mixin, hex_to_bytes32, upload_ipfs_digest, BodyTextMixinMessage, ItemMessage,
        LanguageMixinMessage, MixinPayloadMessage, SelectedImage, TitleMixinMessage,
        BODY_TEXT_MIXIN_ID, DEFAULT_LANGUAGE_TAG, IMAGE_MIXIN_ID, LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID,
    },
    feed::FEED_TYPE_MIXIN_ID,
    profile::PROFILE_MIXIN_ID,
    runtime_client::connect as connect_acuity_client,
};
use prost::Message;

// ── Types ─────────────────────────────────────────────────────────────────────

/// The editable text fields of any item (title + body).
#[derive(Clone, PartialEq, Default)]
pub struct ItemDraft {
    pub title: String,
    pub body: String,
}

/// Input to [`publish_item_revision`].
pub struct PublishRevisionRequest {
    /// On-chain item ID (32-byte array).
    pub item_id: [u8; 32],
    /// Content type string: `"Feed"`, `"Profile"`, or `"Content"`.  Used to
    /// preserve the type-marker mixin in the revised payload.
    pub content_type: String,
    /// Edited title and body text.
    pub draft: ItemDraft,
    /// A newly-chosen image file to encode and upload, if any.
    pub selected_image: Option<SelectedImage>,
    /// The raw image mixin payload bytes from the current published revision.
    /// Retained so the edit form can keep the existing image when no new image
    /// is selected.
    pub existing_image_payload: Option<Vec<u8>>,
}

// ── Encode helper ─────────────────────────────────────────────────────────────

/// Builds a revised protobuf item payload, preserving the type-marker mixin
/// (Feed or Profile) from the original content type, and replacing title,
/// body, and optionally image with the draft values.
pub fn encode_revised_item(
    content_type: &str,
    draft: &ItemDraft,
    image_payload: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut mixins: Vec<MixinPayloadMessage> = Vec::new();

    // Preserve the type-marker mixin at the front if this is a Feed or Profile.
    if content_type == "Feed" {
        mixins.push(MixinPayloadMessage {
            mixin_id: FEED_TYPE_MIXIN_ID,
            payload: vec![],
        });
    } else if content_type == "Profile" {
        // For profile items we keep an empty profile mixin to preserve the
        // type marker; account_type / location edits are out of scope here.
        mixins.push(MixinPayloadMessage {
            mixin_id: PROFILE_MIXIN_ID,
            payload: vec![],
        });
    }

    mixins.push(MixinPayloadMessage {
        mixin_id: LANGUAGE_MIXIN_ID,
        payload: LanguageMixinMessage {
            language_tag: DEFAULT_LANGUAGE_TAG.to_string(),
        }
        .encode_to_vec(),
    });

    mixins.push(MixinPayloadMessage {
        mixin_id: TITLE_MIXIN_ID,
        payload: TitleMixinMessage {
            title: draft.title.clone(),
        }
        .encode_to_vec(),
    });

    mixins.push(MixinPayloadMessage {
        mixin_id: BODY_TEXT_MIXIN_ID,
        payload: BodyTextMixinMessage {
            body_text: draft.body.clone(),
        }
        .encode_to_vec(),
    });

    if let Some(image_payload) = image_payload {
        mixins.push(MixinPayloadMessage {
            mixin_id: IMAGE_MIXIN_ID,
            payload: image_payload,
        });
    }

    ItemMessage {
        mixin_payload: mixins,
    }
    .encode_to_vec()
}

// ── publish_item_revision ─────────────────────────────────────────────────────

/// Encodes a revised item payload, uploads it to IPFS, and submits a
/// `content::publish_revision` extrinsic.  Returns `Ok(())` on success.
///
/// This follows the same pattern as [`crate::post::publish_post`],
/// [`crate::feed::publish_feed`], and [`crate::profile::save_profile`].
pub async fn publish_item_revision(
    store: &AccountStore,
    req: PublishRevisionRequest,
) -> Result<(), String> {
    // Resolve the signer for the active account.
    let signer = store
        .active_account_id
        .as_deref()
        .and_then(|id| store.unlocked_signers.get(id))
        .cloned()
        .ok_or_else(|| "Unlock the active account before saving.".to_string())?;

    // Build image payload: encode new image if one was chosen, otherwise keep
    // the existing payload bytes (which may be None if no image was ever set).
    let image_payload = match req.selected_image {
        Some(ref img) => {
            let built = build_image_mixin(img).await?;
            Some(built.payload)
        }
        None => req.existing_image_payload,
    };

    // Encode the revised protobuf payload.
    let item_payload = encode_revised_item(&req.content_type, &req.draft, image_payload);

    // Upload to IPFS and get the sha2-256 digest hex.
    let revision_ipfs_hash = upload_ipfs_digest(&item_payload).await?;
    let revision_ipfs_hash_bytes = hex_to_bytes32(&revision_ipfs_hash)?;

    // Connect and submit the `publish_revision` extrinsic.
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to access latest block: {e}"))?;

    let publish_revision_tx = api::tx().content().publish_revision(
        api::runtime_types::pallet_content::pallet::ItemId(req.item_id),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
        api::runtime_types::pallet_content::pallet::IpfsHash(revision_ipfs_hash_bytes),
    );

    at_block
        .tx()
        .sign_and_submit_then_watch_default(&publish_revision_tx, &signer)
        .await
        .map_err(|e| format!("Failed to submit revision: {e}"))?
        .wait_for_finalized_success()
        .await
        .map_err(|e| format!("Revision transaction failed: {e}"))?;

    Ok(())
}
