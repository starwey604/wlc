# RPC ELF layout

Generated runtime C defaults to `WLC_RPC_LAYOUT_ENABLE=1`. A module such as
`task` inherits this setting unless `TASK_RPC_LAYOUT_ENABLE=0` or `1` is defined.
Layout macro prefixes uppercase the emitted C module name verbatim, preserving
underscores: runtime `switch_` uses `SWITCH__RPC_LAYOUT_ENABLE`. Use that same
C module name as the CMake helper's `MODULE` argument.
After macro expansion, only the single token `0` or `1` is accepted; macro
aliases expanding to those tokens are valid. C enum constants (including
`enum { X = 1 };` with the setting `X`) and expressions such as `4/2` are
rejected, so validation and the preprocessor select the same layout.
This is a build choice: API, wire bytes,
schema and binding identities, replay, result ownership and threading contracts
are unchanged. The codegen contract remains 0.12 and the SDK binding API remains 2.

On ELF GCC/Clang, selected RPC helpers and runtime dispatch/progress entries get
separate `.text.wlc.task.rpc.<function>` input sections. Static helpers, inline
out-of-line copies and their declarations follow the same policy. Compiler
clones may share their originating function's section; inlining can remove an
out-of-line body. Nothing uses `used`, `retain` or a group-wide `KEEP`.
Linker GC follows actual relocations: a dispatcher can keep methods reachable
that the application has not yet called. Other object formats/toolchains emit
ordinary code. The generated C does not include platform headers.

## Build configuration

Apply the setting when compiling **runtime.c**. The same generated files support
both choices, without regeneration:

```sh
cc -DWLC_RPC_LAYOUT_ENABLE=0 -c task_runtime.c # all generated modules default OFF
cc -DWLC_RPC_LAYOUT_ENABLE=0 -DTASK_RPC_LAYOUT_ENABLE=1 -c task_runtime.c
```

CMake (including its use from Zephyr applications) defaults to
`WIRELINK_WLC_RPC_LAYOUT_ENABLE=ON`. Override a runtime independently:

```cmake
wirelink_wlc_generate_runtime(TARGET task_protocol CODEC_TARGET task_codec
  PROFILE task.bind.wl RPC_LAYOUT OFF)
# For a manually created target that compiles checked-in generated C:
wirelink_wlc_configure_rpc_layout(TARGET task_protocol MODULE task ENABLE OFF)
```

The target's `WIRELINK_WLC_RPC_LAYOUT_ENABLE` property reports the configured
numeric value. Do not configure the same module on one target twice. Helpers
set a private module definition, so other modules can choose independently.
Zephyr's global application `CONFIG_COMPILER_OPT`/caller flags do not change an
archive that has already been built. Direct-source Zephyr builds can instead
set `WLC_RPC_LAYOUT_ENABLE` on their actual runtime target.

Generated SDK projects offer `<NAME>_RPC_LAYOUT_ENABLE` (for example
`CALCULATOR_RPC_LAYOUT_ENABLE`), inheriting the CMake global default. Reconfigure
and rebuild the SDK archive or wheel to change it. Defining a macro in an
application linking a prebuilt SDK cannot change that SDK. Installed CMake
packages report `<Package>_RPC_LAYOUT_ENABLE`; `share/<name>/wlc-sdk-build.json`
and the wheel's package-local `wlc-sdk-build.json` record the actual build
selection. The source provenance manifest describes generated files, not the
selection of a future native build. Check final ELF/map and hashes when
identifying a firmware; a requested flag alone does not prove placement.

## Flash and final ordering

With a normal `.text.*` script, ON inputs stay in ordinary executable Flash (or
whatever region that script assigns). Marking does not promise RAM residency,
cache performance, or a particular final order. OFF emits no WLC section
attributes, including on private helpers and forward declarations; it requires
no WLC linker fragment and obeys the compiler/user's function-section options.

For a GNU-compatible host ELF linker, an optional module-scoped helper groups
surviving inputs without changing their GC granularity:

```cmake
wirelink_wlc_group_rpc(TARGET my_app MODULE task)
```

Zephyr can register the same module-scoped inputs at a platform insertion point:

```cmake
wirelink_wlc_group_rpc(TARGET app MODULE task LOCATION ROM_START)
```

On Xtensa GNU BFD, later literal pools can migrate ahead of the RPC group.
An application can bound literal colocation to the module:

```cmake
wirelink_wlc_group_rpc(TARGET app MODULE task LOCATION ROM_START ISOLATE_LITERALS)
# Or use the same boundary inside a custom MEMORY script:
wirelink_wlc_rpc_layout_fragment(MODULE task ISOLATE_LITERALS
  OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/task-rpc.ld")
```

This surrounds the inputs with `. = .;`, zero-size layout boundaries. It adds no fixed address,
padding or retention and preserves separate function inputs. It does not
force declaration order within the group. Select and measure literal scope
for each application ELF; one choice need not suit both endpoints. Keep the
same application policy across CPU-statistics builds and measure the actual
pair before accepting latency or CPU results. Other GNU-compatible ELF
linkers, including LLD, accept the same fragment syntax.

The tested ESP32-S3 multi/fixed applications use explicitly recorded placement
policies for each application and existing framing profile, unchanged between
CPU-statistics builds. Multi client input order uses GNU BFD filename sorting;
LLD does not support that recipe. It is not a generic helper default. See the
[Round 3.5 results](../../docs/wlc-elf-layout-round-3-5-result-cn.md) for the exact
policy, three alternating rounds, OFF costs and retained failures. The default
markers alone do not imply that an unmodified platform script meets those
performance gates.

The platform must include `snippets-rom-start.ld` inside executable Flash before
its text catchall. ESP32-S3 requires the optional
`adapters/zephyr/patches/esp32s3-rpc-layout.patch` for this insertion point.
Registering a fragment alone does not prove the platform consumes it; check the
final map. The helper does not rewrite a platform's `SECTIONS` or memory map.

For a custom embedded `MEMORY` script, generate an input fragment and include it
inside the desired executable output section, **before** its `.text.*` and
`.literal.*` catchalls:

```cmake
wirelink_wlc_rpc_layout_fragment(MODULE task OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/task-rpc.ld")
```

```ld
.text : {
  INCLUDE task-rpc.ld
  *(.text .text.*)
} > FLASH
```

The fragment selects only `.literal.wlc.task.rpc.*` and `.text.wlc.task.rpc.*`;
it supplies no address, padding, retention, region or complete `SECTIONS`
replacement. A Zephyr platform must use a linker insertion point **before** its
text catchall. Do not append the fragment to `SECTIONS` after inputs were already
consumed, or put executable code into a data-mapped RODATA region. Platforms
without such a hook can use ordinary `.text.*` placement and inspect the map,
or add a platform linker integration. The unpatched ESP32-S3 script uses ordinary `.text.*` placement. Round 3.5
records ordinary, literal-locality and explicit-grouping experiments separately.

Xtensa literal reachability belongs to platform build/link configuration.
Where supported by the actual GCC, a platform may apply
`-mtext-section-literals` to its runtime target. This remains a platform choice,
not a default generated-runtime compile option. Round 3.5 records its separate
measurement and does not assume that it improves every layout. No generated C recognizes Xtensa
or a vendor memory macro. Check function/literal addresses and linker support
on the target, including under LTO.

## Overrides

Supply hooks through a configuration header passed with `-include` **to the
runtime compilation**. The module hook replaces the default marker for selected
RPC functions. A function hook takes precedence and can also place a cold
runtime entry. Declaration and definition use the same resolved hook, avoiding
conflicting stacked section attributes. Hooks act only when the module is ON
and the ELF attribute backend is supported. OFF suppresses all layout hooks.
Generated default/internal macros are cleaned up at the end of the C file;
caller-provided configuration remains intact. Layout configuration names are
reserved against schema symbol collisions. The RPC runtime namespace `wlc` is
also reserved: its module switch would collide with the global switch.

Ordinary Flash, or an explicit per-function RAM placement:

```c
/* Optional module-wide ordinary Flash marker. */
#define TASK_RPC_LAYOUT(function) \
  __attribute__((section(".text.my_task." #function)))
/* Only this function goes to the user-selected input section. */
#define TASK_RPC_LAYOUT_task_runtime_get_deadline_hint \
  __attribute__((noinline, section(".ram.task.hint")))
```

```ld
/* In a platform script with the appropriate RAM loading/copy mechanism. */
.ram_code : { *(.ram.task.hint) } > RAM AT > FLASH
```

The platform must copy/initialize that region as needed. Callees, literals and
constant data do not automatically share the function's residency; inspect
relocations, clones and final map. A hook can add the platform's supported
no-inline/no-clone policy when residency requires a separate function body.

`cargo test --test rpc_layout` checks real static archives and final ELF/maps
with GCC and Clang, LTO on/off, ordinary and strict region scripts, module and
function overrides, GC, invalid settings and the non-ELF fallback. These tests
measure linker behavior; target performance requires separate board captures.
