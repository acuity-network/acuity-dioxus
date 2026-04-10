use dioxus::prelude::*;
#[cfg(feature = "desktop")]
use dioxus::desktop::{Config as DesktopConfig, WindowEvent, tao::event::Event};
use acuity_index_api_rs::{Bytes32, CustomKey, CustomValue, IndexerClient, Key, Span as IndexerSpan};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::watch;

use acuity_runtime::api;
use accounts::load_account_store;
use runtime_client::connect as connect_acuity_client;
use views::{
    ChainStatus, CreateAccount, Home, IndexerStatus, IpfsStatus, ItemView, ManageAccounts, Navbar,
    ProfileEdit, ProfileView, PublishFeed, PublishPost,
};

pub(crate) const ACUITY_NODE_URL: &str = "ws://127.0.0.1:9944";
pub(crate) const INDEXER_URL: &str = "ws://127.0.0.1:8172";
const IPFS_DAEMON_ADDR: &str = "/ip4/127.0.0.1/tcp/5001";
pub(crate) const IPFS_API_URL: &str = "http://127.0.0.1:5001";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const IPFS_HEALTHCHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct AppShutdown {
    tx: watch::Sender<bool>,
}

impl AppShutdown {
    fn new() -> Self {
        let (tx, _) = watch::channel(false);
        Self { tx }
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }

    fn trigger(&self) {
        let _ = self.tx.send(true);
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct ChainConnection {
    pub(crate) details: ChainDetails,
    pub(crate) status: ConnectionStatus,
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct ChainDetails {
    pub(crate) best_block_number: Option<String>,
    pub(crate) best_block_hash: Option<String>,
    pub(crate) finalized_block_number: Option<String>,
    pub(crate) finalized_block_hash: Option<String>,
    pub(crate) genesis_hash: Option<String>,
    pub(crate) ss58_prefix: Option<u16>,
    pub(crate) existential_deposit: Option<String>,
    pub(crate) spec_version: Option<u32>,
    pub(crate) transaction_version: Option<u32>,
    /// Free balance (planck) of the currently active account, or `None` if
    /// no account is selected / balance hasn't been fetched yet.
    pub(crate) active_account_balance: Option<u128>,
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

mod accounts;
mod acuity_runtime;
mod comment;
mod content;
mod feed;
mod item;
mod post;
mod profile;
mod runtime_client;
mod views;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Navbar)]
        #[route("/")]
        Home {},
        #[route("/accounts")]
        ManageAccounts {},
        #[route("/accounts/create")]
        CreateAccount {},
        #[route("/profile")]
        ProfileView {},
        #[route("/profile/edit")]
        ProfileEdit {},
        #[route("/feed/publish")]
        PublishFeed {},
        #[route("/feed/:encoded_feed_id/publish-post")]
        PublishPost { encoded_feed_id: String },
        #[route("/item/:encoded_item_id")]
        ItemView { encoded_item_id: String },
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
    #[cfg(feature = "desktop")]
    {
        let shutdown = AppShutdown::new();
        let shutdown_handler = shutdown.clone();
        dioxus::LaunchBuilder::desktop()
            .with_context(shutdown)
            .with_cfg(DesktopConfig::new().with_custom_event_handler(move |event, _| {
                if let Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } = event
                {
                    tracing::info!(target: "acuity_dioxus::shutdown", "received desktop close request");
                    shutdown_handler.trigger();
                }
            }))
            .launch(App);
    }

    #[cfg(not(feature = "desktop"))]
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let chain_connection = use_signal(ChainConnection::default);
    let indexer_connection = use_signal(IndexerConnection::default);
    let ipfs_connection = use_signal(IpfsConnection::default);
    let account_store = use_signal(load_account_store);
    #[cfg(feature = "desktop")]
    let shutdown = use_context::<AppShutdown>();

    #[cfg(not(feature = "desktop"))]
    let shutdown = use_hook(AppShutdown::new);
    use_context_provider(|| chain_connection);
    use_context_provider(|| indexer_connection);
    use_context_provider(|| ipfs_connection);
    use_context_provider(|| account_store);

    // Channel that publishes the active account address whenever it changes.
    // The balance watcher task uses this to know which account to subscribe to.
    let (active_address_tx, active_address_rx) = use_hook(|| {
        let initial = account_store()
            .active_account()
            .map(|a| a.address.clone());
        watch::channel(initial)
    });

    // Whenever the active account changes, push the new address to the watchers.
    use_effect(move || {
        let addr = account_store().active_account().map(|a| a.address.clone());
        let _ = active_address_tx.send(addr);
    });

    // Channel that shares the live IndexerClient so the balance watcher can
    // call subscribe_events on the same underlying WebSocket connection.
    let (indexer_client_tx, indexer_client_rx) = use_hook(|| {
        watch::channel::<Option<IndexerClient>>(None)
    });

    // Provide a shared IndexerClient to all components via context.
    // A background task watches the channel and keeps the signal in sync.
    let indexer_client: Signal<Option<IndexerClient>> = use_signal(|| None);
    use_context_provider(|| indexer_client);
    let _indexer_client_sync = use_hook({
        let mut rx = indexer_client_tx.subscribe();
        let mut indexer_client = indexer_client;
        move || {
            spawn(async move {
                loop {
                    if rx.changed().await.is_err() {
                        break;
                    }
                    indexer_client.set(rx.borrow().clone());
                }
            })
        }
    });

    let chain_shutdown = shutdown.clone();
    let _connection_task = use_hook(move || {
        let chain_connection = chain_connection;
        let shutdown = chain_shutdown.subscribe();

        spawn(async move {
            watch_acuity_chain(chain_connection, shutdown).await;
        })
    });

    let ipfs_shutdown = shutdown.clone();
    let _ipfs_connection_task = use_hook(move || {
        let ipfs_connection = ipfs_connection;
        let shutdown = ipfs_shutdown.subscribe();

        spawn(async move {
            watch_ipfs_daemon(ipfs_connection, shutdown).await;
        })
    });

    let indexer_shutdown = shutdown.clone();
    let _indexer_connection_task = use_hook(move || {
        let indexer_connection = indexer_connection;
        let shutdown = indexer_shutdown.subscribe();

        spawn(async move {
            watch_indexer(indexer_connection, indexer_client_tx, shutdown).await;
        })
    });

    let balance_shutdown = shutdown.clone();
    let _balance_task = use_hook(move || {
        let active_address_rx = active_address_rx;
        let shutdown = balance_shutdown.subscribe();

        spawn(async move {
            watch_active_account_balance(
                chain_connection,
                active_address_rx,
                indexer_client_rx,
                shutdown,
            )
            .await;
        })
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }

        Router::<Route> {}
    }
}

async fn watch_acuity_chain(
    mut chain_connection: Signal<ChainConnection>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            tracing::info!(target: "acuity_dioxus::shutdown", "shutting down chain connection watcher");
            return;
        }

        let details = chain_connection().details.clone();
        chain_connection.set(ChainConnection::connecting(details));

        let result = stream_best_blocks(chain_connection, shutdown.clone()).await;

        let error = match result {
            Ok(()) => "Block stream ended".to_string(),
            Err(error) => error,
        };

        let details = chain_connection().details.clone();
        chain_connection.set(ChainConnection::reconnecting(details, error));

        tokio::select! {
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "shutting down chain connection watcher");
                return;
            }
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
        }
    }
}

async fn stream_best_blocks(
    mut chain_connection: Signal<ChainConnection>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let client = connect_acuity_client().await?;

    let finalized_block = client
        .at_current_block()
        .await
        .map_err(|error| format!("Failed to inspect the latest finalized block: {error}"))?;

    let version = finalized_block
        .constants()
        .entry(&api::constants().system().version())
        .map_err(|error| format!("Failed to read System::Version constant: {error}"))?;
    let ss58_prefix = finalized_block
        .constants()
        .entry(&api::constants().system().ss58_prefix())
        .map_err(|error| format!("Failed to read System::SS58Prefix constant: {error}"))?;
    let existential_deposit = finalized_block
        .constants()
        .entry(&api::constants().balances().existential_deposit())
        .map_err(|error| format!("Failed to read Balances::ExistentialDeposit constant: {error}"))?;

    let mut details = chain_connection().details.clone();
    let finalized_block_number = finalized_block.block_number().to_string();
    let finalized_block_hash = finalized_block.block_hash().to_string();

    details.finalized_block_number = Some(finalized_block_number.clone());
    details.finalized_block_hash = Some(finalized_block_hash.clone());
    details.best_block_number = Some(finalized_block_number);
    details.best_block_hash = Some(finalized_block_hash);
    details.genesis_hash = Some(client.genesis_hash().to_string());
    details.ss58_prefix = Some(ss58_prefix);
    details.existential_deposit = Some(existential_deposit.to_string());
    details.spec_version = Some(version.spec_version);
    details.transaction_version = Some(version.transaction_version);
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
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "stopping chain block subscriptions");
                return Ok(());
            }
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
                let version = finalized_at
                    .constants()
                    .entry(&api::constants().system().version())
                    .map_err(|error| format!("Failed to read System::Version constant: {error}"))?;
                details.spec_version = Some(version.spec_version);
                details.transaction_version = Some(version.transaction_version);
                chain_connection.set(ChainConnection::connected(details));
            }
        }
    }
}

async fn watch_ipfs_daemon(
    mut ipfs_connection: Signal<IpfsConnection>,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = Client::new();

    loop {
        if *shutdown.borrow() {
            tracing::info!(target: "acuity_dioxus::shutdown", "shutting down IPFS connection watcher");
            return;
        }

        let details = ipfs_connection().details.clone();
        ipfs_connection.set(IpfsConnection::connecting(details));

        let result = maintain_ipfs_connection(&client, ipfs_connection, shutdown.clone()).await;

        let error = match result {
            Ok(()) => format!("Lost connection to {IPFS_DAEMON_ADDR}"),
            Err(error) => error,
        };

        let details = ipfs_connection().details.clone();
        ipfs_connection.set(IpfsConnection::reconnecting(details, error));

        tokio::select! {
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "shutting down IPFS connection watcher");
                return;
            }
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
        }
    }
}

async fn watch_indexer(
    mut indexer_connection: Signal<IndexerConnection>,
    indexer_client_tx: watch::Sender<Option<IndexerClient>>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            tracing::info!(target: "acuity_dioxus::shutdown", "shutting down indexer connection watcher");
            return;
        }

        let details = indexer_connection().details.clone();
        indexer_connection.set(IndexerConnection::connecting(details));

        let result = maintain_indexer_connection(
            indexer_connection,
            &indexer_client_tx,
            shutdown.clone(),
        )
        .await;

        // Signal to the balance watcher that the indexer is no longer connected.
        let _ = indexer_client_tx.send(None);

        let error = match result {
            Ok(()) => format!("Lost connection to {INDEXER_URL}"),
            Err(error) => error,
        };

        let details = indexer_connection().details.clone();
        indexer_connection.set(IndexerConnection::reconnecting(details, error));

        tokio::select! {
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "shutting down indexer connection watcher");
                return;
            }
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
        }
    }
}

async fn maintain_indexer_connection(
    mut indexer_connection: Signal<IndexerConnection>,
    indexer_client_tx: &watch::Sender<Option<IndexerClient>>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let client = IndexerClient::connect(INDEXER_URL)
        .await
        .map_err(|error| format!("Failed to connect to {INDEXER_URL}: {error}"))?;
    let spans = client
        .status()
        .await
        .map_err(|error| format!("Failed to request indexer status snapshot: {error}"))?;

    indexer_connection.set(IndexerConnection::connected(IndexerDetails { spans }));

    // Share the live client with the balance watcher task.
    let _ = indexer_client_tx.send(Some(client.clone()));

    let mut subscription = client
        .subscribe_status()
        .await
        .map_err(|error| format!("Failed to subscribe to indexer status: {error}"))?;

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "closing indexer status subscription");
                break;
            }
            update = subscription.next() => {
                let Some(update) = update else {
                    break;
                };

                let update = update.map_err(|error| format!("Failed to read indexer message: {error}"))?;
                indexer_connection.set(IndexerConnection::connected(IndexerDetails {
                    spans: update.spans,
                }));
            }
        }
    }

    subscription
        .unsubscribe()
        .await
        .map_err(|error| format!("Failed to unsubscribe from indexer status: {error}"))?;
    tracing::info!(target: "acuity_dioxus::shutdown", "unsubscribed from indexer status");

    client
        .close()
        .await
        .map_err(|error| format!("Failed to close indexer connection: {error}"))?;
    tracing::info!(target: "acuity_dioxus::shutdown", "closed indexer websocket");

    Ok(())
}

/// Watches for account-tagged indexer events and re-fetches the active
/// account's balance whenever one arrives.
///
/// This replaces the previous approach of re-fetching on every finalised
/// block: instead the indexer's `subscribe_events` with `Key::Custom {
/// name: "account_id" }` fires only when an event that touches the account
/// has been indexed, so balance queries are driven by actual on-chain
/// activity.
async fn watch_active_account_balance(
    mut chain_connection: Signal<ChainConnection>,
    mut active_address_rx: watch::Receiver<Option<String>>,
    mut indexer_client_rx: watch::Receiver<Option<IndexerClient>>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        // ── Wait for a non-None active account address ──────────────────────
        let address = loop {
            let addr = active_address_rx.borrow_and_update().clone();
            if let Some(a) = addr {
                break a;
            }
            // No active account — clear the balance and wait.
            chain_connection.with_mut(|c| c.details.active_account_balance = None);
            tokio::select! {
                _ = shutdown.changed() => return,
                _ = active_address_rx.changed() => {}
            }
        };

        // Decode the SS58 address to raw bytes for the indexer key.
        let raw_bytes = match decode_ss58_bytes(&address) {
            Some(b) => b,
            None => {
                tracing::warn!(
                    target: "acuity_dioxus::balance",
                    "Could not decode active account address '{address}' as SS58; skipping balance watch"
                );
                tokio::select! {
                    _ = shutdown.changed() => return,
                    _ = active_address_rx.changed() => {}
                }
                continue;
            }
        };

        // ── Initial balance fetch via Subxt ──────────────────────────────────
        match connect_acuity_client().await {
            Ok(client) => {
                match runtime_client::fetch_account_balance(&client, &address).await {
                    Ok(balance) => chain_connection
                        .with_mut(|c| c.details.active_account_balance = Some(balance)),
                    Err(error) => tracing::warn!(
                        target: "acuity_dioxus::balance",
                        "Initial balance fetch failed for {address}: {error}"
                    ),
                }
            }
            Err(error) => tracing::warn!(
                target: "acuity_dioxus::balance",
                "Could not connect to node for initial balance fetch: {error}"
            ),
        }

        // ── Wait for a live IndexerClient ────────────────────────────────────
        let indexer_client = {
            let mut result: Option<IndexerClient> = None;
            loop {
                let client = indexer_client_rx.borrow_and_update().clone();
                if let Some(c) = client {
                    result = Some(c);
                    break;
                }
                tokio::select! {
                    _ = shutdown.changed() => return,
                    _ = active_address_rx.changed() => {
                        // Address changed while waiting for indexer — restart outer loop.
                        break;
                    }
                    _ = indexer_client_rx.changed() => {}
                }
            }
            match result {
                Some(c) => c,
                None => continue, // active address changed; re-enter outer loop
            }
        };

        // ── Subscribe to account-tagged events ──────────────────────────────
        let key = Key::Custom(CustomKey {
            name: "account_id".to_string(),
            value: CustomValue::Bytes32(Bytes32(raw_bytes)),
        });

        let mut sub = match indexer_client.subscribe_events(key).await {
            Ok(s) => s,
            Err(error) => {
                tracing::warn!(
                    target: "acuity_dioxus::balance",
                    "Failed to subscribe to account events for {address}: {error}"
                );
                tokio::select! {
                    _ = shutdown.changed() => return,
                    _ = active_address_rx.changed() => {}
                    _ = indexer_client_rx.changed() => {}
                }
                continue;
            }
        };

        tracing::info!(
            target: "acuity_dioxus::balance",
            "Subscribed to account events for {address}"
        );

        // ── Inner event loop ─────────────────────────────────────────────────
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    tracing::info!(
                        target: "acuity_dioxus::balance",
                        "shutting down balance watcher"
                    );
                    let _ = sub.unsubscribe().await;
                    return;
                }
                _ = active_address_rx.changed() => {
                    // Active account switched — resubscribe for the new address.
                    let _ = sub.unsubscribe().await;
                    break;
                }
                _ = indexer_client_rx.changed() => {
                    // Indexer reconnected — resubscribe on the new client.
                    let _ = sub.unsubscribe().await;
                    break;
                }
                notification = sub.next() => {
                    match notification {
                        None => {
                            // Subscription stream ended (indexer disconnected).
                            tracing::warn!(
                                target: "acuity_dioxus::balance",
                                "Account event subscription ended for {address}; will resubscribe"
                            );
                            break;
                        }
                        Some(Err(error)) => {
                            tracing::warn!(
                                target: "acuity_dioxus::balance",
                                "Account event subscription error for {address}: {error}"
                            );
                            break;
                        }
                        Some(Ok(_)) => {
                            // An event touching this account was indexed — re-fetch balance.
                            match connect_acuity_client().await {
                                Ok(client) => {
                                    match runtime_client::fetch_account_balance(&client, &address).await {
                                        Ok(balance) => chain_connection.with_mut(|c| {
                                            c.details.active_account_balance = Some(balance);
                                        }),
                                        Err(error) => tracing::warn!(
                                            target: "acuity_dioxus::balance",
                                            "Balance re-fetch failed for {address}: {error}"
                                        ),
                                    }
                                }
                                Err(error) => tracing::warn!(
                                    target: "acuity_dioxus::balance",
                                    "Could not connect for balance re-fetch: {error}"
                                ),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Decodes an SS58-encoded address to its raw 32-byte public key.
fn decode_ss58_bytes(address: &str) -> Option<[u8; 32]> {
    use sp_core::{crypto::Ss58Codec, sr25519::Public};
    Public::from_ss58check(address).ok().map(|p| p.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_indexed_block_returns_highest_span_end() {
        let details = IndexerDetails {
            spans: vec![IndexerSpan { start: 1, end: 8 }, IndexerSpan { start: 20, end: 25 }],
        };

        assert_eq!(details.latest_indexed_block(), Some(25));
    }
}

async fn maintain_ipfs_connection(
    client: &Client,
    mut ipfs_connection: Signal<IpfsConnection>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    loop {
        let response = tokio::select! {
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "stopping IPFS healthcheck loop");
                return Ok(());
            }
            response = client.post(format!("{IPFS_API_URL}/api/v0/id")).send() => response,
        }
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

        tokio::select! {
            _ = shutdown.changed() => {
                tracing::info!(target: "acuity_dioxus::shutdown", "stopping IPFS healthcheck loop");
                return Ok(());
            }
            _ = tokio::time::sleep(IPFS_HEALTHCHECK_INTERVAL) => {}
        }
    }
}
