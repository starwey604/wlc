# 生成 codec 和查看部署

[English](codegen.md) · [类型化端点](static-kernel-cn.md) · [SDK](sdk-cn.md)

使用 WLC 0.13.0 / 契约 0.13 与匹配的 Wirelink 0.13.0。在包含 schema 的目录运行：

```sh
wlc validate device.wl
wlc compile device.wl --out-dir generated-codec
wlc compile device.wl --profile device.bind.wl --out-dir generated-endpoint
wlc plan device.wl --profile device.bind.wl
wlc identity device.wl --profile device.bind.wl
wlc dependencies device.wl
wlc module-name device.wl
```

不带 profile 时，`compile` 生成 `<module>.h`、`_values.h`、`.c` 和 manifest。公开 codec 使用 `wirelink/codec.h`，不会把端点引入固件。带 profile 时，还生成[类型化 kernel](static-kernel-cn.md)及所选 connector。`compile-kernel` 是 `compile --profile` 的别名。

## 借用视图或保存自持值

公开 Schema-v1 codec 保留整数宽度、未知有符号枚举值、IEEE 浮点 bits、optional presence 与默认值、有界 UTF-8/NUL、bytes、固定 packed 数组、嵌套消息及 repeated 借用视图。repeated 字段使用调用方提供的 pointer/count/capacity 存储。未知字段经校验后跳过。参阅 [schema 与 wire 规则](https://github.com/starwey604/wirelink/blob/main/docs/schema-v1-cn.md)。

有限类型在 `_values.h` 中生成 `<message>_value_t`。string/bytes 使用内嵌存储，嵌套值拥有其内容。`*_value_from_view` 复制借用输入；`*_value_to_view` 借用自持值。检查转换结果。bound 按编码字节计算，字符串额外的终止 NUL 不编码。`*_VALUE_SIZE` 是目标平台的 `sizeof`，不是最大 wire 长度。无界类型标记 `*_HAS_VALUE=0`。

## 校验组合与演进

import 相对导入文件解析。`dependencies` 列出规范化传递输入；`module-name` 输出确切的 C 文件名/符号前缀，包含关键字转义。重复 `--profile` 组合声明并检查冲突，不做覆盖。profile version 2 必须显式选择 connector。

使用 `validate --previous previous.wl` 或 `compile --previous previous.wl` 检查 schema 演进。identity 是精确标识，不表示兼容关系。`plan` 报告实际的四 call/四 reply kernel 资源、有界 TX 存储和所选 codec 字段信息。identity 与 FNV 产物摘要用于诊断，不用于认证。

## 接入构建

安装的 Wirelink CMake 包提供 `wirelink_wlc_generate(TARGET ... SCHEMA ... PROFILE ...)` 生成端点，`wirelink_wlc_generate_codec` 生成 codec。把 `WIRELINK_WLC_EXECUTABLE` 设为匹配的本地编译器；import 和 profile 成为构建/重新配置依赖。参阅[构建命令](https://github.com/starwey604/wirelink/blob/main/docs/development-cn.md)。

每种部署使用独立输出目录。遇到不兼容产物集合，包括其他部署遗留的 stream connector，使用新目录。生成文件与 provenance 一起保存。模板编译进工具，因此修改模板后必须先重建 WLC 再重新生成。

修改生成器时，除生成 C 编译/执行外，还运行公开字节向量和畸形输入测试。snapshot 变化本身不能证明 wire 正确。
