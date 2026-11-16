# WLC：从一个文件生成消息与节点

[Wirelink 首页](../README.md) · [Wire v2](../docs/wire-format.md) · [驱动接入](../docs/porting-adapters.md)

WLC 校验 `.wl` 中的消息、RPC 和节点角色，生成零堆分配 C11 编解码器与类型化节点，也可生成 C++20 / Python 3.10+ UDP 客户端 SDK。当前本地候选配对为 **WLC 0.14.0、Wirelink 0.14.0、Codegen 0.14、SDK Binding API 3**。

## 构建与选择节点

在仓库根目录执行：

```sh
cargo build --release --locked --manifest-path wlc/Cargo.toml
export PATH="$PWD/wlc/target/release:$PATH"
wlc --version          # wlc 0.14.0
wlc codegen-contract   # 0.14
wlc compile examples/01_rpc/calculator.wl --node server --out-dir build/server
wlc compile examples/01_rpc/calculator.wl --all-nodes --out-dir build/loopback
```

`compile` 只生成 `<stem>.h`、`<stem>.c`、`<stem>_manifest.json`。文件名来自输入 schema；函数与节点类型使用节点名。无节点时生成纯编解码器；只有一个节点时自动选择；有多个节点时必须指定 `--node` 或 `--all-nodes`，两者互斥。选择单节点可避免为其他角色实现静态钩子；`--all-nodes` 适合同进程双端回环。

| 命令 | 用途 |
| --- | --- |
| `wlc validate app.wl [--previous old.wl]` | 校验 schema、节点及可选的 schema 演进兼容性 |
| `wlc compile app.wl --out-dir generated [--node name \| --all-nodes] [--previous old.wl]` | 生成单对 C 文件和 manifest |
| `wlc plan app.wl [--node name \| --all-nodes]` | 查看编码方案、节点方向与帧长上界 |
| `wlc identity app.wl` | 输出确定性的 schema 身份信息 |
| `wlc dependencies app.wl` | 输出包含传递 import 的输入文件清单 |
| `wlc sdk app.wl --out-dir sdk [--node name] [--name package]` | 生成可安装的 C++20 / Python UDP SDK |

升级使用新输出目录，清理旧生成文件的构建引用，避免把新旧生成 ABI 混编。旧 `--profile`、`compile-kernel` 和 `compile-runtime` 已移除。

## 消息、RPC 与路由写在一起

<!-- [docs:calculator_schema] -->
```wl
message AddRequest @id(20) {
  int32 left @id(1);
  int32 right @id(2);
}
message AddResponse @id(21) { int32 sum @id(1); }
rpc Add(AddRequest) -> AddResponse;
node client { call Add; }
node server { serve Add; }
```
<!-- [/docs:calculator_schema] -->

`version 1;` 可省略，缺省为 1。`import "shared.wl";` 可复用消息、RPC 和节点定义；节点只启用自己声明的 `send`、`recv`、`call`、`serve` 路由。普通消息可在节点或路由上选择编码；RPC 固定使用 Schema-v1。

```wl
message Sample @id(10) {
  uint32 sequence @id(1);                  // 默认必选，无 has_sequence
  optional int32 temperature @id(2);      // 有 has_temperature
  packed float32 axes[3] @id(3);          // 必选定长数组，无 has_axes
  optional packed fixed32 flags[2] @id(4);
}
node sensor {
  framing = cobs_crc16;
  codec = positional_v1;
  send Sample;
}
```

`required` 是默认必选的显式同义词。仅 `optional` / `optional packed` 使用 `has_<field>`；必选字段在线上缺失仍会解码失败。`repeated` 使用长度与调用者提供的 backing，不使用单字段存在位。旧版裸 `packed` 若要保留可选语义，必须迁移为 `optional packed`。

有界 `string<N>` / `bytes<N>` 将数据存放在消息结构体中，用生成的 setter 检查长度。无界 string/bytes 是借用视图：解码后的数据寿命随输入缓冲；需要跨回调保存时复制数据。所有生成接收回调中的消息对象仅在回调期间有效。

## 默认配置与显式选择

| 节点项 | 默认 | 可选值与用途 |
| --- | --- | --- |
| `io` | `embedded` | `external` 使用应用提供的端口，适合异步 DMA、零拷贝和已有执行器 |
| `handlers` | `static` | `dynamic` 显式传入 handler 表与上下文 |
| `framing` | `native` | `cobs_crc16` 用于串口；`length16` 用于保留字节顺序的可靠流 |
| `codec` | `schema_v1` | `positional_v1` 仅普通消息，要求两端字段布局一致 |
| `timeout_ms` | `500` | 简化 RPC 的默认超时，范围 `1..INT32_MAX` |

静态 handler 名为 `<node>_on_<message/rpc>`，首参是 `<node>_node_t *`；可从 `self->user` 取业务状态。`recv Message = custom_handler;` 可命名接收钩子，`call Rpc = done;` 可绑定固定完成钩子，此时简化调用只需 `(self, request)`。

动态节点的 `<node>_handlers_t` 含 `on_<message/rpc>` 成员，回调首参改为 `void *user`。动态 `init` 在普通参数后追加 `const <node>_handlers_t *` 和 `void *user`。简单固定固件优先使用静态模式，确实需要运行时选择回调时再使用动态模式。

## 生成 API：初始化、调用、步进、关闭

以下 `n` 代表节点名：

- `n_init(&node)`：embedded 初始化端口与缓冲，自动取得非零实例号。`n_init_with_instance(&node, id)` 显式指定实例号；自动分配只保证当前进程内自动分配之间不冲突，持久链路的跨重启身份由应用管理。
- `n_init(&node, &port)`：external 初始化，端口及物理驱动由应用持有。
- `n_send_message(&node, &value)`：成功即复制编码后的消息，发送完成前再次提交可能背压。
- `n_call_rpc(&node, &request, done)`：回调首参为节点指针；从下一次 owner `step(now)` 开始计算默认超时。
- `n_call_rpc_ex(&node, &request, now, timeout, done, user, out)`：回调首参为 `void *user`；从显式 `now` 计时，`timeout = 0` 使用节点默认值。无需取消时 `out` 可为 `NULL`，否则用获得的句柄调用 `n_cancel()`。
- `n_step(&node, now)`：由单一 owner 提供毫秒时钟，驱动发送、接收、超时和完成回调。API 不读取系统时钟。
- `n_close(&node)` / `n_quiescent(&node)`：先停 RX 生产者，再关闭并继续步进，直到回调和物理租约排空。external 驱动还需完成其 stop/drain 协议。

embedded 提供 `n_tx_write()` 物理钩子、`n_tx_ready()` 恢复通知，以及 native 的 `n_rx_packet()` 或 stream 的 `n_rx_feed(..., &consumed)`。钩子只能整帧同步接受或整帧拒绝；流式背压必须保留未消费尾部。完整收发与关闭约束见 [移植指南](../docs/porting-adapters.md)。

缓冲上界宏为 `<N>_{REQUEST,REPLY,MESSAGE,RX,TX}_FRAME_MAX` 与 `<N>_{RX,TX}_WIRE_MAX`，分别表示规范帧和物理线上帧。没有相应发送角色时不分配对应存储；不要将所有方向都按最大报文统一预留。

## CMake 与 SDK

加载 Wirelink 包或源码项目后：

```cmake
wirelink_wlc_generate(
  TARGET calculator_node
  SCHEMA "${CMAKE_CURRENT_SOURCE_DIR}/calculator.wl"
  NODE server)
target_link_libraries(firmware PRIVATE calculator_node)
```

同进程双端用 `ALL_NODES` 替代 `NODE server`。helper 跟踪传递 import，自动链接 embedded 所需 stream 连接器；通过 `WIRELINK_WLC_EXECUTABLE` 指定本地匹配编译器。

```sh
wlc sdk examples/01_rpc/calculator.wl --node client \
  --name calculator --out-dir build/calculator-sdk
```

SDK 自动选择唯一带 `call` 的节点；有多个客户端角色时须 `--node`。没有显式客户端节点时可从顶层 RPC 合成客户端。SDK 将选中角色投影为 external/static 的 RPC 客户端，移除固定完成钩子，不继承 MCU 的 embedded 物理胶水。SDK 的超时预算仍包含请求排队时间。

## 编译器开发

```sh
cargo fmt --manifest-path wlc/Cargo.toml --check
cargo clippy --manifest-path wlc/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path wlc/Cargo.toml --locked
```

代码生成位于 `wlc/src/codegen/`。改变生成契约后须同时更新核心、夹具、所有消费者与 SDK；不能只让生成器自身测试通过。更多门禁见 [开发指南](../docs/development.md)。
