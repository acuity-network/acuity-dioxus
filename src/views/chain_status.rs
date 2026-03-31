use crate::{ChainConnection, ConnectionStatus, ACUITY_NODE_URL, RECONNECT_DELAY};
use dioxus::prelude::*;

const CHAIN_STATUS_CSS: Asset = asset!("/assets/styling/chain-status.css");

#[component]
pub fn ChainStatus() -> Element {
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let chain_connection = chain_connection();

    let (status_badge_class, status_label) = match chain_connection.status {
        ConnectionStatus::Connected => ("connected", "Connected"),
        ConnectionStatus::Connecting => ("connecting", "Connecting"),
        ConnectionStatus::Reconnecting => ("reconnecting", "Reconnecting"),
    };

    let best_block_number = chain_connection
        .details
        .best_block_number
        .clone()
        .unwrap_or_else(|| "—".to_string());
    let best_block_hash = chain_connection
        .details
        .best_block_hash
        .clone()
        .unwrap_or_default();
    let finalized_block_number = chain_connection
        .details
        .finalized_block_number
        .clone()
        .unwrap_or_else(|| "—".to_string());
    let finalized_block_hash = chain_connection
        .details
        .finalized_block_hash
        .clone()
        .unwrap_or_default();
    let genesis_hash = chain_connection
        .details
        .genesis_hash
        .clone()
        .unwrap_or_default();
    let spec_version = chain_connection
        .details
        .spec_version
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string());
    let transaction_version = chain_connection
        .details
        .transaction_version
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string());
    let ss58_prefix = chain_connection
        .details
        .ss58_prefix
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string());
    let existential_deposit = chain_connection
        .details
        .existential_deposit
        .clone()
        .unwrap_or_else(|| "—".to_string());

    rsx! {
        document::Link { rel: "stylesheet", href: CHAIN_STATUS_CSS }

        div {
            class: "chain-shell",

            // ── Compact status bar ──────────────────────────────────────────
            div {
                class: "status-bar panel-surface",
                div {
                    class: "status-bar-left",
                    div { class: "status-badge {status_badge_class}", "{status_label}" }
                    div {
                        class: "status-bar-title",
                        span { class: "status-bar-name", "Acuity Node" }
                        span { class: "status-bar-url", "{ACUITY_NODE_URL}" }
                    }
                }
                div {
                    class: "status-bar-right",
                    span { class: "status-bar-meta-label", "Reconnect delay" }
                    span { class: "status-bar-meta-value", "{RECONNECT_DELAY.as_secs()}s" }
                }
            }

            // ── Error banner ────────────────────────────────────────────────
            if let Some(error) = chain_connection.last_error.clone() {
                div {
                    class: "chain-banner warning",
                    div {
                        class: "banner-title-row",
                        span { class: "banner-title", "Chain error" }
                        span { class: "banner-pill", "retrying" }
                    }
                    p { class: "banner-copy", "{error}" }
                }
            }

            // ── Primary block metrics ───────────────────────────────────────
            div {
                class: "blocks-row",
                div {
                    class: "block-metric panel-surface",
                    p { class: "label", "Best block" }
                    p { class: "primary-metric {status_badge_class}", "#{best_block_number}" }
                }
                div {
                    class: "block-metric panel-surface",
                    p { class: "label", "Finalized block" }
                    p { class: "primary-metric {status_badge_class}", "#{finalized_block_number}" }
                }
            }

            // ── Chain identity + runtime ────────────────────────────────────
            div {
                class: "chain-grid",
                section {
                    class: "panel-surface identity-panel",
                    div { class: "panel-heading", h2 { "Chain identity" } }
                    div {
                        class: "field-stack",
                        HashField { label: "Genesis hash".to_string(), value: genesis_hash }
                        HashField { label: "Best block hash".to_string(), value: best_block_hash }
                        HashField { label: "Finalized block hash".to_string(), value: finalized_block_hash }
                    }
                }

                section {
                    class: "panel-surface runtime-panel",
                    div { class: "panel-heading", h2 { "Runtime" } }
                    div {
                        class: "facts-grid",
                        InfoCard { title: "Spec version".to_string(), value: spec_version }
                        InfoCard { title: "Tx version".to_string(), value: transaction_version }
                        InfoCard { title: "SS58 prefix".to_string(), value: ss58_prefix }
                        InfoCard { title: "Existential deposit".to_string(), value: existential_deposit }
                    }
                }
            }
        }
    }
}

// ── Truncated hash field with copy button ─────────────────────────────────────

#[component]
fn HashField(label: String, value: String) -> Element {
    let display = if value.len() > 20 {
        format!("{}…{}", &value[..10], &value[value.len() - 8..])
    } else if value.is_empty() {
        "—".to_string()
    } else {
        value.clone()
    };

    let copy_value = value.clone();

    rsx! {
        div {
            class: "metadata-field",
            div {
                class: "field-header",
                p { class: "label", "{label}" }
                if !value.is_empty() {
                    button {
                        class: "copy-btn",
                        title: "Copy full hash",
                        onclick: move |_| {
                            let v = copy_value.clone();
                            spawn(async move {
                                let _ = document::eval(&format!(
                                    "navigator.clipboard.writeText('{v}')"
                                )).await;
                            });
                        },
                        "Copy"
                    }
                }
            }
            p { class: "field-value mono", title: "{value}", "{display}" }
        }
    }
}

// ── Simple info card ──────────────────────────────────────────────────────────

#[component]
fn InfoCard(title: String, value: String) -> Element {
    rsx! {
        div {
            class: "info-card",
            p { class: "label", "{title}" }
            p { class: "info-value", "{value}" }
        }
    }
}
