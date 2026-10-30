# WLC

WLC 0.13.0 validates Wirelink schemas and emits public C11 codecs, a static
endpoint connected to the shared C kernel, or an owning C++20/Python UDP SDK.
The generated execution contract is 0.13; profile version 2 is required.

```sh
cargo build --manifest-path Cargo.toml
cargo test --manifest-path Cargo.toml
cargo clippy --manifest-path Cargo.toml --all-targets --all-features -- -D warnings
wlc compile application.wl --out-dir generated
wlc compile application.wl --profile application.bind.wl --out-dir generated
wlc sdk application.wl --profile host.bind.wl --name application --out-dir sdk
```

Without a profile, `compile` emits only public `.h`, `_values.h`, `.c` and a
provenance manifest. Public codecs retain the Schema-v1 legal value domain,
including unknown signed enum values, IEEE bit patterns, nested and repeated
views, bounded owned values, UTF-8/NUL handling and unknown-field validation.
Canonical fingerprint helpers in standalone codecs are independent pure tools;
they do not provide replay or duplicate suppression in the endpoint.

With a profile, `compile` emits `_kernel.h/.c`, private RPC codecs, deployment
and provenance manifests, and only the explicitly selected stream connector.
`compile-kernel` is an alias for the same generator. `compile-runtime` and old
runtime/bindings/advanced endpoint artifacts have been removed. An output
directory containing those artifacts is rejected; migrate into a fresh directory.
`module-name` prints the canonical C filename prefix, including keyword escaping.

```text
profile version 2;
endpoint { envelope = native_packet; }
rpc Execute { request = ExecuteRequest; response = ExecuteResponse; }
send Sample { delivery = unreliable; }
direct Event { delivery = unreliable; }
```

Choose `native_packet`/NONE, `cobs_stream`/CRC16, or `bus_length16`/NONE explicitly.
There is no negotiation or fallback. Both endpoint RPC roles remain enabled.
Ordinary routes support Schema-v1 and explicit positional-v1 public codecs.
RPC uses the existing managed tagged wire, with instance identity and operation
ID metadata; positional RPC must be explicitly migrated, never silently changed.

The kernel owns four client calls, four reply snapshots and one ordinary TX
snapshot. Admission encodes before returning success; rejected admission creates
no call or later callback. Timeout includes queued time. Exactly one terminal
callback runs on the owner; its response is borrowed only until callback return.
Typed cancellation is local and cannot undo remote execution. The owner supplies
`now`; the C core has no clock, heap, thread or OS dependency. Generated routing
uses the same kernel for ordinary messages and RPC.

Profile v1, reliable/ACK execution, RPC guarantee/replay/cache/TTL, mapped RPC,
LATEST/FIFO execution policies and deferred replies are removed and explicitly
rejected. Applications own retry/idempotency, duplicate handling and long jobs
(e.g. acknowledge a job ID, then exchange ordinary status messages). Standalone
framing/CRC/COBS and container tools are not an alternative communication engine.

`validate --profile`, `identity --profile` and `plan --profile` enforce the same
v2 contract; repeated `--profile` composes declarations with conflict checks.
`plan` reports actual kernel resources and selected codec field facts. Identities
and FNV artifact digests are diagnostic, not authentication or compatibility proof.
`--previous` checks schema evolution, independently of endpoint execution.

The SDK generator accepts bounded RPC-only native-packet profiles. C owns atomic
port storage and typed callbacks; C++ Session only queues owner admission, wraps
waiting and copies borrowed results. Sync/async clients share four C call slots.
Python dataclasses own their data, the native owner never invokes Python, and
blocking waits release the GIL. Source binding API 3 exposes outcome/status/OS
errors, including distinct business rejection and local failure. Close produces
ClosedError; explicit cancel produces CancelledError. Results outlive clients.

Build SDKs against a matching Wirelink 0.13 installation with `cpp_host` enabled.
Generated projects contain C artifacts and can build without WLC/Rust. Wheels
contain private static dependencies and separate nanobind domains. Existing old
SDK projects need an explicit migration into a fresh generated directory;
`--overwrite` alone cannot retain a second execution engine.

The installed CMake helper uses a matching local compiler via
`WIRELINK_WLC_EXECUTABLE`. `wirelink_wlc_generate(TARGET ... SCHEMA ... PROFILE ...)`
builds a typed endpoint; `wirelink_wlc_generate_codec` emits pure codecs. The
separate runtime/layout and historical compiler download helpers are removed.
No new release is published or assumed by this development tree.

See the repository's [kernel API](../docs/static-endpoint-kernel-api.md) and
[migration record](../docs/static-endpoint-kernel-k5-migration-20261031-cn.md).
Historical generation documents describe the previous execution contract and
must not be used as current API instructions.
