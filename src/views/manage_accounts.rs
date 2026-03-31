use crate::{
    accounts::{create_account, delete_account, AccountStore},
    ChainConnection, Route, ACUITY_NODE_URL,
};
use dioxus::prelude::*;
use fast_qr::{
    convert::{svg::SvgBuilder, Builder, Shape},
    QRBuilder,
};
use std::{collections::{HashMap, HashSet}, time::Duration};
use subxt::{dynamic, utils::AccountId32, OnlineClient, PolkadotConfig};

const MANAGE_ACCOUNTS_CSS: Asset = asset!("/assets/styling/manage_accounts.css");
const BALANCE_RECONNECT_DELAY: Duration = Duration::from_secs(2);

// ── Balance helpers ───────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct TokenFormat {
    decimals: u8,
    symbol: String,
}

impl Default for TokenFormat {
    fn default() -> Self {
        Self {
            decimals: 12,
            symbol: "UNIT".to_string(),
        }
    }
}

#[derive(Clone, PartialEq)]
struct BalanceState {
    value: Option<u128>,
    error: Option<String>,
    loading: bool,
}

impl BalanceState {
    fn loading() -> Self {
        Self {
            value: None,
            error: None,
            loading: true,
        }
    }

    fn ready(value: u128) -> Self {
        Self {
            value: Some(value),
            error: None,
            loading: false,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            value: None,
            error: Some(error),
            loading: false,
        }
    }
}

type BalanceStore = Signal<HashMap<String, BalanceState>>;

fn format_balance(raw: u128, fmt: &TokenFormat) -> String {
    if fmt.decimals == 0 {
        return format!("{} {}", raw, fmt.symbol);
    }
    let divisor = 10u128.pow(fmt.decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    // Show up to 4 significant fractional digits
    let frac_str = format!("{:0>width$}", frac, width = fmt.decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    if trimmed.is_empty() {
        format!("{} {}", whole, fmt.symbol)
    } else {
        let display = &trimmed[..trimmed.len().min(4)];
        format!("{}.{} {}", whole, display, fmt.symbol)
    }
}

fn decode_ss58(address: &str) -> Result<[u8; 32], String> {
    use sp_core::{crypto::Ss58Codec, sr25519::Public};
    let public = Public::from_ss58check(address)
        .map_err(|e| format!("Invalid SS58 address: {e:?}"))?;
    Ok(public.0)
}

/// Query free balance for a given SS58 address using dynamic value decoding.
async fn fetch_balance_with_client(
    client: &OnlineClient<PolkadotConfig>,
    address: &str,
) -> Result<u128, String> {
    use subxt::dynamic::Value;

    let addr_bytes = decode_ss58(address)?;
    let account_id = AccountId32(addr_bytes);

    // Use the Value-typed dynamic storage query (no custom decode type needed)
    let storage_addr = dynamic::storage::<(AccountId32,), Value>("System", "Account");

    let at_block = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to get latest block: {e}"))?;

    // try_fetch returns None if the account doesn't exist yet (zero balance)
    let maybe_value = at_block
        .storage()
        .try_fetch(storage_addr, (account_id,))
        .await
        .map_err(|e| format!("Storage query failed: {e}"))?;

    let Some(value_thunk) = maybe_value else {
        return Ok(0);
    };

    let value = value_thunk
        .decode()
        .map_err(|e| format!("Decode failed: {e}"))?;

    // Navigate composite: { data: { free: u128, ... }, ... }
    let free = extract_free_balance(&value)
        .ok_or_else(|| "Could not read free balance from account storage value".to_string())?;

    Ok(free)
}

fn render_balance(balance: Option<&BalanceState>, fmt: &TokenFormat) -> Element {
    match balance {
        None => rsx! { span { class: "ma-loading", "…" } },
        Some(balance) if balance.loading => rsx! { span { class: "ma-loading", "…" } },
        Some(balance) => {
            if let Some(raw) = balance.value {
                let label = format_balance(raw, fmt);
                rsx! { span { "{label}" } }
            } else if let Some(error) = balance.error.as_deref() {
                rsx! { span { class: "ma-error-cell", title: "{error}", "Error" } }
            } else {
                rsx! { span { class: "ma-loading", "…" } }
            }
        }
    }
}

fn format_balance_label(balance: Option<&BalanceState>, fmt: &TokenFormat) -> String {
    match balance {
        None => "Loading…".to_string(),
        Some(balance) if balance.loading => "Loading…".to_string(),
        Some(balance) => {
            if let Some(raw) = balance.value {
                format_balance(raw, fmt)
            } else if let Some(error) = balance.error.as_deref() {
                format!("Error: {error}")
            } else {
                "Loading…".to_string()
            }
        }
    }
}

async fn watch_balances(mut balances: BalanceStore, addresses: Vec<String>) {
    if addresses.is_empty() {
        balances.set(HashMap::new());
        return;
    }

    loop {
        match maintain_balance_subscription(balances, &addresses).await {
            Ok(()) => return,
            Err(error) => {
                balances.with_mut(|store| {
                    for address in &addresses {
                        store.insert(address.clone(), BalanceState::failed(error.clone()));
                    }
                });
                tokio::time::sleep(BALANCE_RECONNECT_DELAY).await;
            }
        }
    }
}

async fn maintain_balance_subscription(
    mut balances: BalanceStore,
    addresses: &[String],
) -> Result<(), String> {

    let tracked_accounts = addresses
        .iter()
        .map(|address| decode_ss58(address).map(|bytes| (bytes, address.clone())))
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| error.to_string())?;

    let client = OnlineClient::<PolkadotConfig>::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Connection failed: {error}"))?;

    for address in addresses {
        match fetch_balance_with_client(&client, address).await {
            Ok(value) => balances.with_mut(|store| {
                store.insert(address.clone(), BalanceState::ready(value));
            }),
            Err(error) => balances.with_mut(|store| {
                store.insert(address.clone(), BalanceState::failed(error));
            }),
        }
    }

    let mut blocks = client
        .stream_blocks()
        .await
        .map_err(|error| format!("Failed to subscribe to finalized blocks: {error}"))?;

    while let Some(block) = blocks.next().await {
        let block = block.map_err(|error| format!("Failed to read finalized block: {error}"))?;

        let at = block
            .at()
            .await
            .map_err(|error| format!("Failed to inspect finalized block: {error}"))?;

        let events = at
            .events()
            .fetch()
            .await
            .map_err(|error| format!("Failed to fetch finalized block events: {error}"))?;

        let mut changed_addresses = HashSet::new();
        for event in events.iter() {
            let event = event
                .map_err(|error| format!("Failed to decode finalized block event: {error}"))?;

            match (event.pallet_name(), event.event_name()) {
                ("Balances", "Transfer") => {
                    track_event_accounts::<(AccountId32, AccountId32, u128)>(
                        &event,
                        &tracked_accounts,
                        &mut changed_addresses,
                        &[0, 1],
                    );
                }
                ("Balances", "Deposit")
                | ("Balances", "Withdraw")
                | ("Balances", "Slashed")
                | ("Balances", "Minted")
                | ("Balances", "Burned") => {
                    track_event_accounts::<(AccountId32, u128)>(
                        &event,
                        &tracked_accounts,
                        &mut changed_addresses,
                        &[0],
                    );
                }
                ("Balances", "DustLost")
                | ("Balances", "Endowed") => {
                    track_event_accounts::<(AccountId32, u128)>(
                        &event,
                        &tracked_accounts,
                        &mut changed_addresses,
                        &[0],
                    );
                }
                ("System", "KilledAccount") | ("System", "NewAccount") => {
                    track_event_accounts::<(AccountId32,)>(
                        &event,
                        &tracked_accounts,
                        &mut changed_addresses,
                        &[0],
                    );
                }
                _ => {}
            }
        }

        for address in changed_addresses {
            match fetch_balance_with_client(&client, &address).await {
                Ok(value) => balances.with_mut(|store| {
                    store.insert(address.clone(), BalanceState::ready(value));
                }),
                Err(error) => balances.with_mut(|store| {
                    store.insert(address.clone(), BalanceState::failed(error));
                }),
            }
        }
    }

    Ok(())
}

fn track_event_accounts<E>(
    event: &subxt::events::Event<PolkadotConfig>,
    tracked_accounts: &HashMap<[u8; 32], String>,
    changed_addresses: &mut HashSet<String>,
    indexes: &[usize],
) where
    E: parity_scale_codec::Decode + subxt::ext::scale_decode::DecodeAsFields + EventAccounts,
{
    let Ok(decoded) = event.decode_fields_unchecked_as::<E>() else {
        return;
    };

    for account in account_ids_from_tuple(&decoded, indexes) {
        if let Some(address) = tracked_accounts.get(&account) {
            changed_addresses.insert(address.clone());
        }
    }
}

trait EventAccounts {
    fn account_ids(&self) -> Vec<[u8; 32]>;
}

impl EventAccounts for (AccountId32,) {
    fn account_ids(&self) -> Vec<[u8; 32]> {
        vec![self.0 .0]
    }
}

impl EventAccounts for (AccountId32, u128) {
    fn account_ids(&self) -> Vec<[u8; 32]> {
        vec![self.0 .0]
    }
}

impl EventAccounts for (AccountId32, AccountId32, u128) {
    fn account_ids(&self) -> Vec<[u8; 32]> {
        vec![self.0 .0, self.1 .0]
    }
}

fn account_ids_from_tuple<E>(decoded: &E, indexes: &[usize]) -> Vec<[u8; 32]>
where
    E: EventAccounts,
{
    decoded
        .account_ids()
        .into_iter()
        .enumerate()
        .filter_map(|(index, account)| indexes.contains(&index).then_some(account))
        .collect()
}

/// Walk a scale_value::Value composite to extract the `data.free` field as u128.
fn extract_free_balance(value: &subxt::dynamic::Value) -> Option<u128> {
    use subxt::ext::scale_value::At;
    // Navigate: AccountInfo { data: AccountData { free: u128 } }
    value
        .at("data")
        .and_then(|data| data.at("free"))
        .and_then(|free| free.as_u128())
}

/// Generate a QR code SVG string for the given data.
fn qr_svg(data: &str) -> String {
    let Ok(qr) = QRBuilder::new(data).build() else {
        return String::new();
    };
    SvgBuilder::default()
        .shape(Shape::RoundedSquare)
        .to_str(&qr)
}

// ── Manage Accounts page ──────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum Dialog {
    None,
    Fund(String),   // account_id
    Delete(String), // account_id
}

#[component]
pub fn ManageAccounts() -> Element {
    let account_store = use_context::<Signal<AccountStore>>();
    let snap = account_store();

    let mut dialog = use_signal(|| Dialog::None);
    let balances: BalanceStore = use_signal(HashMap::new);
    let addresses: Vec<String> = snap
        .accounts
        .iter()
        .map(|account| account.address.clone())
        .collect();

    use_effect(move || {
        let addresses = addresses.clone();
        let mut balances = balances;

        balances.with_mut(|store| {
            let address_set: HashSet<&str> = addresses.iter().map(String::as_str).collect();
            store.retain(|address, _| address_set.contains(address.as_str()));
            for address in &addresses {
                store.entry(address.clone()).or_insert_with(BalanceState::loading);
            }
        });

        spawn(async move {
            watch_balances(balances, addresses).await;
        });
    });

    // Derive token format from chain properties; fall back to generic defaults.
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let fmt = {
        let details = &chain_connection().details;
        TokenFormat {
            symbol: details
                .token_symbol
                .clone()
                .unwrap_or_else(|| "UNIT".to_string()),
            decimals: details.token_decimals.unwrap_or(12),
        }
    };

    rsx! {
        document::Link { rel: "stylesheet", href: MANAGE_ACCOUNTS_CSS }

        div {
            class: "ma-shell",

            div {
                class: "ma-header",
                h1 { class: "ma-title", "Manage Accounts" }
            }

            if let Some(error_message) = snap.error_message.clone() {
                div { class: "ma-banner error", "{error_message}" }
            }
            if let Some(notice_message) = snap.notice_message.clone() {
                div { class: "ma-banner notice", "{notice_message}" }
            }

            if snap.accounts.is_empty() {
                div {
                    class: "ma-empty",
                    p { "No accounts found. Create your first account below." }
                }
            } else {
                div {
                    class: "ma-table-wrap",
                    table {
                        class: "ma-table",
                        thead {
                            tr {
                                th { "Name" }
                                th { "Balance" }
                                th { "Locked" }
                                th { "Actions" }
                            }
                        }
                        tbody {
                            for account in snap.accounts.clone() {
                                {
                                    let account_id = account.id.clone();
                                    let fund_id = account.id.clone();
                                    let delete_id = account.id.clone();
                                    let is_locked = !snap.is_account_unlocked(&account.id);
                                    let address = account.address.clone();
                                    let fmt_clone = fmt.clone();
                                    let balance_cell = render_balance(balances.read().get(&address), &fmt_clone);

                                    rsx! {
                                        tr { key: "{account_id}",
                                            td { class: "ma-td-name", "{account.name}" }
                                            td { class: "ma-td-balance", {balance_cell} }
                                            td { class: "ma-td-locked",
                                                span {
                                                    class: if is_locked { "ma-badge locked" } else { "ma-badge unlocked" },
                                                    if is_locked { "Yes" } else { "No" }
                                                }
                                            }
                                            td { class: "ma-td-actions",
                                                button {
                                                    class: "ma-btn fund",
                                                    onclick: move |_| dialog.set(Dialog::Fund(fund_id.clone())),
                                                    "Fund"
                                                }
                                                button {
                                                    class: "ma-btn danger",
                                                    onclick: move |_| dialog.set(Dialog::Delete(delete_id.clone())),
                                                    "Delete"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div {
                class: "ma-footer",
                Link {
                    to: Route::CreateAccount {},
                    class: "ma-create-link",
                    "+ Create account"
                }
            }
        }

        // Dialogs rendered outside the table so they overlay correctly
        match dialog() {
            Dialog::Fund(ref account_id) => {
                let id = account_id.clone();
                let account = snap.accounts.iter().find(|a| a.id == id).cloned();
                if let Some(acct) = account {
                    rsx! {
                        FundDialog {
                            account_name: acct.name.clone(),
                            address: acct.address.clone(),
                            token_fmt: fmt.clone(),
                            balance: balances.read().get(&acct.address).cloned(),
                            on_close: move |_| dialog.set(Dialog::None),
                        }
                    }
                } else {
                    rsx! {}
                }
            }
            Dialog::Delete(ref account_id) => {
                let id = account_id.clone();
                let account = snap.accounts.iter().find(|a| a.id == id).cloned();
                if let Some(acct) = account {
                    rsx! {
                        DeleteDialog {
                            account_id: acct.id.clone(),
                            account_name: acct.name.clone(),
                            on_close: move |_| dialog.set(Dialog::None),
                        }
                    }
                } else {
                    rsx! {}
                }
            }
            Dialog::None => rsx! {},
        }
    }
}

// ── Fund dialog ───────────────────────────────────────────────────────────────

#[component]
fn FundDialog(
    account_name: String,
    address: String,
    token_fmt: TokenFormat,
    balance: Option<BalanceState>,
    on_close: EventHandler<()>,
) -> Element {
    let qr = qr_svg(&address);
    let fmt = token_fmt.clone();
    let balance_label = format_balance_label(balance.as_ref(), &fmt);

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| on_close.call(()),

            div {
                class: "modal-card fund-modal",
                onclick: move |evt| evt.stop_propagation(),

                p { class: "modal-title", "Fund account" }
                p { class: "modal-account-name", "{account_name}" }

                div {
                    class: "fund-qr",
                    dangerous_inner_html: "{qr}",
                }

                div {
                    class: "fund-address",
                    p { class: "fund-address-label", "Address" }
                    p { class: "fund-address-value", "{address}" }
                }

                div {
                    class: "fund-balance",
                    p { class: "fund-balance-label", "Balance" }
                    p { class: "fund-balance-value", "{balance_label}" }
                }

                div {
                    class: "modal-actions",
                    button {
                        class: "modal-btn-secondary",
                        onclick: move |_| on_close.call(()),
                        "Close"
                    }
                }
            }
        }
    }
}

// ── Delete dialog ─────────────────────────────────────────────────────────────

#[component]
fn DeleteDialog(
    account_id: String,
    account_name: String,
    on_close: EventHandler<()>,
) -> Element {
    let mut account_store = use_context::<Signal<AccountStore>>();
    let id = account_id.clone();

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| on_close.call(()),

            div {
                class: "modal-card",
                onclick: move |evt| evt.stop_propagation(),

                p { class: "modal-title", "Delete account" }
                p { class: "modal-account-name", "{account_name}" }

                p {
                    class: "delete-warning",
                    "This will permanently delete the account keystore file. This action cannot be undone."
                }

                div {
                    class: "modal-actions",
                    button {
                        class: "modal-btn-danger",
                        onclick: move |_| {
                            let delete_id = id.clone();
                            account_store.with_mut(|store| delete_account(store, &delete_id));
                            on_close.call(());
                        },
                        "Delete"
                    }
                    button {
                        class: "modal-btn-secondary",
                        onclick: move |_| on_close.call(()),
                        "Cancel"
                    }
                }
            }
        }
    }
}

// ── Create Account page ───────────────────────────────────────────────────────

#[component]
pub fn CreateAccount() -> Element {
    let mut account_store = use_context::<Signal<AccountStore>>();
    let mut create_name = use_signal(String::new);
    let mut create_password = use_signal(String::new);
    let navigator = use_navigator();

    rsx! {
        document::Link { rel: "stylesheet", href: MANAGE_ACCOUNTS_CSS }

        div {
            class: "ma-shell",
            div {
                class: "ma-header",
                h1 { class: "ma-title", "Create Account" }
            }

            if let Some(error_message) = account_store().error_message.clone() {
                div { class: "ma-banner error", "{error_message}" }
            }

            div {
                class: "ma-create-form panel",

                p { class: "panel-copy",
                    "Generate a fresh sr25519 account and save it as a Polkadot-JS compatible JSON file."
                }

                label {
                    class: "field",
                    span { "Name" }
                    input {
                        value: create_name,
                        placeholder: "My account",
                        oninput: move |e| create_name.set(e.value()),
                    }
                }

                label {
                    class: "field",
                    span { "Password" }
                    input {
                        r#type: "password",
                        value: create_password,
                        placeholder: "Protect this account file",
                        oninput: move |e| create_password.set(e.value()),
                    }
                }

                div {
                    class: "ma-create-actions",
                    button {
                        class: "primary-action",
                        onclick: move |_| {
                            let name = create_name();
                            let password = create_password();
                            account_store.with_mut(|store| create_account(store, &name, &password));
                            if account_store().error_message.is_none() {
                                navigator.push(Route::ManageAccounts {});
                            }
                        },
                        "Create account"
                    }
                    button {
                        class: "secondary-action",
                        onclick: move |_| { navigator.push(Route::ManageAccounts {}); },
                        "Cancel"
                    }
                }
            }
        }
    }
}
