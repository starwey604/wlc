# 组装类型化 kernel 端点

[English](static-kernel.md) · [代码生成](codegen-cn.md)

为部署生成 route 和有界存储：

```sh
wlc compile device.wl --profile device.bind.wl --out-dir generated
```

schema 定义消息，profile version 2 选择 route 与 connector：

```text
profile version 2;
endpoint { envelope = native_packet; }
rpc Read { request = ReadRequest; response = ReadResponse; }
send Sample { delivery = unreliable; }
direct Sample { delivery = unreliable; }
```

使用 WLC 0.13.0 / 契约 0.13 和 Wirelink 0.13.0。选择 `native_packet`/NONE、`cobs_stream`/CRC16 或 `bus_length16`/NONE；RPC 两端均启用发起/处理能力。普通消息使用公开 Schema-v1 或显式 positional-v1；RPC 使用 managed tagged body 与 instance/operation 元数据。kernel route 要求有界类型；repeated view 属于独立 codec。

## 组装 port、route 和 owner

`<module>_kernel.h/.c` 将类型化 route 连接到 `wl_kernel_t`。私有 RPC codec 位于 `_kernel_codecs.h`；`_kernel_deployment.json` 报告 bounds/roles/storage，`_kernel_manifest.json` 记录输入及生成产物。stream profile 只生成所选 connector。

应用拥有从 C 初始化的 packet port、两个 RX unit、生成端点和时间源。提供非零的新鲜 instance 身份、sink 和通知回调，再调用 `<module>_kernel_init`。同一 owner 执行接纳、取消、handler、终态回调和 `wl_kernel_step`。等待前，将 `wl_kernel_ready` 返回的 readiness/最早 deadline 与 adapter readiness、锁存唤醒合并。参阅 [owner loop 教程](https://github.com/starwey604/wirelink/blob/main/docs/tutorial-integration-cn.md)。

## 接纳、完成与关闭

kernel 有四个出站调用、四份 reply snapshot 和一份普通 TX snapshot。类型化 `_start` 在成功返回前完成请求快照；失败不创建调用，也不产生后续回调。传入 owner tick 的 `now` 与 `1..INT32_MAX` 的相对 timeout；排队计入 timeout。物理 TX lease 必要时会超过逻辑终态，仍需源已消费和物理已 drain/error 通知。

handler 是短同步业务函数：填写自持响应，返回零表示成功，非零有符号业务 status 表示拒绝。终态回调借用响应 view 到返回；用 `*_value_from_view` 复制以保留。每个已接纳调用交付一次终态，随后 handle 失效。回调可提交/取消，但不能递归 step/close。本地 cancel/timeout 不能撤回远端执行。应用负责重试/幂等和[长任务](https://github.com/starwey604/wirelink/blob/main/docs/tutorial-jobs-cn.md)。

关闭 kernel，停止/join 真实 I/O，交付最终 completion，并 step 到 `wl_kernel_quiescent` 后再释放存储。[kernel API](https://github.com/starwey604/wirelink/blob/main/docs/kernel-api-cn.md)定义错误与生命周期细节；[开发检查](https://github.com/starwey604/wirelink/blob/main/docs/development-cn.md)提供构建与测试命令。
