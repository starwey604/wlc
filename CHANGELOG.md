# Changelog

## Unreleased

- **Breaking API:** generated `*_endpoint_config_defaults()` and quick
  `*_endpoint_init()` now require a `wl_link_defaults_t` argument. Generic
  `wl_link_defaults()` selects CRC16; the shared USB integration preset
  `wl_usb_bulk_link_defaults()` selects NONE. Explicit overrides and complete
  configs remain authoritative. Regenerate current SDKs with matching WLC;
  envelope/storage bounds, wire version and schema identities are unchanged.

- **Breaking default:** generated ordinary endpoints using generic defaults use CRC16 (CCITT-FALSE).
  Update both peers together or explicitly select CRC32C before initialization.
  Maximum storage bounds, explicit CRC32C support, ABI and identities are unchanged.

- Share RPC body/wire bounds across runtime defaults, server guarantee groups
  and default endpoints. C array minimums, u16 limits and target ABI sizing stay
  in the backend; generated artifacts and plan JSON remain unchanged.

- Remove the Rust `RpcEndpointPlan::client_result_union` API. Plans expose pure
  `validate_client_results` validation; the C runtime backend emits owned-result
  unions from the same validated plan. Generated C and codegen contract stay unchanged.

- Preserve underscores in RPC layout configuration prefixes so CMake OFF also
  applies to keyword-escaped runtime names such as `switch_`. Validate the whole
  expanded setting in both C and the preprocessor; reject expressions like `4/2`
  and C enum constants while allowing macro aliases that expand to `0` or `1`.
- Add optional `ISOLATE_LITERALS` grouping to bound literal colocation without
  merging function inputs or retaining dead code.

- Make WLC RPC ELF layout default ON with global/module OFF controls, independent
  function input sections, module/function placement hooks and optional linker
  grouping. Add native SDK build metadata; changing a prebuilt SDK's layout
  requires rebuilding its archive/wheel. Preserve codegen 0.12, SDK binding API 2
  and schema/binding/wire identities.
- Supply Zephyr UART IRQ memory attributes from the platform build, removing
  the generic adapter's ESP-IDF header and SoC dependency.

- Emit runtime-local canonical Schema-v1 fingerprints for small scalar managed
  replay requests from the shared field/key plan, keeping the ordinary encoder
  output unchanged. Skip a duplicate pump readiness query; preserve checked
  service/backpressure paths, complex-message hashing and contract 0.12.

- Keep route/method codec selection local to its top-level message. Generate
  separate tagged descriptors when a nested child also uses positional-v1,
  preserving enclosing wire bytes, owned decoding, canonical fingerprints and
  standalone runtime capacities regardless of message IDs or service order.

- Remove private codec copies for non-RPC messages and the unused private
  fingerprint/validated-measure path. Replay continues to hash canonical public
  Schema-v1 values. Preserve contract 0.12 and regenerate checked-in artifacts.

- Require matched core/codegen contract 0.12. Generate managed tagged and
  positional-v1 directly from borrowed field plans and explicit symbol policy;
  delete private model cloning and generated-source rewriting. Add strict
  positional bitmap/scalar/packed readers, direct small scalar shapes, owned
  transactional decode vectors, independent ordinary route profiles and
  source/header identity guards. Record formats in provenance manifests.

- Generate replay, correlate, and mixed RPC servers with different resource
  groups. Retain replay by default and prune canonical helpers for correlate
  methods in matched codec/profile generation. Keep stable response/TX storage
  and generation-scoped deferred tokens for every execution. Move mapped peer
  observation after successful business/metadata validation. Require contract
  0.11 in P3; P4 adds positional execution under contract 0.12.

- Share TLV traversal and primitive readers for owned decoding; write managed replies
  into their final typed fields and encode scalar owned values without temporary views.
  Keep RPC execution functions together on ELF targets, with a portable fallback.

- Generate actual per-call typed result unions and checked caller-owned pools.
  Decode managed replies directly once; mapped replies decode once into owned
  scratch before correlation. Completion and retained access copy/borrow values.
  Remove client raw response pools and repeat decoding APIs; require bounded
  client responses and regenerate SDK provenance for contract 0.10.

- Add explicit codec/RPC resource plans and the `plan` JSON command. Parse and
  validate static `guarantee`/`codec` choices; keep implicit tagged/replay bytes
  and identities unchanged. Reject execution generation for unimplemented
  positional paths. Share checked bounds without cloning the schema,
  and reject codec/runtime handler name collisions before emitting C.
- Prune the generated shared codec and canonical fingerprint engine to the field
  types used by the complete schema, including imports. Omit unused UTF-8,
  nested-message, packed and scalar helpers at generation time. Preserve wire
  bytes, canonical fingerprints, unknown-field validation and public layouts;
  initialize the empty owned-to-view placeholder. Regenerate checked-in SDKs.

- Generate direct codecs and canonical fingerprint paths for small scalar
  messages, preserving public TLV bytes and codec semantics while avoiding
  descriptor type/cardinality dispatch on their hot paths.
- Replace unreleased managed RPC wire encoding with BE64 originating sessions
  and varuint33 correlation/rejection metadata (9–13 bytes; rejection 13–17).
  Encode scalar fixed32 RPC business fields as full-range tagged uint32 varints
  in a runtime-private codec; retain public codecs, canonical fingerprints,
  mapped RPC and Compact-v1. Preserve full operation/session IDs, replay,
  deferred completion and stale-response isolation. Include private maximum
  sizes and wire-format provenance; regenerate both peers without fallback.

## 0.8.0 — 2026-09-13

- Add `wlc sdk` and the `generate_sdk` API for deterministic standalone C++20 /
  typed Python SDK projects, reusing the C codec and managed UDP runtime.
- Generate owning synchronous clients, cancellable C++ operations and Python
  `AsyncClient`, bounded completion delivery, typed exceptions and package metadata.
- Support bounded strings/bytes, optional fields/defaults, open enums, fixed arrays
  and nested values. Reject unsupported profiles and generated-name collisions.
- Emit installable CMake packages, nanobind bridges, stubs, wheels/sdists and
  dependency notices. Require matching Wirelink 0.8.0 native development packages.
- Binding source API is revision **2**; C codegen ABI stays **32** and protocol v1
  is unchanged. No stable cross-compiler C++ binary ABI is promised.
- Reduce generated RPC dispatch size with role-aware shared control flow and a
  Cortex-M7 text-size regression gate.
- Retain verified host compiler archives for all five release platforms.

## 0.7.0-rc.1 — 2026-09-12

First published ABI **32** compiler, paired with Wirelink v0.7.0-rc.1.
Static schema imports and borrowed direct routes require consumers to regenerate
and rebuild together. This prerelease preserves the development ABI 32 layouts
and wire formats and publishes checksummed host packages for all five platforms.

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
