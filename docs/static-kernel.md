# Assemble a typed node from a single `.wl` schema

[中文](static-kernel-cn.md) · [Code generation](codegen.md)

Declare messages, RPCs, and `node` endpoints in one `.wl` file to emit a single `<stem>.h/.c` pair and `<stem>_manifest.json`:

```sh
wlc compile device.wl --node device --out-dir generated
```

```wl
message ReadRequest @id(1) { uint32 address @id(1); }
message ReadResponse @id(2) { uint32 value @id(1); }
message Sample @id(3) { uint32 value @id(1); }

rpc Read(ReadRequest) -> ReadResponse;

node device {
  framing = native;       // native | cobs_crc16 | length16
  io = embedded;          // embedded | external
  handlers = static;      // static | dynamic
  timeout_ms = 500;       // 1..2147483647
  serve Read;
  send Sample;
}
```

Use WLC 0.14.0 / codegen contract 0.14 with matching Wirelink 0.14.0. RPC routes always use Schema-v1; ordinary messages may select `positional_v1` per node or per route. Node routes require bounded types, and TX storage is pruned by declared `call` / `serve` / `send` roles.

## Embedded defaults vs. external drivers

- **Embedded (default)**: `<node>_node_t` embeds `wl_packet_port_t`, two RX slots, and stream state when using `cobs_crc16` / `length16`. Call `<node>_init(&node)` (or `<node>_init_with_instance(&node, id)`), implement whole-frame `<node>_tx_write(self, data, length)`, and call `<node>_tx_ready(&node)` when a busy sink becomes writable. Deliver RX via `<node>_rx_packet(...)` (native) or `<node>_rx_feed(..., &consumed)` (streams, preserving any unconsumed tail).
- **External (`io = external`)**: The application supplies an initialized `wl_packet_port_t` via `<node>_init(&node, &port)` for async DMA, zero-copy, or UDP/UART adapters, preserving `SENT / STARTED / BUSY / FAILED` physical lease ownership.
- **Static (default) vs. dynamic**: Static handlers link directly as `<node>_on_<msg/rpc>(self, ...)` with context in `self->user`; `handlers = dynamic` generates `<node>_handlers_t` taking `void *user`.

## Admission, stepping, and shutdown

The underlying kernel provides **four outbound call slots, three reply slots, and one ordinary message snapshot**, driven by a single owner executing `init`, `send`, `call`, `cancel`, `step`, `close`, and `quiescent`.

- **Simple call vs. `_ex`**: `<node>_call_<rpc>(self, &req, done)` passes the node pointer to `done` and arms the node's `timeout_ms` on the **next owner `step(now)`**. `<node>_call_<rpc>_ex(self, &req, now, timeout, done, user, out)` times from explicit `now` with custom `timeout`, `user` pointer, and optional cancellation handle.
- **Handlers and callbacks**: Service handlers populate the response and return `0` or a nonzero `int32_t` business rejection status. Terminal callbacks borrow response views until return; bounded response structs can be copied by value. Local cancellation or timeout cannot roll back remote execution.
- **Close sequence**: Stop RX producers first, call `<node>_close(&node)` (and stop/drain external hardware leases), then step until `<node>_quiescent(&node)` is nonzero before reusing or freeing storage. See [porting adapters](https://github.com/starwey604/wirelink/blob/main/docs/porting-adapters.md).
