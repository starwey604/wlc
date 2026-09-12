// SPDX-License-Identifier: Apache-2.0
use std::{collections::BTreeMap, fmt::Write};

use super::{
    BINDING_API_VERSION,
    plan::{Plan, count, enum_name, identifier, optional, python_string, scalar_cpp, type_name},
};
use crate::semantic::{FieldDefault, FieldSymbol, ResolvedType};

pub(super) fn emit(plan: &Plan<'_>, files: &mut BTreeMap<String, String>) {
    let package = &plan.package;
    let name = &plan.options.name;
    files.insert(
        format!("python/{package}/_runtime.py"),
        include_str!("templates/runtime.py").into(),
    );
    files.insert(format!("python/{package}/py.typed"), String::new());
    let mut py = format!(
        "# SPDX-License-Identifier: Apache-2.0\n\"\"\"Generated Wirelink SDK with synchronous and asyncio clients. Message data outlives the connection.\"\"\"\nfrom __future__ import annotations\n\nimport builtins as _builtins\nfrom dataclasses import dataclass as _dataclass\nfrom types import TracebackType as _TracebackType\nfrom typing import Any as _Any\nfrom . import _native, _runtime\nfrom ._runtime import (Udp, WirelinkError, ClosedError, RpcTimeoutError, CancelledError,\n    RejectedError, QueueFullError, TransportError, CodecError, InvalidArgumentError)\n\n__version__ = {}\ncore_version: str = _native.core_version\ncodegen_abi: int = _native.codegen_abi\nbinding_api: int = _native.binding_api\n",
        python_string(&plan.options.package_version)
    );
    for enumeration in &plan.enums {
        writeln!(
            py,
            "\n\nclass {}(_runtime.OpenIntEnum):",
            type_name(&enumeration.name)
        )
        .unwrap();
        for value in &enumeration.values {
            writeln!(py, "    {} = {}", enum_name(&value.name), value.number).unwrap();
        }
    }
    for message in &plan.messages {
        let ty = type_name(&message.name);
        writeln!(py, "\n\n@_dataclass(frozen=True, slots=True)\nclass {ty}:").unwrap();
        // Required fields first keeps positional construction useful and permits
        // an optional lower-numbered field before a required one in the schema.
        for field in message
            .fields
            .iter()
            .filter(|f| !optional(f))
            .chain(message.fields.iter().filter(|f| optional(f)))
        {
            writeln!(
                py,
                "    {}: {}{}",
                identifier(&field.name),
                field_python(field),
                if optional(field) { " = None" } else { "" }
            )
            .unwrap();
        }
        py.push_str("\n    def __post_init__(self) -> None:\n");
        if message.fields.is_empty() {
            py.push_str("        pass\n");
        }
        for field in &message.fields {
            let f = identifier(&field.name);
            let indent = if optional(field) {
                writeln!(py, "        if self.{f} is not None:").unwrap();
                "            "
            } else {
                "        "
            };
            writeln!(
                py,
                "{indent}object.__setattr__(self, \"{f}\", {})",
                validate(field, &format!("self.{f}"))
            )
            .unwrap();
        }
        for field in &message.fields {
            if field.default.is_some() {
                let f = identifier(&field.name);
                writeln!(py, "\n    @_builtins.property\n    def {f}_or_default(self) -> {}:\n        return {} if self.{f} is None else self.{f}", scalar_python(&field.ty), default_python(field)).unwrap();
            }
        }
        py.push_str("\n    def _to_native(self) -> tuple[_Any, ...]:\n        return (");
        for field in &message.fields {
            let expr = format!("self.{}", identifier(&field.name));
            write!(py, "{}, ", py_to_native(field, &expr)).unwrap();
        }
        py.push_str(")\n");
        writeln!(py, "\n    @_builtins.classmethod\n    def _from_native(cls, value: tuple[_Any, ...]) -> {ty}:\n        return cls(").unwrap();
        for (i, field) in message.fields.iter().enumerate() {
            writeln!(
                py,
                "            {}={},",
                identifier(&field.name),
                py_from_native(field, &format!("value[{i}]"))
            )
            .unwrap();
        }
        py.push_str("        )\n");
    }
    py.push_str(include_str!("templates/client.py.in"));
    for rpc in &plan.profile.rpc_services {
        let method = identifier(&rpc.name);
        let req = type_name(&rpc.request_name);
        let resp = type_name(&rpc.response_name);
        write!(py, "\n    def {method}(self, *, ").unwrap();
        for field in &plan.message(rpc.request_id).fields {
            write!(
                py,
                "{}: {}{}, ",
                identifier(&field.name),
                field_python(field),
                if optional(field) { " = None" } else { "" }
            )
            .unwrap();
        }
        writeln!(
            py,
            "timeout: float = 1.0) -> {resp}:\n        return self.{method}_request({req}("
        )
        .unwrap();
        for field in &plan.message(rpc.request_id).fields {
            let f = identifier(&field.name);
            writeln!(py, "            {f}={f},").unwrap();
        }
        writeln!(py, "        ), timeout=timeout)\n\n    def {method}_request(self, request: {req}, *, timeout: float = 1.0) -> {resp}:\n        if not isinstance(request, {req}):\n            raise TypeError(\"request must be {req}\")\n        value, error = self._client.{method}(request._to_native(), _runtime.timeout_ms(timeout))\n        if error is not None:\n            _runtime._raise(error)\n        assert value is not None\n        return {resp}._from_native(value)").unwrap();
    }
    py.push_str(include_str!("templates/async_client.py.in"));
    for rpc in &plan.profile.rpc_services {
        let method = identifier(&rpc.name);
        let req = type_name(&rpc.request_name);
        let resp = type_name(&rpc.response_name);
        write!(py, "\n    async def {method}(self, *, ").unwrap();
        for field in &plan.message(rpc.request_id).fields {
            write!(
                py,
                "{}: {}{}, ",
                identifier(&field.name),
                field_python(field),
                if optional(field) { " = None" } else { "" }
            )
            .unwrap();
        }
        writeln!(
            py,
            "timeout: float = 1.0) -> {resp}:\n        return await self.{method}_request({req}("
        )
        .unwrap();
        for field in &plan.message(rpc.request_id).fields {
            let f = identifier(&field.name);
            writeln!(py, "            {f}={f},").unwrap();
        }
        writeln!(py, "        ), timeout=timeout)\n\n    async def {method}_request(self, request: {req}, *, timeout: float = 1.0) -> {resp}:\n        if not isinstance(request, {req}):\n            raise TypeError(\"request must be {req}\")\n        return await self._invoke(self._client.{method}_async, request._to_native(),\n                                  {resp}._from_native, _runtime.timeout_ms(timeout))").unwrap();
    }
    py.push_str("\n\n__all__ = [\n    \"Client\", \"AsyncClient\", \"Udp\", \"WirelinkError\", \"ClosedError\", \"RpcTimeoutError\", \"CancelledError\",\n    \"RejectedError\", \"QueueFullError\", \"TransportError\", \"CodecError\", \"InvalidArgumentError\",\n    \"core_version\", \"codegen_abi\", \"binding_api\", \"__version__\",\n");
    for ty in plan
        .enums
        .iter()
        .map(|e| &e.name)
        .chain(plan.messages.iter().map(|m| &m.name))
    {
        writeln!(py, "    {},", python_string(&type_name(ty))).unwrap();
    }
    py.push_str("]\n");
    files.insert(format!("python/{package}/__init__.py"), py);

    let mut native = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n// Generated by WLC. Python headers precede all system headers.\n#include <nanobind/nanobind.h>\n#include <nanobind/stl/array.h>\n#include <nanobind/stl/string.h>\n#include <{name}/client.hpp>\n#include <wirelink/version.h>\nnamespace nb = nanobind;\nusing namespace {name};\n\nnamespace {{\n// Keep nanobind/Python reference ownership off the native owner thread.\nstruct wlc_python_signal_handle {{\n  std::shared_ptr<wirelink::CompletionSignal> value = std::make_shared<wirelink::CompletionSignal>();\n  bool wait() {{ return value->wait(); }}\n  void stop() noexcept {{ value->stop(); }}\n}};\n"
    );
    for message in &plan.messages {
        let ty = type_name(&message.name);
        writeln!(native, "[[maybe_unused]] {ty} read_{ty}(nb::handle object) {{\n  const auto input = nb::cast<nb::tuple>(object);\n  if (input.size() != {}) nb::raise_type_error(\"invalid {ty} tuple length\");\n  {ty} output{{}};", message.fields.len()).unwrap();
        for (i, field) in message.fields.iter().enumerate() {
            let f = identifier(&field.name);
            if optional(field) {
                writeln!(native, "  if (!input[{i}].is_none()) {{").unwrap();
            }
            writeln!(
                native,
                "    output.{f} = {};",
                native_read(field, &format!("input[{i}]"))
            )
            .unwrap();
            if optional(field) {
                native.push_str("  }\n");
            }
        }
        writeln!(native, "  return output;\n}}\n\n[[maybe_unused]] nb::tuple write_{ty}(const {ty}& input) {{\n  (void)input;\n  return nb::make_tuple(").unwrap();
        for (i, field) in message.fields.iter().enumerate() {
            let expr = format!("input.{}", identifier(&field.name));
            writeln!(
                native,
                "      {}{}",
                native_write(field, &expr),
                if i + 1 == message.fields.len() {
                    ""
                } else {
                    ","
                }
            )
            .unwrap();
        }
        native.push_str("  );\n}\n\n");
    }
    native.push_str("} // namespace\n\nNB_MODULE(_native, module) {\n  module.attr(\"core_version\") = WIRELINK_VERSION_STRING;\n");
    writeln!(native, "  module.attr(\"codegen_abi\") = {};\n  module.attr(\"binding_api\") = {BINDING_API_VERSION};", crate::CODEGEN_ABI_VERSION).unwrap();
    native.push_str(include_str!("templates/error.cpp.in"));

    native.push_str("  module.def(\"closed_error\", [] { return wirelink::Error::local(WL_ERR_NOT_INITIALIZED); });\n  module.def(\"queue_full_error\", [] { return wirelink::Error::local(WL_ERR_QUEUE_FULL); });\n  nb::class_<wlc_python_signal_handle>(module, \"CompletionSignal\")\n      .def(nb::init<>())\n      .def(\"wait\", &wlc_python_signal_handle::wait, nb::call_guard<nb::gil_scoped_release>())\n      .def(\"stop\", &wlc_python_signal_handle::stop);\n");
    for message in &plan.messages {
        if !plan
            .profile
            .rpc_services
            .iter()
            .any(|rpc| rpc.response_id == message.id)
        {
            continue;
        }
        let resp = type_name(&message.name);
        writeln!(native, "  nb::class_<wirelink::Operation<{resp}>>(module, \"Operation{resp}\")\n      .def_prop_ro(\"done\", &wirelink::Operation<{resp}>::done)\n      .def(\"cancel\", &wirelink::Operation<{resp}>::cancel)\n      .def(\"notify_on_completion\", [](const wirelink::Operation<{resp}>& operation, const wlc_python_signal_handle& signal) {{ operation.notify_on_completion(signal.value); }})\n      .def(\"result\", [](const wirelink::Operation<{resp}>& operation) {{\n        auto result = [&] {{\n          nb::gil_scoped_release release;\n          return operation.result();\n        }}();\n        if (!result) return nb::make_tuple(nb::none(), wirelink::Error(result.error()));\n        return nb::make_tuple(write_{resp}(result.value()), nb::none());\n      }});\n").unwrap();
    }

    writeln!(native, "\n  nb::class_<{name}::Client>(module, \"Client\")\n      .def(\"close\", &{name}::Client::close, nb::call_guard<nb::gil_scoped_release>())\n      .def_prop_ro(\"is_open\", &{name}::Client::is_open)\n      .def_prop_ro(\"local_port\", &{name}::Client::local_port)").unwrap();
    for rpc in &plan.profile.rpc_services {
        let method = identifier(&rpc.name);
        let req = type_name(&rpc.request_name);
        let resp = type_name(&rpc.response_name);
        writeln!(native, "      .def(\"{method}\", []({name}::Client& client, nb::tuple request, std::int64_t timeout_ms) {{\n        auto owned = read_{req}(request);\n        auto result = [&] {{\n          nb::gil_scoped_release release;\n          return client.{method}(owned, std::chrono::milliseconds(timeout_ms));\n        }}();\n        if (!result) return nb::make_tuple(nb::none(), wirelink::Error(result.error()));\n        return nb::make_tuple(write_{resp}(result.value()), nb::none());\n      }})").unwrap();
    }
    for rpc in &plan.profile.rpc_services {
        let method = identifier(&rpc.name);
        let req = type_name(&rpc.request_name);
        writeln!(native, "      .def(\"{method}_async\", []({name}::Client& client, nb::tuple request, std::int64_t timeout_ms) {{\n        auto owned = read_{req}(request);\n        auto result = [&] {{\n          nb::gil_scoped_release release;\n          return client.{method}_async(owned, std::chrono::milliseconds(timeout_ms));\n        }}();\n        if (!result) return nb::make_tuple(nb::none(), wirelink::Error(result.error()));\n        return nb::make_tuple(std::move(result).value(), nb::none());\n      }})").unwrap();
    }
    writeln!(native, "      ;\n  module.def(\"connect\", [](const std::string& peer_address, std::uint16_t peer_port,\n                           const std::string& bind_address, std::uint16_t bind_port) {{\n    auto result = [&] {{\n      nb::gil_scoped_release release;\n      return {name}::Client::connect({{peer_address, peer_port, bind_address, bind_port}});\n    }}();\n    if (!result) return nb::make_tuple(nb::none(), wirelink::Error(result.error()));\n    return nb::make_tuple(std::move(result).value(), nb::none());\n  }});\n}}").unwrap();
    files.insert("src/python.cpp".into(), native);
    let mut stub = include_str!("templates/native.pyi.in").to_owned();
    for rpc in &plan.profile.rpc_services {
        writeln!(stub, "    def {}(self, request: tuple[Any, ...], timeout_ms: int) -> tuple[tuple[Any, ...] | None, Error | None]: ...", identifier(&rpc.name)).unwrap();
    }
    for rpc in &plan.profile.rpc_services {
        writeln!(stub, "    def {}_async(self, request: tuple[Any, ...], timeout_ms: int) -> tuple[Operation{} | None, Error | None]: ...", identifier(&rpc.name), type_name(&rpc.response_name)).unwrap();
    }
    stub.push_str("\ndef closed_error() -> Error: ...\ndef queue_full_error() -> Error: ...\n\nclass CompletionSignal:\n    def __init__(self) -> None: ...\n    def wait(self) -> bool: ...\n    def stop(self) -> None: ...\n");
    for message in &plan.messages {
        if plan
            .profile
            .rpc_services
            .iter()
            .any(|rpc| rpc.response_id == message.id)
        {
            writeln!(stub, "\nclass Operation{}:\n    @property\n    def done(self) -> bool: ...\n    def cancel(self) -> bool: ...\n    def notify_on_completion(self, signal: CompletionSignal) -> None: ...\n    def result(self) -> tuple[tuple[Any, ...] | None, Error | None]: ...", type_name(&message.name)).unwrap();
        }
    }
    files.insert(format!("python/{package}/_native.pyi"), stub);
}

fn scalar_python(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Bool => "bool".into(),
        ResolvedType::Bytes => "bytes".into(),
        ResolvedType::String => "str".into(),
        ResolvedType::Float32 | ResolvedType::Float64 => "float".into(),
        ResolvedType::Message { name, .. } | ResolvedType::Enum { name, .. } => type_name(name),
        _ => "int".into(),
    }
}

fn field_python(field: &FieldSymbol) -> String {
    let mut ty = scalar_python(&field.ty);
    if count(field).is_some() {
        ty = format!("tuple[{ty}, ...]");
    }
    if optional(field) {
        ty.push_str(" | None");
    }
    ty
}

fn validate(field: &FieldSymbol, value: &str) -> String {
    let name = python_string(&identifier(&field.name));
    if let Some(n) = count(field) {
        return format!(
            "tuple({} for element in _runtime.array({value}, {name}, {n}))",
            validate_scalar(field, "element", &name)
        );
    }
    validate_scalar(field, value, &name)
}

fn validate_scalar(field: &FieldSymbol, value: &str, name: &str) -> String {
    let (minimum, maximum) = match &field.ty {
        ResolvedType::Bool => return format!("_runtime.boolean({value}, {name})"),
        ResolvedType::String => {
            return format!(
                "_runtime.string({value}, {name}, {})",
                field.max_length.unwrap()
            );
        }
        ResolvedType::Bytes => {
            return format!(
                "_runtime.blob({value}, {name}, {})",
                field.max_length.unwrap()
            );
        }
        ResolvedType::Message { name: ty, .. } => {
            return format!("_runtime.message({value}, {name}, {})", type_name(ty));
        }
        ResolvedType::Enum { name: ty, .. } => {
            return format!(
                "{}(_runtime.integer({value}, {name}, -(2**31), 2**31 - 1))",
                type_name(ty)
            );
        }
        ResolvedType::Float32 => return format!("_runtime.real({value}, {name}, 32)"),
        ResolvedType::Float64 => return format!("_runtime.real({value}, {name}, 64)"),
        ResolvedType::Int8 => ("-128", "127"),
        ResolvedType::Uint8 => ("0", "255"),
        ResolvedType::Int16 => ("-32768", "32767"),
        ResolvedType::Uint16 => ("0", "65535"),
        ResolvedType::Int32 => ("-(2**31)", "2**31 - 1"),
        ResolvedType::Uint32 | ResolvedType::Fixed32 => ("0", "2**32 - 1"),
        ResolvedType::Int64 => ("-(2**63)", "2**63 - 1"),
        ResolvedType::Uint64 | ResolvedType::Fixed64 => ("0", "2**64 - 1"),
    };
    format!("_runtime.integer({value}, {name}, {minimum}, {maximum})")
}

fn default_python(field: &FieldSymbol) -> String {
    match field.default.as_ref().unwrap() {
        FieldDefault::String(v) => python_string(v),
        FieldDefault::Bool(v) => if *v { "True" } else { "False" }.into(),
        FieldDefault::Enum(v) => format!("{}({v})", scalar_python(&field.ty)),
        FieldDefault::Int8(v) => v.to_string(),
        FieldDefault::Uint8(v) => v.to_string(),
        FieldDefault::Int16(v) => v.to_string(),
        FieldDefault::Uint16(v) => v.to_string(),
        FieldDefault::Int32(v) => v.to_string(),
        FieldDefault::Uint32(v) | FieldDefault::Fixed32(v) => v.to_string(),
        FieldDefault::Int64(v) => v.to_string(),
        FieldDefault::Uint64(v) | FieldDefault::Fixed64(v) => v.to_string(),
    }
}

fn py_to_native(field: &FieldSymbol, expr: &str) -> String {
    match field.ty {
        ResolvedType::Message { .. } if optional(field) => {
            format!("None if {expr} is None else {expr}._to_native()")
        }
        ResolvedType::Message { .. } => format!("{expr}._to_native()"),
        _ => expr.to_owned(),
    }
}

fn py_from_native(field: &FieldSymbol, expr: &str) -> String {
    match &field.ty {
        ResolvedType::Message { name, .. } if optional(field) => format!(
            "None if {expr} is None else {}._from_native({expr})",
            type_name(name)
        ),
        ResolvedType::Message { name, .. } => format!("{}._from_native({expr})", type_name(name)),
        _ => expr.to_owned(),
    }
}

fn native_read(field: &FieldSymbol, expr: &str) -> String {
    if let Some(n) = count(field) {
        return format!(
            "nb::cast<std::array<{}, {n}>>({expr})",
            scalar_cpp(&field.ty)
        );
    }
    match &field.ty {
        ResolvedType::Message { name, .. } => format!("read_{}({expr})", type_name(name)),
        ResolvedType::Enum { .. } => format!(
            "static_cast<{}>(nb::cast<std::int32_t>({expr}))",
            scalar_cpp(&field.ty)
        ),
        ResolvedType::Bytes => format!(
            "[&] {{ const auto bytes = nb::cast<nb::bytes>({expr}); const auto* data = reinterpret_cast<const std::uint8_t*>(bytes.c_str()); return std::vector<std::uint8_t>(data, data + bytes.size()); }}()"
        ),
        _ => format!("nb::cast<{}>({expr})", scalar_cpp(&field.ty)),
    }
}

fn native_write(field: &FieldSymbol, expr: &str) -> String {
    let value = if optional(field) {
        format!("(*{expr})")
    } else {
        expr.to_owned()
    };
    let result = if count(field).is_some() {
        format!("nb::cast({value})")
    } else {
        match &field.ty {
            ResolvedType::Message { name, .. } => {
                format!("nb::object(write_{}({value}))", type_name(name))
            }
            ResolvedType::Enum { .. } => format!("nb::cast(static_cast<std::int32_t>({value}))"),
            ResolvedType::Bytes => format!(
                "nb::object(nb::bytes(reinterpret_cast<const char*>({value}.data()), {value}.size()))"
            ),
            _ => format!("nb::cast({value})"),
        }
    };
    if optional(field) {
        format!("({expr}.has_value() ? {result} : nb::none())")
    } else {
        result
    }
}
