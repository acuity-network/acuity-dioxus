# acuity-dioxus

A [Dioxus 0.7](https://dioxuslabs.com/learn/0.7) desktop dapp for the [Acuity](https://acuity.social) decentralized social media platform. It connects to a local Acuity Substrate node, an IPFS daemon, and an Acuity indexer to provide account management, content publishing, and browsing.

See [ARCHITECTURE.md](ARCHITECTURE.md) for a full description of the module structure, routing, service connections, content protocol, and key dependencies.

---

## Prerequisites

| Requirement | Notes |
|---|---|
| Rust (2024 edition) | Install via [rustup](https://rustup.rs) |
| `dx` CLI | `curl -sSL https://dioxus.dev/install.sh \| sh` |
| `just` | `cargo install just` — needed to regenerate Subxt bindings |
| `subxt` CLI | `cargo install subxt-cli` — needed to regenerate Subxt bindings |
| Acuity node | Listening at `ws://127.0.0.1:9944` |
| IPFS daemon | API at `http://127.0.0.1:5001` |
| Acuity indexer | WebSocket at `ws://127.0.0.1:8172` |
| `acuity-index-api-rs` | Local path dependency expected at `/home/jbrown/acuity-index-api-rs` |

The app starts and reconnects gracefully even if services are unavailable, but publishing and browsing content requires all three.

The dapp uses the local `acuity-index-api-rs` crate as its indexer client instead of implementing the indexer websocket protocol directly in this repository.

---

## Running

```sh
dx serve                        # desktop (default)
dx serve --platform web         # web
dx serve --platform mobile      # mobile
```

---

## Regenerating Subxt Bindings

`src/acuity_runtime.rs` is auto-generated from a live node. With the node running:

```sh
just generate-runtime-api
```

Do not edit `src/acuity_runtime.rs` manually.

---

## Contributing

Follow standard Rust conventions. The project enforces a `clippy.toml` rule that bans holding Dioxus signal guards (`GenerationalRef`, `GenerationalRefMut`, `WriteLock`) across `await` points — ensure `cargo clippy` passes before submitting changes. Update `ARCHITECTURE.md` whenever structural changes are made (new routes, modules, pallets, or service URLs).
