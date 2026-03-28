use dioxus::prelude::*;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use subxt::{OnlineClient, PolkadotConfig};

use accounts::load_account_store;
use views::{Home, IpfsStatus, Navbar};

const ACUITY_NODE_URL: &str = "ws://127.0.0.1:9944";
const IPFS_DAEMON_ADDR: &str = "/ip4/127.0.0.1/tcp/5001";
const IPFS_API_URL: &str = "http://127.0.0.1:5001";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const IPFS_HEALTHCHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, PartialEq)]
struct ChainConnection {
    block_number: Option<String>,
    status: ConnectionStatus,
    last_error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct IpfsConnection {
    status: ConnectionStatus,
    last_error: Option<String>,
    details: Option<IpfsDaemonDetails>,
}

#[derive(Clone, PartialEq)]
struct IpfsDaemonDetails {
    peer_id: String,
    public_key: Option<String>,
    addresses: Vec<String>,
    agent_version: Option<String>,
    protocol_version: Option<String>,
    protocols: Vec<String>,
}

impl IpfsConnection {
    fn connecting(details: Option<IpfsDaemonDetails>) -> Self {
        Self {
            status: ConnectionStatus::Connecting,
            last_error: None,
            details,
        }
    }

    fn connected(details: IpfsDaemonDetails) -> Self {
        Self {
            status: ConnectionStatus::Connected,
            last_error: None,
            details: Some(details),
        }
    }

    fn reconnecting(details: Option<IpfsDaemonDetails>, error: String) -> Self {
        Self {
            status: ConnectionStatus::Reconnecting,
            last_error: Some(error),
            details,
        }
    }
}

impl Default for IpfsConnection {
    fn default() -> Self {
        Self::connecting(None)
    }
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

#[derive(Deserialize)]
struct IpfsIdResponse {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "PublicKey")]
    public_key: Option<String>,
    #[serde(rename = "Addresses")]
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(rename = "AgentVersion")]
    agent_version: Option<String>,
    #[serde(rename = "ProtocolVersion")]
    protocol_version: Option<String>,
    #[serde(rename = "Protocols")]
    #[serde(default)]
    protocols: Vec<String>,
}

mod accounts;
mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Navbar)]
        #[route("/")]
        Home {},
        #[route("/ipfs")]
        IpfsStatus {},
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
    let ipfs_connection = use_signal(IpfsConnection::default);
    let account_store = use_signal(load_account_store);
    use_context_provider(|| chain_connection);
    use_context_provider(|| ipfs_connection);
    use_context_provider(|| account_store);

    let _connection_task = use_hook(move || {
        let chain_connection = chain_connection;

        spawn(async move {
            watch_acuity_chain(chain_connection).await;
        })
    });

    let _ipfs_connection_task = use_hook(move || {
        let ipfs_connection = ipfs_connection;

        spawn(async move {
            watch_ipfs_daemon(ipfs_connection).await;
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

async fn watch_ipfs_daemon(mut ipfs_connection: Signal<IpfsConnection>) {
    let client = Client::new();

    loop {
        let details = ipfs_connection().details.clone();
        ipfs_connection.set(IpfsConnection::connecting(details));

        let result = maintain_ipfs_connection(&client, ipfs_connection).await;

        let error = match result {
            Ok(()) => format!("Lost connection to {IPFS_DAEMON_ADDR}"),
            Err(error) => error,
        };

        let details = ipfs_connection().details.clone();
        ipfs_connection.set(IpfsConnection::reconnecting(details, error));
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn maintain_ipfs_connection(
    client: &Client,
    mut ipfs_connection: Signal<IpfsConnection>,
) -> Result<(), String> {
    loop {
        let response = client
            .post(format!("{IPFS_API_URL}/api/v0/id"))
            .send()
            .await
            .map_err(|error| format!("Failed to reach {IPFS_DAEMON_ADDR}: {error}"))?;

        let response = response
            .error_for_status()
            .map_err(|error| format!("IPFS daemon returned an error: {error}"))?;

        let payload = response
            .json::<IpfsIdResponse>()
            .await
            .map_err(|error| format!("Failed to decode IPFS daemon response: {error}"))?;

        if payload.id.is_empty() {
            return Err(format!(
                "IPFS daemon at {IPFS_DAEMON_ADDR} returned an empty peer ID"
            ));
        }

        ipfs_connection.set(IpfsConnection::connected(IpfsDaemonDetails {
            peer_id: payload.id,
            public_key: payload.public_key,
            addresses: payload.addresses,
            agent_version: payload.agent_version,
            protocol_version: payload.protocol_version,
            protocols: payload.protocols,
        }));
        tokio::time::sleep(IPFS_HEALTHCHECK_INTERVAL).await;
    }
}
