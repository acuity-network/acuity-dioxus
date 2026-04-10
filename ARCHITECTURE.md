# Architecture

`acuity-dioxus` is a Dioxus 0.7 desktop application that acts as a dapp for the Acuity Substrate chain. It maintains persistent connections to three local services and exposes a content publishing and browsing UI.

---

## Service Connections

Three background async loops are spawned once in `App` (`src/main.rs`) via `use_hook` + `spawn`. Each loop feeds a `Signal<T>` that is provided globally via `use_context_provider`, making connection state available to every component. A shared `tokio::sync::watch` shutdown signal is created at launch and triggered from the desktop `Config::with_custom_event_handler` close event so all three loops can stop cleanly before the app exits.

| Signal type | Service | Protocol | Default URL |
|---|---|---|---|
| `Signal<ChainConnection>` | Acuity Substrate node | Subxt WebSocket | `ws://127.0.0.1:9944` |
| `Signal<IpfsConnection>` | IPFS daemon | HTTP polling (`/api/v0/id`) | `http://127.0.0.1:5001` |
| `Signal<IndexerConnection>` | Acuity indexer | `acuity-index-api-rs` over WebSocket | `ws://127.0.0.1:8172` |

All three connections share the same lifecycle pattern: `Connecting → Connected → Reconnecting` (2 s delay), represented by the `ConnectionStatus` enum. When shutdown is requested, each loop exits instead of entering another reconnect cycle.

**Chain loop** (`watch_acuity_chain`): Reads chain constants (SS58 prefix, existential deposit, spec/tx version) on connect, then subscribes to both best-block and finalized-block streams concurrently via `tokio::select!`. The stream loop also listens for the app shutdown signal so the Subxt websocket is dropped promptly on exit. Block numbers are exposed in the shared `Navbar` layout.

**IPFS loop** (`watch_ipfs_daemon`): Polls `POST /api/v0/id` every 5 s. The polling request and sleep interval are both interruptible by the app shutdown signal so the task does not linger during exit. Used only for status display and as the upload/download endpoint; content ops call the HTTP API directly from async helpers in `src/content.rs`.

**Indexer loop** (`watch_indexer`): Uses the published `acuity-index-api-rs` library client to connect to the indexer, fetch the initial status snapshot, and subscribe to ongoing status updates. The subscription loop listens for the shared shutdown signal and then explicitly closes the websocket with `IndexerClient::close()` before exit. One-off `get_events` queries for item data are also routed through that shared client from `src/content.rs`. Indexed events are consumed through the crate's typed `DecodedEvent`/`StoredEvent` model and its field helpers instead of reparsing raw JSON event blobs in the app.

---

## Global Context

Consumed anywhere in the tree with `use_context::<Signal<T>>()`:

```
Signal<ChainConnection>   – best/finalized block numbers, genesis hash, runtime constants,
                            and active_account_balance (free balance of the currently active
                            account in planck, updated on every finalised block and on
                            active-account switches)
Signal<IpfsConnection>    – IPFS peer ID, addresses, connection status
Signal<IndexerConnection> – indexed block spans, connection status
Signal<AccountStore>      – local keystore: account list, active account, unlocked signers
```

### Active account balance tracking

`ChainDetails.active_account_balance: Option<u128>` is managed by the
`watch_active_account_balance` background task (spawned from `App`).  It
updates the balance in three situations:

1. **Account switch** — whenever the active account address changes (delivered
   via a `watch::Sender<Option<String>>` set by a `use_effect` in `App`), the
   task does a one-shot Subxt balance query so the UI reflects the new
   account's balance immediately.
2. **Indexer event notification** — the task calls
   `IndexerClient::subscribe_events` with
   `Key::Custom { name: "account_id", value: Bytes32(raw_account_bytes) }`.
   Whenever the indexer delivers a notification (any event that tagged this
   account — Balances::Transfer, Balances::Deposit, Content::PublishItem,
   etc.) the task does a one-shot Subxt query to re-fetch the balance.
3. **Account deselected** — balance is set to `None` immediately.

The `IndexerClient` used by this task is the same underlying WebSocket
connection as the status-subscription client.  `maintain_indexer_connection`
sends `Some(client.clone())` to a `watch::Sender<Option<IndexerClient>>`
after a successful connect, and `None` before closing.  The balance watcher
waits on `indexer_client_rx.changed()` so it resubscribes automatically
whenever the indexer reconnects.

### Per-extrinsic fee estimation

`src/runtime_client.rs` exposes:

```rust
pub(crate) async fn estimate_fee<Call: subxt::tx::Payload>(
    call: &Call,
    signer: &SignerKeypair,
) -> Result<u128, String>
```

Each transaction view uses `use_resource` to call `estimate_fee` with a
representative (dummy-data) version of its specific extrinsic.  A `use_memo`
then computes `insufficient_funds = balance < fee`.  Transaction buttons are
disabled and an `InsufficientFundsHint` paragraph is rendered below them when
`insufficient_funds` is true.

---

## Routing

All routes are defined in `src/main.rs` as the `Route` enum. Every route is wrapped in the `Navbar` layout (`src/views/navbar.rs`), which renders a persistent `AccountSidebar` and an `Outlet<Route>`.

```
/                                        → Home
/accounts                                → ManageAccounts
/accounts/create                         → CreateAccount
/profile                                 → ProfileView
/profile/edit                            → ProfileEdit
/feed/publish                            → PublishFeed
/feed/:encoded_feed_id/publish-post      → PublishPost
/item/:encoded_item_id                   → ItemView
/chain                                   → ChainStatus
/indexer                                 → IndexerStatus
/ipfs                                    → IpfsStatus
```

Item and feed IDs are Base58-encoded 32-byte hashes in URL segments.

---

## Source Files

| File | Purpose |
|---|---|
| `src/main.rs` | Entry point, `Route` enum, `App` component, global context providers, three connection-watcher loops |
| `src/runtime_client.rs` | `type AcuityClient`, `connect()`, `fetch_account_balance()`, `estimate_fee()` targeting `ws://127.0.0.1:9944` |
| `src/acuity_runtime.rs` | **Auto-generated** (~9 900 lines). Typed Subxt bindings for the Acuity Substrate runtime. Regenerate with `just generate-runtime-api`. Do not edit manually. |
| `src/content.rs` | Protobuf message types, mixin ID constants, IPFS upload/download helpers, item ID derivation, indexer event queries via `acuity-index-api-rs`, typed indexer event field extraction (`is_content_event`, `is_content_reactions_event`), CID↔hex conversion utilities, `short_hex` display helper |
| `src/accounts.rs` | Local keystore: sr25519 keypair generation, Polkadot-JS–compatible scrypt + XSalsa20-Poly1305 encryption, `AccountStore` CRUD |
| `src/profile.rs` | Load profile (indexer + IPFS) and save profile (encode protobuf, upload to IPFS, submit batched extrinsics) |
| `src/feed.rs` | Publish feeds (`publish_item` + `account_content::add_item`), resolve feed item summaries, list account pinned content |
| `src/post.rs` | Publish posts (single `content.publish_item` with parent feed reference) |
| `src/item.rs` | `publish_item_revision` — encode revised item payload, upload to IPFS, submit `content::publish_revision` extrinsic; `encode_revised_item` builder |
| `src/comment.rs` | Publish, revise, and recursively load comment trees; detects `COMMENT_TYPE_MIXIN_ID` |
| `src/views/` | Dioxus UI components (see Views section below) |

---

## Views

| Component | File | Description |
|---|---|---|
| `Navbar` | `navbar.rs` | Layout wrapper; `AccountSidebar` with account list, block numbers, connection status dots, nav links. Alt+Arrow keyboard nav. |
| `UnlockModal` | `navbar.rs` | Password modal; runs scrypt decrypt on `tokio::task::spawn_blocking` |
| `Home` | `home.rs` | Landing page with nav cards |
| `ManageAccounts` | `manage_accounts.rs` | Account table with live balance subscriptions, Fund (QR code) dialog, Delete dialog |
| `CreateAccount` | `manage_accounts.rs` | Form to generate and persist a new sr25519 keypair |
| `ChainStatus` | `chain_status.rs` | Displays block numbers, genesis hash, runtime constants |
| `IndexerStatus` | `indexer_status.rs` | Indexed block spans table, connection status, coverage stats |
| `IpfsStatus` | `ipfs_status.rs` | IPFS peer ID, public key, multiaddresses, protocols |
| `ProfileView` | `profile/view.rs` | Read-only profile: avatar, name, type pill, location, bio, pinned content cards |
| `ProfileEdit` | `profile/edit.rs` | Edit form with drag-and-drop image upload; saves via batched extrinsics |
| `PublishFeed` | `publish_feed.rs` | Create a new Feed item (title + description + optional image) |
| `PublishPost` | `publish_post.rs` | Create a Post inside a Feed (title + body + optional image) |
| `ItemView` | `item_view/mod.rs` | Full item viewer: revision history selector, owner Edit tab, emoji reactions, feed child posts, recursive `CommentCard` tree |
| `Reactions` | `item_view/reactions.rs` | Emoji reaction chips with picker; loads reaction state from indexer `SetReactions` events; submits `set_reactions` extrinsic with the full emoji set; uses optimistic local update after tx submission, reverts on failure |
| `CommentCard` | `item_view/comment_card.rs` | Recursive comment card: body, revision selector, reactions, inline reply/edit forms, nested children |
| `ImageDropZone` | `components.rs` | Shared drag-and-drop / click-to-pick image zone; used by `ProfileEdit`, `PublishFeed`, `PublishPost`, and `ItemView` edit tab |
| `EmptyState` | `components.rs` | Shared centred empty/not-found card with optional CTA link; used by `ProfileView`, `ItemView`, `PublishPost`, `PublishFeed` |
| `InsufficientFundsHint` | `components.rs` | Renders a hint paragraph below transaction buttons when `balance < fee`; hidden while loading |

---

## Content Protocol

All on-chain content uses a **protobuf + mixin** encoding defined in `src/content.rs`.

An `ItemMessage` is a list of `MixinPayloadMessage` entries, each tagged with a 32-bit mixin ID:

| Mixin | Constant | ID | Payload type |
|---|---|---|---|
| Language | `LANGUAGE_MIXIN_ID` | `0x9bc7a0e6` | `LanguageMixinMessage` – BCP-47 language tag |
| Title | `TITLE_MIXIN_ID` | `0x344f4812` | `TitleMixinMessage` |
| Body text | `BODY_TEXT_MIXIN_ID` | `0x2d382044` | `BodyTextMixinMessage` |
| Image | `IMAGE_MIXIN_ID` | `0x045eee8c` | `ImageMixinMessage` – full-res + mipmap levels |
| Profile | `PROFILE_MIXIN_ID` | `0xbeef2144` | `ProfileMixinMessage` – account type + location |
| Feed type | `FEED_TYPE_MIXIN_ID` | `0xbcec8faa` | Empty marker |
| Comment type | `COMMENT_TYPE_MIXIN_ID` | `0x874aba65` | Empty marker |

**Publish flow:**
1. Encode `ItemMessage` with the relevant mixins using `prost`.
2. Upload bytes to IPFS via `POST /api/v0/add` → returns a CIDv0.
3. Convert CID → 32-byte sha2-256 digest (strip 2-byte multihash prefix).
4. Submit `content::publish_item(item_id, ipfs_hash)` extrinsic (often batched with a type-specific call like `account_content::add_item`).

**Read flow:**
1. Derive or look up the item ID.
2. Query the indexer for `Content::PublishRevision` events to get the IPFS hash for each revision.
3. Fetch bytes from IPFS and decode with `prost`.
4. Extract individual mixins with `decode_single_mixin<M>(item, MIXIN_ID)`.

### Item ID Derivation

Item IDs are computed client-side before submission:

```rust
blake2_256(SCALE(account_id) ++ SCALE(nonce) ++ SCALE(1000))
```

The constant `1000` is `ITEM_ID_NAMESPACE`. The result is verified against the `Content::PublishItem` chain event.

### Images

`build_image_mixin` in `src/content.rs` generates a mipmap pyramid: the original image is re-encoded as JPEG (quality 82) and then repeatedly halved until both dimensions are ≤ 64 px. Each level is uploaded to IPFS independently. The smallest mipmap is used as the preview thumbnail.

---

## Account / Keystore

Accounts are stored as Polkadot-JS–compatible JSON files under `~/.config/acuity-dioxus/accounts/`. The format uses:

- **scrypt** (N=32768, r=8, p=1) for key derivation from password
- **XSalsa20-Poly1305** (`crypto_secretbox`) for symmetric encryption of the PKCS#8-encoded keypair

`subxt-signer`'s `polkadot_js_compat::decrypt_json` handles the decrypt side. CPU-heavy unlock runs on `tokio::task::spawn_blocking` to avoid blocking the Dioxus async executor.

Multiple accounts can be simultaneously unlocked; `AccountStore.unlocked_signers` is a `HashMap<account_id, SignerKeypair>`.

---

## Substrate Pallets Used

Accessed via the auto-generated `src/acuity_runtime.rs`:

| Pallet | Usage |
|---|---|
| `System` | Runtime version, SS58 prefix constant, account nonce |
| `Balances` | Existential deposit constant, balance subscriptions, transfers |
| `Content` | `publish_item`, `PublishItem`/`PublishRevision` events |
| `AccountContent` | `add_item` (pin content to account feed) |
| `AccountProfile` | `set_profile` (link profile item to account) |
| `ContentReactions` | Stateless emoji reactions via `set_reactions` extrinsic; reaction state loaded from indexer `SetReactions` events |
| `Utility` | `batch` / `batch_all` for multi-call extrinsics |

---

## Key Dependencies

| Crate | Role |
|---|---|
| `dioxus 0.7.1` | UI framework (desktop target by default) |
| `subxt 0.50.0` | Typed Substrate client (WebSocket, native feature) |
| `subxt-signer 0.50.0` | sr25519 signing + Polkadot-JS keystore compat |
| `sp-core 39.0.0` | `AccountId32`, SS58, `blake2_256` |
| `prost 0.13` | Protobuf encode/decode |
| `parity-scale-codec 3` | SCALE encoding for item ID derivation |
| `schnorrkel 0.11.4` | sr25519 keypair expansion |
| `scrypt 0.11` | Password key derivation |
| `crypto_secretbox 0.1.1` | XSalsa20-Poly1305 encryption |
| `reqwest 0.12` | HTTP client for IPFS API |
| `acuity-index-api-rs 0.1.1` | Shared indexer client used for status subscriptions and event queries |
| `tokio-tungstenite 0.28` | WebSocket client for one-off IPFS queries |
| `image 0.25` | Image decode and mipmap resize |
| `rfd 0.15` | Native file picker dialog |
| `fast_qr 0.12` | SVG QR code generation (Fund dialog) |
