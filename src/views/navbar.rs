use crate::{
    accounts::{
        apply_unlock_result, lock_account, select_active_account, unlock_account_blocking,
        AccountStore,
    },
    ChainConnection, ConnectionStatus, IndexerConnection, IpfsConnection, Route,
};
use dioxus::prelude::*;

const NAVBAR_CSS: Asset = asset!("/assets/styling/navbar.css");
const SIDEBAR_CSS: Asset = asset!("/assets/styling/sidebar.css");

// ── Top navbar ────────────────────────────────────────────────────────────────

#[component]
pub fn Navbar() -> Element {
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let chain_connection = chain_connection();
    let indexer_connection = use_context::<Signal<IndexerConnection>>();
    let indexer_connection = indexer_connection();
    let ipfs_connection = use_context::<Signal<IpfsConnection>>();
    let ipfs_connection = ipfs_connection();
    let account_store = use_context::<Signal<AccountStore>>();
    let account_store_snap = account_store();

    let status_class = match chain_connection.status {
        ConnectionStatus::Connected => "connected",
        ConnectionStatus::Connecting => "connecting",
        ConnectionStatus::Reconnecting => "reconnecting",
    };

    let status_label = match chain_connection.status {
        ConnectionStatus::Connected => {
            match chain_connection.details.best_block_number.as_deref() {
                Some(block_number) => format!("Block {block_number}"),
                None => "Connected".to_string(),
            }
        }
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

    let indexer_status_class = match indexer_connection.status {
        ConnectionStatus::Connected => "connected",
        ConnectionStatus::Connecting => "connecting",
        ConnectionStatus::Reconnecting => "reconnecting",
    };

    let indexer_status_label = match indexer_connection.status {
        ConnectionStatus::Connected => match indexer_connection.details.latest_indexed_block() {
            Some(block_number) => format!("Indexed to {block_number}"),
            None => "Indexer connected".to_string(),
        },
        ConnectionStatus::Connecting => "Connecting to indexer".to_string(),
        ConnectionStatus::Reconnecting => "Reconnecting to indexer".to_string(),
    };

    let active_account = account_store_snap.active_account().cloned();
    let account_status_class = if account_store_snap.is_active_unlocked() {
        "unlocked"
    } else {
        "locked"
    };

    let account_label = match active_account {
        Some(account) => {
            let address = short_address(&account.address);
            if account_store_snap.is_active_unlocked() {
                format!("{} ({address}) unlocked", account.name)
            } else {
                format!("{} ({address}) locked", account.name)
            }
        }
        None => "No active account".to_string(),
    };

    rsx! {
        document::Link { rel: "stylesheet", href: NAVBAR_CSS }
        document::Link { rel: "stylesheet", href: SIDEBAR_CSS }

        div {
            id: "navbar",
            div {
                class: "nav-links",
                span { class: "brand", "Acuity" }
                Link {
                    to: Route::Home {},
                    "Dashboard"
                }
                Link {
                    to: Route::Profile {},
                    "Profile"
                }
            }
            div {
                class: "nav-statuses",
                div {
                    class: "account-status {account_status_class}",
                    "{account_label}"
                }
                Link {
                    class: "status-link chain-status {status_class}",
                    to: Route::ChainStatus {},
                    "{status_label}"
                }
                Link {
                    class: "status-link indexer-status {indexer_status_class}",
                    to: Route::IndexerStatus {},
                    "{indexer_status_label}"
                }
                Link {
                    class: "status-link ipfs-status {ipfs_status_class}",
                    to: Route::IpfsStatus {},
                    "{ipfs_status_label}"
                }
            }
        }

        // Two-column body: sidebar on the left, outlet on the right
        div {
            class: "app-body",
            AccountSidebar {}
            div {
                class: "app-content",
                Outlet::<Route> {}
            }
        }
    }
}

// ── Account sidebar ───────────────────────────────────────────────────────────

#[component]
fn AccountSidebar() -> Element {
    let mut account_store = use_context::<Signal<AccountStore>>();
    let snap = account_store();

    // Which account's unlock modal is open (None = closed)
    let mut unlock_target_id: Signal<Option<String>> = use_signal(|| None);

    rsx! {
        aside {
            class: "account-sidebar",

            p { class: "sidebar-heading", "Accounts" }

            if snap.accounts.is_empty() {
                p { class: "sidebar-empty", "No accounts found." }
            }

            for account in snap.accounts.clone() {
                {
                    let is_active = snap.active_account_id.as_deref() == Some(account.id.as_str());
                    let is_unlocked = snap.is_account_unlocked(&account.id);
                    let row_class = if is_active { "sidebar-account active" } else { "sidebar-account" };
                    let select_id = account.id.clone();
                    let padlock_id = account.id.clone();

                    rsx! {
                        div {
                            class: "{row_class}",
                            // Clicking the row body selects the account
                            button {
                                class: "sidebar-account-body",
                                onclick: move |_| {
                                    account_store.with_mut(|store| select_active_account(store, &select_id));
                                },
                                span { class: "sidebar-account-name", "{account.name}" }
                                span { class: "sidebar-account-addr", "{short_address(&account.address)}" }
                            }
                            // Padlock button
                            if is_unlocked {
                                button {
                                    class: "padlock unlocked",
                                    title: "Lock account",
                                    onclick: move |_| {
                                        account_store.with_mut(|store| lock_account(store, &padlock_id));
                                    },
                                    "\u{1F513}" // 🔓
                                }
                            } else {
                                button {
                                    class: "padlock locked",
                                    title: "Unlock account",
                                    onclick: move |_| {
                                        unlock_target_id.set(Some(padlock_id.clone()));
                                    },
                                    "\u{1F512}" // 🔒
                                }
                            }
                        }
                    }
                }
            }
        }

        // Render modal outside the sidebar so it overlays the whole screen
        if let Some(target_id) = unlock_target_id() {
            UnlockModal {
                account_id: target_id,
                on_close: move |_| unlock_target_id.set(None),
            }
        }
    }
}

// ── Unlock modal ──────────────────────────────────────────────────────────────

#[component]
fn UnlockModal(account_id: String, on_close: EventHandler<()>) -> Element {
    let mut account_store = use_context::<Signal<AccountStore>>();
    let snap = account_store();

    let account = snap.accounts.iter().find(|a| a.id == account_id).cloned();

    let mut password = use_signal(String::new);
    let mut unlocking = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    let account_name = account
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "Unknown account".to_string());

    rsx! {
        // Backdrop — clicking it closes the modal
        div {
            class: "modal-backdrop",
            onclick: move |_| on_close.call(()),

            // Dialog card — clicks inside must not bubble to backdrop
            div {
                class: "modal-card",
                onclick: move |evt| evt.stop_propagation(),

                p { class: "modal-title", "Unlock account" }
                p { class: "modal-account-name", "{account_name}" }

                if let Some(err) = error_msg() {
                    p { class: "modal-error", "{err}" }
                }

                label {
                    class: "modal-field",
                    span { "Password" }
                    input {
                        r#type: "password",
                        value: password,
                        placeholder: "Enter account password",
                        autofocus: true,
                        oninput: move |e| {
                            password.set(e.value());
                            error_msg.set(None);
                        },
                        onkeydown: {
                            let account_id = account_id.clone();
                            let account = account.clone();
                            move |e: KeyboardEvent| {
                                if e.key() == Key::Enter {
                                    let pw = password();
                                    if pw.is_empty() {
                                        error_msg.set(Some("Enter a password.".to_string()));
                                        return;
                                    }
                                    let Some(ref acct) = account else { return; };
                                    let path = acct.path.clone();
                                    let id = account_id.clone();
                                    let name = acct.name.clone();
                                    unlocking.set(true);
                                    spawn(async move {
                                        let result = tokio::task::spawn_blocking(move || {
                                            unlock_account_blocking(path, id, name, pw)
                                        })
                                        .await
                                        .unwrap_or_else(|e| Err(format!("Task panicked: {e}")));
                                        let failed = result.is_err();
                                        if failed {
                                            if let Err(ref msg) = result {
                                                error_msg.set(Some(msg.clone()));
                                            }
                                        }
                                        account_store.with_mut(|store| apply_unlock_result(store, result));
                                        unlocking.set(false);
                                        if !failed {
                                            on_close.call(());
                                        }
                                    });
                                }
                            }
                        },
                    }
                }

                div {
                    class: "modal-actions",
                    button {
                        class: "modal-btn-primary",
                        disabled: unlocking(),
                        onclick: {
                            let account_id = account_id.clone();
                            let account = account.clone();
                            move |_| {
                                let pw = password();
                                if pw.is_empty() {
                                    error_msg.set(Some("Enter a password.".to_string()));
                                    return;
                                }
                                let Some(ref acct) = account else { return; };
                                let path = acct.path.clone();
                                let id = account_id.clone();
                                let name = acct.name.clone();
                                unlocking.set(true);
                                spawn(async move {
                                    let result = tokio::task::spawn_blocking(move || {
                                        unlock_account_blocking(path, id, name, pw)
                                    })
                                    .await
                                    .unwrap_or_else(|e| Err(format!("Task panicked: {e}")));
                                    let failed = result.is_err();
                                    if failed {
                                        if let Err(ref msg) = result {
                                            error_msg.set(Some(msg.clone()));
                                        }
                                    }
                                    account_store.with_mut(|store| apply_unlock_result(store, result));
                                    unlocking.set(false);
                                    if !failed {
                                        on_close.call(());
                                    }
                                });
                            }
                        },
                        if unlocking() { "Unlocking…" } else { "Unlock" }
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

fn short_address(address: &str) -> String {
    if address.len() <= 14 {
        return address.to_string();
    }
    format!("{}…{}", &address[..6], &address[address.len() - 6..])
}
