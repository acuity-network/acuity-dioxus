use crate::{ConnectionStatus, IndexerConnection, IndexerSpan, INDEXER_URL, RECONNECT_DELAY};
use dioxus::prelude::*;

const INDEXER_STATUS_CSS: Asset = asset!("/assets/styling/indexer-status.css");

#[component]
pub fn IndexerStatus() -> Element {
    let indexer_connection = use_context::<Signal<IndexerConnection>>();
    let indexer_connection = indexer_connection();

    let (status_badge_class, status_label) = match indexer_connection.status {
        ConnectionStatus::Connected => ("connected", "Connected"),
        ConnectionStatus::Connecting => ("connecting", "Connecting"),
        ConnectionStatus::Reconnecting => ("reconnecting", "Reconnecting"),
    };

    let latest_indexed_block = indexer_connection
        .details
        .latest_indexed_block()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Waiting for indexed spans".to_string());

    let earliest_indexed_block = indexer_connection
        .details
        .spans
        .iter()
        .map(|span| span.start)
        .min()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Pending".to_string());

    let coverage_blocks = indexer_connection
        .details
        .spans
        .iter()
        .map(|span| span.end.saturating_sub(span.start) + 1)
        .sum::<u32>()
        .to_string();

    let span_count = indexer_connection.details.spans.len().to_string();

    let overall_range = match (
        indexer_connection
            .details
            .spans
            .iter()
            .map(|span| span.start)
            .min(),
        indexer_connection
            .details
            .spans
            .iter()
            .map(|span| span.end)
            .max(),
    ) {
        (Some(start), Some(end)) => format!("{start} -> {end}"),
        _ => "No indexed spans yet".to_string(),
    };

    let mut spans = indexer_connection.details.spans.clone();
    spans.sort_by(|left, right| {
        right
            .end
            .cmp(&left.end)
            .then_with(|| right.start.cmp(&left.start))
    });

    let newest_span = spans
        .first()
        .map(span_label)
        .unwrap_or_else(|| "Waiting for the first span".to_string());

    rsx! {
        document::Link { rel: "stylesheet", href: INDEXER_STATUS_CSS }

        div {
            class: "indexer-shell",

            // ── Compact status bar ──────────────────────────────────────────
            div {
                class: "status-bar panel-surface",
                div {
                    class: "status-bar-left",
                    div { class: "status-badge {status_badge_class}", "{status_label}" }
                    div {
                        class: "status-bar-title",
                        span { class: "status-bar-name", "Acuity Indexer" }
                        span { class: "status-bar-url", "{INDEXER_URL}" }
                    }
                }
                div {
                    class: "status-bar-right",
                    span { class: "status-bar-meta-label", "Reconnect delay" }
                    span { class: "status-bar-meta-value", "{RECONNECT_DELAY.as_secs()}s" }
                }
            }

            if let Some(error) = indexer_connection.last_error.clone() {
                div {
                    class: "indexer-banner warning",
                    div {
                        class: "banner-title-row",
                        span { class: "banner-title", "Latest indexer error" }
                        span { class: "banner-pill", "retrying" }
                    }
                    p { class: "banner-copy", "{error}" }
                }
            }

            div {
                class: "indexer-grid",
                section {
                    class: "panel-surface overview-panel",
                    div {
                        class: "panel-heading",
                        div {
                            p { class: "label", "Overview" }
                            h2 { "Connection snapshot" }
                        }
                    }
                    div {
                        class: "facts-grid",
                        InfoCard {
                            title: "Indexer websocket".to_string(),
                            value: INDEXER_URL.to_string(),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Reconnect delay".to_string(),
                            value: format!("{} seconds", RECONNECT_DELAY.as_secs()),
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Indexed tip".to_string(),
                            value: latest_indexed_block,
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Earliest span start".to_string(),
                            value: earliest_indexed_block,
                            tone: "neutral".to_string(),
                        }
                        InfoCard {
                            title: "Tracked spans".to_string(),
                            value: span_count,
                            tone: status_badge_class.to_string(),
                        }
                        InfoCard {
                            title: "Indexed blocks".to_string(),
                            value: coverage_blocks,
                            tone: "neutral".to_string(),
                        }
                    }
                }

                section {
                    class: "panel-surface coverage-panel",
                    div {
                        class: "panel-heading",
                        div {
                            p { class: "label", "Coverage" }
                            h2 { "Status payload" }
                        }
                    }
                    div {
                        class: "field-stack",
                        MetadataField {
                            label: "Overall indexed range".to_string(),
                            value: overall_range,
                            mono: true,
                        }
                        MetadataField {
                            label: "Newest reported span".to_string(),
                            value: newest_span,
                            mono: true,
                        }
                        MetadataField {
                            label: "Subscription mode".to_string(),
                            value: "acuity-index-api-rs status snapshot + subscription".to_string(),
                            mono: false,
                        }
                    }
                }
            }

            section {
                class: "panel-surface spans-panel",
                div {
                    class: "panel-heading",
                    div {
                        p { class: "label", "Spans" }
                        h2 { "Indexed block windows" }
                    }
                    p { class: "span-count", "{spans.len()} entries" }
                }

                if spans.is_empty() {
                    div {
                        class: "empty-state",
                        "The indexer has not published any spans yet. Once it answers the first status request, the indexed windows will appear here."
                    }
                } else {
                    div {
                        class: "span-list",
                        for span in spans {
                            SpanRow { span }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn InfoCard(title: String, value: String, tone: String) -> Element {
    rsx! {
        div {
            class: "info-card {tone}",
            p { class: "label", "{title}" }
            p { class: "info-value", "{value}" }
        }
    }
}

#[component]
fn MetadataField(label: String, value: String, mono: bool) -> Element {
    let value_class = if mono {
        "field-value mono"
    } else {
        "field-value"
    };

    rsx! {
        div {
            class: "metadata-field",
            p { class: "label", "{label}" }
            p { class: "{value_class}", "{value}" }
        }
    }
}

#[component]
fn SpanRow(span: IndexerSpan) -> Element {
    let block_count = span.end.saturating_sub(span.start) + 1;

    rsx! {
        div {
            class: "span-row",
            span { class: "span-badge", "range" }
            div {
                class: "span-copy",
                p { class: "span-title", "{span_label(&span)}" }
                p { class: "span-meta", "{block_count} indexed blocks in this span" }
            }
        }
    }
}

fn span_label(span: &IndexerSpan) -> String {
    format!("{} -> {}", span.start, span.end)
}
