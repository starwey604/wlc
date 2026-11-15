# WLC (Wirelink Compiler)

[简体中文主页 (Simplified Chinese)](README.md) · [Wirelink Repository](../README-en.md)

**Documentation in English is coming soon.**

WLC validates Wirelink schemas (`.wl`) and binding profiles (`.bind.wl`), generating allocation-free ISO C11 codecs, statically typed kernel endpoints, and C++20 / Python UDP SDKs for Wirelink **0.13.0** (codegen contract **0.13**, binding API **3**).

Please read the Simplified Chinese documentation:

- [WLC README (简体中文)](README.md)
- [Wirelink Main README (简体中文)](../README.md)

## Quick Build & Test

```sh
cargo build --release --locked
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```
