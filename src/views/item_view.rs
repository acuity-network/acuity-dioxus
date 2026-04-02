use crate::{
    acuity_runtime::api,
    accounts::AccountStore,
    content::{
        build_image_mixin, bytes32_to_hex, decode_single_mixin, fetch_events_for_item,
        fetch_ipfs_digest_bytes, fetch_latest_revision_hash, hex_to_bytes32,
        preview_data_url_for_image_mixin, preview_data_url_for_path, upload_ipfs_digest,
        BodyTextMixinMessage, IndexerStoredEvent, ItemMessage, LanguageMixinMessage,
        MixinPayloadMessage, SelectedImage, TitleMixinMessage, BODY_TEXT_MIXIN_ID, IMAGE_MIXIN_ID,
        LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID, DEFAULT_LANGUAGE_TAG,
    },
    feed::FEED_TYPE_MIXIN_ID,
    profile::PROFILE_MIXIN_ID,
    runtime_client::connect as connect_acuity_client,
    Route,
};
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use prost::Message;
use rfd::FileDialog;
use sp_core::crypto::Ss58Codec;
use std::collections::HashMap;

const ITEM_VIEW_CSS: Asset = asset!("/assets/styling/item_view.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

fn short_hex(value: &str) -> String {
    if value.len() <= 18 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..10], &value[value.len() - 8..])
    }
}

fn content_type_label(item: &ItemMessage) -> &'static str {
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

#[derive(Clone, PartialEq, Default)]
struct LoadedItem {
    encoded_item_id: String,
    item_id: [u8; 32],
    item_id_hex: String,
    revision_ipfs_hash_hex: String,
    content_type: String,
    title: String,
    body_text: String,
    language: String,
    image_preview_data_url: Option<String>,
    /// Raw image mixin payload bytes from IPFS — retained so the edit form can
    /// keep the existing image when no new image is selected.
    existing_image_payload: Option<Vec<u8>>,
    parents: Vec<ParentSummary>,
    /// SS58 address of the on-chain item owner (from Content.ItemState storage).
    owner_address: String,
    /// Current revision ID from Content.ItemState — used by the reactions pallet.
    revision_id: u32,
}

// ── Reaction types ─────────────────────────────────────────────────────────────

/// Unicode codepoints matching the emoji set from the original Vue browser.
const AVAILABLE_EMOJI_CODEPOINTS: &[u32] = &[
    0x1F44D, // 👍
    0x1F44E, // 👎
    0x1F60D, // 😍
    0x1F618, // 😘
    0x1F61C, // 😜
    0x1F911, // 🤑
    0x1F92B, // 🤫
    0x1F914, // 🤔
    0x1F910, // 🤐
    0x1F62C, // 😬
    0x1F925, // 🤥
    0x1F915, // 🤕
    0x1F922, // 🤢
    0x1F603, // 😃
    0x1F60E, // 😎
    0x1F913, // 🤓
    0x1F9D0, // 🧐
    0x1F62D, // 😭
    0x1F621, // 😡
    0x1F4AF, // 💯
    0x1F4A4, // 💤
    0x1F44C, // 👌
    0x1F91E, // 🤞
    0x1F44F, // 👏
    0x1F64F, // 🙏
    0x1F9D9, // 🧙
];

#[derive(Clone, PartialEq)]
struct ReactionSummary {
    /// The rendered emoji character(s).
    emoji_char: String,
    /// Unicode scalar value — used as the on-chain `Emoji(u32)` argument.
    codepoint: u32,
    /// Total number of accounts that reacted with this emoji.
    count: usize,
    /// SS58 addresses of all reactors (shown in tooltip).
    reactors: Vec<String>,
    /// Whether the currently active account has already reacted with this emoji.
    i_reacted: bool,
}

#[derive(Clone, PartialEq)]
struct ParentSummary {
    encoded_item_id: String,
    title: String,
    content_type: String,
}

#[derive(Clone, PartialEq)]
struct FeedPost {
    encoded_item_id: String,
    title: String,
    body_preview: String,
    image_preview_data_url: Option<String>,
}

#[derive(Clone, PartialEq)]
enum ActiveTab {
    View,
    Edit,
}

#[derive(Clone, PartialEq, Default)]
struct ItemDraft {
    title: String,
    body: String,
}

async fn load_item(encoded_item_id: &str) -> Result<LoadedItem, String> {
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
    let revision_ipfs_hash = fetch_latest_revision_hash(item_id_hex.clone()).await?;
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

    let language = decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID)
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
    let parents = load_parent_summaries(&item_id_hex).await.unwrap_or_default();

    // ── Query item owner + revision_id from on-chain Content.ItemState ────
    let (owner_address, revision_id) = fetch_item_state(item_id).await.unwrap_or_default();

    Ok(LoadedItem {
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
        revision_id,
    })
}

/// Queries `Content.ItemState` on-chain and returns `(owner_ss58, revision_id)`.
async fn fetch_item_state(item_id: [u8; 32]) -> Result<(String, u32), String> {
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

/// Fetches all reactions for a given `(item_id, revision_id)` by iterating
/// the `ContentReactions::ItemAccountReactions` storage map with a 2-key
/// prefix.  Each entry maps one reactor account to its set of emoji reactions.
async fn fetch_reactions(
    item_id: [u8; 32],
    revision_id: u32,
    active_address: Option<String>,
) -> Result<Vec<ReactionSummary>, String> {
    let client = connect_acuity_client().await?;
    let at_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to access latest block for reactions: {error}"))?;

    let storage_addr = api::storage().content_reactions().item_account_reactions();

    let mut entries = at_block
        .storage()
        .iter(
            storage_addr,
            (
                api::runtime_types::pallet_content::pallet::ItemId(item_id),
                revision_id,
            ),
        )
        .await
        .map_err(|error| format!("Failed to start reactions storage iteration: {error}"))?;

    // Map from codepoint → (count, reactors, i_reacted)
    let mut map: HashMap<u32, (usize, Vec<String>, bool)> = HashMap::new();

    while let Some(result) = entries.next().await {
        let entry = result.map_err(|error| format!("Failed to read reaction entry: {error}"))?;

        // Decode the reactor AccountId32 from the last 32 bytes of the storage key.
        // For Blake2_128Concat keys the layout is: 16-byte hash || raw key bytes.
        // The third key component (AccountId32) occupies the last 32 raw bytes.
        let key_bytes = entry.key_bytes();
        let reactor_address = if key_bytes.len() >= 32 {
            let start = key_bytes.len() - 32;
            let account_bytes: [u8; 32] = key_bytes[start..].try_into().unwrap_or([0u8; 32]);
            let sp_account = sp_core::crypto::AccountId32::from(account_bytes);
            sp_account.to_ss58check()
        } else {
            String::new()
        };

        let emojis = entry
            .value()
            .decode()
            .map_err(|error| format!("Failed to decode emoji list: {error}"))?;

        let i_am_reactor = active_address
            .as_deref()
            .map(|addr| addr == reactor_address)
            .unwrap_or(false);

        for emoji in &emojis.0 {
            let codepoint = emoji.0;
            let entry = map.entry(codepoint).or_insert((0, Vec::new(), false));
            entry.0 += 1;
            if !reactor_address.is_empty() {
                entry.1.push(reactor_address.clone());
            }
            if i_am_reactor {
                entry.2 = true;
            }
        }
    }

    // Build sorted output (preserve AVAILABLE_EMOJI_CODEPOINTS order first, then unknowns).
    let mut summaries: Vec<ReactionSummary> = map
        .into_iter()
        .filter_map(|(codepoint, (count, reactors, i_reacted))| {
            let emoji_char = char::from_u32(codepoint)?.to_string();
            Some(ReactionSummary {
                emoji_char,
                codepoint,
                count,
                reactors,
                i_reacted,
            })
        })
        .collect();

    summaries.sort_by_key(|r| {
        AVAILABLE_EMOJI_CODEPOINTS
            .iter()
            .position(|&c| c == r.codepoint)
            .unwrap_or(usize::MAX)
    });

    Ok(summaries)
}

/// Finds this item's own `Content::PublishItem` event in the indexer and
/// returns a lightweight summary for each declared parent.  Parents that
/// fail to load are silently skipped.
async fn load_parent_summaries(item_id_hex: &str) -> Result<Vec<ParentSummary>, String> {
    let decoded_events = fetch_events_for_item(item_id_hex.to_string()).await?;

    // Find the PublishItem event whose item_id matches this item (not a child).
    let mut parent_hex_ids: Vec<String> = Vec::new();
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

        let event_item_id = event
            .fields
            .get("item_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        // Only process the event that belongs to *this* item, not child items.
        if event_item_id != item_id_hex {
            continue;
        }

        // Extract the parents array — the indexer stores it as a JSON array
        // of hex strings or objects with an inner value field.
        if let Some(parents_val) = event.fields.get("parents") {
            if let Some(arr) = parents_val.as_array() {
                for entry in arr {
                    // Try plain string first, then {"0": "0xabc..."} or nested.
                    let hex = if let Some(s) = entry.as_str() {
                        s.to_string()
                    } else if let Some(s) = entry.get("0").and_then(|v| v.as_str()) {
                        s.to_string()
                    } else {
                        continue;
                    };
                    if !hex.is_empty() {
                        parent_hex_ids.push(hex);
                    }
                }
            }
        }

        // There is only one PublishItem event for this item; stop after it.
        break;
    }

    let mut summaries = Vec::new();
    for parent_hex in parent_hex_ids {
        match load_parent_summary(&parent_hex).await {
            Ok(summary) => summaries.push(summary),
            Err(_) => continue,
        }
    }

    Ok(summaries)
}

async fn load_parent_summary(item_id_hex: &str) -> Result<ParentSummary, String> {
    let revision_hash = fetch_latest_revision_hash(item_id_hex.to_string()).await?;
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

/// Loads child posts for a feed by querying the indexer for all events
/// keyed by the feed's item_id, then filtering for `Content::PublishItem`
/// events where this feed appears as a parent.
async fn load_feed_posts(item_id_hex: &str) -> Result<Vec<FeedPost>, String> {
    let decoded_events = fetch_events_for_item(item_id_hex.to_string()).await?;

    // Collect child item IDs from PublishItem events where this feed is a parent
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

    // Load each child post's content from IPFS
    let mut posts = Vec::new();
    for child_id_hex in child_item_ids {
        let post = load_single_post(&child_id_hex).await;
        match post {
            Ok(p) => posts.push(p),
            Err(_) => continue, // Skip posts that fail to load
        }
    }

    Ok(posts)
}

async fn load_single_post(item_id_hex: &str) -> Result<FeedPost, String> {
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

    // Truncate body for preview
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

    // Convert hex item_id to base58 for the URL
    let item_id_bytes = hex_to_bytes32(item_id_hex)?;
    let encoded_item_id = bs58::encode(item_id_bytes).into_string();

    Ok(FeedPost {
        encoded_item_id,
        title,
        body_preview,
        image_preview_data_url,
    })
}

// ── Reactions component ───────────────────────────────────────────────────────

#[component]
fn Reactions(item_id: [u8; 32], revision_id: u32) -> Element {
    let account_store = use_context::<Signal<AccountStore>>();

    let mut reactions: Signal<Vec<ReactionSummary>> = use_signal(Vec::new);
    let mut reactions_loading = use_signal(|| false);
    let mut reactions_error: Signal<Option<String>> = use_signal(|| None);
    let mut show_picker = use_signal(|| false);
    let mut is_submitting = use_signal(|| false);
    let mut tx_error: Signal<Option<String>> = use_signal(|| None);
    let mut reload_tick = use_signal(|| 0_u64);

    // Load reactions whenever item_id, revision_id, or reload_tick changes.
    use_effect(move || {
        let _tick = reload_tick();
        let active_address = account_store()
            .active_account()
            .map(|a| a.address.clone());
        spawn(async move {
            reactions_loading.set(true);
            reactions_error.set(None);
            match fetch_reactions(item_id, revision_id, active_address).await {
                Ok(r) => reactions.set(r),
                Err(e) => reactions_error.set(Some(e)),
            }
            reactions_loading.set(false);
        });
    });

    // Helper: submit a transaction (add or remove reaction).
    let mut submit_tx = move |codepoint: u32, remove: bool| {
        let store_snap = account_store();
        let signer = store_snap
            .active_account_id
            .as_deref()
            .and_then(|id| store_snap.unlocked_signers.get(id))
            .cloned();
        let Some(signer) = signer else {
            tx_error.set(Some(
                "Unlock the active account to react.".to_string(),
            ));
            return;
        };

        spawn(async move {
            is_submitting.set(true);
            tx_error.set(None);
            show_picker.set(false);

            let result: Result<(), String> = async {
                let client = connect_acuity_client().await?;
                let at_block = client
                    .at_current_block()
                    .await
                    .map_err(|e| format!("Failed to access latest block: {e}"))?;

                let item_id_param =
                    api::runtime_types::pallet_content::pallet::ItemId(item_id);
                let emoji_param =
                    api::runtime_types::pallet_content_reactions::pallet::Emoji(codepoint);

                if remove {
                    at_block
                        .tx()
                        .sign_and_submit_then_watch_default(
                            &api::tx()
                                .content_reactions()
                                .remove_reaction(item_id_param, revision_id, emoji_param),
                            &signer,
                        )
                        .await
                        .map_err(|e| format!("Failed to submit reaction: {e}"))?
                        .wait_for_finalized_success()
                        .await
                        .map_err(|e| format!("Reaction transaction failed: {e}"))?;
                } else {
                    at_block
                        .tx()
                        .sign_and_submit_then_watch_default(
                            &api::tx()
                                .content_reactions()
                                .add_reaction(item_id_param, revision_id, emoji_param),
                            &signer,
                        )
                        .await
                        .map_err(|e| format!("Failed to submit reaction: {e}"))?
                        .wait_for_finalized_success()
                        .await
                        .map_err(|e| format!("Reaction transaction failed: {e}"))?;
                }

                Ok(())
            }
            .await;

            if let Err(e) = result {
                tx_error.set(Some(e));
            } else {
                reload_tick.with_mut(|t| *t += 1);
            }
            is_submitting.set(false);
        });
    };

    // Which emoji the active account has already reacted with.
    let reacted_codepoints: Vec<u32> = reactions()
        .iter()
        .filter(|r| r.i_reacted)
        .map(|r| r.codepoint)
        .collect();

    // Emojis available to add (not yet reacted by this account).
    let picker_emojis: Vec<(u32, String)> = AVAILABLE_EMOJI_CODEPOINTS
        .iter()
        .filter(|&&cp| !reacted_codepoints.contains(&cp))
        .filter_map(|&cp| Some((cp, char::from_u32(cp)?.to_string())))
        .collect();

    rsx! {
        div {
            class: "reactions-section",

            if let Some(err) = tx_error() {
                div { class: "status-bar error reactions-tx-error", "{err}" }
            }

            div {
                class: "reactions-bar",

                // Existing reaction chips
                for reaction in reactions() {
                    {
                        let cp = reaction.codepoint;
                        let removing = reaction.i_reacted;
                        let tooltip = reaction.reactors.join(", ");
                        let chip_class = if reaction.i_reacted {
                            "reaction-chip reacted"
                        } else {
                            "reaction-chip"
                        };
                        rsx! {
                            button {
                                class: chip_class,
                                title: "{tooltip}",
                                disabled: is_submitting(),
                                onclick: move |_| submit_tx(cp, removing),
                                "{reaction.emoji_char} {reaction.count}"
                            }
                        }
                    }
                }

                // "+" button to open/close the picker
                if !picker_emojis.is_empty() {
                    button {
                        class: "reaction-add",
                        disabled: is_submitting(),
                        onclick: move |_| show_picker.with_mut(|v| *v = !*v),
                        "+"
                    }
                }
            }

            // Emoji picker dropdown
            if show_picker() {
                div {
                    class: "reaction-picker",
                    for (cp, ch) in picker_emojis {
                        button {
                            class: "picker-emoji",
                            disabled: is_submitting(),
                            onclick: move |_| submit_tx(cp, false),
                            "{ch}"
                        }
                    }
                }
            }

            if reactions_loading() {
                p { class: "reactions-loading", "Loading reactions..." }
            }

            if let Some(err) = reactions_error() {
                p { class: "reactions-loading", "Reactions unavailable: {err}" }
            }
        }
    }
}

/// Builds a revised protobuf item payload, preserving the type-marker mixin
/// (Feed or Profile) from the original content type, and replacing title,
/// body, and optionally image with the draft values.
fn encode_revised_item(
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

#[component]
pub fn ItemView(encoded_item_id: String) -> Element {
    let account_store = use_context::<Signal<AccountStore>>();

    let mut loaded: Signal<Option<LoadedItem>> = use_signal(|| None);
    let mut is_loading = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);

    // Feed posts state (only used when the item is a Feed)
    let mut feed_posts: Signal<Vec<FeedPost>> = use_signal(Vec::new);
    let mut posts_loading = use_signal(|| false);
    let mut posts_error: Signal<Option<String>> = use_signal(|| None);

    // ── Edit tab state ──────────────────────────────────────────────────────
    let mut active_tab = use_signal(|| ActiveTab::View);
    let mut draft = use_signal(ItemDraft::default);
    let mut selected_image = use_signal(|| None::<SelectedImage>);
    let mut drag_over = use_signal(|| false);
    let mut is_saving = use_signal(|| false);
    let mut save_error: Signal<Option<String>> = use_signal(|| None);
    let mut save_notice: Signal<Option<String>> = use_signal(|| None);
    // Incrementing this signal re-triggers the load effect after a save.
    let mut reload_tick = use_signal(|| 0_u64);

    let encoded_id = use_memo({
        let encoded_item_id = encoded_item_id.clone();
        move || encoded_item_id.clone()
    });

    use_effect(move || {
        let id = encoded_id();
        let _tick = reload_tick();
        spawn(async move {
            error_message.set(None);
            posts_error.set(None);
            feed_posts.set(Vec::new());
            is_loading.set(true);
            match load_item(&id).await {
                Ok(item) => {
                    let is_feed = item.content_type == "Feed";
                    let item_id_hex = item.item_id_hex.clone();
                    // Pre-populate edit draft from loaded content.
                    draft.set(ItemDraft {
                        title: item.title.clone(),
                        body: item.body_text.clone(),
                    });
                    selected_image.set(None);
                    active_tab.set(ActiveTab::View);
                    loaded.set(Some(item));
                    is_loading.set(false);

                    // If this is a feed, load its child posts
                    if is_feed {
                        posts_loading.set(true);
                        match load_feed_posts(&item_id_hex).await {
                            Ok(posts) => feed_posts.set(posts),
                            Err(err) => posts_error.set(Some(err)),
                        }
                        posts_loading.set(false);
                    }
                }
                Err(err) => {
                    loaded.set(None);
                    error_message.set(Some(err));
                    is_loading.set(false);
                }
            }
        });
    });

    // True when the active account is the on-chain owner of this item.
    let is_owner = use_memo(move || {
        let Some(ref item) = loaded() else {
            return false;
        };
        account_store()
            .active_account()
            .map(|a| a.address == item.owner_address)
            .unwrap_or(false)
    });

    // Edit status bar: error > saving > notice
    let edit_status: Option<(&'static str, String)> = if let Some(ref err) = save_error() {
        Some(("error", err.clone()))
    } else if is_saving() {
        Some((
            "loading",
            "Publishing the updated revision to IPFS and the chain...".to_string(),
        ))
    } else {
        save_notice().map(|n| ("notice", n))
    };

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }
        document::Link { rel: "stylesheet", href: ITEM_VIEW_CSS }

        div {
            class: "item-view-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    p { class: "page-eyebrow", "Content item" }
                    h1 { class: "page-title",
                        if let Some(ref item) = loaded() {
                            if item.title.trim().is_empty() {
                                "Untitled item"
                            } else {
                                "{item.title}"
                            }
                        } else {
                            "Item"
                        }
                    }
                }
            }

            // ── Load status bar ────────────────────────────────────────────
            if let Some(err) = error_message() {
                div { class: "status-bar error", "{err}" }
            } else if is_loading() {
                div { class: "status-bar loading", "Loading item from the indexer and IPFS..." }
            }

            // ── Tab bar (only when item is loaded and owned by active account) ──
            if loaded().is_some() && is_owner() {
                div {
                    class: "iv-tab-bar",
                    button {
                        class: if active_tab() == ActiveTab::View { "iv-tab active" } else { "iv-tab" },
                        onclick: move |_| {
                            save_error.set(None);
                            save_notice.set(None);
                            active_tab.set(ActiveTab::View);
                        },
                        "View"
                    }
                    button {
                        class: if active_tab() == ActiveTab::Edit { "iv-tab active" } else { "iv-tab" },
                        onclick: move |_| {
                            save_error.set(None);
                            save_notice.set(None);
                            active_tab.set(ActiveTab::Edit);
                        },
                        "Edit"
                    }
                }
            }

            if is_loading() {
                div { class: "item-view-grid",
                    div { class: "panel-surface item-view-main skeleton-block" }
                    div { class: "panel-surface item-view-side skeleton-block" }
                }
            } else if let Some(item) = loaded() {

                // ── View tab ───────────────────────────────────────────────
                if active_tab() == ActiveTab::View {
                    div {
                        class: "item-view-grid",

                        // ── Left: content ──────────────────────────────────
                        section {
                            class: "panel-surface item-view-main",

                            // Parent links
                            if !item.parents.is_empty() {
                                div { class: "iv-parents",
                                    span { class: "iv-parents-label", "In" }
                                    for parent in item.parents.clone() {
                                        Link {
                                            class: "iv-parent-link",
                                            to: Route::ItemView {
                                                encoded_item_id: parent.encoded_item_id.clone(),
                                            },
                                            span { class: "iv-parent-type", "{parent.content_type}" }
                                            span { class: "iv-parent-title", "{parent.title}" }
                                        }
                                    }
                                }
                            }

                            // Image
                            if let Some(img_url) = item.image_preview_data_url.clone() {
                                img {
                                    class: "iv-image",
                                    src: img_url,
                                    alt: "Item image",
                                }
                            }

                            // Type pill
                            span { class: "iv-type-pill", "{item.content_type}" }

                            // Title
                            if !item.title.trim().is_empty() {
                                h2 { class: "iv-title", "{item.title}" }
                            }

                            // Body text
                            if !item.body_text.trim().is_empty() {
                                p { class: "iv-body", "{item.body_text}" }
                            }

                            // Empty content notice
                            if item.title.trim().is_empty() && item.body_text.trim().is_empty() {
                                p { class: "pv-notice",
                                    "This item has no title or body text."
                                }
                            }

                            // ── Reactions ──────────────────────────────────
                            Reactions {
                                item_id: item.item_id,
                                revision_id: item.revision_id,
                            }
                        }

                        // ── Right: metadata ────────────────────────────────
                        aside {
                            class: "panel-surface item-view-side",

                            div { class: "pv-meta-section",
                                p { class: "pv-section-label", "Item metadata" }
                                div { class: "metadata-list",
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Item ID" }
                                        code { class: "meta-code",
                                            "{short_hex(&item.item_id_hex)}"
                                        }
                                    }
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Latest revision" }
                                        code { class: "meta-code",
                                            "{short_hex(&item.revision_ipfs_hash_hex)}"
                                        }
                                    }
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Type" }
                                        span { class: "meta-copy", "{item.content_type}" }
                                    }
                                    if !item.language.is_empty() {
                                        div { class: "metadata-row",
                                            span { class: "meta-label", "Language" }
                                            span { class: "meta-copy", "{item.language}" }
                                        }
                                    }
                                }
                            }

                            // ── Publish Post button (feeds only) ──────────
                            if item.content_type == "Feed" {
                                div { class: "pv-meta-section",
                                    Link {
                                        class: "btn-primary",
                                        to: Route::PublishPost {
                                            encoded_feed_id: item.encoded_item_id.clone(),
                                        },
                                        "Publish post"
                                    }
                                }
                            }
                        }
                    }

                    // ── Feed posts section (feeds only) ───────────────────
                    if item.content_type == "Feed" {
                        section {
                            class: "iv-feed-posts-section",

                            h3 { class: "iv-feed-posts-heading", "Posts" }

                            if posts_loading() {
                                div { class: "status-bar loading", "Loading posts..." }
                            }

                            if let Some(err) = posts_error() {
                                div { class: "status-bar error", "{err}" }
                            }

                            if !posts_loading() && feed_posts().is_empty() && posts_error().is_none() {
                                p { class: "pv-notice", "No posts in this feed yet." }
                            }

                            for post in feed_posts() {
                                Link {
                                    class: "iv-feed-post-card panel-surface",
                                    to: Route::ItemView {
                                        encoded_item_id: post.encoded_item_id.clone(),
                                    },

                                    if let Some(img_url) = post.image_preview_data_url.clone() {
                                        img {
                                            class: "iv-feed-post-thumb",
                                            src: img_url,
                                            alt: "Post image",
                                        }
                                    }

                                    div { class: "iv-feed-post-text",
                                        if !post.title.trim().is_empty() {
                                            h4 { class: "iv-feed-post-title", "{post.title}" }
                                        }
                                        if !post.body_preview.trim().is_empty() {
                                            p { class: "iv-feed-post-body", "{post.body_preview}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                // ── Edit tab ───────────────────────────────────────────────
                } else {
                    section {
                        class: "panel-surface iv-edit-editor",

                        // Edit status bar
                        if let Some((kind, msg)) = edit_status {
                            div { class: "status-bar {kind}", "{msg}" }
                        }

                        // ── Title ──────────────────────────────────────────
                        label { class: "field",
                            span { "Title" }
                            input {
                                value: draft().title,
                                placeholder: "Item title",
                                disabled: is_saving(),
                                oninput: move |e| draft.with_mut(|d| d.title = e.value()),
                            }
                        }

                        // ── Body ───────────────────────────────────────────
                        label { class: "field",
                            span { "Body" }
                            textarea {
                                value: draft().body,
                                rows: "10",
                                placeholder: "Item body text",
                                disabled: is_saving(),
                                oninput: move |e| draft.with_mut(|d| d.body = e.value()),
                            }
                        }

                        // ── Image ──────────────────────────────────────────
                        div { class: "field",
                            span { "Image (optional)" }
                            div {
                                class: if drag_over() {
                                    "drop-zone drop-zone-active"
                                } else if selected_image()
                                    .and_then(|img| img.preview_data_url.clone())
                                    .or_else(|| item.image_preview_data_url.clone())
                                    .is_some()
                                {
                                    "drop-zone drop-zone-has-image"
                                } else {
                                    "drop-zone"
                                },
                                onclick: move |_| {
                                    if is_saving() { return; }
                                    if let Some(path) = FileDialog::new()
                                        .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                                        .pick_file()
                                    {
                                        let preview = preview_data_url_for_path(&path).ok();
                                        let file_name = path
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("image")
                                            .to_string();
                                        selected_image.set(Some(SelectedImage {
                                            path: path.display().to_string(),
                                            file_name,
                                            preview_data_url: preview,
                                        }));
                                    }
                                },
                                ondragover: move |e: DragEvent| {
                                    e.prevent_default();
                                    drag_over.set(true);
                                },
                                ondragleave: move |_| drag_over.set(false),
                                ondrop: move |e: DragEvent| {
                                    e.prevent_default();
                                    drag_over.set(false);
                                    let file_list = e.files();
                                    if let Some(first) = file_list.first() {
                                        let path = first.path();
                                        let preview = preview_data_url_for_path(&path).ok();
                                        let file_name = first.name();
                                        selected_image.set(Some(SelectedImage {
                                            path: path.display().to_string(),
                                            file_name,
                                            preview_data_url: preview,
                                        }));
                                    }
                                },

                                if let Some(preview_url) = selected_image()
                                    .and_then(|img| img.preview_data_url.clone())
                                    .or_else(|| item.image_preview_data_url.clone())
                                {
                                    img {
                                        class: "drop-zone-preview",
                                        src: preview_url,
                                        alt: "Image preview",
                                    }
                                    if selected_image().is_some() {
                                        button {
                                            class: "drop-zone-clear",
                                            title: "Remove staged image",
                                            onclick: move |e| {
                                                e.stop_propagation();
                                                selected_image.set(None);
                                            },
                                            "x"
                                        }
                                    }
                                } else {
                                    div { class: "drop-zone-hint",
                                        span { class: "drop-zone-icon", "I" }
                                        span { "Drop an image here or click to choose" }
                                    }
                                }
                            }

                            if let Some(ref img) = selected_image() {
                                p { class: "field-note", "Pending: {img.file_name}" }
                            } else if item.existing_image_payload.is_some() {
                                p { class: "field-note field-note-muted", "Using the currently published image." }
                            }
                        }

                        // ── Actions ────────────────────────────────────────
                        div { class: "form-actions",
                            button {
                                class: "btn-primary",
                                disabled: is_saving(),
                                onclick: {
                                    let item_id = item.item_id;
                                    let content_type = item.content_type.clone();
                                    let existing_image_payload = item.existing_image_payload.clone();
                                    move |_| {
                                        let draft_snap = draft();
                                        let selected_img = selected_image();
                                        let content_type = content_type.clone();
                                        let existing_image_payload = existing_image_payload.clone();
                                        let account_store_snap = account_store();
                                        spawn(async move {
                                            save_error.set(None);
                                            save_notice.set(None);
                                            is_saving.set(true);

                                            // Get signer
                                            let signer = account_store_snap
                                                .active_account_id
                                                .as_deref()
                                                .and_then(|id| account_store_snap.unlocked_signers.get(id))
                                                .cloned();
                                            let Some(signer) = signer else {
                                                save_error.set(Some(
                                                    "Unlock the active account before saving.".to_string(),
                                                ));
                                                is_saving.set(false);
                                                return;
                                            };

                                            // Build image payload
                                            let image_payload = match selected_img {
                                                Some(ref img) => {
                                                    match build_image_mixin(img).await {
                                                        Ok(built) => Some(built.payload),
                                                        Err(err) => {
                                                            save_error.set(Some(err));
                                                            is_saving.set(false);
                                                            return;
                                                        }
                                                    }
                                                }
                                                None => existing_image_payload,
                                            };

                                            // Encode revised payload
                                            let item_payload =
                                                encode_revised_item(&content_type, &draft_snap, image_payload);

                                            // Upload to IPFS
                                            let revision_ipfs_hash =
                                                match upload_ipfs_digest(&item_payload).await {
                                                    Ok(h) => h,
                                                    Err(err) => {
                                                        save_error.set(Some(err));
                                                        is_saving.set(false);
                                                        return;
                                                    }
                                                };
                                            let revision_ipfs_hash_bytes =
                                                match hex_to_bytes32(&revision_ipfs_hash) {
                                                    Ok(b) => b,
                                                    Err(err) => {
                                                        save_error.set(Some(err));
                                                        is_saving.set(false);
                                                        return;
                                                    }
                                                };

                                            // Submit publish_revision extrinsic
                                            let client = match connect_acuity_client().await {
                                                Ok(c) => c,
                                                Err(err) => {
                                                    save_error.set(Some(err));
                                                    is_saving.set(false);
                                                    return;
                                                }
                                            };
                                            let at_block = match client.at_current_block().await {
                                                Ok(b) => b,
                                                Err(err) => {
                                                    save_error.set(Some(format!(
                                                        "Failed to access latest block: {err}"
                                                    )));
                                                    is_saving.set(false);
                                                    return;
                                                }
                                            };

                                            let publish_revision_tx = api::tx().content().publish_revision(
                                                api::runtime_types::pallet_content::pallet::ItemId(item_id),
                                                api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                                                api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![]),
                                                api::runtime_types::pallet_content::pallet::IpfsHash(
                                                    revision_ipfs_hash_bytes,
                                                ),
                                            );

                                            match at_block
                                                .tx()
                                                .sign_and_submit_then_watch_default(
                                                    &publish_revision_tx,
                                                    &signer,
                                                )
                                                .await
                                            {
                                                Err(err) => {
                                                    save_error.set(Some(format!(
                                                        "Failed to submit revision: {err}"
                                                    )));
                                                    is_saving.set(false);
                                                    return;
                                                }
                                                Ok(progress) => {
                                                    if let Err(err) =
                                                        progress.wait_for_finalized_success().await
                                                    {
                                                        save_error.set(Some(format!(
                                                            "Revision transaction failed: {err}"
                                                        )));
                                                        is_saving.set(false);
                                                        return;
                                                    }
                                                }
                                            }

                                            is_saving.set(false);
                                            save_notice.set(Some(
                                                "Revision published successfully.".to_string(),
                                            ));
                                            // Reload the item to reflect the new revision.
                                            reload_tick.with_mut(|t| *t += 1);
                                        });
                                    }
                                },
                                if is_saving() { "Saving..." } else { "Save changes" }
                            }
                            button {
                                class: "btn-ghost",
                                disabled: is_saving(),
                                onclick: move |_| {
                                    save_error.set(None);
                                    save_notice.set(None);
                                    active_tab.set(ActiveTab::View);
                                },
                                "Cancel"
                            }
                        }

                        if !account_store().is_active_unlocked() {
                            p { class: "save-locked-hint",
                                "Unlock the account from the sidebar to save."
                            }
                        }
                    }
                }

            } else if !is_loading() && error_message().is_none() {
                div { class: "empty-state panel-surface",
                    p { class: "empty-state-title", "Item not found" }
                    p { class: "empty-state-body",
                        "The item could not be loaded. It may not have been indexed yet."
                    }
                    Link {
                        class: "btn-secondary",
                        to: Route::Home {},
                        "Go to Dashboard"
                    }
                }
            }
        }
    }
}
