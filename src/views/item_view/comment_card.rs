use crate::{
    accounts::AccountStore,
    comment::{
        load_comment_revision_history, publish_comment, publish_comment_revision, CommentDraft,
        LoadedComment, PublishCommentRequest,
    },
    content::{
        decode_single_mixin, fetch_ipfs_digest_bytes, BodyTextMixinMessage, ItemMessage,
        RevisionEntry, BODY_TEXT_MIXIN_ID,
    },
};
use dioxus::prelude::*;

use super::reactions::Reactions;

/// Recursive component that renders one comment and all its nested children.
#[component]
pub fn CommentCard(
    comment: LoadedComment,
    /// Nesting depth — used to indent nested replies.
    depth: u32,
    /// Incrementing this tick from the parent causes a comment reload.
    mut reload_tick: Signal<u64>,
    account_store: Signal<AccountStore>,
) -> Element {
    // ── Reply state ───────────────────────────────────────────────────────────
    let mut reply_open = use_signal(|| false);
    let mut reply_body: Signal<String> = use_signal(String::new);
    let mut reply_submitting = use_signal(|| false);
    let mut reply_error: Signal<Option<String>> = use_signal(|| None);

    // ── Edit state ────────────────────────────────────────────────────────────
    let mut edit_open = use_signal(|| false);
    let mut edit_body: Signal<String> = use_signal(|| comment.body_text.clone());
    let mut edit_submitting = use_signal(|| false);
    let mut edit_error: Signal<Option<String>> = use_signal(|| None);

    // ── Revision-browsing state ───────────────────────────────────────────────
    // The body text shown in the card — updated when the user picks an old revision.
    let mut viewed_body: Signal<String> = use_signal(|| comment.body_text.clone());
    let mut revision_switching = use_signal(|| false);
    // Tracks which revision the <select> is showing so re-selecting the latest
    // after having picked an older one still fires a state change.
    let mut selected_revision_id = use_signal(|| comment.revision_id);

    // Load the revision history for this comment lazily (non-blocking).
    let comment_item_id_hex = comment.item_id_hex.clone();
    let revision_history =
        use_resource(move || load_comment_revision_history(comment_item_id_hex.clone()));

    let parent_item_id = comment.item_id;
    let comment_item_id = comment.item_id;
    let indent_px = depth * 20;
    let short_address = {
        let addr = &comment.owner_address;
        if addr.len() > 16 {
            format!("{}…{}", &addr[..8], &addr[addr.len() - 6..])
        } else {
            addr.clone()
        }
    };

    // Determine whether the active account is the comment owner.
    let is_owner = {
        let store = account_store();
        store
            .active_account()
            .map(|a| a.address == comment.owner_address)
            .unwrap_or(false)
    };
    let can_edit = is_owner && comment.is_revisionable;

    // ── Submit handlers ───────────────────────────────────────────────────────
    let submit_reply = {
        move |_| {
            let body = reply_body();
            if body.trim().is_empty() {
                return;
            }
            let store = account_store();
            spawn(async move {
                reply_error.set(None);
                reply_submitting.set(true);
                let req = PublishCommentRequest {
                    draft: CommentDraft { body },
                    parent_item_id,
                };
                match publish_comment(&store, req).await {
                    Ok(_) => {
                        reply_body.set(String::new());
                        reply_open.set(false);
                        reload_tick.with_mut(|t| *t += 1);
                    }
                    Err(e) => reply_error.set(Some(e)),
                }
                reply_submitting.set(false);
            });
        }
    };

    let submit_edit = {
        move |_| {
            let body = edit_body();
            if body.trim().is_empty() {
                return;
            }
            let store = account_store();
            spawn(async move {
                edit_error.set(None);
                edit_submitting.set(true);
                match publish_comment_revision(
                    &store,
                    comment_item_id,
                    CommentDraft { body },
                )
                .await
                {
                    Ok(()) => {
                        edit_open.set(false);
                        reload_tick.with_mut(|t| *t += 1);
                    }
                    Err(e) => edit_error.set(Some(e)),
                }
                edit_submitting.set(false);
            });
        }
    };

    // ── Revision history (resolved value) ─────────────────────────────────────
    let history: Vec<RevisionEntry> = revision_history
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    rsx! {
        div {
            class: "comment-card",
            style: "margin-left: {indent_px}px;",

            div { class: "comment-header",
                span { class: "comment-author", title: "{comment.owner_address}", "{short_address}" }
            }

            p { class: "comment-body", "{viewed_body}" }

            // ── Inline revision selector (visible to everyone, only when >1 revision) ──
            if history.len() > 1 {
                div { class: "comment-revisions",
                    select {
                        class: "comment-revision-select",
                        disabled: revision_switching(),
                        value: selected_revision_id().to_string(),
                        onchange: {
                            let history = history.clone();
                            let latest_body = comment.body_text.clone();
                            let latest_revision_id = comment.revision_id;
                            move |e: Event<FormData>| {
                                let selected_rid: u32 = e.value().parse().unwrap_or(0);
                                selected_revision_id.set(selected_rid);
                                // Re-selecting the latest revision: restore from the
                                // already-loaded body text without a network round-trip.
                                if selected_rid == latest_revision_id {
                                    viewed_body.set(latest_body.clone());
                                    return;
                                }
                                let Some(entry) = history.iter().find(|r| r.revision_id == selected_rid) else {
                                    return;
                                };
                                let hash = entry.ipfs_hash_hex.clone();
                                revision_switching.set(true);
                                spawn(async move {
                                    match fetch_ipfs_digest_bytes(&hash).await {
                                        Ok(bytes) => {
                                            match prost::Message::decode(bytes.as_slice())
                                                .map(|m: ItemMessage| {
                                                    decode_single_mixin::<BodyTextMixinMessage>(
                                                        &m,
                                                        BODY_TEXT_MIXIN_ID,
                                                    )
                                                    .map(|b| b.body_text)
                                                    .unwrap_or_default()
                                                }) {
                                                Ok(body) => viewed_body.set(body),
                                                Err(_) => {}
                                            }
                                        }
                                        Err(_) => {}
                                    }
                                    revision_switching.set(false);
                                });
                            }
                        },
                        for rev in history.clone() {
                            {
                                let is_latest = rev.revision_id == comment.revision_id;
                                let label = if is_latest {
                                    format!("Revision {} (latest)", rev.revision_id)
                                } else {
                                    format!("Revision {}", rev.revision_id)
                                };
                                let rid_str = rev.revision_id.to_string();
                                rsx! {
                                    option {
                                        value: rid_str,
                                        selected: rev.revision_id == selected_revision_id(),
                                        "{label}"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Reactions ─────────────────────────────────────────────────────
            Reactions {
                item_id: comment_item_id,
                revision_id: selected_revision_id(),
            }

            // ── Action buttons (Reply / Edit) ─────────────────────────────────
            div { class: "comment-actions",
                button {
                    class: "comment-reply-btn",
                    onclick: move |_| {
                        reply_open.with_mut(|v| *v = !*v);
                        if reply_open() { edit_open.set(false); }
                    },
                    if reply_open() { "Cancel reply" } else { "Reply" }
                }
                if can_edit {
                    button {
                        class: "comment-edit-btn",
                        onclick: move |_| {
                            edit_open.with_mut(|v| *v = !*v);
                            if edit_open() {
                                // Pre-fill with the latest viewed body.
                                edit_body.set(viewed_body());
                                reply_open.set(false);
                            }
                        },
                        if edit_open() { "Cancel edit" } else { "Edit" }
                    }
                }
            }

            // ── Inline edit form ──────────────────────────────────────────────
            if edit_open() {
                div { class: "comment-edit-form",
                    if let Some(err) = edit_error() {
                        div { class: "status-bar error", "{err}" }
                    }
                    textarea {
                        class: "comment-textarea",
                        rows: "4",
                        placeholder: "Edit your comment…",
                        disabled: edit_submitting(),
                        value: edit_body(),
                        oninput: move |e| edit_body.set(e.value()),
                    }
                    div { class: "comment-edit-actions",
                        button {
                            class: "btn-primary",
                            disabled: edit_submitting() || edit_body().trim().is_empty(),
                            onclick: submit_edit,
                            if edit_submitting() { "Saving…" } else { "Save" }
                        }
                    }
                }
            }

            // ── Inline reply form ─────────────────────────────────────────────
            if reply_open() {
                div { class: "comment-reply-form",
                    if let Some(err) = reply_error() {
                        div { class: "status-bar error", "{err}" }
                    }
                    textarea {
                        class: "comment-textarea",
                        rows: "3",
                        placeholder: "Write a reply…",
                        disabled: reply_submitting(),
                        value: reply_body(),
                        oninput: move |e| reply_body.set(e.value()),
                    }
                    div { class: "comment-reply-actions",
                        button {
                            class: "btn-primary",
                            disabled: reply_submitting() || reply_body().trim().is_empty(),
                            onclick: submit_reply,
                            if reply_submitting() { "Posting…" } else { "Post reply" }
                        }
                    }
                }
            }

            // ── Nested children ───────────────────────────────────────────────
            for child in comment.children.clone() {
                CommentCard {
                    comment: child,
                    depth: depth + 1,
                    reload_tick,
                    account_store,
                }
            }
        }
    }
}
