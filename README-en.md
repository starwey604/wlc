# WLC (Wirelink Compiler)

[简体中文主页 (Simplified Chinese)](README.md) · [Wirelink Repository](../README-en.md) · [Codegen guide](docs/codegen.md) · [SDK guide](docs/sdk.md)

WLC validates single-file Wirelink schemas (`.wl`) containing messages, RPCs, and node declarations, generating allocation-free ISO C11 codecs and typed node endpoints, plus installable C++20 / Python 3.10+ UDP client SDKs. WLC and Wirelink use development version **0.0.99**. Generated C uses **codegen contract 1**, and the SDK binding source API is revision **3**.

- Fields are required by default (no `has_*` presence flags); only `optional` and `optional packed` generate presence members.
- Nodes default to `handlers = static`, `framing = native`, `codec = schema_v1`, and `timeout_ms = 500`.
- `wlc compile` emits one `<stem>.h/.c` pair and `<stem>_manifest.json`. Zero nodes emit pure codecs; one node is auto-selected; multiple nodes require `--node <name>` or `--all-nodes`. Legacy `--profile` and `compile-kernel` have been removed.

Please read [WLC README (简体中文)](README.md) or [Wirelink (简体中文)](../README.md) for the full Chinese documentation.

## Quick Build & Test

```sh
cargo build --release --locked
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```
