# Current code generation

WLC 0.13.0 emits contract 0.13 and requires matching development headers/tools.
`wlc codegen-contract` reports the contract; `wlc dependencies schema.wl` reports
canonical transitive imports; `wlc module-name schema.wl` reports the exact C
filename prefix, including C/C++ keyword escaping.

`compile schema.wl --out-dir generated` emits pure public codecs and owned value
types. It has no endpoint/link/runtime binding output or communication dependency.
Public Schema-v1 vectors and legal schema values remain supported: full integer
widths, open signed enums, IEEE bits, bounded UTF-8 strings including NUL, bytes,
fixed packed arrays, nested messages, optional presence/defaults and repeated
borrowed views. Bounds/requiredness and schema evolution are validated before
emission. Pure standalone canonical helpers do not implement endpoint replay.

`compile ... --profile v2.bind.wl` emits the [static kernel](static-kernel.md).
Ordinary route codecs and private managed tagged RPC codecs are selected at
generation time. No runtime envelope/codec selector, old pending/replay table,
service dispatcher or fallback engine is emitted. `compile-kernel` is the same
generator; `compile-runtime` is removed with an explicit migration diagnostic.

Profiles can be composed by repeating `--profile`. Names, message IDs and route
roles are resolved canonically; conflicts are errors. `validate`, `identity` and
`plan` use the same current profile contract. Plan v2 reports the deployment's
actual four call/four reply slots, bounded TX resources and selected codec field
facts. Identity/digest values are diagnostic, not compatibility/security proofs.

The installed CMake package exposes `wirelink_wlc_generate(TARGET name SCHEMA ...
PROFILE ... PROFILES ...)` for a typed endpoint and `wirelink_wlc_generate_codec`
for codec-only generation. Supply matching local `WIRELINK_WLC_EXECUTABLE`; an
old compiler, old profile or missing compiler fails before a legacy engine can
be compiled. No historical release download is assumed for this development
contract. Schema imports and profiles are build/reconfigure dependencies.

Use a dedicated fresh output directory for migration. Existing old runtime,
bindings or endpoint artifacts are rejected rather than silently retained.
When changing stream deployment to native, stale stream output also requires
a fresh directory. Generated files and their provenance belong together.

Tests retain independent byte vectors and malformed-input cases; snapshots
record intentional output changes and do not replace wire conformance tests.
Removed feature test ownership is recorded in the root migration evidence.
