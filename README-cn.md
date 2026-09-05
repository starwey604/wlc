# wlc 中文指南

`wlc` 是 Wirelink schema compiler：解析/验证 `.wl` schema、对照前一 revision 检查
兼容性，并生成无动态分配的 C11 payload codec 及可选 typed binding/runtime。

> 英文版 [`README.md`](README.md) 是规范来源。本文件用于中文 API 审阅。

## 预编译 Compiler

tagged release 为 Windows x86-64、Linux x86-64/aarch64（static musl）和 macOS
x86-64/Apple Silicon 发布 host tool。使用前以 release 的 `SHA256SUMS` 校验 archive。

compiler version 与 generated-code ABI 是两个兼容轴。`wlc --version` 报告 release，
manifest 的 `compiler.codegen_abi` 记录生成 ABI；build 必须同时 pin 两者，不能跟随 branch
或自动使用最新版。

## Schema Grammar

完整 grammar 和 wire 约束见 [Wirelink schema 文档](https://github.com/starwey604/wirelink/blob/dev/wirelink-p0-hardening/docs/schema-v1-cn.md)。所有
declaration/field ID 都显式分配；message/enum 共用非零 16-bit global ID namespace，field
ID 在 message 内唯一。packed count 和 borrowed-field bound 为 1…65535。
新源码用 `@id(n)` 标记编号；原 `= n` 仍接受，生成 C、manifest、identity 和字节相同。
枚举值和默认值仍使用 `=`。仅修改编号拼法不需要递增 schema revision，不按排列自动分配编号。

```wl
message JointControl @id(16) {
  packed float32 position[6] @id(1);
  packed float32 velocity[6] @id(2);
}
```

内建类型包括 bool、bytes、string、8/16/32/64-bit signed/unsigned integer、fixed32/64、
float32/64。窄整数生成精确宽度 C storage；float 要求 IEEE-754 4/8-byte，使用 `memcpy`
搬运 bits。`string<MAX>`/`bytes<MAX>` 仍是借用 view，MAX 按编码字节计算，不引入 copy、
heap、lock 或隐藏 ownership。

optional default 必须匹配类型和范围。required 不能带 default/repeated；固定向量使用
`required packed`。float 暂无显式 default，缺失时为正零；bytes/repeated/packed 无 default。
bounded string default 在 schema analysis 时按 UTF-8 byte length 检查。

## Wire Rule

encoder 按 field number 升序输出。key 为 unsigned LEB128
`(field_number << 3) | wire_type`。unsigned integer 用 LEB128，signed integer 用 ZigZag；
窄类型超范围返回 `WL_CODEC_ERR_OVERFLOW`，不截断。fixed/float 使用大端 4/8-byte。

bounded string/bytes 对 optional、required、repeated 全部 enforce。超 bound 返回稳定的
`WL_CODEC_ERR_INVALID_VALUE`，invalid UTF-8 返回 `WL_CODEC_ERR_UTF8`。decode 在借用 input
view 前立即检查 bound，不截断或复制。

packed 在 C 中是 presence flag 加 inline array，wire 上是一次 type-2 field、一个 length、
恰好 count 个大端 fixed-width element。duplicate、wrong wire type 或 length 不精确都拒绝；
无 pointer/count/capacity/heap/per-element tag。普通 repeated 仍用调用方 pointer/count/
capacity，并逐元素编码完整 tag/value。

required 和 optional 的 wire form 相同；required 缠失时 encode/decode 返回
`WL_CODEC_ERR_MISSING_REQUIRED_FIELD`。decoder 接受 unknown field。每个 message 都生成
`*_HAS_MAX_ENCODED_SIZE`；schema 可证明有界时还生成包含最坏 key/varint/length/nesting 的
`*_MAX_ENCODED_SIZE`。

## 生成 Typed Binding

schema 编译确定性生成 `<module>.h/.c`、`<module>_bindings.h/.c` 和 manifest。
`compile-runtime` 只生成命名 profile runtime 和独立 manifest。codec 只依赖
`wirelink/codec.h`；binding 独立依赖 public `wirelink/link.h`，codec-only firmware 不会
拉入 send/dispatch/core。

router 的每条 message route 有强类型 `int32_t` callback、调用方 scratch 和 user pointer。
典型组装如下：

```c
static int32_t on_status(void *user, const status_t *status,
                         wl_delivery_t delivery);

motor_api_router_t router = {0};
static status_t status_scratch;
static uint32_t sample_storage[8];

status_scratch.samples = sample_storage;
status_scratch.samples_capacity = 8U;
router.status = (motor_api_status_route_t){
    &status_scratch, on_status, application
};
```

dispatch 对任何有效 RX outcome 恰好 `wl_event_release()` 一次，包括 success、unknown ID、
missing route/scratch、codec/handler failure。非 RX 返回 `*_DISPATCH_NON_RX`，不 release。
callback 期间 bytes/string 是借用，不可保留、release event 或递归 dispatch 同一 context。

每个 message 生成一个 typed send，显式接收 `wl_delivery_t`，claim core 最终 TX payload、
原地 encode、commit，不经过中间 copy。结果保留 codec status、raw core result、length 和
reliable handle。`*_SEND_OK` 只表示已提交；后续 ACK/TX success 仍仅是 link delivery。

## 语义与兼容性

semantic analysis 解析类型并拒绝 unknown、recursive/过深 nesting、非法 default 和非固定
packed element。semantic model 按 ID/field number 排序，因此源码重排不改变产物。
`reserved N` 永久保留删除的 declaration/field/enum value。兼容检查拒绝 ID、name、type、
cardinality 修改或删除不 reserved。required field 的增删不兼容；整数宽度/符号、packed
element type/count、string/bytes bound 都是 identity。

library API 为 `parse_schema()`、`analyze_schema()`、`check_compatibility()`、
`generate_c()`、`generate_runtime_c_named()`。

## CLI

```sh
# 验证 schema
cargo run -- validate path/to/schema.wl

# 对照旧版并生成 codec + typed binding
cargo run -- compile path/to/schema.wl \
  --previous path/to/previous.wl \
  --out-dir generated

# 只生成命名 runtime
cargo run -- compile-runtime path/to/schema.wl \
  --profile path/to/device.bind.wl \
  --runtime-name device_api \
  --out-dir generated

# 打印 schema/profile identity
cargo run -- identity path/to/schema.wl \
  --profile path/to/device.bind.wl

cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

diagnostic 使用 `line:column: message`，CLI 通过 `miette` 显示错误 token 的源码片段。

## 可选 Binding Profile

应用 routing policy 放在独立、带版本的 sidecar，不进入冻结 `.wl` wire grammar：

```text
profile version 1;

latest ArmMitCommand { delivery = unreliable; }
fifo AlarmEvent { delivery = reliable; }

rpc Home {
  request = HomeRequest;
  response = HomeResponse;
  request_delivery = reliable;
  response_delivery = reliable;
}
```

三个编号／状态映射全部省略，即选择托管 RPC，`.wl` 只定义业务参数。
runtime 管理 12 字节前缀：零区分字节、版本、请求／响应类型、保留零、
大端 uint32 调用编号和大端 int32 状态。成功响应带业务体，非零拒绝只带前缀。
默认端点生成 `*_call_t`、`*_result_t` 和回复 token，使用
`call/inspect/release/cancel/complete/reject`；容量包含元数据，纯托管路径不再
分配用于注入字段的类型化编码暂存区，请求／响应直接写 TX／缓存。

接入已有 schema 时，可以显式写出 `request_operation_id`、`response_operation_id`、
`response_status` 三个映射，保持旧编码；只写部分会报错。托管与映射两种模式不能直接
互通，模式进入 profile identity，迁移需同步两端。仅 retained 策略和本地角色不同仍可共享 codec。
调用关联与有界重放不等于持久化业务幂等。本地 token 在 runtime 重建后应丢弃，
默认端点增加归属／代次检查，但不保证客户端跨重启或线上编号复用后的响应新鲜度。



不同 host/device profile 可共用同一 wire schema。`--runtime-name` 给非对称角色独立 C
namespace。生成 dispatcher 直接解码进 LATEST/FIFO claim，成功才 publish、失败都 abort，
并对有效 RX 恰好 release 一次。manifest 的 `bounded_fields` 记录有界字段的名称/ID/kind/
MAX，bound 参与 schema identity。

### 默认 Assembly 与初始化

全部 payload 有界时，生成无 heap 的默认 assembly：

```c
control_runtime_config_t config;
control_runtime_default_storage_t arena;
control_runtime_instance_t instance;

control_runtime_config_defaults(&config);
control_runtime_config_enable_client(&config);
control_runtime_storage_t storage =
    control_runtime_default_storage_descriptor(&arena);
control_runtime_init(&instance, &config, &storage);
```

默认值采用一个 FIFO/RPC slot、generation/operation ID 1、精确 encoded maximum、role
disabled、timeout 0、reject-new cache。role helper 只 enable；handler 和业务 expiry 仍由应用
填写。不完全有界时 `*_RUNTIME_HAS_DEFAULT_STORAGE=0`，必须根据
`*_runtime_requirements()` 自备 aligned arena。

init 会验证 overflow、size、alignment、与 instance overlap，并把 LATEST/FIFO、RPC slot/
cache、scratch、handler/user pointer 全部 wiring 进 `instance.runtime`。config/storage
descriptor 可临时存在；instance/backing bytes 必须地址稳定且 init 后不可复制。
`*_runtime_init_checked()` 额外返回 rejected field 和 required/provided；证明配置后使用普通
init，让 linker GC 移除诊断。

client/server role 可独立启用，disabled role 的 sizing 被忽略且 runtime pointer 为 null。
capacity、timeout、cache policy 是 deployment config，不参与 schema/profile identity。

### Result、Pump 与体积

result 由小型公共头（`domain`、event/message identity、`detail_kind`、`event_consumed`）和
tagged union 组成。只在匹配 tag 时通过生成 accessor 读取 detail；字符串仅用于日志。
dispatch release RX 或 reclaim 匹配 TX handle 后设置 `event_consumed`，owner fallback 只能在
其为零时执行。

当前固定宏为 `<MODULE>_RUNTIME_CODEGEN_ABI_VERSION 20`；`wlc codegen-abi` 可直接查询。
ABI 改变时所有 runtime
source 和字段访问一起更新。pump helper 共用一次 `now_ms`，最多 service 一个 response，
合并 RPC deadline，并可把借用 diagnostic result 交给 observer。

ABI 19 在 runtime 头文件中增加默认端点 `*_endpoint_t`：自动组合连接缓冲区、
runtime arena 和 pump。应用使用 `endpoint_init`、`step`、`close`，以及按 profile
选择传输方式的 `endpoint_send_*`、返回用户副本的 `endpoint_read_*`。
ABI 20 的托管 RPC 使用 call/inspect/release/cancel/complete/reject 和生成句柄。
旧映射 RPC 保留 start/inspect/release/complete。对象必须从零初始化且不能移动，
`private_state` 成员不属于应用 API。

`endpoint_handle()` 用于连接适配器，`endpoint_runtime()` 保留高级借用接口。
容量根据 profile 选中的消息推导；消息无界或超过单帧上限时，
`*_HAS_DEFAULT_ENDPOINT=0`，继续使用高级自定义存储。默认整包传输和 CRC32C，
RPC 角色与过期策略仍显式选择。生成产物必须与支持端点 API 的 Wirelink 配套。

Cortex-M4/Thumb/`-Os` regression fixture 当前 gate：dispatch/RPC/consumer 3328 bytes、
assembly helper 1200、pump 320、optional diagnostic 288、完整 object 5088；retained-only
result 24 bytes，RPC/combined result 112 bytes。它们是 generator regression，不是整机估算；
每个函数独立 section，`--gc-sections` 移除未引用 API。

### Retained 与 RPC

runtime dispatch 是传入 RX event 的终态 owner，不可作为 chainable try-dispatch。匹配 RPC
TX terminal 会推进 runtime 并 reclaim core transaction；不匹配非 RX 仍由 caller 拥有。
WLC 拒绝把含 bytes/string/repeated（包括嵌套）的 message 放进 retained route。每条 route
生成 typed acquire/release；成功 release 会清空 view。

可靠 ACK 在 event admission 后、typed decode/application retention 前调度，因此 FIFO full、
LATEST coalescing、codec/handler/RPC failure 不会 NACK 或重启 ARQ。peer-visible completion
必须通过 RPC response/status 和应用 deadline 表达。

每个 RPC service 生成 client start/inspect/decode/release、request handler 和 server
complete/reject；默认端点还提供句柄与类型化结果。request/response input 为 `const`，
只有映射模式在 shared typed scratch 上注入 ID/status，纯托管模式不分配这个暂存区。
client response 原始字节保留到 release；借用字段也只在此前有效。server canonical
re-encode 后计算 domain-tagged fingerprint，按 NEW/PENDING_DUPLICATE/REPLAY/CONFLICT
处理；complete/reject 先 cache 后 send。

异步 completion 必须复制包含 peer session 的 identity。ABI 18 在可靠 request 前自动观察
session；切换清理旧工作并请求取消 detached response，`peer_changed` 和 take API 通知产品。
所有 `now_ms`/poll/hint 使用同一 monotonic ms clock。link ARQ、client deadline、server
replay cache 是独立机制；runtime 不自动端到端重试。

## Identity 与 Manifest

`schema_identity()` 和 `binding_profile_identity()` 使用 `fnv1a64-v1` 对规范化 semantic
model 哈希，不受空白/声明顺序影响，但不是 cryptographic hash。schema identity 是 exact
而非 compatibility-aware；profile identity 依赖具体 schema。诊断时一起报告
`(algorithm, schema identity, profile identity)`。

manifest 记录 compiler release、codegen ABI、identity，以及排序后的 artifact byte size 和
domain-tagged FNV digest；省略 timestamp、绝对路径和 host 信息，所以相同输入/版本在不同
workspace 产生 byte-identical manifest。digest 用于诊断完整性，不是签名。

## 依赖策略

`miette` 用于源码诊断，`thiserror` 用于 typed error，`clap` 用于 CLI，`heck` 用于稳定 C
symbol，`insta` 用于 golden snapshot，`proptest` 用于 parser/property，`assert_cmd`/
`tempfile` 用于隔离 CLI 和 generated-C test。parser 手写以保持 grammar 明确；生成 codec C
只依赖 `wirelink/codec.h`，typed binding 只依赖 public Wirelink header。
