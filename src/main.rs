use dioxus::prelude::*;
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use subxt::{OnlineClient, PolkadotConfig};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use accounts::load_account_store;
use views::{ChainStatus, Home, IndexerStatus, IpfsStatus, Navbar};

const ACUITY_NODE_URL: &str = "ws://127.0.0.1:9944";
const INDEXER_URL: &str = "ws://127.0.0.1:8172";
const IPFS_DAEMON_ADDR: &str = "/ip4/127.0.0.1/tcp/5001";
const IPFS_API_URL: &str = "http://127.0.0.1:5001";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const IPFS_HEALTHCHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, PartialEq)]
struct ChainConnection {
    details: ChainDetails,
    status: ConnectionStatus,
    last_error: Option<String>,
}

#[derive(Clone, PartialEq, Default)]
struct ChainDetails {
    best_block_number: Option<String>,
    best_block_hash: Option<String>,
    finalized_block_number: Option<String>,
    finalized_block_hash: Option<String>,
    genesis_hash: Option<String>,
    spec_version: Option<u32>,
    transaction_version: Option<u32>,
}

#[derive(Clone, PartialEq)]
struct IpfsConnection {
    status: ConnectionStatus,
    last_error: Option<String>,
    details: Option<IpfsDaemonDetails>,
}

#[derive(Clone, PartialEq)]
struct IndexerConnection {
    status: ConnectionStatus,
    last_error: Option<String>,
    details: IndexerDetails,
}

#[derive(Clone, PartialEq, Default)]
struct IndexerDetails {
    spans: Vec<IndexerSpan>,
}

#[derive(Clone, Deserialize, PartialEq)]
struct IndexerSpan {
    start: u32,
    end: u32,
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

impl IndexerConnection {
    fn connecting(details: IndexerDetails) -> Self {
        Self {
            status: ConnectionStatus::Connecting,
            last_error: None,
            details,
        }
    }

    fn connected(details: IndexerDetails) -> Self {
        Self {
            status: ConnectionStatus::Connected,
            last_error: None,
            details,
        }
    }

    fn reconnecting(details: IndexerDetails, error: String) -> Self {
        Self {
            status: ConnectionStatus::Reconnecting,
            last_error: Some(error),
            details,
        }
    }
}

impl Default for IndexerConnection {
    fn default() -> Self {
        Self::connecting(IndexerDetails::default())
    }
}

impl IndexerDetails {
    fn latest_indexed_block(&self) -> Option<u32> {
        self.spans.iter().map(|span| span.end).max()
    }
}

impl ChainConnection {
    fn connecting(details: ChainDetails) -> Self {
        Self {
            details,
            status: ConnectionStatus::Connecting,
            last_error: None,
        }
    }

    fn connected(details: ChainDetails) -> Self {
        Self {
            details,
            status: ConnectionStatus::Connected,
            last_error: None,
        }
    }

    fn reconnecting(details: ChainDetails, error: String) -> Self {
        Self {
            details,
            status: ConnectionStatus::Reconnecting,
            last_error: Some(error),
        }
    }
}

impl Default for ChainConnection {
    fn default() -> Self {
        Self::connecting(ChainDetails::default())
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

#[derive(Deserialize)]
struct IndexerResponse {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    data: Option<Vec<IndexerSpan>>,
}

mod accounts;
mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Navbar)]
        #[route("/")]
        Home {},
        #[route("/chain")]
        ChainStatus {},
        #[route("/indexer")]
        IndexerStatus {},
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
    let indexer_connection = use_signal(IndexerConnection::default);
    let ipfs_connection = use_signal(IpfsConnection::default);
    let account_store = use_signal(load_account_store);
    use_context_provider(|| chain_connection);
    use_context_provider(|| indexer_connection);
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

    let _indexer_connection_task = use_hook(move || {
        let indexer_connection = indexer_connection;

        spawn(async move {
            watch_indexer(indexer_connection).await;
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
        let details = chain_connection().details.clone();
        chain_connection.set(ChainConnection::connecting(details));

        let result = stream_best_blocks(chain_connection).await;

        let error = match result {
            Ok(()) => "Block stream ended".to_string(),
            Err(error) => error,
        };

        let details = chain_connection().details.clone();
        chain_connection.set(ChainConnection::reconnecting(details, error));
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn stream_best_blocks(mut chain_connection: Signal<ChainConnection>) -> Result<(), String> {
    let client = OnlineClient::<PolkadotConfig>::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Failed to connect to {ACUITY_NODE_URL}: {error}"))?;

    let finalized_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to inspect the latest finalized block: {error}"))?;

    let mut details = chain_connection().details.clone();
    let finalized_block_number = finalized_block.block_number().to_string();
    let finalized_block_hash = finalized_block.block_hash().to_string();

    details.finalized_block_number = Some(finalized_block_number.clone());
    details.finalized_block_hash = Some(finalized_block_hash.clone());
    details.best_block_number = Some(finalized_block_number);
    details.best_block_hash = Some(finalized_block_hash);
    details.genesis_hash = Some(client.genesis_hash().to_string());
    details.spec_version = Some(finalized_block.spec_version());
    details.transaction_version = Some(finalized_block.transaction_version());
    chain_connection.set(ChainConnection::connected(details));

    let mut best_blocks = client
        .stream_best_blocks()
        .await
        .map_err(|error| format!("Failed to subscribe to best blocks: {error}"))?;

    let mut finalized_blocks = client
        .stream_blocks()
        .await
        .map_err(|error| format!("Failed to subscribe to finalized blocks: {error}"))?;

    loop {
        tokio::select! {
            next_best = best_blocks.next() => {
                let block = match next_best {
                    Some(block) => block.map_err(|error| format!("Failed to read best block: {error}"))?,
                    None => return Ok(()),
                };

                let mut details = chain_connection().details.clone();
                details.best_block_number = Some(block.number().to_string());
                details.best_block_hash = Some(block.hash().to_string());
                chain_connection.set(ChainConnection::connected(details));
            }
            next_finalized = finalized_blocks.next() => {
                let block = match next_finalized {
                    Some(block) => block.map_err(|error| format!("Failed to read finalized block: {error}"))?,
                    None => return Ok(()),
                };

                let finalized_at = block
                    .at()
                    .await
                    .map_err(|error| format!("Failed to inspect finalized runtime metadata: {error}"))?;

                let mut details = chain_connection().details.clone();
                details.finalized_block_number = Some(block.number().to_string());
                details.finalized_block_hash = Some(block.hash().to_string());
                details.spec_version = Some(finalized_at.spec_version());
                details.transaction_version = Some(finalized_at.transaction_version());
                chain_connection.set(ChainConnection::connected(details));
            }
        }
    }
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

async fn watch_indexer(mut indexer_connection: Signal<IndexerConnection>) {
    loop {
        let details = indexer_connection().details.clone();
        indexer_connection.set(IndexerConnection::connecting(details));

        let result = maintain_indexer_connection(indexer_connection).await;

        let error = match result {
            Ok(()) => format!("Lost connection to {INDEXER_URL}"),
            Err(error) => error,
        };

        let details = indexer_connection().details.clone();
        indexer_connection.set(IndexerConnection::reconnecting(details, error));
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn maintain_indexer_connection(
    indexer_connection: Signal<IndexerConnection>,
) -> Result<(), String> {
    let (stream, _) = connect_async(INDEXER_URL)
        .await
        .map_err(|error| format!("Failed to connect to {INDEXER_URL}: {error}"))?;

    let (mut sender, mut receiver) = stream.split();

    sender
        .send(Message::Text(r#"{"type":"SubscribeStatus"}"#.into()))
        .await
        .map_err(|error| format!("Failed to subscribe to indexer status: {error}"))?;

    sender
        .send(Message::Text(r#"{"type":"Status"}"#.into()))
        .await
        .map_err(|error| format!("Failed to request indexer status snapshot: {error}"))?;

    while let Some(message) = receiver.next().await {
        let message =
            message.map_err(|error| format!("Failed to read indexer message: {error}"))?;

        match message {
            Message::Text(payload) => apply_indexer_message(indexer_connection, payload.as_ref())?,
            Message::Binary(payload) => {
                let payload = std::str::from_utf8(&payload)
                    .map_err(|error| format!("Indexer sent invalid UTF-8 payload: {error}"))?;
                apply_indexer_message(indexer_connection, payload)?;
            }
            Message::Close(_) => return Ok(()),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    Ok(())
}

fn apply_indexer_message(
    mut indexer_connection: Signal<IndexerConnection>,
    payload: &str,
) -> Result<(), String> {
    let response = serde_json::from_str::<IndexerResponse>(payload)
        .map_err(|error| format!("Failed to decode indexer response: {error}"))?;

    if response.message_type == "status" {
        let spans = response
            .data
            .ok_or_else(|| "Indexer status response was missing span data".to_string())?;
        indexer_connection.set(IndexerConnection::connected(IndexerDetails { spans }));
    }

    Ok(())
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
