# wlc

`wlc` is the Wirelink schema compiler. It parses and validates `.wl` schemas,
checks a revision against its predecessor, and generates allocation-free C11
payload codecs plus optional typed Wirelink bindings.

A Chinese review version is available in [`README-cn.md`](README-cn.md). This
English document and the generated/public interfaces remain normative.

## Prebuilt compiler

Tagged releases publish host tools for Windows x86-64, Linux x86-64/aarch64
(static musl executables), and macOS x86-64/Apple Silicon. Each archive
contains the `wlc` executable and this README. Verify an archive against the
release's `SHA256SUMS` before using it in a build or CMake download cache.

The compiler version and generated-code ABI are separate compatibility axes.
`wlc --version` reports the release version, while every generated manifest
records `compiler.codegen_abi`. Build integrations should pin both rather than
following a branch or the newest release.

## Schema grammar

```text
schema         = "version" positive-integer ";" item+
item           = declaration | reservation
declaration    = message | enum
reservation    = "reserved" positive-integer ";"
message        = "message" identifier "=" positive-integer
                 "{" (field | reservation)* "}"
field          = optional-field | required-field | repeated-field
                 | packed-field | required-packed-field
optional-field = "optional" type identifier "=" positive-integer
                 ("[" "default" "=" literal "]")? ";"
required-field = "required" type identifier "=" positive-integer ";"
repeated-field = "repeated" type identifier "=" positive-integer ";"
packed-field   = "packed" packed-type identifier "[" positive-integer "]"
                 "=" positive-integer ";"
required-packed-field = "required" "packed" packed-type identifier
                        "[" positive-integer "]" "=" positive-integer ";"
packed-type    = "float32" | "float64" | "fixed32" | "fixed64"
type           = identifier | bounded-borrowed-type
bounded-borrowed-type = ("string" | "bytes") "<" positive-integer ">"
enum           = "enum" identifier "=" positive-integer
                 "{" enum-value (enum-value | enum-reservation)* "}"
enum-value     = identifier "=" integer ";"
enum-reservation = "reserved" integer ";"
literal        = integer | string | "true" | "false"
```

For example, a six-axis control vector is declared as:

```wl
message JointControl = 16 {
  packed float32 position[6] = 1;
  packed float32 velocity[6] = 2;
}
```

Line comments start with `//`. `version` must be first and positive. Message
and enum identifiers share one global namespace and one nonzero 16-bit ID
namespace. Field IDs are nonzero 16-bit numbers and unique within a message.
Packed element counts and borrowed-field length bounds are in `1..=65535`.

The built-in types are `bool`, `bytes`, `string`, `int8`, `uint8`, `int16`,
`uint16`, `int32`, `uint32`, `int64`, `uint64`, `fixed32`, `fixed64`, `float32`,
and `float64`. Narrow integers generate exact-width C `int8_t`, `uint8_t`,
`int16_t`, and `uint16_t` storage. `float32` and `float64` generate C `float`
and `double`. Generated headers enforce 4/8-byte IEEE-754 binary32/binary64
value formats with compile-time assertions. Codecs move floating-point bits
through `memcpy`; they never use aliasing casts or numeric conversions, so
signed zero, infinities, and NaN payload bits round trip exactly.
`string<MAX>` and `bytes<MAX>` retain the same `wl_codec_string_t` and
`wl_codec_bytes_t` borrowed-view representation as their unbounded forms; MAX
is the encoded byte length, not a Unicode scalar count, and adds no copy, heap,
lock, or hidden ownership.

Optional scalar defaults must match and fit their declared type, including the
exact range of narrow integers. Required fields cannot declare defaults and
cannot be repeated; fixed-width vectors use `required packed`.
Generated C retains a `has_*` flag for required scalar, nested, and packed
fields, and `*_clear()` resets it to false. Explicit floating-point
defaults are intentionally not accepted until the schema has a canonical,
locale-independent floating literal syntax; absent floats clear to positive
zero. `bytes`, repeated fields, and packed fields cannot have defaults. A
bounded string default is checked against MAX at schema-analysis time using its
UTF-8 byte length.

## Wire rules

The encoder emits fields in field-number order. Each field starts with an
unsigned LEB128 key `(field_number << 3) | wire_type`. Unsigned integers use
unsigned LEB128 values and signed integers use zigzag values, including the
8- and 16-bit types. A narrow decoder rejects a varint outside its declared
range with `WL_CODEC_ERR_OVERFLOW`; it never truncates into the C storage.
`fixed32` and `float32` use wire type 5 and four big-endian bytes; `fixed64`
and `float64` use wire type 1 and eight big-endian bytes. Adding a narrower
integer type does not alter the bytes of existing integer fields.

Encode and decode enforce every bounded string/bytes length for optional,
required, and repeated fields. A value longer than MAX returns the stable
`WL_CODEC_ERR_INVALID_VALUE` status; generated `*_encoded_size()` consequently
returns `SIZE_MAX`. Strings retain their independent UTF-8 validation and
return `WL_CODEC_ERR_UTF8` for an in-bound invalid sequence. Decode checks the
declared bound immediately after the length prefix, before borrowing the input
view, and never truncates or copies the payload.

A packed declaration is one field occurrence, represented in C by a presence
flag and an inline array. Plain `packed` is optional; `required packed` uses
the same layout but requires presence:

```c
bool has_position;
float position[6];
```

On the wire it always uses type 2 and contains one length prefix followed by
exactly `count` fixed-width, big-endian elements. A decoder rejects duplicate
occurrences, a wrong wire type, or any byte length other than
`count * sizeof(element)`. There is no caller pointer, count, capacity, heap
allocation, or per-element tag. Thus `packed float32 values[30] = 7;` encodes
as 122 bytes when present: one-byte tag, one-byte length (`120`), and 120 data
bytes.

Ordinary `repeated` fields keep their previous representation: a caller-owned
pointer/count/capacity in C and one complete tag/value pair per element.

Required and optional fields have the same wire representation. Before encode,
all required `has_*` flags must be true; otherwise encode returns
`WL_CODEC_ERR_MISSING_REQUIRED_FIELD` and `*_encoded_size()` returns `SIZE_MAX`.
Decode accepts unknown fields, but after consuming the complete payload it
rejects any absent required field with the same status. Nested message decode
checks the nested message's own required fields. Malformed or truncated wire
data remains a malformed-input error rather than being mistaken for absence.

Every generated message defines `<MESSAGE>_HAS_MAX_ENCODED_SIZE`. It is `1`
when the encoder output is provably bounded from the schema, in which case the
header also defines `<MESSAGE>_MAX_ENCODED_SIZE` as an exact conservative
`UINT64_C(...)` upper bound suitable for compile-time scratch sizing. The bound
includes worst-case tags, integer varints, packed payload lengths, bounded
string/bytes length prefixes, and nested message length prefixes. Bounded
optional or required string/bytes fields therefore participate in the static
maximum, including through nested messages. An unbounded `string`/`bytes` field
or any ordinary `repeated` field—bounded element or not—defines the `HAS` macro
as `0` and does not define a maximum. Analysis overflow is handled the same way
rather than emitting a truncated bound. The bound covers the encoded WLC
payload, not the Wirelink frame envelope; consumers targeting a narrower
`size_t` should also assert that the generated `UINT64_C` value fits `SIZE_MAX`.

## Generated typed bindings

Schema compilation produces deterministic `<module>.h/.c`,
`<module>_bindings.h/.c`, and `<module>_manifest.json` files. A separate
`compile-runtime` invocation produces only a named profile runtime and its
manifest. The codec files contain only the payload data model and codec and
continue to depend solely on `wirelink/codec.h`. The binding files form a
separate translation unit which depends on the public `wirelink/link.h` API.
A codec-only firmware therefore does not pull send, dispatch, or Wirelink core
symbols into its link.

The bindings header declares a module-prefixed router. Each message route has
a strongly typed `int32_t` callback, caller-owned message scratch, and an
opaque user pointer:

```c
static int32_t on_status(void *user, const status_t *status,
                         wl_delivery_t delivery);

motor_api_router_t router = {0};
static status_t status_scratch;
static uint32_t sample_storage[8];

status_scratch.samples = sample_storage;
status_scratch.samples_capacity = 8U;
router.status = (motor_api_status_route_t){
    &status_scratch, on_status, application
};
```

`<module>_dispatch_event(ctx, event, router)` accepts events returned by
`wl_poll()`. For every RX event and valid non-null `ctx`/`event`, it decodes,
optionally invokes the handler, and calls `wl_event_release()` exactly once on
success, unknown ID, missing route or scratch, codec failure, and handler
failure. A null router is a missing route, not a leak. TX and other non-RX
events return `*_DISPATCH_NON_RX` and are not released. Null `ctx` or `event`
is an API error; callers must not use it to dispose of a leased event.

Before decoding, the codec clears field presence and repeated counts while
retaining caller-configured repeated pointers and capacities, including
repeated storage nested in message scratch. Borrowed `bytes` and `string`
fields are valid only during the typed callback. The callback must not retain
them, release the event, or recursively dispatch the same context. Returning
zero reports success; any other `int32_t` value is preserved as a handler-
domain result.

The dispatch result keeps unknown message, missing route, missing scratch,
codec, and handler outcomes in separate domains and preserves the codec or
handler status. Per-domain router counters saturate at `UINT32_MAX` rather
than wrapping.

Every message has one module-prefixed `<module>_<message>_send()` function. It
takes an explicit `wl_delivery_t`, claims the final core TX payload span,
encodes directly into that span, and commits without an intermediate copy.
The returned struct preserves codec status, raw core result, encoded length,
and a reliable handle. A claim error such as `WL_ERR_NOT_SUPPORTED` is returned
unchanged in `core_result`; codec failure aborts the claim. Commit consumes the
claim on both success and failure.

A reliable wrapper returning `*_SEND_OK` means the send was submitted. Later
link ACK or `WL_EVT_TX_SUCCESS` still proves link delivery only—not successful
application execution. Reliability remains a call-site policy and is never
stored in the schema or payload wire format.

## Semantic and compatibility rules

All declaration and field numbers are explicit allocations; `wlc` never
auto-assigns IDs. Semantic analysis resolves every field type and rejects
unknown types, recursive or overly deep messages, invalid defaults, and
non-fixed-width packed element types. The public semantic model is sorted by
IDs and field numbers, so source reordering does not change generated output.

`reserved N;` permanently reserves a removed declaration ID, field number, or
enum value at its respective scope. Compatibility checks reject ID, name,
type, and cardinality changes or removal without a reservation. Adding a
required field to an existing message or removing one is incompatible even if
the removed number is reserved. Integer width and signedness are part of field
identity even when two types share wire type 0, so changing between narrow or
wide integer types is incompatible. A packed field's element type and fixed
count are both part of its wire identity; changing either is incompatible.
Adding, removing, increasing, or decreasing a string/bytes length bound is also
incompatible. Existing reservations must remain reserved.

The library API is `parse_schema()`, `analyze_schema()`,
`check_compatibility()`, `generate_c()`, and `generate_runtime_c_named()`.

## CLI usage

```sh
# Validate one schema.
cargo run -- validate path/to/schema.wl

# Validate compatibility and generate codec plus typed-binding C files.
cargo run -- compile path/to/schema.wl \
  --previous path/to/previous.wl \
  --out-dir generated

# Resolve an application policy sidecar and generate only its runtime files.
cargo run -- compile-runtime path/to/schema.wl \
  --profile path/to/device.bind.wl \
  --runtime-name device_api \
  --out-dir generated

# Print the exact schema/profile diagnostic identity pair.
cargo run -- identity path/to/schema.wl \
  --profile path/to/device.bind.wl

cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Diagnostics use `line:column: message`; at the CLI boundary `miette` renders a
source snippet at the invalid token.

## Optional binding profiles

Application routing policy lives in a separate, versioned sidecar rather than
the frozen `.wl` wire-schema grammar. A profile currently describes retained
`LATEST`/`FIFO` routes and RPC message/field mappings:

```text
profile version 1;

latest ArmMitCommand {
  delivery = unreliable;
}

fifo AlarmEvent {
  delivery = reliable;
}

rpc Home {
  request = HomeRequest;
  response = HomeResponse;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
```

Use `--profile path/to/device.bind.wl` with `validate`, or use
`compile-runtime` after compiling the schema. Runtime compilation writes only
`<runtime-name>_runtime.h/.c` and its own manifest; `--runtime-name` gives
asymmetric roles independent C namespaces while they include and call the
schema-stem codec/binding artifacts. The legacy `compile --profile` composition
remains available for one-runtime callers. The runtime header embeds the
separate schema/profile diagnostic identities. Its generated dispatcher
decodes a retained message directly into a `wl_latest_t` or `wl_fifo_t` write
claim, publishes only after successful decode, aborts every failed claim, and
releases each valid RX event exactly once. The same wire schema can therefore
use different host and device profiles without changing message IDs or
payload bytes.

The manifest's `bounded_fields` array records each bounded field's message and
field names/IDs, kind, and exact maximum byte length. Bounds also contribute to
the schema identity; unbounded legacy types retain their existing identity tags.

For a bounded profile, the generated header provides a no-heap default assembly
path with one retained/RPC slot:

```c
control_runtime_config_t config;
control_runtime_default_storage_t arena;
control_runtime_instance_t instance;

control_runtime_config_defaults(&config);
control_runtime_config_enable_client(&config);
control_runtime_storage_t storage =
    control_runtime_default_storage_descriptor(&arena);
control_runtime_init(&instance, &config, &storage);
```

Defaults use generation/operation ID one, one FIFO/RPC slot, exact schema
encoded maxima, disabled roles, zero timeouts, and reject-new cache policy.
The client/server helpers only enable a role; applications still set handlers
and business expiry policy. `*_RUNTIME_HAS_DEFAULT_STORAGE` is `0` when an RPC
payload is unbounded, in which case no misleading static arena type is emitted.
Use the custom path below after supplying explicit capacities.

`<module>_runtime_requirements()` validates all enabled component sizes and
reports the exact byte count and base alignment for caller-owned storage.
`<module>_runtime_init()` partitions that storage and wires every declared
LATEST/FIFO route, enabled RPC client/server, RPC slot/cache array, typed
request/response scratch object, canonical-request buffer, handler and user
pointer into `instance.runtime`. It validates the complete layout, including
overflow, size, alignment and overlap with the instance, before modifying the
instance or storage. The configuration and storage descriptors are copied and
may be temporary; the instance and backing bytes must remain at stable
addresses for their full runtime lifetime and must not be copied after init.
For bring-up, `<module>_runtime_init_checked()` returns the same `wl_err_t` and
also fills a diagnostic with the rejected field plus required/provided values.
`<module>_runtime_init_issue_str()` supplies optional human-readable text;
ordinary `runtime_init()` stays on the smaller firmware path, so checked-init
diagnostics can be removed by function/data-section linker garbage collection.

RPC client and server roles can be enabled independently. Sizing fields for a
disabled role are ignored and its runtime pointer remains null. FIFO capacity,
RPC slot counts, response capacities, expiry policy, and canonical-request
capacity are deployment configuration and deliberately do not participate in
the schema or binding-profile identity. Applications may still construct the
lower-level `<module>_runtime_t` manually when they need a custom layout.

Runtime results have a small common header (`domain`, event/message identity,
`detail_kind`, and `event_consumed`) followed by a tagged union. Dispatch sets
`event_consumed` after it releases an RX event or reclaims a matching terminal
TX handle; owner loops may apply their fallback action only while it is zero. Inspect
`result.detail.retained` only for `*_RUNTIME_DETAIL_RETAINED`, and
`result.detail.rpc` only for `*_RUNTIME_DETAIL_RPC`; `*_RUNTIME_DETAIL_NONE`
has no domain payload. A retained-only profile therefore does not carry the
larger RPC result fields. Generated runtime headers likewise include only the
LATEST, FIFO, and RPC public headers selected by that profile. The fixed
`<MODULE>_RUNTIME_CODEGEN_ABI_VERSION` macro is `18` for this surface; regenerate
all runtime sources and update field access together when that value changes.

Every generated result exposes `*_runtime_result_ok()` for the common success
check and `*_runtime_result_str()` for diagnostic text. Profiles emit checked
`*_runtime_result_retained_detail()` and/or `*_runtime_result_rpc_detail()`
accessors only when those detail variants exist. Accessors return null for a
null result or a mismatched `detail_kind`; diagnostic strings are not a stable
machine-readable error protocol.

Generated runtimes also expose `<module>_runtime_pump_init()` and
`<module>_runtime_pump_hooks()`. The returned hooks pass the pump's single
`now_ms` sample into dispatch, apply `event_consumed`, service at most one
queued RPC response per owner pass, and merge RPC deadlines. An optional
result observer receives borrowed diagnostic results; RPC profiles retain the
last service outcome in the pump state.
ABI 8 changes RPC server completion from a bare operation ID to a copied
`wl_rpc_request_identity_t`: generated callback types are named
`<module>_<service>_rpc_request_handler_fn`, receive `completion_identity`
before `delivery`, and generated `server_complete`/`server_reject` take that
identity pointer in place of `operation_id`.

`tests/runtime_size.rs` keeps deterministic size regression gates. On
`arm-none-eabi-gcc 16.2.0` with Cortex-M4, Thumb, and `-Os`, its representative
LATEST + FIFO + RPC runtime measures 3328 bytes for dispatch/RPC/consumer code,
1104 bytes for static assembly helpers, 282 bytes for the pump bridge, and 268
bytes for optional diagnostic strings/code. The respective gates are 3328,
1200, 320, and 288 bytes, with a 5088-byte full-object gate. Host layout gates
cap a retained-only result at 24 bytes and an RPC/combined result at 112 bytes.
These are generator regression fixtures, not whole-firmware estimates:
generated functions use individual sections, so a normal `--gc-sections` link
omits APIs and diagnostic strings that the application never references.

`<module>_runtime_dispatch_event()` is the terminal owner of every RX event
passed with non-null `ctx` and `event`. Every RX outcome—including an unknown
ID, null runtime, missing route or scratch, delivery mismatch, decode/storage/
RPC/application error, and replay-send failure—calls `wl_event_release()`
exactly once. It is not a chainable try-dispatch: do not release the event
yourself or pass it to the ordinary `<module>_dispatch_event()` afterward.
Null `ctx` or `event` cannot dispose of an RX lease. A matching RPC TX terminal
event advances the runtime and reclaims its core transaction. Unmatched non-RX
events remain caller-owned.

`LATEST` and `FIFO` retain decoded values after the RX callback. WLC rejects a
retained route whose message contains `bytes`, `string`, or `repeated` storage,
including through nested messages. RPC operation IDs must map to optional or
required `uint32` fields. Its response status must map to an optional or
required `int32` or enum;
an enum status domain must declare numeric value zero for success. Request and
response delivery are explicit and may be `reliable` or `unreliable`.

Each retained route emits a typed acquire/release pair. The acquired view owns
an explicit core lease and exposes `const <message>_t *value`; LATEST views also
expose the observed generation. The value remains borrowed until the matching
generated release succeeds. A successful release clears the view so accidental
reuse fails locally; callers must still serialize access according to the
underlying LATEST/FIFO contract.

`delivery = reliable` selects and validates Wirelink reliable DATA only. The
peer ACK is scheduled after admission to RX event storage and before typed
decode or application retention. FIFO full, LATEST coalescing or claim failure,
missing storage, codec/handler/RPC failure, and replay-send failure therefore
do not NACK the packet or restart link ARQ. Use an RPC response/status,
capacity policy, and application deadline for peer-visible completion.

For each RPC service the runtime header emits one typed client-start function,
a request callback route, and typed server complete/reject functions. Client
start uses a present nonzero operation ID exactly, or allocates one when the
field is absent or zero. Request and response inputs are `const`: the runtime
copies them into one shared, typed encode scratch before injecting operation ID
and status. Reusing an explicit ID with the same canonical request lets an
application retry address the server's bounded replay cache without adding a
second generated start API.
An encode or local submit failure releases the allocated client slot and returns
operation ID zero. Each service emits a nonblocking client-inspect helper that returns
generic operation metadata, a typed response decoder, and a service-checked
release helper. Borrowed `bytes`/`string` response fields point into client
response storage and remain valid only until that release. Response dispatch
validates the mapped ID and status, copies the raw payload into
`wl_rpc_client_t` before releasing the RX event, and leaves typed response
scratch under caller ownership.

The static instance owns per-service decode scratch plus one shared RPC encode
scratch, and the storage arena owns canonical-request bytes. With manual
runtime assembly, the caller supplies those objects directly and sets
`runtime.rpc_encode_scratch`. Borrowed `bytes` and `string`
fields in request/response scratch remain valid only until the callback or
dispatcher returns. If an RPC message contains a `repeated` field, its element
pointer and capacity are still application policy and must be set on the
instance scratch object after init and before dispatch. Canonical scratch must
be writable and large enough for the complete decoded request; re-encoding
drops unknown fields and makes field order irrelevant to duplicate
classification. Client start requires a Wirelink envelope with direct TX
support. The shared encoder scratch is used synchronously on the runtime owner
thread and no pointer into it escapes the generated call.

Response dispatch copies the original encoded bytes into the client's fixed
response storage before releasing RX. That copy remains valid until client
release; decode it again for a durable typed view rather than retaining
borrowed pointers from response scratch. Server complete/reject copy the const
response into shared encode scratch and set operation ID/status only on that
private copy.

Server dispatch decodes and canonically re-encodes the complete request before
computing a separately domain-tagged payload fingerprint. `NEW` invokes the
typed callback, `PENDING_DUPLICATE` suppresses it, `REPLAY` sends cached bytes,
and `CONFLICT` reports an RPC-domain error. Complete/reject encode exactly once,
move those bytes into the server cache, and send the identical cached sequence;
cached retry is public for a failed or deferred transport send. Request decode,
response decode, and canonical encode backing are supplied through the
generated runtime struct, so borrowed RPC fields remain valid only until the
callback/dispatcher returns. Reliable delivery confirms the Wirelink transfer,
not application acceptance; application rejection is represented by the
schema status and replay cache.

The callback's completion identity includes the reliable RX event's peer
session. Copy it before returning when completion is asynchronous, then pass
the copy to `server_complete` or `server_reject`. This makes equal operation
IDs from different peer sessions independent while retaining conflict
detection within one session. `server_response` remains populated only for a
cached replay or an explicit completion/rejection result.

Before dispatching a reliable RPC request, ABI 18 automatically observes its
nonzero peer session. A transition discards the preceding session's pending
and cached server work and asks the link to cancel any detached in-flight
response. `result.detail.rpc.peer_changed` identifies that dispatch; call
`<module>_runtime_peer_observation_take()` to obtain the transition and revoke
product leases or other non-RPC authority. Unreliable requests have no peer
session and do not trigger this point-to-point transition path.
Steady-state requests compare the already observed session inline and skip the
observer/cancellation path entirely.

All generated `now_ms` arguments, `wl_poll()`, RPC poll functions, and deadline
hints must use one monotonic millisecond clock and epoch. Generated dispatch
passes time into server duplicate tracking but does not advance client/server
expiry. RPC profiles emit a runtime poll wrapper that advances each enabled
role and reports per-call client timeout, server pending-expiry, and cache-expiry
counts. A side-effect-free deadline-hint wrapper returns the nearest enabled
role deadline (`0` means due and `WL_RPC_NO_DEADLINE_MS` means none), allowing a
host executor or event loop to choose its next wakeup without a generated
thread. Generated clients do not automatically repeat an end-to-end RPC after
timeout: link ARQ, the client deadline, and the server replay cache are separate
mechanisms.

`NEW` calls the request handler; zero accepts and leaves the operation pending,
while nonzero abandons it without manufacturing a response.
`PENDING_DUPLICATE` suppresses another execution. `REPLAY` sends the exact
cached response bytes, and `CONFLICT` reports reuse of an operation ID for a
different canonical request in the same peer session. Complete/reject cache
before sending, so a core send failure leaves a replayable response that
`runtime_service()` retains for a matching duplicate request. Cache
TTL/eviction, process restart, or explicit expiry ends
replay protection; this is bounded duplicate suppression, not durable
exactly-once execution. The domain-tagged FNV request fingerprint is likewise
a non-security classifier rather than authentication or a collision-resistant
digest.

WLC also exposes `schema_identity(&SemanticModel)` and
`binding_profile_identity(&BindingProfileModel)`. The CLI prints the same
values for diagnostics:

```sh
cargo run -- identity control.wl --profile device.bind.wl
```

The `fnv1a64-v1` identities hash normalized semantic models, so whitespace and
declaration order do not affect them. The schema identity is exact rather than
compatibility-aware: revisions, names, defaults, reservations, and wire types
all participate. The profile identity covers resolved IDs, field mappings,
roles, and delivery policy. They are deliberately not placed on the wire and
are not cryptographic hashes, authentication values, or substitutes for WLC's
compatibility checker. Report both values together when diagnosing generated
artifacts.

For profiled output, treat `(identity algorithm, schema identity, binding
profile identity)` as one diagnostic tuple. The profile identity is resolved
against a schema and is not meaningful alone. Compatible schema revisions may
have different exact identities. `wlc --version` reports the compiler package
release. The generated manifest records that release, the codegen ABI, both
available identities, and a sorted list of artifact byte sizes and
domain-tagged FNV digests. It deliberately omits timestamps, absolute source
paths, host details, and output-directory paths, so identical inputs and WLC
versions produce byte-identical manifests in different workspaces. The
artifact digests are diagnostic integrity values, not cryptographic hashes or
signatures.

## Dependency policy

The compiler deliberately keeps one dependency per concern:

| Crate | Role |
| --- | --- |
| `miette` | Source-aware, terminal-friendly diagnostics. |
| `thiserror` | Typed library errors. |
| `clap` | Declarative `validate`, `compile`, and `identity` CLI. |
| `heck` | Stable generated C symbol conversion. |
| `insta` | Reviewed generator golden snapshots. |
| `proptest` | Parser robustness and scalar-boundary properties. |
| `assert_cmd`, `tempfile` | Isolated CLI and generated-C tests. |

The hand-written parser keeps the grammar explicit. Generated codec C is
emitted with a small self-contained runtime and depends only on
`wirelink/codec.h` for status and borrowed byte/string types. Typed bindings
remain separately linkable and use only Wirelink public headers.
