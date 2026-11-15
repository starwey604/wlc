# WLC (Wirelink 编译器)

[English Placeholder](README-en.md) · [Wirelink 仓库](../README.md) · [Wire v2 与编码规范](../docs/wire-format.md)

**WLC**（Wirelink Compiler）校验 `.wl` Schema 与 `.bind.wl` 绑定配置，生成**纯 C11 零动态分配编解码器**、直连共享微内核的**静态类型化端点**，以及跨平台 **C++20 / Python 3.10+ 上位机 UDP SDK**。

> 版本对齐基线：WLC **0.13.0** · Codegen 契约 **0.13** · SDK Binding API **3** · 匹配 **Wirelink 0.13.0**（Profile **v2**，Wire **v2**）。

---

## 一、快速构建与常用命令

在 `wlc/` 目录构建本地发布版编译器并验证契约版本：

```sh
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
wlc --version           # wlc 0.13.0
wlc codegen-contract    # 0.13
```

### 核心子命令一览

| 命令 | 输入 | 输出产物与用途 |
| --- | --- | --- |
| `wlc compile <app.wl> --out-dir <dir>` | `.wl` | `<mod>.h` / `<mod>.c` / `<mod>_manifest.json`，生成单结构体 `<msg>_t` API 与基于共享 `wirelink/codec.h` 的 TLV/定长小端编解码器 |
| `wlc compile <app.wl> --profile <app.bind.wl> --out-dir <dir>` | `.wl` + `.bind.wl` | `<mod>.h/.c` + `<mod>_kernel.h/.c`（可选 `<mod>_kernel_transport.c`）与部署 manifest；提供免间接跳转的强类型发送/RPC/Step API |
| `wlc sdk <app.wl> --profile <host.bind.wl> --name <pkg> --out-dir <dir>` | `.wl` + `.bind.wl` | 可安装的 C++20 客户端、Python 同步/异步（`Client` / `AsyncClient`）wheel/sdist 源码包与 CMake 导出配置 |
| `wlc validate <app.wl> [--previous <old.wl>] [--profile <p>]` | `.wl` | Schema 语法/语义校验、前后向兼容检查与路由绑定制约校验 |
| `wlc plan <app.wl> --profile <p>` | `.wl` + `.bind.wl` | 打印静态 Codec 选型、各通道帧长上界与路由方向裁剪清单 JSON |
| `wlc identity <app.wl> [--profile <p>]` | `.wl` | 输出确定性 Schema / Profile 哈希指纹 |

> [!TIP]
> 每次切换传输绑定或移除旧部署产物时，请使用全新输出目录；`wlc` 会主动拒绝把新版单结构体代码混合写入包含旧版 `_runtime.c` / `_endpoint.h` 的目录。

---

## 二、典型 `.wl` 与 `.bind.wl` 示例

<!-- [docs:calculator_schema] -->
```wl
message AddRequest @id(20) {
  required int32 left @id(1);
  required int32 right @id(2);
}

message AddResponse @id(21) {
  required int32 sum @id(1);
}
```
<!-- [/docs:calculator_schema] -->

<!-- [docs:calculator_bind] -->
```wl
profile version 2;
endpoint { envelope=native_packet; }

rpc Add {
  request = AddRequest;
  response = AddResponse;
}
```
<!-- [/docs:calculator_bind] -->

可选传输包封（`endpoint.envelope`）：
- `native_packet`（NONE）：直接传输 4 字节小端规范帧（用于 USB Bulk / UDP / 内存回环）。
- `cobs_stream`（CRC16）：自动生成 `COBS + CRC-16/CCITT-FALSE + 0x00` 流分帧胶水（用于 UART/串口）。
- `bus_length16`（NONE）：2 字节小端长度前缀分帧（用于无丢字硬件总线）。

---

## 三、WP-4 生成架构特点

- **单一结构体 `<msg>_t` API**：彻底移除 View / Owned Value 双套类型转换胶水。
- **共享运行时 + 窄字段特化**：小端定长标量直写 + 共享 `src/codec.c` 表驱动描述符引擎（仅被路由实际引用的消息方向才会生成对应描述符与代码）。
- **扁平化端点直调**：`<mod>_kernel_send_*` 与 `<mod>_kernel_*_start` 直接执行 claim -> encode -> 4B inline seal -> commit，编码路径零函数指针间接调用。

---

## 四、编译器开发与回归验证

修改 `wlc/` 后（代码生成核心位于 `src/codegen/`），请运行全部校验套件：

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

更多 Task 细节见 [docs/codegen-cn.md](docs/codegen-cn.md)、[docs/static-kernel-cn.md](docs/static-kernel-cn.md)、[docs/sdk-cn.md](docs/sdk-cn.md) 与 [Wirelink 仓库开发指南](../docs/development.md)。
