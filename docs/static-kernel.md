# Static endpoint kernel prototype

`compile-kernel` generates the isolated K1 endpoint for Wirelink's static kernel
contract 1. It requires a matching core with `experimental/static_kernel/` and
an explicitly migrated profile version 2. Existing `compile`, `compile-runtime`
and SDK generation continue to accept version 1 during prototype evaluation.
This is temporary isolation, not a permanent legacy/fast product switch.

```sh
wlc compile-kernel usb.wl --profile usb.bind.wl --out-dir generated
```

```text
profile version 2;
endpoint { envelope = native_packet; rpc_role = both; }
rpc Setpoint { request = SetpointRequest; response = SetpointResponse; }
send Sample { delivery = unreliable; }
direct Sample { delivery = unreliable; }
```

Version 2 defaults RPC delivery to unreliable. Explicit reliable, guarantee
(including replay/correlate), mapped field declarations, LATEST/FIFO and unknown
deferred declarations fail. Version 1 keeps its old defaults and is rejected by
this command. K1 supports native packets, NONE integrity and both RPC roles;
other deployments fail explicitly. Selected messages need finite codec bounds
and must fit the native payload limit. RPC uses the existing managed tagged
business codec and metadata; ordinary messages use the unchanged Schema-v1
codec. There is no wire negotiation or automatic downgrade.

Output includes ordinary codec/value headers and source, a typed kernel
header/source, a private inline RPC codec header, deployment JSON, and a
provenance manifest. It emits no bindings to the old link, RPC server/client or
pump. Codec generation shares the existing plans and emitters. The private
header is an implementation artifact, not a new application API.

The generated endpoint embeds a single call table through `wl_kernel_t`, four
request frame buffers, four reply buffers, one ordinary frame buffer, typed
handler pointers and one reply value union. Bounds come from each selected
codec, including managed metadata and the native prefix. RX buffers and the
packet port are supplied separately. Deployment JSON reports capacities and
all selected message IDs/codecs. The contract guard requires kernel contract 1;
profile version 2 is recorded independently of the business schema identity.

`<module>_kernel_<method>_start` snapshots a pointer-free request before return.
Its typed callback receives `wl_kernel_result_t` and a borrowed response view,
valid until callback return. Submission never invokes the callback inline.
Business rejection, local failure, timeout, cancel and close remain distinct.
Callback function pointers are erased by a C11 function-pointer conversion and
converted back to their original type before invocation; they are not stored
as object pointers or invoked through an incompatible signature.

Handlers receive a borrowed request and writable owned response, returning zero
or a nonzero business rejection status. No reply token or late completion API
is exposed to application handlers. Reply storage is reserved before invoking
the handler; BUSY retains the encoded reply and never repeats side effects.
Encoding failure releases the reservation and is returned by step and through
the optional diagnostic callback. It does not produce a success response.

All endpoint operations run on one owner. The owner passes a monotonic u32 tick
snapshot to step/start; timeouts must be 1..INT32_MAX ticks. Callbacks may submit
or cancel other calls; recursive step/close fails. The outer runtime supplies
latched notifications, waiting and an adapter stop acknowledgement. Core close
terminates waiting calls locally, then physical I/O must drain before storage
or instance identity can be reused. Reinitialization requires a new unique
nonzero instance, an idle owner and stopped I/O.

This command does not yet supply product USB adapters, a host executor, SDKs or
COBS connectors. Software fixtures do not establish K1 hardware integration or
K2 performance acceptance.
