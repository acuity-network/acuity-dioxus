use crate::{
    content::{preview_data_url_for_path, SelectedImage},
    Route,
};
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use rfd::FileDialog;

// ── Balance / fee helpers ─────────────────────────────────────────────────────

/// Token display format: decimal places and unit symbol.
/// Defaults to 12 decimals and "UNIT" to match the Acuity chain constants.
#[derive(Clone, PartialEq)]
pub struct TokenFormat {
    pub decimals: u8,
    pub symbol: String,
}

impl Default for TokenFormat {
    fn default() -> Self {
        Self {
            decimals: 12,
            symbol: "UNIT".to_string(),
        }
    }
}

/// Formats a raw planck value as a human-readable token string.
pub fn format_balance(raw: u128, fmt: &TokenFormat) -> String {
    if fmt.decimals == 0 {
        return format!("{} {}", raw, fmt.symbol);
    }
    let divisor = 10u128.pow(fmt.decimals as u32);
    let whole = raw / divisor;
    let frac = raw % divisor;
    let frac_str = format!("{:0>width$}", frac, width = fmt.decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    if trimmed.is_empty() {
        format!("{} {}", whole, fmt.symbol)
    } else {
        let display = &trimmed[..trimmed.len().min(4)];
        format!("{}.{} {}", whole, display, fmt.symbol)
    }
}

/// Renders an "insufficient funds" hint paragraph below a transaction button.
///
/// Pass:
/// - `balance` — the active account's current free balance (planck), or `None`
///   if not yet fetched.
/// - `fee`     — the estimated transaction fee (planck), or `None` if not yet
///   estimated.
///
/// Nothing is rendered while either value is loading, or when funds are
/// sufficient.
#[component]
pub fn InsufficientFundsHint(balance: Option<u128>, fee: Option<u128>) -> Element {
    let fmt = TokenFormat::default();
    match (balance, fee) {
        (Some(b), Some(f)) if b < f => rsx! {
            p {
                class: "save-locked-hint",
                "Insufficient funds — need {format_balance(f, &fmt)} but balance is {format_balance(b, &fmt)}."
            }
        },
        _ => rsx! {},
    }
}

// ── ImageDropZone ─────────────────────────────────────────────────────────────

/// A drag-and-drop / click-to-pick image zone.
///
/// Manages its own drag-over highlight state internally.  The caller owns
/// `selected_image` so it can read the staged file for upload and display a
/// "Pending" note below the zone.
///
/// `existing_preview_url` is the data URL of the currently published image (if
/// any) so it shows through when no new image has been staged.
///
/// Set `disabled` to `true` while a save/publish operation is in progress.
#[component]
pub fn ImageDropZone(
    mut selected_image: Signal<Option<SelectedImage>>,
    existing_preview_url: Option<String>,
    disabled: bool,
) -> Element {
    let mut drag_over = use_signal(|| false);

    let preview_url = selected_image()
        .and_then(|img| img.preview_data_url.clone())
        .or_else(|| existing_preview_url.clone());

    let zone_class = if drag_over() {
        "drop-zone drop-zone-active"
    } else if preview_url.is_some() {
        "drop-zone drop-zone-has-image"
    } else {
        "drop-zone"
    };

    rsx! {
        div {
            class: zone_class,

            onclick: move |_| {
                if disabled { return; }
                if let Some(path) = FileDialog::new()
                    .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff"])
                    .pick_file()
                {
                    let preview = preview_data_url_for_path(&path).ok();
                    let file_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("image")
                        .to_string();
                    selected_image.set(Some(SelectedImage {
                        path: path.display().to_string(),
                        file_name,
                        preview_data_url: preview,
                    }));
                }
            },

            ondragover: move |e: DragEvent| {
                e.prevent_default();
                drag_over.set(true);
            },
            ondragleave: move |_| drag_over.set(false),
            ondrop: move |e: DragEvent| {
                e.prevent_default();
                drag_over.set(false);
                if disabled { return; }
                let file_list = e.files();
                if let Some(first) = file_list.first() {
                    let path = first.path();
                    let preview = preview_data_url_for_path(&path).ok();
                    let file_name = first.name();
                    selected_image.set(Some(SelectedImage {
                        path: path.display().to_string(),
                        file_name,
                        preview_data_url: preview,
                    }));
                }
            },

            if let Some(ref img_url) = preview_url {
                img {
                    class: "drop-zone-preview",
                    src: img_url.clone(),
                    alt: "Image preview",
                }
                if selected_image().is_some() {
                    button {
                        class: "drop-zone-clear",
                        title: "Remove staged image",
                        onclick: move |e| {
                            e.stop_propagation();
                            selected_image.set(None);
                        },
                        "×"
                    }
                }
            } else {
                div { class: "drop-zone-hint",
                    span { class: "drop-zone-icon", "I" }
                    span { "Drop an image here or click to choose" }
                }
            }
        }
    }
}

// ── EmptyState ────────────────────────────────────────────────────────────────

/// A centred empty/not-found state card with an optional call-to-action link.
#[component]
pub fn EmptyState(
    title: String,
    body: String,
    /// Label for the CTA button.  Requires `cta_route` to be set.
    cta_label: Option<String>,
    /// Route the CTA button navigates to.
    cta_route: Option<Route>,
) -> Element {
    rsx! {
        div {
            class: "empty-state panel-surface",
            p { class: "empty-state-title", "{title}" }
            p { class: "empty-state-body", "{body}" }
            if let (Some(label), Some(route)) = (cta_label, cta_route) {
                Link {
                    class: "btn-secondary",
                    to: route,
                    "{label}"
                }
            }
        }
    }
}
