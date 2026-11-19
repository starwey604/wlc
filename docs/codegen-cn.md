# 生成 codec、节点与查看部署计划

[English](codegen.md) · [类型化节点](static-endpoint-cn.md) · [SDK](sdk-cn.md)

使用 WLC 0.14.0 / 生成契约 0.14 与匹配的 Wirelink 0.14.0。消息、RPC 与节点全部声明在同一 `.wl` 文件中：

```sh
wlc validate device.wl [--previous old.wl]
wlc compile device.wl --out-dir generated-codec                # 无 node 时生成纯编解码器
wlc compile device.wl --node server --out-dir generated-server # 多节点按角色生成
wlc compile device.wl --all-nodes --out-dir generated-loopback # 同进程双端回环
wlc plan device.wl --node server
wlc identity device.wl
wlc dependencies device.wl
wlc module-name device.wl
```

`compile` 生成单个 `<stem>.h`、`<stem>.c` 和 `<stem>_manifest.json`（旧 `_values.h`、`_kernel.h`、`.bind.wl`、`--profile` 与 `compile-kernel` 已移除）。只有一个 `node` 时自动选中；存在多个节点时必须指定 `--node <name>` 或 `--all-nodes`。

## 默认必选字段、借用视图与有界值

- **默认必选**：无修饰符字段与裸 `packed` 固定数组默认必选，不生成 `has_<field>`；仅 `optional` / `optional packed` 生成存在位。解码器仍验证所有必选字段在线上到齐。从旧版可选 `packed` 迁移时须显式改写为 `optional packed`。
- **有界与无界存储**：`string<N>` / `bytes<N>` 将数据内联保存在消息结构体中，通过生成的 setter 检查容量并复制；无界 `string` / `bytes` 与 `repeated` 在独立 codec 中使用调用方借用视图/backing，节点路由要求消息有界。
- **契约规则**：详情见 [Wire v2 与编码规范](https://github.com/starwey604/wirelink/blob/main/docs/wire-format.md)。

## 校验组合与演进

`import` 相对当前导入文件解析。`dependencies` 输出规范化的传递输入清单；`module-name` 输出合法化的 C 文件/符号前缀。

使用 `validate --previous previous.wl` 或 `compile --previous previous.wl` 检查 schema 演进兼容性；`identity` 输出确定性 schema/node 身份；`plan` 输出各节点路由方向、帧上界、节点存储及字段编码计划（诊断用途）。

## 接入 CMake 构建

加载 Wirelink 后，使用 `wirelink_wlc_generate(TARGET ... SCHEMA ... [NODE name | ALL_NODES] [PREVIOUS old.wl])` 生成节点，使用 `wirelink_wlc_generate_codec(TARGET ... SCHEMA ...)` 生成纯编解码器。通过 `WIRELINK_WLC_EXECUTABLE` 指定本地匹配的 0.14 编译器；传递 `import` 自动成为构建/重配置依赖。参阅 [开发与验证指南](https://github.com/starwey604/wirelink/blob/main/docs/development.md)。

每种角色或部署使用独立输出目录，切勿混编旧版产物。修改内置模板或生成逻辑后，需先重建 WLC 再重新生成 C 与 SDK，并同时运行公开字节向量、畸形输入和编译/执行验证。
