# wlc 中文指南

## C++ / Python SDK 生成预览

`wlc sdk calculator.wl --profile calculator.bind.wl --out-dir sdk --name calculator`
可生成 C++20 Client、拥有数据的消息类型、带类型提示的 Python 包、nanobind 桥接和
CMake／wheel／sdist 工程。无需手写桥接代码。此轮支持 UDP 上的托管同步 RPC，
包括有界字符串／bytes、可选字段及默认值、未知 enum、固定 packed 数组和嵌套消息。
可选值保留缺失状态，显式默认值通过 `<field>_or_default` 读取。

源码构建需安装匹配的 Wirelink 0.7.0 开发包并启用 CPP_BINDINGS／PLATFORM；wheel
用户无需 WLC、Rust 或 C++ 编译器。输出目录重复生成内容相同时可直接执行；替换已有生成文件
须显式指定 `--overwrite`，修改 SDK 名称须使用新目录。详见 [英文 SDK 契约](docs/sdk.md)。

`wlc` 是 Wirelink schema compiler：解析/验证 `.wl` schema、对照前一 revision 检查
兼容性，并生成无动态分配的 C11 payload codec 及可选 typed binding/runtime。

> 英文版 [`README.md`](README.md) 是规范来源。本文件用于中文 API 审阅。

## Codec 编译期规划

维护编译器时先看 [生成器源码导航与正确性验收](docs/codegen.md)。
`codegen.rs`/`runtime_codegen.rs` 是入口；编译期规划、校验、C 输出和存储组装分属私有模块。
此次分层不改变公开接口、生成 API 或 ABI，也不增加消息展开策略。

编译器自动选择 codec 实现：不超过 8 个字段保持线性查找；更多字段在编号连续时直接索引，
编号稀疏时二分查找已排序描述表。wire type 由 WLC 预计算。只有单个 packed
fixed32/fixed64/float32/float64 数组的消息才生成专用入口，长度与前缀为编译期常量，
数组元素仍用循环处理；清零及其他结构保留通用引擎。
这些是可继续调整的实现策略，不是新的
schema 属性或稳定阈值，不改变 codec 的线上字节。

## 预编译 Compiler


tagged release 为 Windows x86-64、Linux x86-64/aarch64（static musl）和 macOS
x86-64/Apple Silicon 发布 host tool。使用前以 release 的 `SHA256SUMS` 校验 archive。

compiler version 与 generated-code ABI 是两个兼容轴。`wlc --version` 报告 release，
manifest 的 `compiler.codegen_abi` 记录生成 ABI；build 必须同时 pin 两者，不能跟随 branch
或自动使用最新版。

当前开发版为 `0.7.0-dev`，`wlc codegen-abi` 输出 32（未发布）；核心和所有生成
消费者必须配套重建。此轮支持根 schema 的 `import "arm.wl";` 静态组合，以及
profile 的 `direct BulkChunk { delivery = reliable; }` 借用式接收路由。
导入相对当前文件解析，共享文件去重，循环与全局 ID/名称冲突在编译期拒绝；根版本
表示组合协议版本。`wlc dependencies product.wl` 输出传递依赖，供增量构建使用。
`config.on_bulk_chunk` 的输入及其 bytes/string 只在回调期间有效，延后处理必须复制。
WLC 负责唯一 RX 释放；direct 不能与同 ID 的 retained/RPC 同时接收，不能含 repeated
字段（含嵌套），支持固定 packed 数组。详细契约见英文 README 的 Static product composition。

## 多服务定义复用与只发送消息

把 RPC 声明保留在 `services.bind.wl`，端侧文件只描述自己的路由，重复传入 `--profile`：

```sh
wlc compile-runtime device.wl --profile services.bind.wl --profile server.bind.wl \
  --runtime-name device_server --out-dir generated
```

`validate`、`identity`、`compile` 同样支持组合。每份文件独立验证；顺序不影响结果，
重复定义和跨文件冲突报错，不做覆盖。CMake `wirelink_wlc_generate_runtime` 对应参数为
`PROFILES`；原 `PROFILE` 仍用于单个文件。

只发送的端点可写 `send DeviceTelemetry { delivery = unreliable; }`。
它生成类型化发送函数并参与端点容量推导，但不分配接收邮箱。对端独立声明 `latest/fifo`。
原有 retained 声明仍提供对称发送助手；同消息显式 `send` 负责选择出站 delivery。
RPC 请求/响应不能再声明为普通消息路由；无界或过大的 send 会禁用默认端点组装。
没有 send 声明的 profile 保持原 identity。ABI 27 不改变 ABI 26 的 codec 和线上格式。

## 自持业务值与高级视图

普通业务包含 `<module>_values.h`，使用 `<message>_value_t`。有界 string/bytes
内嵌 `length` 与 `data[]`，嵌套消息递归拥有数据；结构体赋值就是独立副本，
不要求用户准备 backing buffer、分配或释放。只有现有类型能推导有限上限时才生成值；
无界 repeated/string/bytes（包括嵌套）标记 `*_HAS_VALUE=0`，不擅自指定容量。

`*_value_clear/encode/decode/encoded_size` 复用原 codec 的线上语义。
长度按 UTF-8 字节计算，允许嵌入 NUL；clear/decode 附加的末尾 NUL 不编码。
required、presence 与默认值不变，解码失败不修改输出。
`*_VALUE_SIZE` 是目标 C 平台的 sizeof，不是最大编码长度。

需要借用时才包含 `<module>.h`，使用原 `<message>_t`，或者显式
`*_value_from_view/to_view`。转换失败不修改输出；输入输出存储不能重叠。
得到的视图仍借用原值，不能比原值活得更久。普通 RPC 直接消费自持值，
不需要业务手工做这些转换。

## 默认 RPC 端点（ABI 29）

有默认静态存储配方的 RPC profile 解码暂存跨服务共用。ABI 29 边遍历规范化编码
边计算指纹，删除规范化请求缓冲及容量配置。含无界消息的高级 runtime 保留逐服务解码对象，
避免覆盖用户设置的 repeated backing。每个服务的配置上限继续生效；同一 runtime
不能重入分发，延迟工作须复制回调输入，不能保存暂存区指针。线上格式不变。

`endpoint_init(endpoint, wl_platform_environment())` 自动生成实例身份并配置时钟。
自定义 `wl_environment_t.session.next` 可接入裸机身份来源；业务无需传 session ID。
托管 RPC 元数据 v2 为 20 字节，回送客户端身份以隔离旧实例响应；所有 delivery 组合
适用。v1/v2 须两端成对升级；显式字段映射和 Compact-v1 帧格式不变。

`<runtime>_endpoint.h` 是普通入口；因静态布局传递包含 runtime 头，
不承诺所有高级声明都不可见。使用 `endpoint_<service>_async()` 提交自持请求，
完成 callback 取得自持响应，调用槽自动回收。需要取消才保存 `wl_rpc_call_t`；
普通路径没有 inspect/release。失败提交不回调，已接受调用在持续推进或有序关闭时通知一次。
回调指针只在回调内有效，复制 `*response` 则独立于端点。

服务端用 `config.on_<service>` 注册即时 handler，返回 0 表示成功，
非零仅表示业务拒绝。普通 handler 和诊断共用 `config.user_data`；
非 NULL 的 `config.<service>_user_data` 可覆盖单个服务，NULL 表示继承。
高级 deferred handler 和单次调用完成回调仍使用显式上下文，不隐式继承。
client 初始化就绪，注册 handler 自动提供 server 能力。
默认四槽有界提交及最近结果缓存，保护未送达结果，只淘汰最旧已送达结果。
TTL 是最长保留而非保留窗口承诺。统一构建定义 `<PREFIX>_ENDPOINT_RPC_CAPACITY`
可缩小静态容量，运行时 count 不得更大。

`endpoint { rpc_role = server; }` / `client` 也裁剪接收分发中的对侧 RPC 路径。
已知但角色不支持的消息仍报告 delivery mismatch / missing route，并释放 RX，
不执行 codec。managed RPC 的请求校验、去重/重放和响应完成共用私有实现，
不改变业务 codec、公共布局或 ABI 32，也不需要开启 `-Os`。
测量和回归门槛见 [RPC Flash](docs/rpc-flash.md)。

`config.advanced` 保留手动角色、容量和缓存策略；原 call/token 是高级路径。
需要手动 endpoint call/inspect/release、complete/reject 助手时显式包含
`<runtime>_advanced.h`；普通端点入口不包含这些助手。
回调可提交或取消其他调用，不可递归 step、同步 close/reinit 本端点。
从 owner 安全点使用生成的 close，然后才释放 adapter；不能只关闭通用 handle。
`endpoint_<service>_sync(endpoint, request, response, timeout_ms)` 返回本次
`wl_rpc_completion_t`，成功才更新自持响应。UDP 自动安装就绪等待；其他平台初始化时
安装 `wl_waiter_t`，缺失则立即返回 FAILED/local NOT_SUPPORTED，不忙等。
本地提交/等待错误记录在 `local_error`，不混用业务拒绝或协议诊断。
sync 复用异步完成，返回后无回调访问其栈，不能从同一 owner 回调调用。
后台主机用生成的 `endpoint_driver()` 绑定 `wirelink::host::Executor`；业务线程
调用同一个 sync 入口通过有界代理提交，不直接推进端点。排队计入原超时，入队读取
同一端点时钟，所以 provider 必须线程安全。停止并 join 所有调用线程后才能销毁
endpoint/adapter/executor。线上格式不变。

ABI 25 提供可选 `endpoint_create(&pointer, &config, &allocator)`：pointer 初始为 NULL，
一次申请完整端点，内部初始化失败则配对释放。allocator 接收 context、size、alignment，
描述符复制，context 活到 destroy。`endpoint_destroy(&pointer)` 先 close/quiesce/通知，
再释放并置空；静态对象仍用 close。没有逐消息分配或隐式堆回退，可用 Wirelink 可选固定池。
后台执行器先 stop/join 所有 owner/调用线程，再 destroy；旧别名/句柄此后无效。
所有翻译单元的容量和对齐配置必须一致。自持业务值的副本不依赖这个对象生命周期。

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
搬运 bits。`string<MAX>`/`bytes<MAX>` 的 MAX 按编码字节计算：高级 codec 使用借用
view，普通自持值使用内嵌数组；两者都没有 heap/lock，也不改变线上字节。

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
}
```

RPC 请求、响应各自默认 `reliable`。有特殊需要才覆盖一个方向，例如
`request = HomeRequest @delivery(unreliable);`。属性属于绑定，不属于 schema 消息。
省略默认值、显式可靠属性、旧 `request_delivery`／`response_delivery` 属性生成相同的
代码、manifest 和标识。同一方向重复声明一律报错，即使值相同。LATEST／FIFO 仍显式指定策略。
这些属性在 ABI 20 期间作为语法扩展加入，时钟注入在 ABI 21 引入；当前配对为 ABI 29。
属性语法本身仍不改变编码字节，需使用配套提交。

三个编号／状态映射全部省略，即选择托管 RPC，`.wl` 只定义业务参数。
runtime 管理 20 字节前缀：零区分字节、版本、请求／响应类型、保留零、
大端 uint32 调用编号、大端 int32 状态和大端 uint64 原客户端 session。
ABI 26 使用元数据 v2，所有可靠性组合均须成对升级。成功响应带业务体，非零拒绝只带前缀。
默认端点生成 `*_call_t`、`*_result_t` 和回复 token，使用
`call/inspect/release/cancel/complete/reject`。端点发起／回复统一返回 `wl_rpc_err_t`，
普通 handler 无需创建通用 runtime result；详细失败保留在 `endpoint_result()`。
容量包含元数据，纯托管路径不再
分配用于注入字段的类型化编码暂存区，请求／响应直接写 TX／缓存。

接入已有 schema 时，可以显式写出 `request_operation_id`、`response_operation_id`、
`response_status` 三个映射，保持旧编码；只写部分会报错。托管与映射两种模式不能直接
互通，模式进入 profile identity，迁移需同步两端。仅 retained 策略和本地角色不同仍可共享 codec。
调用关联与有界重放不等于持久化业务幂等。本地 token 在 runtime 重建后应丢弃，
默认端点增加归属／代次检查；托管 v2 响应必须同时匹配本地 session 和调用编号。
自动编号耗尽会拒绝新调用，安全 close/reinit 后才恢复。映射 RPC 保留原新鲜度限制；
两者都不提供持久 exactly-once 或认证。



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

当前固定宏为 `<MODULE>_RUNTIME_CODEGEN_ABI_VERSION 26`；`wlc codegen-abi` 可直接查询。
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
client response 原始字节保留到 release；借用字段也只在此前有效。server 解码验证成功后，
复用普通编码器的规范化字段遍历，直接计算指纹；RPC domain 由 runtime 提供，codec 不承载
RPC 策略，也不直接哈希收到的原始字节。按 NEW/PENDING_DUPLICATE/REPLAY/CONFLICT
处理；complete/reject 先 cache 后 send。普通 owned handler 走生成器私有转换，避免再次
检查 UTF-8；公开视图转换仍完整检查，失败不改输出。owned 输出整体清零一次（含未用容量
和 padding），嵌套复制不重复清零。业务不承担新的缓冲或生命周期责任。

异步 completion 必须复制包含 peer session 的 identity。ABI 18 在可靠 request 前自动观察
session；切换清理旧工作并请求取消 detached response，`peer_changed` 和 take API 通知产品。
ABI 21 引入初始化时钟；ABI 26 使用 `wl_environment_t`，可覆盖 `config.environment.clock`，
step/call/complete/reject 不再传 `now_ms`。每轮取一次时间，立即回复复用它；
step 外可靠提交取一次供链路/RPC 共用，unreliable 端点发送不取时钟。
描述符复制，上下文借用到 close；核心不选择操作系统时钟。
codec binding 的 `*_send` 在 `delivery` 后新增 `now_ms`，unreliable 时忽略该值。

高级路径所有 `now_ms`/poll/hint 使用同一 monotonic ms clock。link ARQ、client deadline、server
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
