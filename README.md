# WLC 编译器手册：从一份 `.wl` 生成 C11 节点与上位机 SDK

[返回仓库首页](../README.md) · [工程文档总览](../docs/README.md) · [Wire v2 协议规范](../docs/wire-format.md) · [驱动移植指南](../docs/porting-adapters.md)

**WLC**（Wirelink Compiler）负责校验 `.wl` 文件中的消息、RPC 和节点角色，生成**零堆分配（0 `malloc`）**的纯 C11 编解码器与强类型节点，并可一键生成 **C++20 / Python 3.10+ UDP 客户端 SDK**。

---

## 一、 构建 WLC 与命令行速查

WLC 和 Wirelink 当前的开发版本为 `0.0.99`，生成 C 的兼容性由独立的生成契约 `1` 决定。在 Wirelink 仓库根目录构建编译器并生成代码。

```sh
cargo build --release --locked --manifest-path wlc/Cargo.toml
export PATH="$PWD/wlc/target/release:$PATH"

# 1. 为下位机 MCU 生成服务端角色（不引入任何客户端符号与缓冲区）
wlc compile examples/01_rpc/calculator.wl --node server --out-dir build/server

# 2. 为上位机生成 Python / C++20 客户端 SDK
wlc sdk examples/01_rpc/calculator.wl --node client --name calculator --out-dir build/calculator-sdk

# 3. 同时生成所有节点角色（用于同进程双端回环测试）
wlc compile examples/01_rpc/calculator.wl --all-nodes --out-dir build/loopback
```

### 节点选择规则与输出产物

`wlc compile` 每次生成一对 **`<stem>.h`**、**`<stem>.c`** 和一份 **`<stem>_manifest.json`**（文件名取自输入的 `.wl` 主干名，生成的 C 结构体与函数前缀取自 `node` 名）：
- **0 个 `node`**：生成纯编解码器（Codec-only）。
- **1 个 `node`**：自动选中该节点，无需传 `--node`。
- **多个 `node`**：必须显式传入 `--node <name>` 或 `--all-nodes`（两者互斥）。

| CLI 子命令 | 作用与常见用法 |
| :--- | :--- |
| `wlc compile app.wl --out-dir <dir> [--node <name> \| --all-nodes] [--previous old.wl]` | 生成一对 `.h/.c` 与 manifest（可选附加与旧版协议的兼容性检查） |
| `wlc sdk app.wl --out-dir <dir> [--node <client>] [--name <pkg>]` | 生成可构建安装的 C++20 与 Python 3.10+ UDP 客户端 SDK 工程 |
| `wlc validate app.wl [--previous old.wl]` | 校验语法、ID 唯一性、节点路由及可选的向后兼容演进规则 |
| `wlc plan app.wl [--node <name> \| --all-nodes]` | 打印字段编码规划、节点各方向路由及编译期计算出的帧长上限 |
| `wlc dependencies app.wl` | 输出包含所有传递 `import` 在内的 `.wl` 依赖文件路径列表 |
| `wlc codegen-contract` | 输出生成 C 所需的契约整数 |
| `wlc identity app.wl` | 输出确定性的 Schema 身份摘要 |

---

## 二、 `.wl` 语言：消息、RPC 与节点角色

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

- **模块化复用**：顶部的 `version 1;` 可省略；通过 `import "shared.wl";` 可跨文件复用消息、枚举和 RPC 定义。
- **必选与可选字段**：普通字段默认必选，直接对应 C 结构体中的同名成员（显式写 `required` 完全等价；线上若缺失必选字段，解码器会直接报错拒绝）。对于可选字段，在类型前加上 **`optional`**（或定长数组加上 **`optional packed`**），生成的结构体中会附带 `bool has_<field>` 标志位：

```wl
message Sample @id(10) {
  uint32 sequence @id(1);                  // 必选字段：uint32_t sequence;
  optional int32 temperature @id(2);      // 可选字段：bool has_temperature; int32_t temperature;
  packed float32 axes[3] @id(3);          // 必选定长数组：float axes[3];
  optional packed fixed32 flags[2] @id(4); // 可选定长数组：bool has_flags; uint32_t flags[2];
}
node sensor {
  framing = cobs_crc16;
  codec = positional_v1;
  send Sample;
}
```

- **有界内联变长字段**：节点路由消息中的变长字符串与字节块需声明容量上限（如 `string<31>`、`bytes<64>`），数据作为定长数组直接内嵌在生成的 C 结构体中，并提供 `<msg>_set_<field>()` 容量检查 setter。接收回调中的消息指针仅在回调期间有效；因为有界字段全部内联，只需 `*saved = *msg` 按值赋值结构体即可跨回调完整保存。

---

## 三、 节点的 5 项配置与默认值

| 节点配置项 | 默认值 | 可选值与适用场景 |
| :--- | :--- | :--- |
| **`framing`** | `native` | 默认面向带包边界链路（如 UDP）；UART 串口选 `cobs_crc16`；受控可靠字节流选 `length16` |
| **`codec`** | `schema_v1` | 默认 TLV 编码（支持增删 optional 字段平滑演进）；普通消息可选零 Tag 开销的 `positional_v1`（RPC 固定使用 `schema_v1`） |
| **`io`** | `embedded` | 默认内嵌端口、双 RX 槽与流转换器；异步零拷贝 DMA 或对接现有外部适配器时选 `external` |
| **`handlers`** | `static` | 默认直接链接同名静态 C 函数（首参为 `<node>_node_t *`）；需运行时动态传回调表时选 `dynamic`（首参为 `void *user`） |
| **`timeout_ms`** | `500` | 简化调用 `<node>_call_<rpc>()` 的默认超时毫秒数（范围 `1 .. INT32_MAX`） |

在路由声明上还可以直接绑定自定义静态函数名：
- `recv Message = custom_on_msg;` / `serve Rpc = custom_on_rpc;`：自定义接收或服务钩子的 C 函数名。
- `call Rpc = custom_on_done;`：为该 RPC 绑定固定完成回调，绑定后简化调用只需传 `(self, &request)` 两个参数。

---

## 四、 生成的 C11 节点 API 速查

假设节点名为 `n`（即 `node n { ... }`）：

- **初始化**：
  - `n_init(&node)`（`embedded` 模式）：初始化内嵌端口与缓冲，自动分配进程内唯一的非零实例号。若物理链路在 MCU 重启后可能残留旧包，可使用 `n_init_with_instance(&node, id)` 传入应用管理的非零会话 ID。
  - `n_init(&node, &port)`（`external` 模式）：挂载由外部驱动持有的 `wl_packet_port_t`。
- **发消息与调 RPC**：
  - `n_send_<msg>(&node, &value)`：返回 `WL_OK` 时已完成输入编码快照；若唯一的普通消息槽正忙，返回 `WL_ERR_WOULD_BLOCK`。
  - `n_call_<rpc>(&node, &request, done)`：简化调用，回调首参为 `n_node_t *`；从**下一次 `n_step(&node, now)`** 开始计算默认超时。
  - `n_call_<rpc>_ex(&node, &request, now, timeout, done, user, &out)`：显式时间调用，回调首参为 `void *user`；从传入的 `now` 立即计时（`timeout = 0` 使用默认超时），可通过 `out` 句柄调用 `n_cancel(&node, handle)`。
- **步进与安全关闭**：
  - `n_step(&node, now_ms)`：由单一 Owner 传入毫秒时间戳，驱动发送、接收解码、超时与完成回调。
  - `n_close(&node)` / `n_quiescent(&node)`：先停止 RX 生产者，再调用 `close()` 并继续步进，直到 `quiescent()` 返回真（所有回调与物理租约均已排空）。
- **物理层收发入口（`embedded` 模式）**：
  - 应用实现 `n_tx_write(&node, data, len)`（整帧同步接受或返回 `WL_ERR_WOULD_BLOCK`），外设从忙恢复时调用 `n_tx_ready(&node)`；
  - 接收端根据 `framing` 调用 `n_rx_packet(&node, data, len)`（`native`）或 `n_rx_feed(&node, data, len, &consumed)`（流模式，务必按 `consumed` 移走已消费前缀、保留未消费尾部）。详见 [驱动移植指南](../docs/porting-adapters.md)。
- **编译期容量宏**：`<N>_{REQUEST,REPLY,MESSAGE,RX,TX}_FRAME_MAX`（规范帧上限）与 `<N>_{RX,TX}_WIRE_MAX`（计入流分帧后的物理帧上限）。未声明的发送方向不分配缓冲。

---

## 五、 CMake 集成与上位机 SDK 生成

在 CMake 工程中集成 `.wl` 生成：

```cmake
wirelink_wlc_generate(
  TARGET calculator_node
  SCHEMA "${CMAKE_CURRENT_SOURCE_DIR}/calculator.wl"
  NODE server)
target_link_libraries(firmware PRIVATE calculator_node)
```

*(同进程双端回环将 `NODE server` 替换为 `ALL_NODES`；配置时通过 `-DWIRELINK_WLC_EXECUTABLE=...` 指定本地 `wlc` 二进制。)*

生成上位机 C++20 / Python 3.10+ UDP 客户端 SDK：

```sh
wlc sdk examples/01_rpc/calculator.wl --node client \
  --name calculator --out-dir build/calculator-sdk
```

---

## 六、 编译器自身开发与测试

修改 `wlc/`（代码生成核心位于 `wlc/src/codegen/`）后，请运行完整 Rust 测试套件：

```sh
cargo fmt --manifest-path wlc/Cargo.toml --check
cargo clippy --manifest-path wlc/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path wlc/Cargo.toml --locked
```
