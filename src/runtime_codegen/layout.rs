// SPDX-License-Identifier: Apache-2.0
//! Mark independently collectable ELF inputs; final ordering belongs to the
//! platform linker. Hooks resolve before each declaration/definition.
use crate::profile_semantic::BindingProfileModel;
use std::fmt::Write;

// MODULE is the emitted C namespace, not a schema name. Preserve all its
// underscores so CMake and direct source consumers use exactly the same macro.
pub(super) fn prefix(module: &str) -> String {
    module.to_ascii_uppercase()
}

pub(super) fn mark_rpc(source: String, module: &str, profile: &BindingProfileModel) -> String {
    let prefix = prefix(module);
    let private = format!("{module}_rpc_");
    let hot = [
        "dispatch_event",
        "get_deadline_hint",
        "poll",
        "readiness",
        "service",
    ]
    .map(|suffix| format!("{module}_runtime_{suffix}"));
    let helpers = profile
        .rpc_services
        .iter()
        .filter(|s| s.is_managed())
        .flat_map(|s| {
            let name = crate::codegen::c_identifier(&s.name);
            [
                format!("{module}_{name}_server_finish"),
                format!("{module}_{name}_encode_response"),
            ]
        })
        .collect::<Vec<_>>();
    let mut out = crate::template::render(
        include_str!("layout.c.in"),
        &[("PREFIX", &prefix), ("MODULE", module)],
    );
    for line in source.split_inclusive('\n') {
        if !line.starts_with(char::is_whitespace)
            && !line.starts_with('#')
            && let Some((declaration, _)) = line.split_once('(')
        {
            let name = declaration
                .split_whitespace()
                .last()
                .unwrap_or("")
                .trim_start_matches('*');
            if name.starts_with(&format!("{module}_"))
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                // Cold entries also accept a per-function override. Forward
                // declarations take exactly the same path as definitions.
                writeln!(out, "#if WLC_{prefix}_RPC_LAYOUT_ACTIVE && defined({prefix}_RPC_LAYOUT_{name})\n{prefix}_RPC_LAYOUT_{name}").unwrap();
                if (line.starts_with("static ")
                    && (name.starts_with(&private) || helpers.iter().any(|f| f == name)))
                    || hot.iter().any(|f| f == name)
                {
                    writeln!(
                        out,
                        "#elif WLC_{prefix}_RPC_LAYOUT_ACTIVE\n{prefix}_RPC_LAYOUT({name})"
                    )
                    .unwrap();
                }
                out.push_str("#endif\n");
            }
        }
        out.push_str(line);
    }
    writeln!(out, "\n#undef WLC_{prefix}_RPC_LAYOUT_ACTIVE\n#ifdef WLC_{prefix}_RPC_LAYOUT_DEFAULT\n#undef {prefix}_RPC_LAYOUT\n#undef WLC_{prefix}_RPC_LAYOUT_DEFAULT\n#endif\n#ifdef WLC_{prefix}_RPC_LAYOUT_MODULE_DEFAULT\n#undef {prefix}_RPC_LAYOUT_ENABLE\n#undef WLC_{prefix}_RPC_LAYOUT_MODULE_DEFAULT\n#endif\n#ifdef WLC_{prefix}_RPC_LAYOUT_GLOBAL_DEFAULT\n#undef WLC_RPC_LAYOUT_ENABLE\n#undef WLC_{prefix}_RPC_LAYOUT_GLOBAL_DEFAULT\n#endif").unwrap();
    out
}
