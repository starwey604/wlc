# 0.13 execution migration

The default frontend generates the shared static C kernel. Old execution
backends/templates are deleted, not disabled behind a legacy switch. The C
kernel and adapters have their own admission/lease/timeout/close tests; WLC
retains frontend, legal-value/codec vectors, new generated kernel, composition,
SDK and explicit unsupported-input checks. No removed suite is marked xfail.

The following execution-only tests were removed or ported. Their pre-migration
source remains in git at `ec28324` and the sealed K5 entry source archive. Removing
old behavior assertions does not discard public codec/wire vectors: managed wire
payload vectors were ported, positional ordinary vectors kept, and full scalar,
owned/nested, malformed input and frozen framing cases still run.

| Previous file | Decision |
| --- | --- |
| `tests/async_endpoint.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/c_bindings.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/c_rpc_runtime.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/c_runtime.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/c_runtime_assembly.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/default_endpoint.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/direct_routes.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/endpoint_layout.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/managed_rpc.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_flash.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_guarantees.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_layout.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_scalar_fingerprint.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_usability.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_validation.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/runtime_codegen.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/runtime_size.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/send_profiles.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/shared_scratch.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/rpc_codec_plan.rs` | Removed legacy endpoint/pump/ARQ/replay/mapped/deferred execution contract; replaced by current kernel/core/adapter and rejection tests. |
| `tests/fixtures/allocated_endpoint.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/async_endpoint.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/default_endpoint.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/direct_routes.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/endpoint_layout.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/managed_rpc.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_cache_baseline.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_guarantees.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_layout.py` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_policy_identity.txt` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_result_layout.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_scalar_fingerprint.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/rpc_validation.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/send_profiles.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/shared_scratch.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/fixtures/sync_endpoint.c` | Fixture for removed execution-only suite; archived at K5 entry. |
| `tests/managed_rpc_wire.rs + fixtures/managed_rpc_wire.c` | Retain legal payload vectors and full metadata domains on shared kernel primitives; old inspect/release/terminal-slot execution part replaced by new core and C++ lifetime tests. |

Mixed suites were split: profile identity keeps schema identity checks; CLI
keeps standalone behavior and gains current profile/output/version diagnostics;
public positional vectors and shared nested codec dependencies keep their wire
oracles. Old retained/reliable/replay/mapped/long-terminal-slot assertions are
replaced by explicit v2 rejection and new core ownership contracts. Old SDK
HeldOwner queue tests belong to the current root host fixture; sync/async SDK
and rollback/lifetime tests continue against real UDP.

Standalone public canonical helpers are pure codec tools, with no generated
kernel replay dependency. Contract 0.12 can only be used for frozen standalone
public codecs in the new core; old execution headers and link bindings no longer
exist. SDK source API is 3, generated contract 0.13, profile version 2. No old
published compiler is used as a hidden fallback.
