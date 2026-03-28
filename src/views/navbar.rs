use crate::{ChainConnection, ConnectionStatus, Route};
use dioxus::prelude::*;

const NAVBAR_CSS: Asset = asset!("/assets/styling/navbar.css");

#[component]
pub fn Navbar() -> Element {
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let chain_connection = chain_connection();

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
                class: "chain-status {status_class}",
                "{status_label}"
            }
        }

        Outlet::<Route> {}
    }
}
