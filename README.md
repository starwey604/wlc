# wlc

`wlc` is the Wirelink schema compiler. It parses and validates `.wl` schemas,
checks a revision against its predecessor, and generates allocation-free C11
payload codecs plus optional typed Wirelink bindings.

## Schema grammar

```text
schema         = "version" positive-integer ";" item+
item           = declaration | reservation
declaration    = message | enum
reservation    = "reserved" positive-integer ";"
message        = "message" identifier "=" positive-integer
                 "{" (field | reservation)* "}"
field          = optional-field | repeated-field | packed-field
optional-field = "optional" type identifier "=" positive-integer
                 ("[" "default" "=" literal "]")? ";"
repeated-field = "repeated" type identifier "=" positive-integer ";"
packed-field   = "packed" packed-type identifier "[" positive-integer "]"
                 "=" positive-integer ";"
packed-type    = "float32" | "float64" | "fixed32" | "fixed64"
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
Packed element counts are in `1..=65535`.

The built-in types are `bool`, `bytes`, `string`, `int32`, `uint32`, `int64`,
`uint64`, `fixed32`, `fixed64`, `float32`, and `float64`. `float32` and
`float64` generate C `float` and `double`. Generated headers enforce 4/8-byte
IEEE-754 binary32/binary64 value formats with compile-time assertions. Codecs
move floating-point bits through `memcpy`; they never use aliasing casts or
numeric conversions, so signed zero, infinities, and NaN payload bits round
trip exactly.

Optional scalar defaults must match their type. Explicit floating-point
defaults are intentionally not accepted until the schema has a canonical,
locale-independent floating literal syntax; absent floats clear to positive
zero. `bytes`, repeated fields, and packed fields cannot have defaults.

## Wire rules

The encoder emits fields in field-number order. Each field starts with an
unsigned LEB128 key `(field_number << 3) | wire_type`. Integers retain their
existing varint representation. `fixed32` and `float32` use wire type 5 and
four big-endian bytes; `fixed64` and `float64` use wire type 1 and eight
big-endian bytes. Adding float support does not alter any existing scalar or
repeated wire bytes.

A packed declaration is one optional field occurrence, represented in C by a
presence flag and an inline array:

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

## Generated typed bindings

Compilation produces four deterministic files. `<module>.h/.c` contain only
the payload data model and codec and continue to depend solely on
`wirelink/codec.h`. `<module>_bindings.h/.c` form a separate, optional
translation unit which depends on the public `wirelink/wirelink.h` API. A
codec-only firmware therefore does not pull send, dispatch, or Wirelink core
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

Every message also has module-prefixed `send_unreliable` and `send_reliable`
wrappers. They encode into a caller-supplied `<module>_encode_scratch_t`, then
call the matching core API with the permanent message ID. The returned struct
preserves the codec status, raw core result, encoded length, and reliable
handle. `core_called` is zero when scratch encoding failed, in which case
`core_result` must be ignored; it is one once the core was invoked.

`<module>_<message>_send_direct()` is the native-packet fast path. It takes an
explicit `wl_delivery_t`, claims the final core TX payload span, encodes into
that span, and commits it without the scratch-to-core copy. COBS stream users
keep using the scratch wrappers. A claim error such as `WL_ERR_NOT_SUPPORTED`
is returned unchanged in `core_result`. Codec failure always aborts the claim;
`abort_result` exposes that cleanup result, and a failed commit also attempts
an abort in case the core still owns the claim. For ordinary scratch sends,
`abort_result` remains `WL_OK`.

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
type, and cardinality changes or removal without a reservation. A packed
field's element type and fixed count are both part of its wire identity;
changing either is incompatible. Existing reservations must remain reserved.

The library API is `parse_schema()`, `analyze_schema()`,
`check_compatibility()`, and `generate_c()`.

## CLI usage

```sh
# Validate one schema.
cargo run -- validate path/to/schema.wl

# Validate compatibility and generate codec plus typed-binding C files.
cargo run -- compile path/to/schema.wl \
  --previous path/to/previous.wl \
  --out-dir generated

# Resolve an application policy sidecar and add the generated runtime files.
cargo run -- compile path/to/schema.wl \
  --profile path/to/device.bind.wl \
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

Use `--profile path/to/device.bind.wl` with `validate` or `compile`. Profiled
compilation adds `<module>_runtime.h/.c` while leaving the four existing codec
and binding artifacts byte-for-byte unchanged. The runtime header embeds the
separate schema/profile diagnostic identities. Its generated dispatcher
decodes a retained message directly into a `wl_latest_t` or `wl_fifo_t` write
claim, publishes only after successful decode, aborts every failed claim, and
releases each valid RX event exactly once. The same wire schema can therefore
use different host and device profiles without changing message IDs or
payload bytes.

`<module>_runtime_dispatch_event()` is the terminal owner of every RX event
passed with non-null `ctx` and `event`. Every RX outcome—including an unknown
ID, null runtime, missing route or scratch, delivery mismatch, decode/storage/
RPC/application error, and replay-send failure—calls `wl_event_release()`
exactly once. It is not a chainable try-dispatch: do not release the event
yourself or pass it to the ordinary `<module>_dispatch_event()` afterward.
Null `ctx` or `event` cannot dispose of an RX lease; non-RX events are never
released. Feeding TX terminal events to the runtime advances a matching RPC
client slot, but does not reclaim the core transaction; the caller must still
call `wl_tx_take()` when required by the Wirelink transaction lifecycle.

`LATEST` and `FIFO` retain decoded values after the RX callback. WLC rejects a
retained route whose message contains `bytes`, `string`, or `repeated` storage,
including through nested messages. RPC operation IDs must map to optional
`uint32` fields. Its response status must map to an optional `int32` or enum;
an enum status domain must declare numeric value zero for success. Request and
response delivery are explicit and may be `reliable` or `unreliable`.

`delivery = reliable` selects and validates Wirelink reliable DATA only. The
peer ACK is scheduled after admission to RX event storage and before typed
decode or application retention. FIFO full, LATEST coalescing or claim failure,
missing storage, codec/handler/RPC failure, and replay-send failure therefore
do not NACK the packet or restart link ARQ. Use an RPC response/status,
capacity policy, and application deadline for peer-visible completion.

For each RPC service the runtime header emits typed scratch/direct client-start
functions, a request callback route, and typed server complete/reject/retry
functions. Client start allocates an operation ID and writes it into the mutable
request before encoding. The returned result always retains that ID; a failed
send leaves a terminal client slot that the application can inspect and
release. Response dispatch validates the mapped ID and status, copies the raw
payload into `wl_rpc_client_t` before releasing the RX event, and leaves typed
response scratch under caller ownership.

The caller provides request, response, and canonical-encode scratch. Borrowed
`bytes` and `string` fields in request/response scratch remain valid only until
the callback or dispatcher returns. Canonical scratch must be writable and
large enough for the complete decoded request; re-encoding drops unknown
fields and makes field order irrelevant to duplicate classification. Scratch
client sends may reuse their encode buffer after return. Direct client start
encodes into a core claim and is therefore available only when the selected
Wirelink envelope supports direct TX. Both start forms mutate the request's
operation ID. If encoding or sending then fails, the allocated client slot is
terminal but still must be inspected/released with `wl_rpc_client_release()`.

Response dispatch copies the original encoded bytes into the client's fixed
response storage before releasing RX. That copy remains valid until client
release; decode it again for a durable typed view rather than retaining
borrowed pointers from response scratch. Server complete/reject mutate the
response operation ID and status, then cease borrowing the response and encode
scratch after the call returns.

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

All generated `now_ms` arguments, `wl_poll()`, RPC poll functions, and deadline
hints must use one monotonic millisecond clock and epoch. Generated dispatch
passes time into server duplicate tracking but does not advance client/server
expiry; the application must call `wl_rpc_client_poll()` and
`wl_rpc_server_poll()` explicitly. Generated clients do not automatically
repeat an end-to-end RPC after timeout: link ARQ, the client deadline, and the
server replay cache are separate mechanisms.

`NEW` calls the request handler; zero accepts and leaves the operation pending,
while nonzero abandons it without manufacturing a response.
`PENDING_DUPLICATE` suppresses another execution. `REPLAY` sends the exact
cached response bytes, and `CONFLICT` reports reuse of an operation ID for a
different canonical request. Complete/reject cache before sending, so a core
send failure leaves a replayable response; call the generated cached-retry
helper with the returned `server_response`. Its `response_data` is borrowed
only until the next server mutation, poll, or expiry, so copy it before a
deferred retry. Cache TTL/eviction, process restart, or explicit expiry ends
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
have different exact identities; record the WLC version separately when exact
generated-artifact provenance matters.

## Dependency policy

The compiler deliberately keeps one dependency per concern:

| Crate | Role |
| --- | --- |
| `miette` | Source-aware, terminal-friendly diagnostics. |
| `thiserror` | Typed library errors. |
| `clap` | Declarative `compile` and `validate` CLI. |
| `heck` | Stable generated C symbol conversion. |
| `insta` | Reviewed generator golden snapshots. |
| `proptest` | Parser robustness and scalar-boundary properties. |
| `assert_cmd`, `tempfile` | Isolated CLI and generated-C tests. |

The hand-written parser keeps the grammar explicit. Generated codec C is
emitted with a small self-contained runtime and depends only on
`wirelink/codec.h` for status and borrowed byte/string types. Typed bindings
remain separately linkable and use only Wirelink public headers.
