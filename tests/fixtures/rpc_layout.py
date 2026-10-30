# SPDX-License-Identifier: Apache-2.0
"""Exercise generated code using actual ELF linkers; keep maps and GC logs."""
from pathlib import Path
import json
import os
import shutil
import subprocess
import sys

root, output = map(lambda p: Path(p).resolve(), sys.argv[1:])


def run(args, cwd=None, ok=True):
    result = subprocess.run(list(map(str, args)), cwd=cwd, text=True, capture_output=True)
    if ok and result.returncode:
        raise AssertionError(f"{args}\n{result.stdout}\n{result.stderr}")
    return result


def sections(path):
    return run(["objdump", "-h", path]).stdout


def symbols(path):
    return run(["nm", "-S", path]).stdout


main = output / "main.c"
main.write_text('''#include "task_runtime.h"
wl_rpc_err_t (*volatile entry)(const task_runtime_t *, wl_time_ms_t, wl_rpc_deadline_hint_t *) = task_runtime_get_deadline_hint;
int main(void) { wl_rpc_deadline_hint_t hint; return entry(NULL, 0, &hint) == WL_RPC_ERR_INVALID_ARG ? 0 : 1; }
''')
dispatch = output / "dispatch.c"
dispatch.write_text('''#include "task_runtime.h"
task_runtime_result_t (*volatile entry)(wl_ctx_t *, const wl_event_t *, task_runtime_t *, wl_time_ms_t) = task_runtime_dispatch_event;
int main(void) { (void)entry(NULL, NULL, NULL, 0); return 0; }
''')
(output / "single.h").write_text('#define TASK_RPC_LAYOUT_task_runtime_get_deadline_hint __attribute__((noinline,section(".ram.task.hint")))\n')
(output / "module.h").write_text('#define TASK_RPC_LAYOUT(function) __attribute__((section(".flash.task." #function)))\n')
(output / "enum.h").write_text('enum { X = 1, Y = 0 };\n')
# Narrow host fragment and a complete freestanding MEMORY script with orphans
# rejected. The latter does not rely on the host's permissive default .text.*.
(output / "group.ld").write_text('SECTIONS { .rpc : { *(.literal.wlc.task.rpc.* .text.wlc.task.rpc.*) } } INSERT BEFORE .text;\n')
(output / "isolated.ld").write_text(f'SECTIONS {{ .rpc : {{ INCLUDE "{output / "isolated-inputs.ld"}" }} }} INSERT BEFORE .text;\n')
(output / "single.ld").write_text('SECTIONS { .ram.task : { *(.ram.task.hint) } } INSERT BEFORE .text;\n')
(output / "memory.ld").write_text('''ENTRY(_start)
MEMORY { FLASH (rx) : ORIGIN = 0x10000, LENGTH = 1M
         RAM (rwx) : ORIGIN = 0x200000, LENGTH = 1M }
SECTIONS {
 .text : { *(.text._start) *(.literal.wlc.task.rpc.* .text.wlc.task.rpc.*) *(.flash.task.*) *(.text .text.*) *(.iplt) } > FLASH
 .ram : { *(.ram.task.hint) } > RAM
 .rodata : { *(.rodata .rodata.*) } > FLASH
 .data : { *(.data .data.*) *(.got .got.*) *(.igot .igot.*) } > RAM
 .rela : { *(.rela.*) } > FLASH
 .bss (NOLOAD) : { *(.bss .bss.* COMMON) } > RAM
 .symtab 0 : { *(.symtab) }
 .shstrtab 0 : { *(.shstrtab) }
 .strtab 0 : { *(.strtab) }
 /DISCARD/ : { *(.comment .note.* .eh_frame .eh_frame_hdr .llvm_addrsig .gnu.*) }
}
''')
(output / "start.c").write_text('''#include "task_runtime.h"
wl_rpc_err_t (*volatile entry)(const task_runtime_t *, wl_time_ms_t, wl_rpc_deadline_hint_t *) = task_runtime_get_deadline_hint;
void _start(void) { wl_rpc_deadline_hint_t hint; (void)entry(NULL, 0, &hint); for (;;) {} }
''')
# Check the real CMake helper against the actual emitted sections, including
# the keyword escape in a legal C namespace.
(output / 'CMakeLists.txt').write_text(f'''cmake_minimum_required(VERSION 3.21)
project(rpc_layout_names C)
include("{root / 'cmake/WirelinkWlc.cmake'}")
wirelink_wlc_rpc_layout_fragment(MODULE task ISOLATE_LITERALS
 OUTPUT "${{CMAKE_CURRENT_SOURCE_DIR}}/isolated-inputs.ld")
foreach(mode IN ITEMS ON OFF)
 add_library(keyword_${{mode}} STATIC switch__runtime.c)
 target_include_directories(keyword_${{mode}} PRIVATE "{root / 'include'}")
 wirelink_wlc_configure_rpc_layout(TARGET keyword_${{mode}} MODULE switch_ ENABLE ${{mode}})
 get_target_property(value keyword_${{mode}} WIRELINK_WLC_RPC_LAYOUT_ENABLE)
 file(GENERATE OUTPUT "${{mode}}.value" CONTENT "${{value}}")
endforeach()
''')
report = []
rejections = []
for cc in os.environ.get("WLC_ELF_COMPILERS", "gcc clang").split():
    if not shutil.which(cc):
        raise AssertionError(f"Required compiler missing: {cc}")
    for lto in (False, True):
        directory = output / f"{cc}-{'lto' if lto else 'plain'}"
        directory.mkdir(exist_ok=True)
        flags = [cc, '-std=c11', '-O2', '-fno-inline', '-ffunction-sections', '-fdata-sections', '-fno-asynchronous-unwind-tables', '-fno-stack-protector', '-Wall', '-Wextra', '-Wpedantic', '-Werror', '-I', root / 'include', '-I', output]
        # Existing scalar-codec inline fallbacks are intentionally unused in
        # this tiny schema; Clang diagnoses them even without layout markers.
        if 'clang' in cc:
            flags += ['-Wno-unused-function']
        link = [cc]
        if lto:
            flags += ['-flto']
            link += ['-flto']
            if 'clang' in cc:
                link += ['-fuse-ld=lld']
        core = []
        for source in sorted((root / 'src').glob('*.c')):
            obj = directory / f'{source.stem}.o'
            run([*flags, '-c', source, '-o', obj])
            core.append(obj)
        for source in ['task.c', 'task_bindings.c']:
            obj = directory / f'{Path(source).stem}.o'
            run([*flags, '-c', output / source, '-o', obj])
            core.append(obj)
        run(['ar', 'rcs', directory / 'core.a', *core])
        cmake = directory / 'cmake-names'
        run(['cmake', '-S', output, '-B', cmake,
             '-DCMAKE_C_COMPILER=' + shutil.which(cc),
             '-DCMAKE_C_FLAGS=-std=c11 -O2 -ffunction-sections -fdata-sections'])
        run(['cmake', '--build', cmake, '--parallel', '2'])
        for mode, value in [('ON', '1'), ('OFF', '0')]:
            assert (cmake / f'{mode}.value').read_text() == value
            actual = sections(cmake / f'libkeyword_{mode}.a')
            assert ('.text.wlc.switch_.rpc.' in actual) == (value == '1'), actual
        for label, config, enabled, hook in [
            ('ordinary', [], True, None),
            ('ordinary-off', ['-DTASK_RPC_LAYOUT_ENABLE=0'], False, None),
            ('default', [], True, None),
            ('isolated', [], True, None),
            ('isolated-off', ['-DTASK_RPC_LAYOUT_ENABLE=0'], False, None),
            ('without-function-sections', ['-fno-function-sections'], True, None),
            ('global-off', ['-DWLC_RPC_LAYOUT_ENABLE=0'], False, None),
            ('module-off', ['-DTASK_RPC_LAYOUT_ENABLE=0'], False, None),
            ('module-on', ['-DWLC_RPC_LAYOUT_ENABLE=0', '-DTASK_RPC_LAYOUT_ENABLE=1'], True, None),
            ('module-wins', ['-DWLC_RPC_LAYOUT_ENABLE=1', '-DTASK_RPC_LAYOUT_ENABLE=0'], False, None),
            ('global-alias-on', ['-DWLC_RPC_LAYOUT_ENABLE=X', '-DX=Y', '-DY=1'], True, None),
            ('global-alias-off', ['-DWLC_RPC_LAYOUT_ENABLE=X', '-DX=Y', '-DY=0'], False, None),
            ('module-alias-on', ['-DTASK_RPC_LAYOUT_ENABLE=X', '-DX=Y', '-DY=1'], True, None),
            ('module-alias-off', ['-DTASK_RPC_LAYOUT_ENABLE=X', '-DX=Y', '-DY=0'], False, None),
            ('non-elf', ['-U__ELF__'], False, None),
            ('single', ['-include', output / 'single.h'], True, 'single'),
            ('module-hook', ['-include', output / 'module.h'], True, 'module'),
            ('off-hook', ['-DWLC_RPC_LAYOUT_ENABLE=0', '-include', output / 'single.h'], False, None),
        ]:
            case = directory / label
            case.mkdir(exist_ok=True)
            runtime = case / 'runtime.o'
            run([*flags, *config, '-c', output / 'task_runtime.c', '-o', runtime])
            preprocessed = run([*flags, *config, '-E', output / 'task_runtime.c']).stdout
            assert ('.text.wlc.task.rpc.' in preprocessed) == (enabled and hook != 'module')
            if not enabled:
                assert 'section(' not in preprocessed and '.ram.task.hint' not in preprocessed
            macros = run([*flags, *config, '-dM', '-E', output / 'task_runtime.c']).stdout
            assert '#define WLC_TASK_RPC_LAYOUT_' not in macros, macros
            if not config:
                assert '#define WLC_RPC_LAYOUT_ENABLE' not in macros
                assert '#define TASK_RPC_LAYOUT' not in macros
            if not lto:
                objsections = sections(runtime)
                assert (' .text.wlc.task.rpc.' in objsections) == (enabled and hook != 'module')
                assert ' .text.wlc.task.rpc ' not in objsections
                if enabled and hook != 'module':
                    assert '.text.wlc.task.rpc.task_runtime_dispatch_event' in objsections
                    if hook != 'single':
                        assert '.text.wlc.task.rpc.task_runtime_get_deadline_hint' in objsections
            run(['ar', 'rcs', case / 'runtime.a', runtime])
            script = ['-Wl,-T,' + str(output / ('single.ld' if hook == 'single' else 'group.ld'))] if hook != 'module' else []
            if label.startswith('ordinary'):
                script = []
            if label.startswith('isolated'):
                script = ['-Wl,-T,' + str(output / 'isolated.ld')]
            for consumer in ('main', 'dispatch'):
                binary = case / consumer
                result = run([*link, '-I', root / 'include', '-I', output, output / f'{consumer}.c', case / 'runtime.a', directory / 'core.a', '-Wl,--gc-sections,--print-gc-sections', '-Wl,-Map,' + str(case / f'{consumer}.map'), *script, '-o', binary])
                (case / f'{consumer}-gc.log').write_text(result.stderr)
                run([binary])
                names = symbols(binary)
                (case / f'{consumer}-nm.txt').write_text(names)
                if consumer == 'main':
                    assert 'task_runtime_get_deadline_hint' in names
                    assert 'task_runtime_dispatch_event' not in names
                    assert 'task_runtime_service' not in names
                    assert 'task_query_server_finish' not in names
                    assert 'wl_rpc_server_begin' not in names
                else:
                    assert 'task_runtime_dispatch_event' in names
                    # Real dispatcher relocations keep the server admission code.
                    assert 'wl_rpc_server_begin' in names
                if hook == 'single' and consumer == 'main':
                    table = run(['objdump', '-t', binary]).stdout
                    placed = [line for line in table.splitlines() if '.ram.task' in line and ' F ' in line]
                    assert len(placed) == 1 and 'task_runtime_get_deadline_hint' in placed[0], placed
            # Full region script, without libc, with strict orphan handling.
            memory = case / 'memory.elf'
            result = run([*flags, output / 'start.c', case / 'runtime.a', directory / 'core.a', *(['-flto', '-fuse-ld=lld'] if lto and 'clang' in cc else []), '-nostdlib', '-static', '-Wl,--build-id=none,--gc-sections,--orphan-handling=error', '-Wl,-T,' + str(output / 'memory.ld'), '-Wl,-Map,' + str(case / 'memory.map'), '-o', memory])
            report.append({'compiler': cc, 'lto': lto, 'case': label, 'layout': enabled, 'elf_sha256': __import__('hashlib').sha256((case / 'main').read_bytes()).hexdigest(), 'size': run(['size', case / 'main']).stdout})
        for invalid in ('2', '-1', 'on', '', '4/2', '2-1', '1+0', '0+1',
                        '(1)', '00', '01', '1u', '0x1', '1||0', '1/**/+0',
                        'X', 'Y'):
            for macro in ('WLC_RPC_LAYOUT_ENABLE', 'TASK_RPC_LAYOUT_ENABLE'):
                # C enum constants are deliberately visible to the C compiler;
                # accepting X=1 here would silently make the preprocessor OFF.
                config = ['-include', output / 'enum.h'] if invalid in ('X', 'Y') else []
                result = run([*flags, *config, f'-D{macro}={invalid}', '-c', output / 'task_runtime.c', '-o', directory / 'invalid.o'], ok=False)
                assert result.returncode and 'RPC_LAYOUT' in result.stderr, result.stderr
                rejections.append({'compiler': cc, 'lto': lto, 'macro': macro,
                                   'value': invalid, 'stderr': result.stderr})
(output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
(output / 'rejections.json').write_text(json.dumps(rejections, indent=2) + '\n')
print(f'Passed {len(report)} archive/ELF cases and {len(rejections)} invalid settings, including strict region scripts, GC, hooks and enum constants')
