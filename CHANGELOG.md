# Changelog

## 0.5.0 — 2026-09-09

This pre-1.0 release advances generated C API/layout compatibility from the
published v0.4.0's ABI 12 to **ABI 30**. Regenerate codec/runtime artifacts and
rebuild all consumers with matching Wirelink; do not mix old generated headers.
Compiler version, generated ABI and wire protocol version are separate concepts.

- Add default endpoint assembly, bounded owned business values, automatic RPC
  completion/recycling, immediate handlers and platform sync/executor bridges.
- Support endpoint clocks/environments and internal session creation, shared
  profiles/handler contexts, send-only routes, role/envelope-selected storage,
  `@id` syntax and RPC delivery attributes.
- Separate codec and runtime generation; reorganize checked code emission without
  changing the frozen generated artifacts of the corresponding internal ABI.
- Optimize canonical RPC fingerprints, validated value conversion and
  compiler-planned field lookup while preserving canonicalization semantics.
- Update release/CI verification to ABI 30 and pin the matching Wirelink core at
  `5f83685d935f64b17d0b512bfa72b5d667c25669`.

This bundles several previously internal iterations. Existing mapped RPC and
managed RPC metadata are not interchangeable wire formats; coordinate peers when
migrating RPC modes/metadata, rather than assuming that regeneration alone makes
all v0.4.0 traffic compatible. ABI 30's endpoint role/envelope layout change itself
does not alter the internal ABI 26–29 wire formats.
