# Build and use an owning SDK

[中文](sdk-cn.md) · [Code generation](codegen.md)

Generate an installable C++20 client and typed Python sync/async package:

```sh
wlc sdk device.wl --profile host.bind.wl --name device --package-version 0.13.0 --out-dir sdk
cmake -S sdk -B build/sdk -DCMAKE_PREFIX_PATH=/path/to/wirelink-install
cmake --build build/sdk --parallel
```

Use WLC 0.13.0 / generated contract 0.13, binding source API 3 and matching
Wirelink 0.13.0. The SDK package version is chosen by its author. Profiles must
be bounded, RPC-only, version 2 and `native_packet`. Both kernel RPC roles remain
enabled; the client does not register business server handlers.

## Build native libraries or a Python wheel

Install Wirelink with `WIRELINK_BUILD_CPP_BINDINGS=ON` and
`WIRELINK_BUILD_PLATFORM=ON`, supplying standalone Asio. Static dependencies need
PIC for a Linux Python extension. Generated C ships in the SDK source package;
building it needs no Rust/WLC checkout. CMake defaults `<NAME>_BUILD_PYTHON=OFF`;
wheel packaging enables it. Native hand-written tests run with `BUILD_TESTING`.
See the generated README for exact package commands and the maintained
[Calculator guide](https://github.com/starwey604/wirelink/blob/main/examples/06_bindings/GUIDE.md)
for a runnable C service and Python tests.

CMake checks WLC version/codegen contract in the included kernel manifest
against installed Wirelink package metadata. `wlc-sdk-build.json` records build metadata (`wirelink-sdk-build-v2`,
kernel contract 1); it does not select linker layout. Regenerate with a rebuilt,
matching WLC after changing compiled-in templates. `--overwrite` replaces the
same SDK's generated files; keep application tests/guides outside that inventory.

## Share the native owner, retain results

C owns atomic port storage and typed callbacks. C++ Session queues owner commands
and wraps waiting/I/O. Sync and async calls share **four native call slots**;
AsyncClient additionally bounds **eight undelivered operations**, not eight native
calls. The host command queue has 32 entries, drained in batches of at most 8.
Admission snapshots the request before success returns. A failed admission
creates no call or terminal callback.

`Operation<T>` is copyable. Dropping it does not cancel; its owned response/error
can outlive the client. Callback code copies borrowed views before returning.
C++ timeouts are milliseconds in `1..INT32_MAX`; Python timeouts are seconds,
rounded up to milliseconds, at most 2147483.647. Both include owner-command queue time.
Do not block or destroy the Session on its owner callback.

Close completes pending calls with `ClosedError`; explicit operation cancel
produces `CancelledError`. An asyncio task cancel raises `asyncio.CancelledError`
and requests native cancellation. Expiry raises `RpcTimeoutError`. Business
rejection preserves signed `status`. Python errors expose `kind`, `status`,
`outcome`, `os_error`, `os_message`; C++ Error holds `kind`, `completion` and
`system_error`. Local cancel/timeout cannot undo remote effects. RPC-only SDKs
need a separate application status-query RPC for long jobs.

## Keep Python execution off the owner

Blocking Python waits release the GIL. AsyncClient uses one bounded completion
worker and asyncio futures; no Python callback runs on the native owner.
Use `with Client.connect(...)` or `async with AsyncClient.connect(...)` for
close/join. A connection opens local resources, without a peer handshake.

Business values own strings/bytes/nested messages/arrays. UTF-8 bounds are bytes,
NUL is legal, unknown signed enums and floating-point bits are preserved, and
optional absence differs from present-empty/default values. Wheels privately
link native static dependencies and use separate nanobind domains.

The generator provides UDP RPC clients. Subscriptions, Python service handlers,
Serial/USB clients, dynamic schemas, free-threaded Python and subinterpreters
are outside this SDK interface. Rebuild C++ consumers with matching headers and
compiler settings; the source interface does not promise a stable C++ binary ABI.
