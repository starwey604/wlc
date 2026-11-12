# 构建与使用自持 SDK

[English](sdk.md) · [代码生成](codegen-cn.md)

生成可安装的 C++20 client 和带类型的 Python 同步/异步包：

```sh
wlc sdk device.wl --profile host.bind.wl --name device --package-version 0.13.0 --out-dir sdk
cmake -S sdk -B build/sdk -DCMAKE_PREFIX_PATH=/path/to/wirelink-install
cmake --build build/sdk --parallel
```

使用 WLC 0.13.0 / 生成契约 0.13、binding source API 3 和匹配的 Wirelink 0.13.0。SDK 包版本由作者选择。profile 必须有界、只含 RPC、version 2 且选择 `native_packet`。kernel 保留两种 RPC 角色；client 不注册业务 server handler。

## 构建原生库或 Python wheel

安装 Wirelink 时启用 `WIRELINK_BUILD_CPP_BINDINGS=ON` 和 `WIRELINK_BUILD_PLATFORM=ON`，并提供 standalone Asio。Linux Python extension 的静态依赖需要 PIC。SDK 源码包包含生成 C，构建不需要 Rust/WLC checkout。CMake 默认 `<NAME>_BUILD_PYTHON=OFF`，wheel 打包会启用它。原生手写测试由 `BUILD_TESTING` 控制。具体包命令见生成 README；[Calculator 指南](https://github.com/starwey604/wirelink/blob/main/examples/06_bindings/GUIDE.md)提供可运行 C 服务和 Python 测试。

CMake 对照安装的 Wirelink 包元数据，检查随包 kernel manifest 的 WLC 版本/生成契约。`wlc-sdk-build.json` 记录构建元数据（`wirelink-sdk-build-v2`、kernel contract 1），不选择链接布局。修改编译进工具的模板后，用重建的匹配 WLC 重新生成。`--overwrite` 替换同名 SDK 的生成文件；应用测试/指南放在生成清单之外。

## 共用原生 owner，保存结果

C 拥有原子 port 存储和类型化回调。C++ Session 排队 owner 命令并包装等待/I/O。同步和异步调用共用 **四个原生 call slot**；AsyncClient 另限制 **八个尚未交付操作**，不是八个原生调用。host 命令队列有 32 项，每批最多处理 8 条。接纳成功前完成请求快照；接纳失败不创建调用或终态回调。

`Operation<T>` 可复制，丢弃它不取消调用；自持响应/错误可以超过 client 的生命周期。回调在返回前复制借用 view。C++ timeout 单位为毫秒，范围 `1..INT32_MAX`；Python timeout 单位为秒，向上取整到毫秒，上限 2147483.647。两者均计入 owner 命令排队时间。不要从 owner 回调阻塞或销毁 Session。

close 使未完成调用以 `ClosedError` 完成；显式 operation cancel 产生 `CancelledError`。asyncio task cancel 抛出 `asyncio.CancelledError` 并请求原生取消。到期抛出 `RpcTimeoutError`。业务拒绝保留有符号 `status`。Python 错误公开 `kind`、`status`、`outcome`、`os_error`、`os_message`；C++ Error 包含 `kind`、`completion` 和 `system_error`。本地 cancel/timeout 不能撤回远端副作用。只含 RPC 的 SDK 需要另定义应用状态查询 RPC 来处理长任务。

## Python 执行离开 owner

Python 阻塞等待释放 GIL。AsyncClient 使用一个有界 completion worker 和 asyncio future；原生 owner 不执行 Python 回调。使用 `with Client.connect(...)` 或 `async with AsyncClient.connect(...)` 完成 close/join。连接仅打开本地资源，不与 peer 握手。

业务值拥有 string/bytes/嵌套消息/数组。UTF-8 bound 按字节计算，NUL 合法，未知有符号枚举与浮点 bits 保留，optional 缺失不同于显式空值/默认值。wheel 私有静态链接原生依赖，并使用独立 nanobind domain。

生成器提供 UDP RPC client。订阅、Python 服务端、Serial/USB client、动态 schema、free-threaded Python 与子解释器不属于此 SDK 接口。C++ 消费者使用匹配头文件和编译设置重建；源码接口不承诺稳定的 C++ 二进制 ABI。
