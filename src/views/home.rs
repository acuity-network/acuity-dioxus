use crate::accounts::{
    apply_unlock_result, create_account, delete_active_account, lock_active_account,
    unlock_account_blocking, AccountStore,
};
use dioxus::prelude::*;

const HOME_CSS: Asset = asset!("/assets/styling/home.css");

#[component]
pub fn Home() -> Element {
    let mut create_name = use_signal(String::new);
    let mut create_password = use_signal(String::new);
    let mut unlock_password = use_signal(String::new);
    let mut unlocking = use_signal(|| false);
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
                        "Accounts live in the dapp config directory. Use the sidebar to switch accounts and lock or unlock them."
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
                                disabled: unlocking(),
                                onclick: move |_| {
                                    let password = unlock_password();
                                    if password.is_empty() {
                                        account_store.with_mut(|store| {
                                            store.error_message = Some("Enter the password for the active account.".to_string());
                                            store.notice_message = None;
                                        });
                                        return;
                                    }
                                    let Some(account) = account_store().active_account().cloned() else {
                                        account_store.with_mut(|store| {
                                            store.error_message = Some("Select an account first.".to_string());
                                            store.notice_message = None;
                                        });
                                        return;
                                    };
                                    unlocking.set(true);
                                    spawn(async move {
                                        let result = tokio::task::spawn_blocking(move || {
                                            unlock_account_blocking(account.path, account.id, account.name, password)
                                        })
                                        .await
                                        .unwrap_or_else(|e| Err(format!("Unlock task panicked: {e}")));
                                        account_store.with_mut(|store| apply_unlock_result(store, result));
                                        if account_store().error_message.is_none() {
                                            unlock_password.set(String::new());
                                        }
                                        unlocking.set(false);
                                    });
                                },
                                if unlocking() { "Unlocking..." } else { "Unlock active account" }
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
        }
    }
}
