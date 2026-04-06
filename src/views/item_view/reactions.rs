use crate::{
    accounts::AccountStore,
    acuity_runtime::api,
    runtime_client::{connect as connect_acuity_client, estimate_fee},
    ChainConnection,
};
use dioxus::prelude::*;
use sp_core::crypto::Ss58Codec;
use std::collections::HashMap;

use super::types::{AVAILABLE_EMOJI_CODEPOINTS, ReactionSummary};
use crate::views::components::InsufficientFundsHint;

/// Fetches all reactions for a given `(item_id, revision_id)` by iterating
/// the `ContentReactions::ItemAccountReactions` storage map with a 2-key
/// prefix.  Each entry maps one reactor account to its set of emoji reactions.
pub async fn fetch_reactions(
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

// ── Reactions component ───────────────────────────────────────────────────────

#[component]
pub fn Reactions(item_id: [u8; 32], revision_id: ReadSignal<u32>) -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let chain_connection = use_context::<Signal<ChainConnection>>();

    let mut reactions: Signal<Vec<ReactionSummary>> = use_signal(Vec::new);
    let mut reactions_loading = use_signal(|| false);
    let mut reactions_error: Signal<Option<String>> = use_signal(|| None);
    let mut show_picker = use_signal(|| false);
    let mut is_submitting = use_signal(|| false);
    let mut tx_error: Signal<Option<String>> = use_signal(|| None);
    let mut reload_tick = use_signal(|| 0_u64);

    // Fee estimation for add_reaction (representative cost for all reaction txs).
    let reaction_fee_estimate = use_resource(move || async move {
        let signer = account_store().active_signer().cloned()?;
        // Use a dummy emoji codepoint (0x1F44D = 👍) for the estimate.
        let dummy_emoji = api::runtime_types::pallet_content_reactions::pallet::Emoji(0x1F44D);
        let call = api::tx().content_reactions().add_reaction(
            api::runtime_types::pallet_content::pallet::ItemId(item_id),
            revision_id(),
            dummy_emoji,
        );
        estimate_fee(&call, &signer).await.ok()
    });

    let reaction_insufficient_funds = use_memo(move || {
        let balance = chain_connection().details.active_account_balance;
        let fee = reaction_fee_estimate().flatten();
        match (balance, fee) {
            (Some(b), Some(f)) => b < f,
            _ => true, // block until both balance and fee are known
        }
    });

    // Load reactions whenever item_id, revision_id, or reload_tick changes.
    use_effect(move || {
        let _tick = reload_tick();
        let revision_id = revision_id();
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
        let revision_id = revision_id();

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

    // Whether the active account is unlocked (has a signer available).
    let is_unlocked = {
        let store = account_store();
        store
            .active_account_id
            .as_deref()
            .map(|id| store.unlocked_signers.contains_key(id))
            .unwrap_or(false)
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
                                disabled: is_submitting() || reaction_insufficient_funds(),
                                onclick: move |_| submit_tx(cp, removing),
                                "{reaction.emoji_char} {reaction.count}"
                            }
                        }
                    }
                }

                // "+" button to open/close the picker — only when unlocked
                if is_unlocked && !picker_emojis.is_empty() {
                    button {
                        class: "reaction-add",
                        disabled: is_submitting() || reaction_insufficient_funds(),
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
                            disabled: is_submitting() || reaction_insufficient_funds(),
                            onclick: move |_| submit_tx(cp, false),
                            "{ch}"
                        }
                    }
                }
            }

            if reaction_insufficient_funds() {
                InsufficientFundsHint {
                    balance: chain_connection().details.active_account_balance,
                    fee: reaction_fee_estimate().flatten(),
                    fee_state: reaction_fee_estimate.state()(),
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
