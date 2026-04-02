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

// ── Layout ────────────────────────────────────────────────────────────────────

#[component]
pub fn Navbar() -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: NAVBAR_CSS }
        document::Link { rel: "stylesheet", href: SIDEBAR_CSS }

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
    let chain_connection = use_context::<Signal<ChainConnection>>();
    let chain_conn = chain_connection();
    let indexer_connection = use_context::<Signal<IndexerConnection>>();
    let indexer_conn = indexer_connection();
    let ipfs_connection = use_context::<Signal<IpfsConnection>>();
    let ipfs_conn = ipfs_connection();

    let chain_dot_class = match chain_conn.status {
        ConnectionStatus::Connected => "sidebar-status-dot connected",
        ConnectionStatus::Connecting => "sidebar-status-dot connecting",
        ConnectionStatus::Reconnecting => "sidebar-status-dot reconnecting",
    };
    let chain_block_label = match chain_conn.status {
        ConnectionStatus::Connected => match chain_conn.details.best_block_number.as_deref() {
            Some(n) => format!("#{n}"),
            None => "Connected".to_string(),
        },
        ConnectionStatus::Connecting => "Connecting…".to_string(),
        ConnectionStatus::Reconnecting => "Reconnecting…".to_string(),
    };

    let indexer_dot_class = match indexer_conn.status {
        ConnectionStatus::Connected => "sidebar-status-dot connected",
        ConnectionStatus::Connecting => "sidebar-status-dot connecting",
        ConnectionStatus::Reconnecting => "sidebar-status-dot reconnecting",
    };
    let indexer_block_label = match indexer_conn.status {
        ConnectionStatus::Connected => match indexer_conn.details.latest_indexed_block() {
            Some(n) => format!("#{n}"),
            None => "Connected".to_string(),
        },
        ConnectionStatus::Connecting => "Connecting…".to_string(),
        ConnectionStatus::Reconnecting => "Reconnecting…".to_string(),
    };

    let ipfs_dot_class = match ipfs_conn.status {
        ConnectionStatus::Connected => "sidebar-status-dot connected",
        ConnectionStatus::Connecting => "sidebar-status-dot connecting",
        ConnectionStatus::Reconnecting => "sidebar-status-dot reconnecting",
    };

    // Which account's unlock modal is open (None = closed)
    let mut unlock_target_id: Signal<Option<String>> = use_signal(|| None);

    rsx! {
        aside {
            class: "account-sidebar",

            Link { class: "brand", to: Route::Home {}, "Acuity" }

            // Active account selector
            div {
                class: "sidebar-section",
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

            // History navigation — back / forward
            {
                let nav = use_navigator();
                rsx! {
                    div {
                        class: "sidebar-history",
                        button {
                            class: "sidebar-history-btn",
                            disabled: !nav.can_go_back(),
                            onclick: move |_| nav.go_back(),
                            "←"
                        }
                        button {
                            class: "sidebar-history-btn",
                            disabled: !nav.can_go_forward(),
                            onclick: move |_| nav.go_forward(),
                            "→"
                        }
                    }
                }
            }

            // Account section
            div {
                class: "sidebar-section",
                p { class: "sidebar-heading", "Account" }
                Link {
                    class: "sidebar-nav-link",
                    to: Route::ProfileView {},
                    span { class: "sidebar-nav-label", "Profile" }
                }
                Link {
                    class: "sidebar-nav-link",
                    to: Route::PublishFeed {},
                    span { class: "sidebar-nav-label", "Publish Feed" }
                }
            }

            // Administration section
            div {
                class: "sidebar-services-section",
                p { class: "sidebar-heading", "Administration" }
                Link {
                    class: "sidebar-nav-link",
                    to: Route::ManageAccounts {},
                    span { class: "sidebar-nav-label", "Accounts" }
                }
                Link {
                    class: "sidebar-nav-link",
                    to: Route::ChainStatus {},
                    span { class: "{chain_dot_class}" }
                    span { class: "sidebar-nav-label", "Chain" }
                    span { class: "sidebar-nav-meta", "{chain_block_label}" }
                }
                Link {
                    class: "sidebar-nav-link",
                    to: Route::IndexerStatus {},
                    span { class: "{indexer_dot_class}" }
                    span { class: "sidebar-nav-label", "Indexer" }
                    span { class: "sidebar-nav-meta", "{indexer_block_label}" }
                }
                Link {
                    class: "sidebar-nav-link",
                    to: Route::IpfsStatus {},
                    span { class: "{ipfs_dot_class}" }
                    span { class: "sidebar-nav-label", "IPFS" }
                    span { class: "sidebar-nav-meta", "" }
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
