use crate::{content::ItemMessage, feed::FEED_TYPE_MIXIN_ID, profile::PROFILE_MIXIN_ID};

/// Returns a human-readable content type string by inspecting the item's
/// type-marker mixins.
pub fn content_type_label(item: &ItemMessage) -> &'static str {
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

// ── Item ──────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Default)]
pub struct LoadedItem {
    pub encoded_item_id: String,
    pub item_id: [u8; 32],
    pub item_id_hex: String,
    pub revision_ipfs_hash_hex: String,
    pub content_type: String,
    pub title: String,
    pub body_text: String,
    pub language: String,
    pub image_preview_data_url: Option<String>,
    /// Raw image mixin payload bytes from IPFS — retained so the edit form can
    /// keep the existing image when no new image is selected.
    pub existing_image_payload: Option<Vec<u8>>,
    pub parents: Vec<ParentSummary>,
    /// SS58 address of the on-chain item owner (from Content.ItemState storage).
    pub owner_address: String,
    /// Current revision ID from Content.ItemState — used by the reactions pallet.
    pub revision_id: u32,
}

// ── Reactions ─────────────────────────────────────────────────────────────────

/// Unicode codepoints matching the emoji set from the original Vue browser.
pub const AVAILABLE_EMOJI_CODEPOINTS: &[u32] = &[
    0x1F44D, // 👍
    0x1F44E, // 👎
    0x1F60D, // 😍
    0x1F618, // 😘
    0x1F61C, // 😜
    0x1F911, // 🤑
    0x1F92B, // 🤫
    0x1F914, // 🤔
    0x1F910, // 🤐
    0x1F62C, // 😬
    0x1F925, // 🤥
    0x1F915, // 🤕
    0x1F922, // 🤢
    0x1F603, // 😃
    0x1F60E, // 😎
    0x1F913, // 🤓
    0x1F9D0, // 🧐
    0x1F62D, // 😭
    0x1F621, // 😡
    0x1F4AF, // 💯
    0x1F4A4, // 💤
    0x1F44C, // 👌
    0x1F91E, // 🤞
    0x1F44F, // 👏
    0x1F64F, // 🙏
    0x1F9D9, // 🧙
];

#[derive(Clone, PartialEq)]
pub struct ReactionSummary {
    /// The rendered emoji character(s).
    pub emoji_char: String,
    /// Unicode scalar value — used as the on-chain `Emoji(u32)` argument.
    pub codepoint: u32,
    /// Total number of accounts that reacted with this emoji.
    pub count: usize,
    /// SS58 addresses of all reactors (shown in tooltip).
    pub reactors: Vec<String>,
    /// Whether the currently active account has already reacted with this emoji.
    pub i_reacted: bool,
}

// ── Parents / Feed posts ──────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub struct ParentSummary {
    pub encoded_item_id: String,
    pub title: String,
    pub content_type: String,
}

#[derive(Clone, PartialEq)]
pub struct FeedPost {
    pub encoded_item_id: String,
    pub title: String,
    pub body_preview: String,
    pub image_preview_data_url: Option<String>,
}

// ── Edit tab ──────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub enum ActiveTab {
    View,
    Edit,
}
