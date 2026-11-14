# WLC

[English](README.md)

WLC 校验 Wirelink schema，生成无动态分配的 C11 codec、连接共享 C kernel 的类型化端点，或拥有数据的 C++20/Python UDP SDK。使用 **WLC 0.13.0**、生成契约 **0.13** 与匹配的 **Wirelink 0.13.0**；端点 profile 使用版本 **2**。

## 构建与生成

在本仓库构建本地编译器，并加入 PATH：

```sh
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
wlc --version
wlc codegen-contract
wlc compile application.wl --out-dir generated-codec
wlc compile application.wl --profile application.bind.wl --out-dir generated-endpoint
wlc sdk application.wl --profile host.bind.wl --name application --out-dir sdk
```

前两项查询输出 `wlc 0.13.0` 和 `0.13`。其余命令需要你自己的 schema 与 profile；每种部署使用独立输出目录。

- **公开 codec：** `.h`、`_values.h`、`.c` 和 provenance manifest。借用视图覆盖 Schema-v1 值域，包括嵌套和 repeated 字段；有限类型还生成自持值。这些 codec 依赖 `wirelink/codec.h`。
- **类型化端点：** `_kernel.h/.c`、私有 RPC codec、部署/provenance manifest，以及 stream profile 选择的 connector。生成 route 共用 C kernel，应用提供存储、时间、owner 执行和 I/O。
- **SDK：** 可安装的 C++20 client、Python 同步/异步包、生成 C、CMake 导出及 wheel/sdist 配置。SDK profile 必须有界、只含 RPC 且选择 `native_packet`；源码构建需要匹配并启用 `cpp_host` 的 Wirelink。

## 选择 route 和传输

profile 引用你在 schema 中定义的消息：

```text
profile version 2;
endpoint { envelope = native_packet; }
rpc Execute { request = ExecuteRequest; response = ExecuteResponse; }
send Sample { delivery = unreliable; }
direct Event { delivery = unreliable; }
```

显式选择 `native_packet`/NONE、`cobs_stream`/CRC16 或 `bus_length16`/NONE。RPC 两端均保留发起和处理能力。普通 route 使用 Schema-v1 或显式 positional-v1；RPC 使用 managed tagged body 和 instance/operation 元数据。借用式 repeated 字段属于独立 codec 能力，不是 bounded kernel route 能力。

接纳成功前完成输入快照。四个出站调用、handler 和终态回调共用一个 owner；排队计入 timeout，响应 view 借用到回调返回。应用复制需要保留的结果，并负责重试/幂等和长任务协议。参阅 [kernel 执行模型](https://github.com/starwey604/wirelink/blob/main/docs/kernel-cn.md)和 [API 参考](https://github.com/starwey604/wirelink/blob/main/docs/kernel-api-cn.md)。

## 按任务继续

- [生成 codec 和查看 plan](docs/codegen-cn.md)
- [组装类型化 kernel 端点](docs/static-kernel-cn.md)
- [构建与使用自持 SDK](docs/sdk-cn.md)
- [运行 Wirelink 教程与开发检查](https://github.com/starwey604/wirelink/blob/main/docs/development-cn.md)

- [编译器发布演练与产物提升（英文）](docs/releasing.md)

修改编译器后运行：

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
```

生成字节具有确定性。生成文件与 provenance 一起维护；wire 向量、畸形输入测试和生成 C 执行测试补充产物 snapshot，验证实际行为。
