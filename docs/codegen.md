# Maintaining the C generators

Start at `src/codegen.rs` for payload codecs and `src/runtime_codegen.rs` for
profile runtimes. These public entry points orchestrate private modules; the
schema/profile syntax and generated API are not defined by the module layout.

## Read by responsibility

| Responsibility | Source |
| --- | --- |
| C names, collisions, dependency order | `codegen/names.rs` |
| Recursive encoded-size bounds | `codegen/bounds.rs` |
| Shared wire-type and scalar-width facts | `codegen/wire.rs` |
| Validated C model, lookup choice, field keys, packed-array recipe | `codegen/plan.rs` |
| Borrowed declarations and descriptor data | `codegen/header.rs`, `codegen/descriptor.rs` |
| Codec assembly and packed entry points | `codegen/engine.rs`, `codegen/packed.rs` |
| Owned values and private validated conversions | `value_codegen.rs` |
| Core typed-send bindings | `codegen/bindings.rs` |
| Runtime profile/name validation | `runtime_codegen/validation.rs` |
| Runtime declarations and static storage assembly | `runtime_codegen/header.rs`, `runtime_codegen/assembly.rs` |
| Retained routes, RPC, dispatch, pump | Matching files in `runtime_codegen/` |
| Default endpoint and managed RPC glue | `endpoint_codegen.rs`, `rpc_endpoint_codegen.rs`, `managed_rpc_codegen.rs` |
| Shared managed request admission and response completion | `managed_rpc_server.c.in` |
| Local role/envelope capabilities and transport bounds | `endpoint_layout.rs`, `endpoint_transport_codegen.rs` |

`CModel` performs codec-side validation and computes bounds once per generation
entry. Runtime-only generation uses those facts without generating and throwing
away a codec. Runtime storage and endpoint sizing receive the same bounds.
`MessagePlan` holds compiler-side choices reused by descriptors and packed
wrappers; it adds no runtime metadata. Put future strategy decisions here, not
in string formatting or business schemas.

Endpoint capabilities come from the composed profile's single `endpoint` block.
Use `has_rpc_client()` / `has_rpc_server()` consistently for storage and facade
generation. Keep disabled-role checks in both endpoint and advanced runtime
initialization. Generated capability macros describe a fixed layout; they are
not user overrides. Test memory bounds by executing all envelope/role variants,
not by asserting only that a field disappeared.

Dispatch must honor the same role capabilities. Keep tiny known-message error
cases for omitted roles, retaining delivery validation and exactly-once RX
release. Do not turn an omitted role into a decode path or an unknown message.
For managed requests, observe a changed peer only after successful business
decode and canonical fingerprinting. Admission helpers must distinguish new,
pending duplicate, replay, conflict, and failure before entering application
callbacks. Response completion uses correctly typed codec adapters, never
function-pointer casts. Public token ownership/incarnation checks stay intact.

`tests/rpc_flash.rs` measures a 24-service Cortex-M7 runtime at `-O2`, without
LTO. Count extracted helpers and typed completion adapters along with dispatch;
moving bytes to another symbol is not a footprint improvement. See
[RPC Flash](rpc-flash.md) for the full application measurement and limitations.

## Templates are literal fragments

Shared C engine fragments live in `codegen/templates/`; existing RPC, endpoint,
fingerprint and packed templates retain their `.c.in`/`.h.in` paths under `src/`.
Use Rust for branching/iteration and `template::render` for `@NAME@` substitution.
Values are opaque, never recursively expanded; `@@` emits a literal `@`.
Missing, duplicate, invalid or unclosed parameters fail immediately as compiler
implementation errors. Unused supplied values are allowed for shared contexts.

Render child fragments with explicit parameters before inserting them into a
parent. Never rename functions by replacing text across a generated C program.
The encoder and fingerprint traversal share one emitter template with explicit
sink names/types, without a generated runtime callback or mode branch.

No external template framework is needed for literal substitution. This split
reduces responsibility per file, not total C algorithm size; it does not justify
new specialization or claim a Flash/CPU improvement.

## Correctness gate

Run from the WLC root:

```sh
cargo fmt --check
cargo test -j2 -- --test-threads=2
cargo clippy -j2 --all-targets --all-features -- -D warnings
WLC_TEST_SANITIZE=1 cargo test -j2 --test codec_plan --test owned_values \
  --test rpc_validation --test shared_scratch -- --test-threads=2
```

`tests/codegen_snapshot.rs` checks compact artifact manifests for five input
families. Refactors must keep these goldens unchanged. When a frozen compiler
is available, set `WLC_REFERENCE_COMPILER=/absolute/path/to/wlc` to compare every
C/H artifact byte-for-byte as well. Ordinary CI does not require that binary.
Manifests use diagnostic digests, not cryptographic equivalence proofs.

For an intentional output change only, run
`WLC_UPDATE_CODEGEN_SNAPSHOTS=1 cargo test --test codegen_snapshot`, review the
generated source diff, and assess ABI/fixtures/docs separately. Snapshot approval
does not replace generated C/C++ execution, malformed-input, ownership, UDP/FFI,
or Wirelink contract tests. Performance benchmarks remain separate and opt-in.
