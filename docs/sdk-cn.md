# 构建与使用自持 SDK

[English](sdk.md) · [代码生成](codegen-cn.md)

从同一 `.wl` 文件生成可安装的 C++20 客户端和带类型提示的 Python 同步/异步包：

```sh
wlc sdk device.wl --node host --name device --package-version 0.0.99 --out-dir sdk
cmake -S sdk -B build/sdk -DCMAKE_PREFIX_PATH=/path/to/wirelink-install
cmake --build build/sdk --parallel
```

构建使用生成契约 1、SDK 源码 API 版本 3 的 WLC 与 Wirelink。SDK 包版本由作者指定。若有多个含 `call` 的节点须传 `--node`；无显式节点时可从顶层 `rpc` 合成客户端。生成器为目标节点生成静态处理器客户端，通过 `init_io` 将内置端口接到 UDP，并按调用绑定完成回调。

## 构建原生库或 Python wheel

安装 Wirelink 时启用 `WIRELINK_BUILD_CPP_BINDINGS=ON` 与 `WIRELINK_BUILD_PLATFORM=ON`，并提供 standalone Asio 头文件。Linux Python extension 静态依赖需要开启 `CMAKE_POSITION_INDEPENDENT_CODE=ON`。生成源码包内含已生成的 C 代码，编译无需 Rust/WLC。CMake 默认 `<NAME>_BUILD_PYTHON=OFF`，wheel 打包会自动启用；原生测试受 `BUILD_TESTING` 控制。具体包命令见生成目录 README；完整可运行 C 服务端与 Python 测试见 [Calculator 指南](https://github.com/starwey604/wirelink/blob/main/examples/06_bindings/GUIDE.md)。

CMake 将 `<name>_manifest.json` 中的生成契约与已安装的 Wirelink 包比对。`wlc-sdk-build.json` 以 `wirelink-sdk-build-v3` 格式记录端点契约。修改 WLC 内置模板后须重建编译器再重新生成。`--overwrite` 仅替换生成清单内的文件，应用测试与指南请放在生成清单之外。

## 共用原生 owner，保存结果

C 持有原子 port 存储、单对 `<name>.h/.c` 节点状态和类型化回调；C++ `Session` 排队执行 owner 命令并包装等待与 UDP I/O。同步和异步调用共用 **4 个原生 call slot**；`AsyncClient` 另限制最多 **8 个未交付操作**。host 命令队列有 32 项，每批最多处理 8 条。接纳成功前完成请求快照；接纳失败不创建调用或终态回调。

`Operation<T>` 可复制，丢弃它不取消调用；自持响应与错误可跨越 client 生命周期。回调在返回前复制借用视图。C++ timeout 单位为毫秒（`1..INT32_MAX`）；Python timeout 单位为秒，向上取整到毫秒（上限 `2147483.647`），两者均计入 owner 命令排队等待时间。切勿在 owner 回调内阻塞或销毁 Session。

close 使未完成调用以 `ClosedError` 结束；显式 operation cancel 产生 `CancelledError`；asyncio task cancel 抛出 `asyncio.CancelledError` 并请求原生取消；到期抛出 `RpcTimeoutError`；业务拒绝保留有符号 `status`。Python 异常公开 `kind`、`status`、`outcome`、`os_error`、`os_message`；C++ `Error` 包含 `kind`、`completion` 与 `system_error`。本地 cancel/timeout 无法撤回远端副作用；只含 RPC 的 SDK 需由业务另行定义状态查询 RPC 处理长任务。

## Python 执行离开 owner

Python 阻塞等待会释放 GIL。`AsyncClient` 使用独立有界 completion worker 与 asyncio future；原生 owner 永不执行 Python 回调。使用 `with Client.connect(...)` 或 `async with AsyncClient.connect(...)` 确保有序 close/join；`connect()` 仅打开本地资源，不与对端握手。

业务值拥有全部 string / bytes / 嵌套消息 / packed 数组；UTF-8 bound 按字节计算，NUL 合法，未知有符号枚举与浮点 bits 保留，optional 缺失与显式空值/默认值区分明确。wheel 私有静态链接原生依赖并使用独立 nanobind domain。当前生成器只提供 UDP RPC 客户端，不含订阅、Python 服务端、Serial/USB、动态 schema、free-threaded Python 与子解释器。
