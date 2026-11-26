# 组装单文件声明的类型化节点

[English](static-endpoint.md) · [代码生成](codegen-cn.md)

在同一 `.wl` 文件中声明消息、RPC 与 `node`，生成单个 `<stem>.h/.c` 和 `<stem>_manifest.json`：

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
  rx_slots = 4;           // 2（默认）、4 或 8
  rx_unit_align = 1;      // RX 槽步长（字节）：1（默认）、64 或 512
  timeout_ms = 500;       // 1..2147483647
  serve Read;
  send Sample;
}
```

`compile` 和 `plan` 都可通过 `--framing`、`--rx-slots` 和 `--rx-unit-align` 覆盖所选节点的对应设置。生成的 RX 槽按 `rx_wire_max + 1` 预留哨兵字节后，向上取整到 `rx_unit_align` 的倍数。缓冲区首地址对齐由单独的宏 `WL_NODE_BUFFER_ALIGN` 控制，默认为 4 字节。

使用 生成契约为 1 的 WLC 与 Wirelink。RPC 固定使用 Schema-v1；普通消息可使用 `codec = positional_v1` 或 `@codec(positional_v1)`。节点路由要求有界类型，存储严格按所选 `call` / `serve` / `send` 角色裁剪。

当消息含有 `bytes<N>` 且 Core 支持直接解码时，生成的 Node 将数据直接写入传给回调的局部对象；嵌套字段、RPC 请求和响应同样适用。畸形报文不会让应用收到半解码的值。公开消息解码函数的失败语义不变。

## 运行时接线

通过 `<node>_bind(&node, transport)` 绑定驱动。驱动可提供满足 DMA 或 USB Bulk 对齐要求的 RX 槽。同步整帧发送使用 `<node>_bind_sync(&node, tx_write)`，纯接收节点省略回调。持久链路通过相应的 `_instance` 入口传入非零实例号。字节流使用 `wl_node_rx_feed(&node.node, ...)` 并保留未消费尾部，原生包使用 `wl_node_rx_packet` 或驱动侧 claim/commit。动态处理器另传回调表与用户指针。

## 接纳、推进与关闭

核心固定支持 **4 个出站调用槽、3 个回复槽、1 个普通消息快照槽**，并由单一 owner 串行调用 `bind`、`send`、`call`、`cancel`、`step`、`close` 与 `quiescent`。

- **简化调用与 `_ex`**：`<node>_call_<rpc>(self, &req, done)` 回调首参为节点指针，使用 `timeout_ms`，且从**下一次 `step(now)`** 开始计时；`<node>_call_<rpc>_ex(self, &req, now, timeout, done, user, out)` 从显式 `now` 计时，可传入上下文与可选取消句柄。
- **handler 与回调生命周期**：服务端 handler 填写响应后返回 `0`（成功）或非零 `int32_t` 业务拒绝码；完成回调借用响应视图至返回前，需保留时直接按值复制有界结构体。`cancel` / `timeout` 不撤销远端执行，应用自行处理重试与幂等。
- **关闭顺序**：同步模式先停止外部 RX 生产者，再调用 `wl_node_close(&node.node, now_ms)`。收到 `WL_ERR_WOULD_BLOCK` 或 `WL_ERR_BUSY` 时重试，遇到驱动错误时处理错误。成功后即可复用存储。详见[传输层移植指南](https://github.com/starwey604/wirelink/blob/main/docs/porting-adapters.md)。
