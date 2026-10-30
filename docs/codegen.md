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
| Schema-wide type features and shared-engine branch selection | `codegen/features.rs` |
| Codec assembly and packed entry points | `codegen/engine.rs`, `codegen/packed.rs` |
| Owned values and private validated conversions | `value_codegen.rs` |
| Core typed-send bindings | `codegen/bindings.rs` |
| Runtime profile/name validation | `runtime_codegen/validation.rs` |
| Runtime declarations and static storage assembly | `runtime_codegen/header.rs`, `runtime_codegen/assembly.rs` |
| Retained routes, RPC, dispatch, pump | Matching files in `runtime_codegen/` |
| Default endpoint and managed RPC glue | `endpoint_codegen.rs`, `rpc_endpoint_codegen.rs`, `managed_rpc_codegen.rs` |
| Shared managed request admission and response completion | `managed_rpc_server.c.in` |
| Host SDK validation, owned C++/Python types, C endpoint bridge and package templates | `sdk_codegen/` (see [SDK contracts](sdk.md)) |
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

## Codec and RPC plans

`codec_plan.rs` owns profile-specific scalar encodings, field order, presence
bits, bounded owned capabilities and checked maximum wire sizes. Public
Schema-v1, managed tagged bounds and SDK sizing share it without changing or
cloning the business semantic model. `codegen/plan.rs` adds C lookup/key/packed
recipes. The managed private source renderer still uses its existing model
conversion until it is migrated; do not describe that renderer as replaced.

`rpc_plan.rs` separates client owned-result union members, all server pending
responses and replay-only history. C computes union size/alignment for its ABI;
do not use host sizes or cast byte arrays to message objects. `wlc plan SCHEMA
--profile PROFILE` emits `wirelink-binding-plan-v1` JSON, including method
policies, resource bounds and field encodings. `null` capacity is unbounded or
absent; the method lists distinguish those cases.

Profile semantics accept `guarantee = replay|correlate` and explicit codec
identifiers `schema_v1`, `managed_tagged`, `positional_v1`. A plain route defaults
to schema-v1, managed RPC to managed tagged, and mapped RPC to schema-v1. Explicit
defaults normalize to the old model/identity. New policies change the profile
identity. Positional nested/repeated/bytes/string and mismatched metadata codecs
are errors. Send and receive declarations for one message must select one codec,
including across composed fragments.

Contract 0.12 implements positional execution for managed RPC and ordinary routes.
`compile --profile`, `compile-runtime` and `sdk` support explicit choices; unsupported
shapes fail before artifacts are written. The plan's availability flag reports policy
support, not successful full validation. The field plan is borrowed from the original
schema; symbol and storage policies are emitter inputs. There is no semantic-model
clone, public-codec regeneration, or generated-C namespace rewrite. Scalar shape
accesses are emitted as primitive lvalues with schema offsets.

The shared positional traversal covers required/optional scalars and fixed numeric
packed arrays. Existing small scalar shapes use direct positional encoding/decoding
with the same primitive readers and full-domain validation. Owned decoding writes
its final fields; public owned decode commits only after success. Replay canonical
fingerprints retain their independent Schema-v1 business-value traversal.
See [positional-v1](../../docs/positional-v1.md) for the complete format and evolution
contract. The codec, runtime and manifest record explicit formats and check matched
source/header identities. `wirelink_wlc_generate_codec(PROFILE ...)` selects ordinary
route codecs, while the paired runtime target must use the same route choices.

`tests/rpc_codec_plan.rs` and `rpc_policy_identity.txt` freeze the plan rules.
`tests/positional.rs` adds independent wire/error/ownership vectors and actual
endpoint calls; `rpc_guarantees.rs` includes mixed tagged/positional methods.
Both C fixtures support `WLC_TEST_SANITIZE=1`.

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

`Features` collects kinds and helper dependencies from all ordered messages,
including imported and nested definitions. Compiler-owned `// WLC_IF KIND...`
/ `// WLC_END` blocks are selected before literal substitution; nested blocks
use parent-AND / listed-kind-OR semantics. These markers never enter generated
C and cannot interpret substituted schema values. Both emitter sinks use the
same selection. Prune known-type handlers and dependent helpers, while retaining
all legal unknown wire types, validation, canonical order and presence semantics.
`tests/codec_plan.rs` compiles each scalar/packed type in isolation plus an empty
schema, checks frozen bytes, independent FNV hashes, unknowns and failure paths.

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

The current codegen contract is 0.12. Matched `compile --profile` and SDK generation
specialize canonical helpers to replay requests; standalone schema-only codecs
retain canonical helpers for expert runtime composition. `correlate` is executable,
with independent pending/response capacity and no completed history. Explicit positional-v1 execution is supported for its bounded scalar/packed
shapes. Private codecs contain only managed RPC messages and nested dependencies;
their canonical fingerprint engine is omitted because replay hashes public
Schema-v1 values. Ordinary routes retain their independently selected codec.


Endpoint setup requires the portable `wl_link_defaults_t` argument to both
`*_endpoint_config_defaults(config, environment, defaults)` and
`*_endpoint_init(endpoint, environment, defaults)`. The generated header includes
`wirelink/link_defaults.h`; it does not depend on USB integration headers.
Regenerate consumers when adopting this signature. This is part of the current
unreleased 0.12 source API, paired with the matching Wirelink/WLC checkout;
codegen contract 0.12, wire version, schema/profile identities and SDK binding
API 2 remain unchanged. The C++/Python UDP SDK generator uses
`wl_link_defaults()` explicitly. USB callers supply their integration preset
before initialization; `init_config` and `create` retain final config authority.
