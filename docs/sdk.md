# Build and use an owning SDK

[中文](sdk-cn.md) · [Code generation](codegen.md)

Generate an installable C++20 client and typed Python sync/async package from a single `.wl` schema:

```sh
wlc sdk device.wl --node host --name device --package-version 0.0.99 --out-dir sdk
cmake -S sdk -B build/sdk -DCMAKE_PREFIX_PATH=/path/to/wirelink-install
cmake --build build/sdk --parallel
```

Build with WLC and Wirelink that use codegen contract 1 and SDK binding source API 3. The SDK package version is chosen by its author. Pass `--node` when multiple nodes declare `call` routes; if no explicit client node exists, `wlc sdk` synthesizes one from top-level `rpc` declarations. The generator generates a static-handler client, supplies per-call completions, and binds its built-in port to UDP with `init_io`.

## Build native libraries or a Python wheel

Install Wirelink with `WIRELINK_BUILD_CPP_BINDINGS=ON`, `WIRELINK_BUILD_PLATFORM=ON`, and `CMAKE_POSITION_INDEPENDENT_CODE=ON`, supplying standalone Asio headers. Generated C ships inside the SDK source package, so building it needs no Rust/WLC checkout. CMake defaults `<NAME>_BUILD_PYTHON=OFF`; wheel packaging enables it, and `BUILD_TESTING` controls hand-written tests. See the generated README for exact package commands and the [Calculator guide](https://github.com/starwey604/wirelink/blob/main/examples/06_bindings/GUIDE.md) for a runnable C service and Python tests.

CMake checks the codegen contract in `<name>_manifest.json` against the installed Wirelink package. `wlc-sdk-build.json` records the endpoint contract in the `wirelink-sdk-build-v3` format. Rebuild WLC after changing compiled-in templates before regenerating. `--overwrite` replaces generated files only; keep custom tests and guides outside that inventory.

## Share the native owner, retain results

C owns atomic port storage, the generated `<name>.h/.c` node state, and typed callbacks; C++ `Session` queues owner commands and drives waiting and UDP I/O. Sync and async calls share **four native call slots**; `AsyncClient` additionally bounds **eight undelivered operations**, not eight native calls. The host command queue holds 32 entries, drained in batches of at most 8. Admission snapshots the request before returning; a failed admission creates no call or terminal callback.

`Operation<T>` is copyable. Dropping it does not cancel the call, and its owned response/error can outlive the client. C++ timeouts are milliseconds in `1..INT32_MAX`; Python timeouts are seconds rounded up to milliseconds (max `2147483.647`). Both include owner-command queue time. Do not block or destroy the `Session` from an owner callback.

Close settles pending calls with `ClosedError`; explicit operation cancel produces `CancelledError`; an `asyncio` task cancel raises `asyncio.CancelledError` and requests native cancellation; expiry raises `RpcTimeoutError`; business rejection preserves signed `status`. Python errors expose `kind`, `status`, `outcome`, `os_error`, `os_message`; C++ `Error` holds `kind`, `completion`, and `system_error`. Local cancel/timeout cannot undo remote effects.

## Keep Python execution off the owner

Blocking Python waits release the GIL. `AsyncClient` uses one bounded completion worker and `asyncio` futures; no Python callback runs on the native owner. Use `with Client.connect(...)` or `async with AsyncClient.connect(...)` for deterministic close/join; `connect()` opens local resources without a peer handshake.

Business values own strings/bytes/nested messages/arrays. UTF-8 bounds count bytes, NUL is valid, unknown signed enums and float bits are preserved, and optional absence stays distinct from empty/default values. Wheels privately static-link native dependencies under separate nanobind domains. Subscriptions, Python servers, Serial/USB clients, dynamic schemas, free-threaded Python, and subinterpreters are outside this SDK interface.
