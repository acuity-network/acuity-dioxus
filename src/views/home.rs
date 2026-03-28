use crate::accounts::{
    create_account, delete_active_account, lock_active_account, select_active_account,
    unlock_active_account, AccountStore,
};
use dioxus::prelude::*;

const HOME_CSS: Asset = asset!("/assets/styling/home.css");

#[component]
pub fn Home() -> Element {
    let mut create_name = use_signal(String::new);
    let mut create_password = use_signal(String::new);
    let mut unlock_password = use_signal(String::new);
    let mut account_store = use_context::<Signal<AccountStore>>();
    let account_snapshot = account_store();

    let active_account = account_snapshot.active_account().cloned();
    let is_active_unlocked = account_snapshot.is_active_unlocked();
    let config_dir_label = account_snapshot
        .config_dir
        .clone()
        .unwrap_or_else(|| "Unavailable".to_string());
    let accounts_dir_label = account_snapshot
        .accounts_dir
        .clone()
        .unwrap_or_else(|| "Unavailable".to_string());

    rsx! {
        document::Link { rel: "stylesheet", href: HOME_CSS }

        div {
            class: "home-shell",
            div {
                class: "hero-panel",
                div {
                    class: "hero-copy",
                    p { class: "eyebrow", "Account vault" }
                    h1 { "Manage the dapp's Polkadot-JS accounts" }
                    p {
                        class: "hero-text",
                        "Accounts live in the dapp config directory and only the active account can be unlocked in memory."
                    }
                }
                div {
                    class: "path-card",
                    p { class: "path-label", "Config directory" }
                    p { class: "path-value", "{config_dir_label}" }
                    p { class: "path-label", "Accounts directory" }
                    p { class: "path-value", "{accounts_dir_label}" }
                }
            }

            if let Some(error_message) = account_snapshot.error_message.clone() {
                div {
                    class: "banner error",
                    "{error_message}"
                }
            }

            if let Some(notice_message) = account_snapshot.notice_message.clone() {
                div {
                    class: "banner notice",
                    "{notice_message}"
                }
            }

            div {
                class: "account-grid",
                section {
                    class: "panel create-panel",
                    p { class: "panel-label", "Create" }
                    h2 { "New account" }
                    p { class: "panel-copy", "Generate a fresh sr25519 account and save it as a Polkadot-JS compatible JSON file." }

                    label {
                        class: "field",
                        span { "Name" }
                        input {
                            value: create_name,
                            placeholder: "Treasury signer",
                            oninput: move |event| create_name.set(event.value()),
                        }
                    }

                    label {
                        class: "field",
                        span { "Password" }
                        input {
                            r#type: "password",
                            value: create_password,
                            placeholder: "Protect this account file",
                            oninput: move |event| create_password.set(event.value()),
                        }
                    }

                    button {
                        class: "primary-action",
                        onclick: move |_| {
                            let name = create_name();
                            let password = create_password();
                            account_store.with_mut(|store| create_account(store, &name, &password));

                            if account_store().error_message.is_none() {
                                create_name.set(String::new());
                                create_password.set(String::new());
                                unlock_password.set(String::new());
                            }
                        },
                        "Create account"
                    }
                }

                section {
                    class: "panel details-panel",
                    p { class: "panel-label", "Active" }
                    if let Some(active_account) = active_account {
                        h2 { "{active_account.name}" }
                        p { class: "account-address", "{active_account.address}" }

                        div {
                            class: "status-row",
                            span {
                                class: if is_active_unlocked { "status-pill unlocked" } else { "status-pill locked" },
                                if is_active_unlocked { "Unlocked" } else { "Locked" }
                            }
                        }

                        if is_active_unlocked {
                            button {
                                class: "secondary-action",
                                onclick: move |_| {
                                    account_store.with_mut(lock_active_account);
                                    unlock_password.set(String::new());
                                },
                                "Lock active account"
                            }
                        } else {
                            label {
                                class: "field",
                                span { "Unlock password" }
                                input {
                                    r#type: "password",
                                    value: unlock_password,
                                    placeholder: "Enter the account password",
                                    oninput: move |event| unlock_password.set(event.value()),
                                }
                            }

                            button {
                                class: "primary-action",
                                onclick: move |_| {
                                    let password = unlock_password();
                                    account_store.with_mut(|store| unlock_active_account(store, &password));

                                    if account_store().error_message.is_none() {
                                        unlock_password.set(String::new());
                                    }
                                },
                                "Unlock active account"
                            }
                        }

                        button {
                            class: "danger-action",
                            onclick: move |_| {
                                account_store.with_mut(delete_active_account);
                                unlock_password.set(String::new());
                            },
                            "Delete active account"
                        }
                    } else {
                        h2 { "No account selected" }
                        p {
                            class: "panel-copy",
                            "Create an account to start using the dapp, or drop existing Polkadot-JS account files into the accounts directory."
                        }
                    }
                }
            }

            section {
                class: "panel list-panel",
                div {
                    class: "panel-heading",
                    div {
                        p { class: "panel-label", "Accounts" }
                        h2 { "Stored accounts" }
                    }
                    p { class: "account-count", "{account_snapshot.accounts.len()} total" }
                }

                if account_snapshot.accounts.is_empty() {
                    div {
                        class: "empty-state",
                        "No account files found yet."
                    }
                } else {
                    div {
                        class: "account-list",
                        for account in account_snapshot.accounts.clone() {
                            button {
                                class: if account_snapshot.active_account_id.as_deref() == Some(account.id.as_str()) {
                                    "account-row active"
                                } else {
                                    "account-row"
                                },
                                onclick: {
                                    let account_id = account.id.clone();
                                    move |_| {
                                        account_store.with_mut(|store| select_active_account(store, &account_id));
                                        unlock_password.set(String::new());
                                    }
                                },
                                div {
                                    class: "account-row-copy",
                                    span { class: "account-name", "{account.name}" }
                                    span { class: "account-short-address", "{short_address(&account.address)}" }
                                }
                                span {
                                    class: if account_snapshot.active_account_id.as_deref() == Some(account.id.as_str()) {
                                        if is_active_unlocked { "row-status unlocked" } else { "row-status locked" }
                                    } else {
                                        "row-status idle"
                                    },
                                    if account_snapshot.active_account_id.as_deref() == Some(account.id.as_str()) {
                                        if is_active_unlocked { "Active and unlocked" } else { "Active" }
                                    } else {
                                        "Select"
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

fn short_address(address: &str) -> String {
    if address.len() <= 14 {
        return address.to_string();
    }

    format!("{}...{}", &address[..6], &address[address.len() - 6..])
}
