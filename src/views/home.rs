use crate::Route;
use dioxus::prelude::*;

const HOME_CSS: Asset = asset!("/assets/styling/home.css");

#[component]
pub fn Home() -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: HOME_CSS }

        div {
            class: "home-shell",
            div {
                class: "hero-panel",
                div {
                    class: "hero-copy",
                    p { class: "eyebrow", "Welcome" }
                    h1 { "Acuity" }
                    p {
                        class: "hero-text",
                        "A decentralised social media platform built on Polkadot."
                    }
                }
                div {
                    class: "home-links",
                    Link {
                        to: Route::ManageAccounts {},
                        class: "home-nav-link",
                        div { class: "home-nav-icon", "👤" }
                        div {
                            class: "home-nav-label", "Manage Accounts"
                            p { class: "home-nav-desc", "Create, fund and manage your Polkadot-JS accounts." }
                        }
                    }
                    Link {
                        to: Route::ProfileView {},
                        class: "home-nav-link",
                        div { class: "home-nav-icon", "📝" }
                        div {
                            class: "home-nav-label", "Profile"
                            p { class: "home-nav-desc", "View and edit your on-chain profile." }
                        }
                    }
                }
            }
        }
    }
}
