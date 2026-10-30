# Static endpoint kernel

This is the default generated execution path in WLC 0.13.0. `compile-kernel`
is an alias of `compile --profile`; there is no prototype/legacy engine switch.

```sh
wlc compile device.wl --profile device.bind.wl --out-dir generated
```

```text
profile version 2;
endpoint { envelope = native_packet; }
rpc Read { request = ReadRequest; response = ReadResponse; }
send Sample { delivery = unreliable; }
direct Sample { delivery = unreliable; }
```

The profile selects native_packet/NONE, cobs_stream/CRC16 or
bus_length16/NONE. Both RPC roles remain enabled. Ordinary routes use public
Schema-v1 or explicit positional-v1 codecs; managed tagged RPC keeps the prior
metadata/body wire. Removed delivery/guarantee/mapped/policy declarations fail
before output. Positional RPC requires explicit migration and is never downgraded.

`<module>_kernel.h/.c` connects typed routes to the one `wl_kernel_t` in the
Wirelink C core. `_kernel_codecs.h` is generator-private; `_kernel_deployment.json`
reports actual bounds/roles/storage, and `_kernel_manifest.json` records inputs
and artifact digests. Stream deployments emit only their selected connector.

The application owns the port, two RX units, generated endpoint and clock source.
Initialize `wl_packet_port_t` from C with a nonzero fresh instance identity and
real sink/notify callbacks, then call `<module>_kernel_init`. The owner calls
`wl_kernel_step(core, now, budget)`, queries `wl_kernel_ready` and merges that
with physical I/O readiness using a latched event. There is no hidden clock.

There are four call slots, four reply slots and one ordinary TX snapshot. A
successful typed `_start` encodes the request before return. A failure creates
no call or later callback. Pass `now` and a relative timeout in 1..INT32_MAX;
time waiting for physical submission is included. Physical TX leases remain
separate from logical call completion and require consumed/drained acknowledgments.

Callbacks run on the owner and borrow response views until return. Copy with
`<response>_value_from_view` when retaining them. Cancel is local; timeout/cancel
cannot undo remote execution. Each admitted call has one terminal outcome;
its generation handle then becomes invalid. Callbacks may submit/cancel, but
must not recurse into step/close. Close, stop/join real I/O, and step until
`wl_kernel_quiescent` before freeing or reinitializing storage.

Handlers are synchronous short business functions and fill an owned response.
They return int32 status (zero success, nonzero business rejection). There are
no deferred reply tokens, replay cache, ARQ or service scheduler. Applications
handle idempotency/retry and use job IDs/status messages for long operations.

See the [full API](../../docs/static-endpoint-kernel-api.md),
[plan](../../docs/static-endpoint-kernel-refactor-plan-20261030-cn.md), and
[migration record](../../docs/static-endpoint-kernel-k5-migration-20261031-cn.md).
