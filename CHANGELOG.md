# Changelog

## 0.7.0-dev — unreleased

- Generate cancellable C++ operations and typed Python `AsyncClient` methods from
  managed RPC profiles. Emit C-compiled async callbacks and a native-only signal
  bridge; Python resolves bounded completions on its creating event loop.
  Add async API collision diagnostics and advance binding source API to revision
  **2**, retaining synchronous entry points and unchanged C artifacts/ABI 32.

- Add `wlc sdk` and the `generate_sdk` library API for deterministic standalone
  C++20 / typed Python SDK projects, reusing the existing C codec/runtime.
- Generate managed synchronous UDP Clients, bounded owned values, optional
  presence/default accessors, open enums, packed arrays and nested conversions.
- Emit CMake install/export, nanobind bridge/stubs, scikit-build-core wheel/sdist
  projects and dependency notices. Validate target names, unsupported profiles
  and sizes before writing; protect existing outputs unless `--overwrite` is explicit.
- Preserve C ABI **32**, schema/profile identities and all existing C output.
  The separate binding source API is preview revision **2**; this is not a
  release of prebuilt SDK wheels, or a stable C++ binary ABI.

- Trim RPC receive dispatch to the composed endpoint's client/server role,
  preserving missing-route/delivery diagnostics and RX release behavior.
- Share managed request metadata/session checks, peer observation, replay
  admission, and response-cache completion across services. Codec calls remain
  type-safe; no heap or persistent storage is added.
- Add a 24-service Cortex-M7 `-O2` text regression gate covering dispatch,
  shared helpers, typed adapters, and completion wrappers together.

This optimization keeps generated ABI **32**, public layouts, schema/profile
identities, business codecs, and wire formats unchanged. Regenerate runtime
sources to obtain the size improvement; existing codecs remain compatible.

## 0.6.0 — 2026-09-09

This release advances generated compatibility to **ABI 31**. Regenerate and
rebuild consumers with Wirelink v0.6.0; v0.5.0 generated ABI 30. Release tests
pin the matching core implementation at `ba891341feb8a32bb98d39ce59d37e1b269104b1`.

- Allow a target-wide COBS receive FIFO capacity override for generated endpoints.
- Mark generated driver readiness as complete so consumed events alone do not
  cause another host owner pass.
- Add advanced typed request-token inspection for product deadline tables,
  without transferring ownership or promising that a request is still pending.
- Retire cancelled old-peer response handles, including physically in-flight
  sends, so a peer restart cannot permanently block the next reliable response.

These generator changes do not change wire framing or RPC metadata. The FCI arm
consumer's separate mapped-to-managed migration must be deployed on both peers.

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
