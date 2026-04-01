use crate::{
    content::{
        bytes32_to_hex, decode_single_mixin, fetch_ipfs_digest_bytes, fetch_latest_revision_hash,
        preview_data_url_for_image_mixin, BodyTextMixinMessage, ItemMessage,
        LanguageMixinMessage, TitleMixinMessage, BODY_TEXT_MIXIN_ID, IMAGE_MIXIN_ID,
        LANGUAGE_MIXIN_ID, TITLE_MIXIN_ID,
    },
    feed::FEED_TYPE_MIXIN_ID,
    profile::PROFILE_MIXIN_ID,
    Route,
};
use dioxus::prelude::*;
use prost::Message;

const ITEM_VIEW_CSS: Asset = asset!("/assets/styling/item_view.css");
const PROFILE_CSS: Asset = asset!("/assets/styling/profile.css");

fn short_hex(value: &str) -> String {
    if value.len() <= 18 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..10], &value[value.len() - 8..])
    }
}

fn content_type_label(item: &ItemMessage) -> &'static str {
    for mixin in &item.mixin_payload {
        if mixin.mixin_id == FEED_TYPE_MIXIN_ID {
            return "Feed";
        }
        if mixin.mixin_id == PROFILE_MIXIN_ID {
            return "Profile";
        }
    }
    "Content"
}

#[derive(Clone, PartialEq, Default)]
struct LoadedItem {
    item_id_hex: String,
    revision_ipfs_hash_hex: String,
    content_type: String,
    title: String,
    body_text: String,
    language: String,
    image_preview_data_url: Option<String>,
}

async fn load_item(encoded_item_id: &str) -> Result<LoadedItem, String> {
    let item_id_bytes = bs58::decode(encoded_item_id)
        .into_vec()
        .map_err(|error| format!("Invalid item ID encoding: {error}"))?;

    if item_id_bytes.len() != 32 {
        return Err(format!(
            "Item ID must be 32 bytes, got {}.",
            item_id_bytes.len()
        ));
    }

    let item_id: [u8; 32] = item_id_bytes
        .try_into()
        .map_err(|_| "Failed to convert item ID bytes.".to_string())?;

    let item_id_hex = bytes32_to_hex(&item_id);
    let revision_ipfs_hash = fetch_latest_revision_hash(item_id_hex.clone()).await?;
    let item_bytes = fetch_ipfs_digest_bytes(&revision_ipfs_hash).await?;
    let item = ItemMessage::decode(item_bytes.as_slice())
        .map_err(|error| format!("Failed to decode item payload: {error}"))?;

    let content_type = content_type_label(&item).to_string();

    let title = decode_single_mixin::<TitleMixinMessage>(&item, TITLE_MIXIN_ID)
        .map(|m| m.title)
        .unwrap_or_default();

    let body_text = decode_single_mixin::<BodyTextMixinMessage>(&item, BODY_TEXT_MIXIN_ID)
        .map(|m| m.body_text)
        .unwrap_or_default();

    let language = decode_single_mixin::<LanguageMixinMessage>(&item, LANGUAGE_MIXIN_ID)
        .map(|m| m.language_tag)
        .unwrap_or_default();

    let image_mixin_payload = item
        .mixin_payload
        .iter()
        .find(|m| m.mixin_id == IMAGE_MIXIN_ID)
        .map(|m| m.payload.clone());

    let image_preview_data_url = if let Some(ref payload) = image_mixin_payload {
        preview_data_url_for_image_mixin(payload).await?
    } else {
        None
    };

    Ok(LoadedItem {
        item_id_hex,
        revision_ipfs_hash_hex: revision_ipfs_hash,
        content_type,
        title,
        body_text,
        language,
        image_preview_data_url,
    })
}

#[component]
pub fn ItemView(encoded_item_id: String) -> Element {
    let mut loaded: Signal<Option<LoadedItem>> = use_signal(|| None);
    let mut is_loading = use_signal(|| false);
    let mut error_message: Signal<Option<String>> = use_signal(|| None);

    let encoded_id = use_memo({
        let encoded_item_id = encoded_item_id.clone();
        move || encoded_item_id.clone()
    });

    use_effect(move || {
        let id = encoded_id();
        spawn(async move {
            error_message.set(None);
            is_loading.set(true);
            match load_item(&id).await {
                Ok(item) => loaded.set(Some(item)),
                Err(err) => {
                    loaded.set(None);
                    error_message.set(Some(err));
                }
            }
            is_loading.set(false);
        });
    });

    rsx! {
        document::Link { rel: "stylesheet", href: PROFILE_CSS }
        document::Link { rel: "stylesheet", href: ITEM_VIEW_CSS }

        div {
            class: "item-view-shell",

            // ── Page header ────────────────────────────────────────────────
            div {
                class: "page-header",
                div {
                    class: "page-header-text",
                    p { class: "page-eyebrow", "Content item" }
                    h1 { class: "page-title",
                        if let Some(ref item) = loaded() {
                            if item.title.trim().is_empty() {
                                "Untitled item"
                            } else {
                                "{item.title}"
                            }
                        } else {
                            "Item"
                        }
                    }
                }
            }

            // ── Status bar ─────────────────────────────────────────────────
            if let Some(err) = error_message() {
                div { class: "status-bar error", "{err}" }
            } else if is_loading() {
                div { class: "status-bar loading", "Loading item from the indexer and IPFS..." }
            }

            if is_loading() {
                div { class: "item-view-grid",
                    div { class: "panel-surface item-view-main skeleton-block" }
                    div { class: "panel-surface item-view-side skeleton-block" }
                }
            } else if let Some(item) = loaded() {
                div {
                    class: "item-view-grid",

                    // ── Left: content ──────────────────────────────────────
                    section {
                        class: "panel-surface item-view-main",

                        // Image
                        if let Some(img_url) = item.image_preview_data_url.clone() {
                            img {
                                class: "iv-image",
                                src: img_url,
                                alt: "Item image",
                            }
                        }

                        // Type pill
                        span { class: "iv-type-pill", "{item.content_type}" }

                        // Title
                        if !item.title.trim().is_empty() {
                            h2 { class: "iv-title", "{item.title}" }
                        }

                        // Body text
                        if !item.body_text.trim().is_empty() {
                            p { class: "iv-body", "{item.body_text}" }
                        }

                        // Empty content notice
                        if item.title.trim().is_empty() && item.body_text.trim().is_empty() {
                            p { class: "pv-notice",
                                "This item has no title or body text."
                            }
                        }
                    }

                    // ── Right: metadata ────────────────────────────────────
                    aside {
                        class: "panel-surface item-view-side",

                        div { class: "pv-meta-section",
                            p { class: "pv-section-label", "Item metadata" }
                            div { class: "metadata-list",
                                div { class: "metadata-row",
                                    span { class: "meta-label", "Item ID" }
                                    code { class: "meta-code",
                                        "{short_hex(&item.item_id_hex)}"
                                    }
                                }
                                div { class: "metadata-row",
                                    span { class: "meta-label", "Latest revision" }
                                    code { class: "meta-code",
                                        "{short_hex(&item.revision_ipfs_hash_hex)}"
                                    }
                                }
                                div { class: "metadata-row",
                                    span { class: "meta-label", "Type" }
                                    span { class: "meta-copy", "{item.content_type}" }
                                }
                                if !item.language.is_empty() {
                                    div { class: "metadata-row",
                                        span { class: "meta-label", "Language" }
                                        span { class: "meta-copy", "{item.language}" }
                                    }
                                }
                            }
                        }
                    }
                }
            } else if !is_loading() && error_message().is_none() {
                div { class: "empty-state panel-surface",
                    p { class: "empty-state-title", "Item not found" }
                    p { class: "empty-state-body",
                        "The item could not be loaded. It may not have been indexed yet."
                    }
                    Link {
                        class: "btn-secondary",
                        to: Route::Home {},
                        "Go to Dashboard"
                    }
                }
            }
        }
    }
}
