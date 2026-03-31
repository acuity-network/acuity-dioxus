use crate::{ChainConnection, ConnectionStatus, ACUITY_NODE_URL, RECONNECT_DELAY};
use dioxus::prelude::*;

const CHAIN_STATUS_CSS: Asset = asset!("/assets/styling/chain-status.css");

#[component]
pub fn ChainStatus() -> Element {
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let chain_connection = chain_connection();

    let (status_badge_class, status_label, status_copy) = match chain_connection.status {
        ConnectionStatus::Connected => (
            "connected",
            "Connected",
            "The websocket is live and the app is streaming both best and finalized heads from the local Acuity node.",
        ),
        ConnectionStatus::Connecting => (
            "connecting",
            "Connecting",
            "The app is opening a websocket session and waiting for the first block updates from the node.",
        ),
        ConnectionStatus::Reconnecting => (
            "reconnecting",
            "Reconnecting",
            "The last chain subscription failed, so the app is retrying until the local node is reachable again.",
        ),
    };

    let best_block_number = chain_connection
        .details
        .best_block_number
        .clone()
        .unwrap_or_else(|| "Waiting for the first best block".to_string());
    let best_block_hash = chain_connection
        .details
        .best_block_hash
        .clone()
        .unwrap_or_else(|| "Unavailable until the best-head stream responds".to_string());
    let finalized_block_number = chain_connection
        .details
        .finalized_block_number
        .clone()
        .unwrap_or_else(|| "Waiting for finalized head".to_string());
    let finalized_block_hash = chain_connection
        .details
        .finalized_block_hash
        .clone()
        .unwrap_or_else(|| "Unavailable until the finalized stream responds".to_string());
    let genesis_hash = chain_connection
        .details
        .genesis_hash
        .clone()
        .unwrap_or_else(|| "Unavailable until the node handshake completes".to_string());
    let spec_version = chain_connection
        .details
        .spec_version
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let transaction_version = chain_connection
        .details
        .transaction_version
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let ss58_prefix = chain_connection
        .details
        .ss58_prefix
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let existential_deposit = chain_connection
        .details
        .existential_deposit
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());

    let finality_gap = match (
        chain_connection
            .details
            .best_block_number
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok()),
        chain_connection
            .details
            .finalized_block_number
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok()),
    ) {
        (Some(best), Some(finalized)) => best.saturating_sub(finalized).to_string(),
        _ => "Pending".to_string(),
    };

    rsx! {
        document::Link { rel: "stylesheet", href: CHAIN_STATUS_CSS }

        div {
            class: "chain-shell",
            section {
                class: "chain-hero panel-surface",
                div {
                    class: "hero-copy",
                    p { class: "eyebrow", "Blockchain control plane" }
                    h1 { "Inspect the live Acuity chain connection" }
                    p {
                        class: "hero-text",
                        "This page expands the block pill in the header into a full websocket snapshot, including best head, finalized head, runtime versions, and the chain fingerprint the desktop app is following."
                    }
                }
                div {
                    class: "hero-status-card",
                    p { class: "label", "Live status" }
                    div {
                        class: "status-badge {status_badge_class}",
                        "{status_label}"
                    }
                    p { class: "status-copy", "{status_copy}" }
                }
            }

            if let Some(error) = chain_connection.last_error.clone() {
                div {
                    class: "chain-banner warning",
                    div {
                        class: "banner-title-row",
                        span { class: "banner-title", "Latest chain error" }
                        span { class: "banner-pill", "retrying" }
                    }
                    p { class: "banner-copy", "{error}" }
                }
            }

            div {
                class: "chain-grid",
                section {
                    class: "panel-surface overview-panel",
                    div {
                        class: "panel-heading",
                        div {
                            p { class: "label", "Overview" }
                            h2 { "Connection snapshot" }
                        }
                    }
                    div {
                        class: "facts-grid",
                        InfoCard {
                            title: "Node websocket".to_string(),
                            value: ACUITY_NODE_URL.to_string(),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Reconnect delay".to_string(),
                            value: format!("{} seconds", RECONNECT_DELAY.as_secs()),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Best head".to_string(),
                            value: best_block_number.clone(),
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Finalized head".to_string(),
                            value: finalized_block_number.clone(),
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Finality gap".to_string(),
                            value: finality_gap,
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Runtime spec".to_string(),
                            value: spec_version,
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "SS58 prefix".to_string(),
                            value: ss58_prefix,
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Transaction version".to_string(),
                            value: transaction_version,
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Existential deposit".to_string(),
                            value: existential_deposit,
                            tone: "neutral".to_string(),
                        }
                    }
                }

                section {
                    class: "panel-surface identity-panel",
                    div {
                        class: "panel-heading",
                        div {
                            p { class: "label", "Fingerprint" }
                            h2 { "Chain identity" }
                        }
                    }
                    div {
                        class: "field-stack",
                        MetadataField {
                            label: "Genesis hash".to_string(),
                            value: genesis_hash,
                            mono: true,
                        }
                        MetadataField {
                            label: "Best block hash".to_string(),
                            value: best_block_hash,
                            mono: true,
                        }
                        MetadataField {
                            label: "Finalized block hash".to_string(),
                            value: finalized_block_hash,
                            mono: true,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn InfoCard(title: String, value: String, tone: String) -> Element {
    rsx! {
        div {
            class: "info-card {tone}",
            p { class: "label", "{title}" }
            p { class: "info-value", "{value}" }
        }
    }
}

#[component]
fn MetadataField(label: String, value: String, mono: bool) -> Element {
    let value_class = if mono {
        "field-value mono"
    } else {
        "field-value"
    };

    rsx! {
        div {
            class: "metadata-field",
            p { class: "label", "{label}" }
            p { class: "{value_class}", "{value}" }
        }
    }
}
