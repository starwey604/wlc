// SPDX-License-Identifier: Apache-2.0
//! Shared, validated host binding facts. Wire layout remains the C generator's job.
use std::collections::BTreeSet;

use heck::{ToSnakeCase, ToUpperCamelCase};

use crate::{
    BindingProfileModel, SemanticModel,
    ast::Cardinality,
    codegen::{CModel, c_identifier},
    endpoint_layout::EndpointEnvelope,
    semantic::{EnumSymbol, FieldSymbol, MessageSymbol, ResolvedType, Symbol},
};

use super::{SdkCodegenError, SdkOptions};

pub(super) struct Plan<'a> {
    pub options: &'a SdkOptions,
    pub messages: Vec<&'a MessageSymbol>,
    pub enums: Vec<&'a EnumSymbol>,
    pub profile: &'a BindingProfileModel,
    pub package: String,
    pub cmake_package: String,
    pub cmake_version: String,
}

fn error(message: impl Into<String>) -> SdkCodegenError {
    SdkCodegenError(message.into())
}

fn insert(
    names: &mut BTreeSet<String>,
    name: String,
    context: &str,
) -> Result<(), SdkCodegenError> {
    if name.is_empty() || !names.insert(name.clone()) {
        return Err(error(format!("{context}: binding name collision `{name}`")));
    }
    Ok(())
}

impl<'a> Plan<'a> {
    pub fn new(
        schema: &'a SemanticModel,
        profile: &'a BindingProfileModel,
        options: &'a SdkOptions,
    ) -> Result<Self, SdkCodegenError> {
        let name = &options.name;
        if name.is_empty()
            || !name.as_bytes()[0].is_ascii_lowercase()
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || name.ends_with('_')
            || name.contains("__")
            || identifier(name) != *name
            || matches!(name.as_str(), "std" | "wirelink")
        {
            return Err(error(
                "SDK name must be a non-reserved lower_snake_case identifier starting with a letter",
            ));
        }
        let (version, dev) = options
            .package_version
            .split_once(".dev")
            .map_or((options.package_version.as_str(), None), |(v, d)| {
                (v, Some(d))
            });
        let number = |v: &str| {
            !v.is_empty()
                && v.bytes().all(|b| b.is_ascii_digit())
                && (v == "0" || !v.starts_with('0'))
                && v.parse::<u32>().is_ok()
        };
        if version.split('.').count() != 3
            || !version.split('.').all(number)
            || dev.is_some_and(|d| !number(d))
        {
            return Err(error(
                "package version must be MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH.devN",
            ));
        }
        if !profile.has_rpc_client() || profile.rpc_services.iter().any(|rpc| !rpc.is_managed()) {
            return Err(error(
                "SDK generation requires managed RPC services and a client-capable endpoint",
            ));
        }
        if !profile.send_routes.is_empty()
            || !profile.direct_routes.is_empty()
            || !profile.retained_routes.is_empty()
        {
            return Err(error(
                "SDK generation currently supports RPC-only profiles; use a separate host profile for send/direct/latest/fifo routes",
            ));
        }
        if !matches!(
            profile.endpoint_layout().envelope,
            EndpointEnvelope::Any | EndpointEnvelope::NativePacket
        ) {
            return Err(error(
                "UDP SDK generation requires envelope any or native_packet",
            ));
        }
        let c = CModel::new(schema, name).map_err(|e| error(e.to_string()))?;
        let mut types = BTreeSet::from_iter(
            [
                "Client",
                "Udp",
                "WirelinkError",
                "ClosedError",
                "RpcTimeoutError",
                "CancelledError",
                "RejectedError",
                "QueueFullError",
                "TransportError",
                "CodecError",
                "InvalidArgumentError",
                "None",
                "True",
                "False",
            ]
            .map(str::to_owned),
        );
        for symbol in &schema.declarations {
            insert(&mut types, type_name(symbol.name()), symbol.name())?;
        }
        for message in &c.messages {
            let mut fields = BTreeSet::new();
            for field in &message.fields {
                let context = format!("{}.{}", message.name, field.name);
                if field.cardinality == Cardinality::Repeated {
                    return Err(error(format!(
                        "{context}: unbounded repeated fields cannot be owned by this SDK"
                    )));
                }
                if matches!(field.ty, ResolvedType::String | ResolvedType::Bytes)
                    && field.max_length.is_none()
                {
                    return Err(error(format!(
                        "{context}: SDK string/bytes fields require an explicit length bound"
                    )));
                }
                insert(&mut fields, identifier(&field.name), &context)?;
                if field.default.is_some() {
                    insert(
                        &mut fields,
                        format!("{}_or_default", identifier(&field.name)),
                        &context,
                    )?;
                }
            }
            if c.maxima.get(&message.id).copied().flatten().is_none() {
                return Err(error(format!(
                    "{}: SDK messages must have a finite static bound",
                    message.name
                )));
            }
        }
        let enums = schema
            .declarations
            .iter()
            .filter_map(|s| match s {
                Symbol::Enum(e) => Some(e),
                _ => None,
            })
            .collect::<Vec<_>>();
        for enumeration in &enums {
            if enumeration.values.is_empty() {
                return Err(error(format!(
                    "{}: SDK enums require at least one declared value",
                    enumeration.name
                )));
            }
            let mut names = BTreeSet::new();
            for value in &enumeration.values {
                insert(&mut names, enum_name(&value.name), &enumeration.name)?;
            }
        }
        let mut methods =
            BTreeSet::from_iter(["connect", "close", "is_open", "local_port"].map(str::to_owned));
        for rpc in &profile.rpc_services {
            let method = identifier(&rpc.name);
            insert(&mut methods, method.clone(), &rpc.name)?;
            insert(&mut methods, format!("{method}_request"), &rpc.name)?;
            for id in [rpc.request_id, rpc.response_id] {
                if c.maxima[&id]
                    .unwrap()
                    .checked_add(rpc.metadata_size())
                    .is_none_or(|n| n > 2048)
                {
                    return Err(error(format!(
                        "{}: SDK RPC payload including metadata exceeds the default endpoint's 2048-byte bound",
                        rpc.name
                    )));
                }
            }
        }
        Ok(Self {
            options,
            messages: c.messages,
            enums,
            profile,
            package: format!("{name}_sdk"),
            cmake_package: format!("{}Sdk", name.to_upper_camel_case()),
            cmake_version: version.to_owned(),
        })
    }

    pub fn message(&self, id: u16) -> &MessageSymbol {
        self.messages
            .iter()
            .copied()
            .find(|m| m.id == id)
            .expect("resolved RPC message")
    }
}

pub(super) fn optional(field: &FieldSymbol) -> bool {
    matches!(
        field.cardinality,
        Cardinality::Optional | Cardinality::Packed(_)
    )
}

pub(super) fn count(field: &FieldSymbol) -> Option<u16> {
    match field.cardinality {
        Cardinality::Packed(n) | Cardinality::RequiredPacked(n) => Some(n),
        _ => None,
    }
}

// Use one public spelling in both languages. C symbols keep their existing names.
pub(super) fn identifier(name: &str) -> String {
    let mut result = c_identifier(name);
    if matches!(
        result.as_str(),
        "as" | "assert"
            | "async"
            | "await"
            | "def"
            | "del"
            | "elif"
            | "except"
            | "finally"
            | "from"
            | "global"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "pass"
            | "raise"
            | "with"
            | "yield"
            | "match"
            | "case"
            | "none"
            | "self"
            | "timeout"
            | "session"
    ) {
        result.push('_');
    }
    result
}

pub(super) fn type_name(name: &str) -> String {
    name.to_upper_camel_case()
}

pub(super) fn enum_name(name: &str) -> String {
    let value = name.to_snake_case().to_uppercase();
    if matches!(value.as_str(), "NAME" | "VALUE" | "MRO") {
        format!("{value}_")
    } else {
        value
    }
}

pub(super) fn scalar_cpp(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::String => "std::string".into(),
        ResolvedType::Bytes => "std::vector<std::uint8_t>".into(),
        ResolvedType::Message { name, .. } | ResolvedType::Enum { name, .. } => type_name(name),
        _ => crate::codegen::c_type(ty),
    }
}

pub(super) fn field_cpp(field: &FieldSymbol) -> String {
    let mut ty = scalar_cpp(&field.ty);
    if let Some(n) = count(field) {
        ty = format!("std::array<{ty}, {n}>");
    }
    if optional(field) {
        ty = format!("std::optional<{ty}>");
    }
    ty
}

pub(super) fn python_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
