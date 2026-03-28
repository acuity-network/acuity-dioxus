use dioxus::prelude::*;
use std::time::Duration;
use subxt::{OnlineClient, PolkadotConfig};

use accounts::load_account_store;
use views::{Home, Navbar};

const ACUITY_NODE_URL: &str = "ws://127.0.0.1:9944";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq)]
struct ChainConnection {
    block_number: Option<String>,
    status: ConnectionStatus,
    last_error: Option<String>,
}

impl ChainConnection {
    fn connecting() -> Self {
        Self {
            block_number: None,
            status: ConnectionStatus::Connecting,
            last_error: None,
        }
    }

    fn connected(block_number: Option<String>) -> Self {
        Self {
            block_number,
            status: ConnectionStatus::Connected,
            last_error: None,
        }
    }

    fn reconnecting(block_number: Option<String>, error: String) -> Self {
        Self {
            block_number,
            status: ConnectionStatus::Reconnecting,
            last_error: Some(error),
        }
    }
}

impl Default for ChainConnection {
    fn default() -> Self {
        Self::connecting()
    }
}

#[derive(Clone, PartialEq)]
enum ConnectionStatus {
    Connecting,
    Connected,
    Reconnecting,
}

mod accounts;
mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Navbar)]
        #[route("/")]
        Home {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/styling/main.css");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let chain_connection = use_signal(ChainConnection::default);
    let account_store = use_signal(load_account_store);
    use_context_provider(|| chain_connection);
    use_context_provider(|| account_store);

    let _connection_task = use_hook(move || {
        let chain_connection = chain_connection;

        spawn(async move {
            watch_acuity_chain(chain_connection).await;
        })
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }

        Router::<Route> {}
    }
}

async fn watch_acuity_chain(mut chain_connection: Signal<ChainConnection>) {
    loop {
        let last_block = chain_connection().block_number;
        chain_connection.set(ChainConnection {
            block_number: last_block,
            status: ConnectionStatus::Connecting,
            last_error: None,
        });

        let result = stream_best_blocks(chain_connection).await;

        let error = match result {
            Ok(()) => "Block stream ended".to_string(),
            Err(error) => error,
        };

        let last_block = chain_connection().block_number;
        chain_connection.set(ChainConnection::reconnecting(last_block, error));
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn stream_best_blocks(mut chain_connection: Signal<ChainConnection>) -> Result<(), String> {
    let client = OnlineClient::<PolkadotConfig>::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Failed to connect to {ACUITY_NODE_URL}: {error}"))?;

    let existing_block = chain_connection().block_number;
    chain_connection.set(ChainConnection::connected(existing_block));

    let mut blocks = client
        .stream_best_blocks()
        .await
        .map_err(|error| format!("Failed to subscribe to best blocks: {error}"))?;

    while let Some(block) = blocks.next().await {
        let block = block.map_err(|error| format!("Failed to read best block: {error}"))?;
        let block_number = block.header().number.to_string();
        chain_connection.set(ChainConnection::connected(Some(block_number)));
    }

    Ok(())
}
