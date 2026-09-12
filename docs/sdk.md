# Generated host SDKs

`wlc sdk` generates a complete synchronous/asynchronous UDP SDK from a resolved wire schema
and composed binding profiles. The existing C codec/runtime remains the wire
implementation. The C++ / Python layers convert owned values and call the managed
endpoint through Wirelink's owning host Session.

```sh
wlc sdk product.wl --profile services.bind.wl --profile host.bind.wl \
  --name product --package-version 0.1.0.dev1 --out-dir sdk

cmake -S sdk -B build/sdk -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH=/absolute/path/to/wirelink-install \
  -DCMAKE_INSTALL_PREFIX=/absolute/path/to/sdk-install
cmake --build build/sdk --config Release --parallel
cmake --install build/sdk --config Release

python -m pip install build
python -m build sdk -Ccmake.define.CMAKE_PREFIX_PATH=/absolute/path/to/wirelink-install
```

Install the matching Wirelink 0.7.0 development checkout first, enabling
`WIRELINK_BUILD_CPP_BINDINGS=ON`, `WIRELINK_BUILD_PLATFORM=ON`,
`WIRELINK_BUILD_EXAMPLES=OFF`, `BUILD_SHARED_LIBS=OFF` and
`CMAKE_POSITION_INDEPENDENT_CODE=ON`; provide standalone Asio headers through
`WIRELINK_ASIO_INCLUDE_DIR`. No schema compiler is required when building the
project: generated C ships in the sdist. The standard Python build creates an
sdist, extracts it, and builds a wheel from the extracted source.

C++ consumers use `find_package(ProductSdk CONFIG REQUIRED)` and
`ProductSdk::client`; the public header is `<product/client.hpp>`. Python consumers
install a matching wheel and import `product_sdk`, without native development
packages or build tools. Each wheel links private static native dependencies and
has its own nanobind domain. No publishing happens during generation or building.
Local Linux wheels need a supported manylinux build/repair before broad distribution.

## Source API

For service `Configure`, C++ generates
`Result<ConfigureResponse> Client::configure(const ConfigureRequest&, milliseconds)`.
Python generates `client.configure(*, field=value, timeout=1.0)` and
`client.configure_request(ConfigureRequest(...), timeout=1.0)`. Python timeouts are
seconds, rounded up to milliseconds; the native range is 1 through INT32_MAX.
Calls and close are safe from multiple threads. C++ moves/destruction require
exclusive handle access. Keep requests unchanged during synchronous calls and
until an asynchronous submission returns.

Binding source API revision **2** additionally generates
`Result<Operation<ConfigureResponse>> Client::configure_async(request, timeout)`.
Submission owns the request before returning. `Operation<T>` is copyable and
supports `done()`, `wait()`, `wait_for(milliseconds)`, `result()` (an owned result
copy) and `cancel()`. Discarding the handle leaves admitted work owned by Session;
retaining it keeps results readable after client destruction. Cancellation is a
local request, cannot roll back peer effects, and can lose to completion.

Python `AsyncClient` has matching typed `async def` service and `_request` methods.
`async with AsyncClient.connect(Udp(...))` creates the local connection in the
running loop; RPCs use `await`, teardown uses `await close()`. Each client belongs
to one event loop. Task cancellation remains `asyncio.CancelledError`, native
RPC deadlines raise `RpcTimeoutError`, and orderly close cancels remaining calls
with the SDK's `CancelledError`. Concurrent close waiters share shielded cleanup;
cancelling one waiter does not interrupt cleanup. No coroutine facade submits
blocking per-RPC jobs to a Python thread pool.

Sync and async calls share eight host admission slots. The default C endpoint has
four RPC slots and can reject earlier. Python keeps at most eight undelivered
operations, including completed or locally cancelled ones awaiting native drain.
Full capacity raises `QueueFullError`, preserving the original diagnostic.
Timeouts start at host admission and include queue time. The endpoint owner alone
submits, cancels and completes native operations; exactly one terminal result is
published before notification. Close joins the owner and settles all admitted work.

One closeable Python notification thread per client waits on a coalesced native
condition variable and uses `loop.call_soon_threadsafe` to request a bounded drain.
Only the event loop converts Python messages and resolves Futures. The nanobind
signal wrapper owns a *pure C++* shared pointer; never pass nanobind's
Python-retaining shared-pointer caster into the owner, including through deleters.
Deterministic cleanup requires `async with` or `await close()`; idle abandoned
clients can be collected because the notification thread uses a weak reference.

| Schema | C++ | Python |
| --- | --- | --- |
| Scalar | fixed-width integer, bool, float, double | checked int, bool, float |
| `string<N>` | `std::string` | `str`, checked by UTF-8 byte length |
| `bytes<N>` | `std::vector<uint8_t>` | immutable `bytes`; bytearray/memoryview copied |
| Fixed packed numeric array | `std::array<T, N>` | exact-length tuple; lists copied |
| Message | owned struct | frozen dataclass |
| Optional field / optional packed array | `std::optional<T>` | `T \| None` |
| Enum | `enum class : int32_t` | open `IntEnum` |

Absent fields stay absent on re-encoding, including when the schema declares a
default. An explicit default generates `<field>_or_default()` in C++ and a
`<field>_or_default` property in Python. Present empty strings/bytes and zero values
remain distinct from absence. UTF-8 may contain embedded NUL. Unknown signed
32-bit enum values survive round trips; Python does not cache each unknown value.
NaN/infinity follow the C floating-point wire contract.

All response data remains usable after another RPC or close. Blocking native
calls release the GIL; no Python callback executes on the native owner. Failure
classes retain the local, transport, RPC, codec and OS diagnostic domains. Python
validates inputs before conversion and raises TypeError/ValueError or a native
failure's corresponding exception class.

Types use PascalCase, fields/services snake_case, and enum members upper snake
case. Language keywords and `self`, `timeout`, `session` gain a trailing underscore.
Collisions after mapping, with default accessors, or with Client/AsyncClient/Udp and their
methods are diagnostics. The native `<sdk>_sdk_` schema prefix and bridge helper
names are reserved; service names that collide with bridge submission/completion
functions also fail before emission. Required Python constructor fields precede optional ones;
prefer keywords as schemas evolve. Private C typedefs are isolated in the C++
value bridge so `Mode` does not collide with system `mode_t`; C enum macros are removed
from that bridge header after inclusion to preserve scoped C++ enum member access.
Endpoint initialization, callbacks and sync/async entry points are compiled in
`src/endpoint.c`. C++ uses a private opaque bridge with size/alignment queries;
endpoint storage never moves. Do not include the generated runtime in a C++
namespace: its callback types would disagree with the C runtime (UBSan detects this).

## Limits and generated files

Profiles must contain managed RPC services, permit clients, and use `any` or
`native_packet` envelopes. Use separate host profiles for send/direct/latest/fifo
routes. Every message must have a finite bound. Repeated fields, unbounded
strings/bytes, empty enums, mapped RPC, and requests/responses whose maximum encoded
payload plus managed metadata exceeds 2048 bytes are rejected before emission.
The generator currently uses default endpoint storage and capacity.

Subscriptions, Python handlers, Serial/USB, Bulk, dynamic schemas,
free-threaded Python and subinterpreters are separate work. The preview binding
source API is revision 2; C ABI stays 32. No stable C++ binary ABI is promised.
Multiple wheels can coexist, but standalone C++ SDKs with overlapping generated
C symbols require shared codec composition that this command does not provide.

Generation emits sorted relative files without timestamps or host paths.
`generated/<name>_manifest.json` describes unchanged C artifacts;
`wlc-sdk-info.json` records package identity/source API revision and
`wlc-sdk-manifest.json` describes the generated project. Digests are diagnostic,
not cryptographic authentication. Keep source schemas in the maintainer's
repository; no runtime schema parser is packaged.

Identical repeated generation is allowed. Changed existing files require
`--overwrite`; unlisted files are retained. SDK names cannot change in place,
even with overwrite. Use dedicated output directories and keep custom tests/build
integration outside generated files. An optional `tests/CMakeLists.txt` is loaded
when BUILD_TESTING is enabled. Library callers receive a BTreeMap and control
writing policy themselves.

## Maintenance

`sdk_codegen/plan.rs` validates shared facts with CModel ordering/bounds; `cpp.rs`
and `python.rs` emit conversions; `package.rs` renders build templates. Add
semantics to the plan, never by parsing generated C headers. Existing C snapshots
must stay unchanged. Run `cargo test`, `cargo clippy --all-targets --all-features
-- -D warnings`, `cargo fmt --check` and `WLC_TEST_SANITIZE=1 cargo test --test sdk`
with the matching Wirelink source available. Native wheel/RPC and multi-SDK tests
live in Wirelink's binding examples.
