# Owning C++ and Python SDK

WLC 0.13.0 generates binding API 3 against Wirelink contract 0.13. It accepts
bounded RPC-only profile v2 with explicit native_packet deployment. The kernel
retains both RPC roles; the generated client has no business server handlers.

```sh
wlc sdk device.wl --profile host.bind.wl --name device --package-version 0.13.0 --out-dir sdk
cmake -S sdk -B build/sdk -DCMAKE_PREFIX_PATH=/path/to/wirelink-install
cmake --build build/sdk --parallel
```

Build Wirelink with `WIRELINK_BUILD_CPP_BINDINGS=ON` to install `cpp_host` and
its host/Asio/platform dependencies. Pre-generated C ships in the source package,
so SDK compilation needs no WLC/Rust checkout. Python wheels use private static
libraries and a separate nanobind domain; wheel users need no development tools.
See the generated README for CMake/wheel commands.

C owns port atomics, typed callback signatures and kernel storage. Session queues
owner-side admission and wraps I/O waiting; it has no RPC matching/timeout table.
Sync/async methods share four kernel call slots. Admission snapshots requests
before success returns; admission errors create no call or future callback.
Synchronous methods refuse owner-thread blocking. The callback copies borrowed
responses before returning, so Operation results and Python data outlive clients.

Explicit cancel is local and reports CancelledError; connection close reports
ClosedError. Timeout includes queueing. None implies rollback of remote effects.
Business rejection retains its signed status, separately from local failures;
Python/native errors expose outcome/status/OS details, not removed union fields.
Applications own retries, idempotency and long job/status protocols.

Python blocking waits release the GIL. AsyncClient uses one bounded completion
worker, a shared native coalescing signal and asyncio futures; no Python callback
runs on the native kernel owner. Closing waits for owner/I/O completion. Cancelling
an asyncio task requests local operation cancellation without rolling back a peer.

All public business values own strings/bytes/nested messages/arrays. UTF-8 bounds
are bytes, NUL is legal, unknown signed enum values and floating point bit patterns
are preserved. Optional presence differs from absent/default and present-empty.

Old SDK output contains an incompatible execution API. Generate into a fresh
directory and migrate application-owned files explicitly; `--overwrite` cannot
leave the old runtime beside the new kernel. Standalone RPC layout/IRAM knobs
were removed. No stable C++ ABI, dynamic schemas, subscriptions, Serial/USB SDK,
free-threaded Python or subinterpreters are claimed by this generator.

The maintained examples `examples/06_bindings` and `examples/07_device_sdk` run
real C-owned UDP peers and exercise C++, Python sync/async, lifetime, admission,
error-domain and multi-SDK isolation contracts.
