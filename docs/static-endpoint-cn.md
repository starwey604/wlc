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
  io = embedded;          // embedded | external
  handlers = static;      // static | dynamic
  timeout_ms = 500;       // 1..2147483647
  serve Read;
  send Sample;
}
```

使用 生成契约为 1 的 WLC 与 Wirelink。RPC 固定使用 Schema-v1；普通消息可使用 `codec = positional_v1` 或 `@codec(positional_v1)`。节点路由要求有界类型，存储严格按所选 `call` / `serve` / `send` 角色裁剪。

## embedded 默认装配与 external 异步路径

- **embedded（默认）**：`<node>_node_t` 内嵌 `wl_packet_port_t`、双 RX 缓冲与流缓冲（若使用 `cobs_crc16` / `length16`）。调用 `<node>_init(&node)`（或 `<node>_init_with_instance(&node, id)`），实现整帧发送钩子 `<node>_tx_write(self, data, length)`；忙后恢复可写时调用 `<node>_tx_ready(&node)`。接收调用 `<node>_rx_packet(...)`（native）或 `<node>_rx_feed(..., &consumed)`（流模式，保留未消费尾部）。
- **external**：设置 `io = external` 时，由应用装配 `wl_packet_port_t`（异步 DMA、零拷贝、UDP/UART 适配器），通过 `<node>_init(&node, &port)` 传入。物理发送保持 `SENT / STARTED / BUSY / FAILED` 语义。
- **static（默认）vs dynamic**：静态模式下直接链接 `<node>_on_<msg/rpc>(self, ...)`，可从 `self->user` 读取业务上下文；`handlers = dynamic` 才生成 `<node>_handlers_t` 并以 `void *user` 作为回调首参。

## 接纳、推进与关闭

核心固定支持 **4 个出站调用槽、3 个回复槽、1 个普通消息快照槽**，并由单一 owner 串行调用 `init`、`send`、`call`、`cancel`、`step`、`close` 与 `quiescent`。

- **简化调用与 `_ex`**：`<node>_call_<rpc>(self, &req, done)` 回调首参为节点指针，使用 `timeout_ms`，且从**下一次 `step(now)`** 开始计时；`<node>_call_<rpc>_ex(self, &req, now, timeout, done, user, out)` 从显式 `now` 计时，可传入上下文与可选取消句柄。
- **handler 与回调生命周期**：服务端 handler 填写响应后返回 `0`（成功）或非零 `int32_t` 业务拒绝码；完成回调借用响应视图至返回前，需保留时直接按值复制有界结构体。`cancel` / `timeout` 不撤销远端执行，应用自行处理重试与幂等。
- **关闭顺序**：先停止 RX 生产者，再调用 `<node>_close(&node)`（external 同步停止驱动并归还 TX 租约），持续推进 `<node>_step(&node, now)` 直至 `<node>_quiescent(&node)` 返回非零后再释放或复用存储。完整移植契约见 [传输层移植指南](https://github.com/starwey604/wirelink/blob/main/docs/porting-adapters.md)。
