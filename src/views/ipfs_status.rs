use crate::{
    ConnectionStatus, IpfsConnection, IPFS_API_URL, IPFS_DAEMON_ADDR, IPFS_HEALTHCHECK_INTERVAL,
    RECONNECT_DELAY,
};
use dioxus::prelude::*;

const IPFS_STATUS_CSS: Asset = asset!("/assets/styling/ipfs-status.css");

#[component]
pub fn IpfsStatus() -> Element {
    let ipfs_connection = use_context::<Signal<IpfsConnection>>();
    let ipfs_connection = ipfs_connection();

    let (status_badge_class, status_label) = match ipfs_connection.status {
        ConnectionStatus::Connected => ("connected", "Connected"),
        ConnectionStatus::Connecting => ("connecting", "Connecting"),
        ConnectionStatus::Reconnecting => ("reconnecting", "Reconnecting"),
    };

    let daemon_agent = ipfs_connection
        .details
        .as_ref()
        .and_then(|details| details.agent_version.as_deref())
        .unwrap_or("Waiting for daemon metadata");
    let protocol_version = ipfs_connection
        .details
        .as_ref()
        .and_then(|details| details.protocol_version.as_deref())
        .unwrap_or("Unknown");
    let peer_id = ipfs_connection
        .details
        .as_ref()
        .map(|details| details.peer_id.as_str())
        .unwrap_or("Unavailable until the daemon answers /api/v0/id");
    let public_key = ipfs_connection
        .details
        .as_ref()
        .and_then(|details| details.public_key.as_deref())
        .unwrap_or("Unavailable until the daemon answers /api/v0/id");
    let addresses = ipfs_connection
        .details
        .as_ref()
        .map(|details| details.addresses.clone())
        .unwrap_or_default();
    let protocols = ipfs_connection
        .details
        .as_ref()
        .map(|details| details.protocols.clone())
        .unwrap_or_default();

    rsx! {
        document::Link { rel: "stylesheet", href: IPFS_STATUS_CSS }

        div {
            class: "ipfs-shell",

            // ── Compact status bar ──────────────────────────────────────────
            div {
                class: "status-bar panel-surface",
                div {
                    class: "status-bar-left",
                    div { class: "status-badge {status_badge_class}", "{status_label}" }
                    div {
                        class: "status-bar-title",
                        span { class: "status-bar-name", "Acuity IPFS Daemon" }
                        span { class: "status-bar-url", "{IPFS_API_URL}" }
                    }
                }
                div {
                    class: "status-bar-right",
                    span { class: "status-bar-meta-label", "Reconnect delay" }
                    span { class: "status-bar-meta-value", "{RECONNECT_DELAY.as_secs()}s" }
                }
            }

            if let Some(error) = ipfs_connection.last_error.clone() {
                div {
                    class: "ipfs-banner warning",
                    div {
                        class: "banner-title-row",
                        span { class: "banner-title", "Latest daemon error" }
                        span { class: "banner-pill", "retrying" }
                    }
                    p { class: "banner-copy", "{error}" }
                }
            }

            div {
                class: "ipfs-grid",
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
                            title: "Daemon API".to_string(),
                            value: IPFS_API_URL.to_string(),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Daemon address".to_string(),
                            value: IPFS_DAEMON_ADDR.to_string(),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Health check cadence".to_string(),
                            value: format!("Every {} seconds", IPFS_HEALTHCHECK_INTERVAL.as_secs()),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Reconnect delay".to_string(),
                            value: format!("{} seconds", RECONNECT_DELAY.as_secs()),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Agent version".to_string(),
                            value: daemon_agent.to_string(),
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Protocol version".to_string(),
                            value: protocol_version.to_string(),
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Supported protocols".to_string(),
                            value: format!("{} advertised", protocols.len()),
                            tone: "neutral".to_string(),
                        }
                    }
                }

                section {
                    class: "panel-surface identity-panel",
                    div {
                        class: "panel-heading",
                        div {
                            p { class: "label", "Identity" }
                            h2 { "Daemon fingerprint" }
                        }
                    }
                    div {
                        class: "field-stack",
                        MetadataField {
                            label: "Peer ID".to_string(),
                            value: peer_id.to_string(),
                            mono: true,
                        }
                        MetadataField {
                            label: "Public key".to_string(),
                            value: public_key.to_string(),
                            mono: true,
                        }
                    }
                }
            }

            section {
                class: "panel-surface addresses-panel",
                div {
                    class: "panel-heading",
                    div {
                        p { class: "label", "Network" }
                        h2 { "Advertised addresses" }
                    }
                    p { class: "address-count", "{addresses.len()} endpoints" }
                }

                if addresses.is_empty() {
                    div {
                        class: "empty-state",
                        "No addresses have been reported yet. Once the daemon answers successfully, its multiaddrs will appear here."
                    }
                } else {
                    div {
                        class: "address-list",
                        for address in addresses {
                            div {
                                class: "address-row",
                                span { class: "address-badge", "multiaddr" }
                                code { "{address}" }
                            }
                        }
                    }
                }
            }

            section {
                class: "panel-surface addresses-panel",
                div {
                    class: "panel-heading",
                    div {
                        p { class: "label", "Protocols" }
                        h2 { "Advertised protocol set" }
                    }
                    p { class: "address-count", "{protocols.len()} entries" }
                }

                if protocols.is_empty() {
                    div {
                        class: "empty-state",
                        "The daemon has not reported any supported protocols yet."
                    }
                } else {
                    div {
                        class: "address-list",
                        for protocol in protocols {
                            div {
                                class: "address-row",
                                span { class: "address-badge", "proto" }
                                code { "{protocol}" }
                            }
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
