use acuity_index_api_rs::IndexerClient;
use crate::{
    accounts::AccountStore,
    acuity_runtime::api,
    content::{bytes32_to_hex, fetch_events_for_item_revision, is_content_reactions_event},
    runtime_client::{connect as connect_acuity_client, estimate_fee},
    ChainConnection,
};
use dioxus::prelude::*;
use sp_core::crypto::Ss58Codec;
use std::collections::HashMap;

use super::types::{AVAILABLE_EMOJI_CODEPOINTS, ReactionSummary};
use crate::views::components::InsufficientFundsHint;

fn hex_to_ss58(hex: &str) -> Option<String> {
    let bytes = crate::content::hex_to_bytes32(hex).ok()?;
    let account = sp_core::crypto::AccountId32::from(bytes);
    Some(account.to_ss58check())
}

fn parse_u32_from_value(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .or_else(|| value.as_str().and_then(|s| s.parse::<u32>().ok()))
}

fn parse_reactions_from_event(reactions_value: &serde_json::Value) -> Vec<u32> {
    if let Some(arr) = reactions_value.as_array() {
        return arr.iter().filter_map(parse_u32_from_value).collect();
    }

    parse_u32_from_value(reactions_value).into_iter().collect()
}

async fn fetch_reactions_from_indexer(
    client: &IndexerClient,
    item_id_hex: String,
    revision_id: u32,
    active_address: Option<String>,
) -> Result<Vec<ReactionSummary>, String> {
    let decoded_events = fetch_events_for_item_revision(client, item_id_hex, revision_id).await?;

    let mut reactor_latest: HashMap<String, (u32, u16, Vec<u32>)> = HashMap::new();

    for event in &decoded_events {
        if !is_content_reactions_event(event, "SetReactions") {
            continue;
        }

        let reactor_hex = match event.field("reactor").and_then(serde_json::Value::as_str) {
            Some(h) => h.to_string(),
            None => continue,
        };

        let reactions = event
            .field("reactions")
            .map(|v| parse_reactions_from_event(v))
            .unwrap_or_default();

        let block = event.block_number;
        let index = event.event_index;

        let entry = reactor_latest.entry(reactor_hex).or_insert((0, 0, Vec::new()));
        if (block, index) > (entry.0, entry.1) {
            *entry = (block, index, reactions);
        }
    }

    let mut map: HashMap<u32, (usize, Vec<String>, bool)> = HashMap::new();

    for (reactor_hex, (_block, _index, reactions)) in reactor_latest {
        let reactor_ss58 = hex_to_ss58(&reactor_hex).unwrap_or_default();
        let is_me = active_address
            .as_deref()
            .map(|addr| addr == reactor_ss58)
            .unwrap_or(false);

        for codepoint in reactions {
            let entry = map.entry(codepoint).or_insert((0, Vec::new(), false));
            entry.0 += 1;
            if !reactor_ss58.is_empty() {
                entry.1.push(reactor_ss58.clone());
            }
            if is_me {
                entry.2 = true;
            }
        }
    }

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

fn optimistic_update(
    current: &[ReactionSummary],
    active_address: &str,
    new_set: &[u32],
) -> Vec<ReactionSummary> {
    let mut map: HashMap<u32, (usize, Vec<String>, bool)> = HashMap::new();

    for r in current {
        let others: Vec<String> = r
            .reactors
            .iter()
            .filter(|addr| *addr != active_address)
            .cloned()
            .collect();
        if !others.is_empty() {
            let entry = map.entry(r.codepoint).or_default();
            entry.0 += others.len();
            entry.1.extend(others);
        }
    }

    for &cp in new_set {
        let entry = map.entry(cp).or_default();
        entry.0 += 1;
        entry.1.push(active_address.to_string());
        entry.2 = true;
    }

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

    summaries
}

#[component]
pub fn Reactions(item_id: [u8; 32], revision_id: ReadSignal<u32>) -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let indexer_client = use_context::<Signal<Option<IndexerClient>>>();

    let mut reactions: Signal<Vec<ReactionSummary>> = use_signal(Vec::new);
    let mut reactions_loading = use_signal(|| false);
    let mut reactions_error: Signal<Option<String>> = use_signal(|| None);
    let mut show_picker = use_signal(|| false);
    let mut is_submitting = use_signal(|| false);
    let mut tx_error: Signal<Option<String>> = use_signal(|| None);
    let reload_tick = use_signal(|| 0_u64);

    let reaction_fee_estimate = use_resource(move || async move {
        let signer = account_store().active_signer().cloned()?;
        let dummy_reactions =
            api::runtime_types::bounded_collections::bounded_vec::BoundedVec(vec![
                api::runtime_types::pallet_content_reactions::pallet::Emoji(0x1F44D),
            ]);
        let call = api::tx().content_reactions().set_reactions(
            api::runtime_types::pallet_content::pallet::ItemId(item_id),
            revision_id(),
            dummy_reactions,
        );
        estimate_fee(&call, &signer).await.ok()
    });

    let reaction_insufficient_funds = use_memo(move || {
        let balance = chain_connection().details.active_account_balance;
        let fee = reaction_fee_estimate().flatten();
        match (balance, fee) {
            (Some(b), Some(f)) => b < f,
            _ => true,
        }
    });

    use_effect(move || {
        let _tick = reload_tick();
        let revision_id = revision_id();
        let active_address = account_store()
            .active_account()
            .map(|a| a.address.clone());
        let item_id_hex = bytes32_to_hex(&item_id);
        let client = indexer_client().clone();
        spawn(async move {
            let Some(client) = client else {
                return;
            };
            reactions_loading.set(true);
            reactions_error.set(None);
            match fetch_reactions_from_indexer(&client, item_id_hex, revision_id, active_address).await {
                Ok(r) => reactions.set(r),
                Err(e) => reactions_error.set(Some(e)),
            }
            reactions_loading.set(false);
        });
    });

    let is_unlocked = {
        let store = account_store();
        store
            .active_account_id
            .as_deref()
            .map(|id| store.unlocked_signers.contains_key(id))
            .unwrap_or(false)
    };

    let current_user_codepoints: Vec<u32> = reactions()
        .iter()
        .filter(|r| r.i_reacted)
        .map(|r| r.codepoint)
        .collect();

    let picker_emojis: Vec<(u32, String)> = AVAILABLE_EMOJI_CODEPOINTS
        .iter()
        .filter(|&&cp| !current_user_codepoints.contains(&cp))
        .filter_map(|&cp| Some((cp, char::from_u32(cp)?.to_string())))
        .collect();

    let mut toggle_emoji = move |codepoint: u32| {
        let store_snap = account_store();
        let signer = store_snap
            .active_account_id
            .as_deref()
            .and_then(|id| store_snap.unlocked_signers.get(id))
            .cloned();
        let Some(signer) = signer else {
            tx_error.set(Some("Unlock the active account to react.".to_string()));
            return;
        };
        let active_address = store_snap
            .active_account()
            .map(|a| a.address.clone())
            .unwrap_or_default();

        let mut new_set: Vec<u32> = reactions()
            .iter()
            .filter(|r| r.i_reacted)
            .map(|r| r.codepoint)
            .collect();

        if new_set.contains(&codepoint) {
            new_set.retain(|&cp| cp != codepoint);
        } else {
            new_set.push(codepoint);
            new_set.sort();
        }

        let old_reactions = reactions();
        let updated = optimistic_update(&old_reactions, &active_address, &new_set);
        reactions.set(updated);

        let rev_id = revision_id();
        show_picker.set(false);

        spawn(async move {
            is_submitting.set(true);
            tx_error.set(None);

            let result: Result<(), String> = async {
                let client = connect_acuity_client().await?;
                let at_block = client
                    .at_current_block()
                    .await
                    .map_err(|e| format!("Failed to access latest block: {e}"))?;

                let item_id_param =
                    api::runtime_types::pallet_content::pallet::ItemId(item_id);
                let reactions_param =
                    api::runtime_types::bounded_collections::bounded_vec::BoundedVec(
                        new_set
                            .iter()
                            .map(|&cp| {
                                api::runtime_types::pallet_content_reactions::pallet::Emoji(cp)
                            })
                            .collect(),
                    );

                at_block
                    .tx()
                    .sign_and_submit_then_watch_default(
                        &api::tx().content_reactions().set_reactions(
                            item_id_param,
                            rev_id,
                            reactions_param,
                        ),
                        &signer,
                    )
                    .await
                    .map_err(|e| format!("Failed to submit reaction: {e}"))?
                    .wait_for_finalized_success()
                    .await
                    .map_err(|e| format!("Reaction transaction failed: {e}"))?;

                Ok(())
            }
            .await;

            if let Err(e) = result {
                // Revert optimistic update on failure
                let reverted = if let Some(client) = indexer_client().as_ref() {
                    fetch_reactions_from_indexer(
                        client,
                        bytes32_to_hex(&item_id),
                        revision_id(),
                        account_store().active_account().map(|a| a.address.clone()),
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                reactions.set(reverted.unwrap_or(old_reactions));
                tx_error.set(Some(e));
            }
            is_submitting.set(false);
        });
    };

    rsx! {
        div {
            class: "reactions-section",

            if let Some(err) = tx_error() {
                div { class: "status-bar error reactions-tx-error", "{err}" }
            }

            div {
                class: "reactions-bar",

                for reaction in reactions() {
                    {
                        let cp = reaction.codepoint;
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
                                onclick: move |_| toggle_emoji(cp),
                                "{reaction.emoji_char} {reaction.count}"
                            }
                        }
                    }
                }

                if is_unlocked && !picker_emojis.is_empty() {
                    button {
                        class: "reaction-add",
                        disabled: is_submitting() || reaction_insufficient_funds(),
                        onclick: move |_| show_picker.with_mut(|v| *v = !*v),
                        "+"
                    }
                }
            }

            if show_picker() {
                div {
                    class: "reaction-picker",
                    for (cp, ch) in picker_emojis {
                        button {
                            class: "picker-emoji",
                            disabled: is_submitting() || reaction_insufficient_funds(),
                            onclick: move |_| toggle_emoji(cp),
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

#[cfg(test)]
mod tests {
    use super::parse_reactions_from_event;
    use serde_json::json;

    #[test]
    fn parse_reactions_from_event_accepts_scalar_string() {
        assert_eq!(parse_reactions_from_event(&json!("128536")), vec![128536]);
    }

    #[test]
    fn parse_reactions_from_event_accepts_scalar_number() {
        assert_eq!(parse_reactions_from_event(&json!(128536)), vec![128536]);
    }

    #[test]
    fn parse_reactions_from_event_accepts_arrays() {
        assert_eq!(
            parse_reactions_from_event(&json!(["128536", 128540])),
            vec![128536, 128540]
        );
    }

    #[test]
    fn parse_reactions_from_event_rejects_invalid_values() {
        assert!(parse_reactions_from_event(&json!("not-a-number")).is_empty());
    }
}
