# Assemble a typed node from a single `.wl` schema

[中文](static-endpoint-cn.md) · [Code generation](codegen.md)

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
  handlers = static;      // static | dynamic
  rx_slots = 4;           // 2 (default), 4, or 8
  timeout_ms = 500;       // 1..2147483647
  serve Read;
  send Sample;
}
```

Use WLC and Wirelink with codegen contract 1. RPC routes always use Schema-v1; ordinary messages may select `positional_v1` per node or per route. Node routes require bounded types, and TX storage is pruned by declared `call` / `serve` / `send` roles.

For messages containing `bytes<N>`, the generated Node decodes into the callback’s local value when the core supports it. This also applies to nested fields and RPC requests and responses. A malformed payload never reaches the application as a partially decoded value. The public message decoder keeps its existing failure behavior.

## Runtime I/O binding

Use `<node>_bind(&node, transport)` to attach a driver. The driver can provide aligned RX storage for DMA or USB Bulk. Use `<node>_bind_sync(&node, tx_write)` for a synchronous whole-frame sink and omit the callback for receive-only nodes. Supply a fresh nonzero identity on persistent links using the corresponding `_instance` entry point. Feed stream bytes through `wl_node_rx_feed(&node.node, ...)` and retain the unconsumed suffix. Native packets use `wl_node_rx_packet` or driver-side claim/commit. Dynamic handlers add a handler table and user pointer.

## Admission, stepping, and shutdown

The endpoint provides **four outbound call slots, three reply slots, and one ordinary message snapshot**, driven by a single owner executing `bind`, `send`, `call`, `cancel`, `step`, `close`, and `quiescent`.

- **Simple call vs. `_ex`**: `<node>_call_<rpc>(self, &req, done)` passes the node pointer to `done` and arms the node's `timeout_ms` on the **next owner `step(now)`**. `<node>_call_<rpc>_ex(self, &req, now, timeout, done, user, out)` times from explicit `now` with custom `timeout`, `user` pointer, and optional cancellation handle.
- **Handlers and callbacks**: Service handlers populate the response and return `0` or a nonzero `int32_t` business rejection status. Terminal callbacks borrow response views until return; bounded response structs can be copied by value. Local cancellation or timeout cannot roll back remote execution.
- **Close sequence**: Stop any externally driven synchronous RX, then call `wl_node_close(&node.node, now_ms)`. Retry on `WL_ERR_WOULD_BLOCK` or `WL_ERR_BUSY`, handling driver errors before releasing storage. See [porting adapters](https://github.com/starwey604/wirelink/blob/main/docs/porting-adapters.md).
