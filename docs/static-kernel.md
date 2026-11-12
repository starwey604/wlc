# Assemble a typed kernel endpoint

[中文](static-kernel-cn.md) · [Code generation](codegen.md)

Generate routes and bounded storage for your deployment:

```sh
wlc compile device.wl --profile device.bind.wl --out-dir generated
```

The schema defines messages. A profile version 2 selects routes and connector:

```text
profile version 2;
endpoint { envelope = native_packet; }
rpc Read { request = ReadRequest; response = ReadResponse; }
send Sample { delivery = unreliable; }
direct Sample { delivery = unreliable; }
```

Use WLC 0.13.0 / contract 0.13 and Wirelink 0.13.0. Choose `native_packet`/NONE,
`cobs_stream`/CRC16 or `bus_length16`/NONE; both RPC roles are enabled. Ordinary
messages use public Schema-v1 or explicit positional-v1. RPC uses managed tagged
bodies with instance and operation metadata. Kernel routes require bounded types;
repeated views belong to standalone codecs.

## Assemble port, routes and owner

`<module>_kernel.h/.c` connects typed routes to `wl_kernel_t`. Private RPC codecs
are in `_kernel_codecs.h`; `_kernel_deployment.json` reports bounds/roles/storage,
and `_kernel_manifest.json` records inputs and generated artifacts. A stream
profile emits only its selected connector.

The application owns the C-initialized packet port, two RX units, generated
endpoint and time source. Supply a nonzero fresh instance identity, sink and
notification callbacks; then call `<module>_kernel_init`. The same owner performs
admission, cancellation, handlers, terminal callbacks and `wl_kernel_step`.
Merge readiness and the earliest deadline returned by `wl_kernel_ready` with
adapter readiness and a latched wake before waiting. See the [owner-loop tutorial](https://github.com/starwey604/wirelink/blob/main/docs/tutorial-integration.md).

## Admit, complete and close

The kernel has four outbound calls, four reply snapshots and one ordinary TX
snapshot. Typed `_start` snapshots a request before success returns; failure
creates no call or later callback. Supply owner ticks for `now` and a relative
timeout in `1..INT32_MAX`; queueing counts toward timeout. Physical TX leases
outlive logical completion when necessary and still require source-consumed and
physical-drained/error notifications.

Handlers are short synchronous business functions: fill an owned response and
return zero for success or signed nonzero business status. Terminal callbacks
borrow response views until return; copy with `*_value_from_view` to retain them.
Each admitted call has one terminal outcome and then an invalid handle. Callbacks
may submit/cancel, but cannot recursively step/close. Local cancel/timeout cannot
undo remote execution. Applications own retry/idempotency and [long jobs](https://github.com/starwey604/wirelink/blob/main/docs/tutorial-jobs.md).

Close the kernel, stop/join real I/O, deliver final completions, and step until
`wl_kernel_quiescent` before freeing storage. The [kernel API](https://github.com/starwey604/wirelink/blob/main/docs/kernel-api.md)
defines error and lifetime details; [development checks](https://github.com/starwey604/wirelink/blob/main/docs/development.md)
show build and test commands.
