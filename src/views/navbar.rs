use crate::{accounts::AccountStore, ChainConnection, ConnectionStatus, IpfsConnection, Route};
use dioxus::prelude::*;

const NAVBAR_CSS: Asset = asset!("/assets/styling/navbar.css");

#[component]
pub fn Navbar() -> Element {
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let chain_connection = chain_connection();
    let ipfs_connection = use_context::<Signal<IpfsConnection>>();
    let ipfs_connection = ipfs_connection();
    let account_store = use_context::<Signal<AccountStore>>();
    let account_store = account_store();

    let status_class = match chain_connection.status {
        ConnectionStatus::Connected => "connected",
        ConnectionStatus::Connecting => "connecting",
        ConnectionStatus::Reconnecting => "reconnecting",
    };

    let status_label = match chain_connection.status {
        ConnectionStatus::Connected => match chain_connection.block_number.as_deref() {
            Some(block_number) => format!("Block {block_number}"),
            None => "Connected".to_string(),
        },
        ConnectionStatus::Connecting => "Connecting to Acuity".to_string(),
        ConnectionStatus::Reconnecting => match chain_connection.last_error.as_deref() {
            Some(error) => format!("Reconnecting: {error}"),
            None => "Reconnecting to Acuity".to_string(),
        },
    };

    let ipfs_status_class = match ipfs_connection.status {
        ConnectionStatus::Connected => "connected",
        ConnectionStatus::Connecting => "connecting",
        ConnectionStatus::Reconnecting => "reconnecting",
    };

    let ipfs_status_label = match ipfs_connection.status {
        ConnectionStatus::Connected => "IPFS connected".to_string(),
        ConnectionStatus::Connecting => "Connecting to IPFS".to_string(),
        ConnectionStatus::Reconnecting => "Reconnecting to IPFS".to_string(),
    };

    let active_account = account_store.active_account().cloned();
    let account_status_class = if account_store.is_active_unlocked() {
        "unlocked"
    } else {
        "locked"
    };

    let account_label = match active_account {
        Some(account) => {
            let address = short_address(&account.address);
            if account_store.is_active_unlocked() {
                format!("{} ({address}) unlocked", account.name)
            } else {
                format!("{} ({address}) locked", account.name)
            }
        }
        None => "No active account".to_string(),
    };

    rsx! {
        document::Link { rel: "stylesheet", href: NAVBAR_CSS }

        div {
            id: "navbar",
            div {
                class: "nav-links",
                span { class: "brand", "Acuity" }
                Link {
                    to: Route::Home {},
                    "Dashboard"
                }
            }
            div {
                class: "nav-statuses",
                div {
                    class: "account-status {account_status_class}",
                    "{account_label}"
                }
                div {
                    class: "chain-status {status_class}",
                    "{status_label}"
                }
                div {
                    class: "ipfs-status {ipfs_status_class}",
                    "{ipfs_status_label}"
                }
            }
        }

        Outlet::<Route> {}
    }
}

fn short_address(address: &str) -> String {
    if address.len() <= 14 {
        return address.to_string();
    }

    format!("{}...{}", &address[..6], &address[address.len() - 6..])
}
