//! Semantic validation for application binding profiles.

use std::collections::{HashMap, HashSet};

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::{
    ast::{Cardinality, Span, Spanned},
    profile::{BINDING_PROFILE_VERSION, BindingDeclaration, BindingProfile, RpcBinding},
    semantic::{FieldSymbol, MessageSymbol, ResolvedType, SemanticModel, Symbol},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DeliveryPolicy {
    Unreliable,
    Reliable,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RetainedRouteKind {
    Latest,
    Fifo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingProfileModel {
    pub version: u32,
    /// Canonically sorted by message ID and then route kind.
    pub retained_routes: Vec<RetainedRoute>,
    /// Canonically sorted by service name.
    pub rpc_services: Vec<RpcService>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedRoute {
    pub kind: RetainedRouteKind,
    pub message_name: String,
    pub message_id: u16,
    pub delivery: DeliveryPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcFieldMapping {
    pub name: String,
    pub number: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcStatusDomain {
    Int32,
    Enum { name: String, id: u16 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcService {
    pub name: String,
    pub request_name: String,
    pub request_id: u16,
    pub response_name: String,
    pub response_id: u16,
    pub request_operation_id: RpcFieldMapping,
    pub response_operation_id: RpcFieldMapping,
    pub response_status: RpcFieldMapping,
    pub status_domain: RpcStatusDomain,
    pub request_delivery: DeliveryPolicy,
    pub response_delivery: DeliveryPolicy,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("{message}")]
#[diagnostic(code(wlc::profile_semantic))]
pub struct ProfileSemanticError {
    #[label("{message}")]
    source_span: SourceSpan,
    pub span: Span,
    pub message: String,
}

impl ProfileSemanticError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            source_span: (span.offset, span.length).into(),
            span,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("binding profile semantic validation failed")]
pub struct ProfileSemanticErrors {
    #[related]
    errors: Vec<ProfileSemanticError>,
}

impl ProfileSemanticErrors {
    pub fn errors(&self) -> &[ProfileSemanticError] {
        &self.errors
    }
}

/// Resolve a sidecar profile against an already validated wire schema.
pub fn analyze_binding_profile(
    profile: &BindingProfile,
    schema: &SemanticModel,
) -> Result<BindingProfileModel, ProfileSemanticErrors> {
    let mut errors = Vec::new();
    if profile.version.value != BINDING_PROFILE_VERSION {
        errors.push(ProfileSemanticError::new(
            profile.version.span,
            format!(
                "unsupported binding profile version {}; only version {} is supported",
                profile.version.value, BINDING_PROFILE_VERSION
            ),
        ));
    }

    let messages: HashMap<&str, &MessageSymbol> = schema
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some((message.name.as_str(), message)),
            Symbol::Enum(_) => None,
        })
        .collect();

    let mut retained_routes = Vec::new();
    let mut retained_message_ids = HashMap::new();
    for binding in &profile.bindings {
        let (kind, route) = match binding {
            BindingDeclaration::Latest(route) => (RetainedRouteKind::Latest, route),
            BindingDeclaration::Fifo(route) => (RetainedRouteKind::Fifo, route),
            BindingDeclaration::Rpc(_) => continue,
        };
        let Some(message) = resolve_message(&route.message, schema, &messages, &mut errors) else {
            continue;
        };
        let delivery = resolve_delivery(&route.delivery, &mut errors);
        if let Some(previous_kind) = retained_message_ids.insert(message.id, kind) {
            errors.push(ProfileSemanticError::new(
                route.message.span,
                format!(
                    "message `{}` already has a retained {:?} route and cannot also use {:?}",
                    message.name, previous_kind, kind
                ),
            ));
        }
        if let Some(path) = retained_ownership_problem(message, &messages, &mut Vec::new()) {
            errors.push(ProfileSemanticError::new(
                route.message.span,
                format!(
                    "message `{}` cannot use a retained {:?} route because `{path}` contains borrowed or caller-owned storage",
                    message.name, kind
                ),
            ));
        }
        if let Some(delivery) = delivery {
            retained_routes.push(RetainedRoute {
                kind,
                message_name: message.name.clone(),
                message_id: message.id,
                delivery,
            });
        }
    }

    let mut rpc_services = Vec::new();
    let mut service_names = HashSet::new();
    let mut rpc_roles: HashMap<u16, (&str, &str)> = HashMap::new();
    for binding in &profile.bindings {
        let BindingDeclaration::Rpc(rpc) = binding else {
            continue;
        };
        if !service_names.insert(rpc.name.value.as_str()) {
            errors.push(ProfileSemanticError::new(
                rpc.name.span,
                format!("duplicate RPC service `{}`", rpc.name.value),
            ));
            continue;
        }
        let Some(request) = resolve_message(&rpc.request, schema, &messages, &mut errors) else {
            continue;
        };
        let Some(response) = resolve_message(&rpc.response, schema, &messages, &mut errors) else {
            continue;
        };
        if request.id == response.id {
            errors.push(ProfileSemanticError::new(
                rpc.response.span,
                format!(
                    "RPC service `{}` must use different request and response messages",
                    rpc.name.value
                ),
            ));
        }
        check_rpc_role(
            request,
            "request",
            rpc,
            &retained_message_ids,
            &mut rpc_roles,
            rpc.request.span,
            &mut errors,
        );
        check_rpc_role(
            response,
            "response",
            rpc,
            &retained_message_ids,
            &mut rpc_roles,
            rpc.response.span,
            &mut errors,
        );

        let request_operation_id = resolve_operation_id(
            request,
            &rpc.request_operation_id,
            "request_operation_id",
            &mut errors,
        );
        let response_operation_id = resolve_operation_id(
            response,
            &rpc.response_operation_id,
            "response_operation_id",
            &mut errors,
        );
        let response_status = resolve_status(response, &rpc.response_status, schema, &mut errors);
        let request_delivery = resolve_delivery(&rpc.request_delivery, &mut errors);
        let response_delivery = resolve_delivery(&rpc.response_delivery, &mut errors);

        if let (
            Some(request_operation_id),
            Some(response_operation_id),
            Some((response_status, status_domain)),
            Some(request_delivery),
            Some(response_delivery),
        ) = (
            request_operation_id,
            response_operation_id,
            response_status,
            request_delivery,
            response_delivery,
        ) {
            rpc_services.push(RpcService {
                name: rpc.name.value.clone(),
                request_name: request.name.clone(),
                request_id: request.id,
                response_name: response.name.clone(),
                response_id: response.id,
                request_operation_id,
                response_operation_id,
                response_status,
                status_domain,
                request_delivery,
                response_delivery,
            });
        }
    }

    if !errors.is_empty() {
        return Err(ProfileSemanticErrors { errors });
    }
    retained_routes.sort_by_key(|route| (route.message_id, route.kind));
    rpc_services.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(BindingProfileModel {
        version: profile.version.value,
        retained_routes,
        rpc_services,
    })
}

fn resolve_message<'a>(
    name: &Spanned<String>,
    schema: &'a SemanticModel,
    messages: &HashMap<&str, &'a MessageSymbol>,
    errors: &mut Vec<ProfileSemanticError>,
) -> Option<&'a MessageSymbol> {
    if let Some(message) = messages.get(name.value.as_str()) {
        return Some(*message);
    }
    let is_enum = schema
        .declarations
        .iter()
        .any(|symbol| matches!(symbol, Symbol::Enum(value) if value.name == name.value));
    errors.push(ProfileSemanticError::new(
        name.span,
        if is_enum {
            format!("`{}` is an enum; a binding requires a message", name.value)
        } else {
            format!("unknown message `{}`", name.value)
        },
    ));
    None
}

fn resolve_delivery(
    delivery: &Spanned<String>,
    errors: &mut Vec<ProfileSemanticError>,
) -> Option<DeliveryPolicy> {
    match delivery.value.as_str() {
        "unreliable" => Some(DeliveryPolicy::Unreliable),
        "reliable" => Some(DeliveryPolicy::Reliable),
        _ => {
            errors.push(ProfileSemanticError::new(
                delivery.span,
                format!(
                    "invalid delivery `{}`; expected `unreliable` or `reliable`",
                    delivery.value
                ),
            ));
            None
        }
    }
}

fn retained_ownership_problem<'a>(
    message: &'a MessageSymbol,
    messages: &HashMap<&str, &'a MessageSymbol>,
    path: &mut Vec<String>,
) -> Option<String> {
    path.push(message.name.clone());
    for field in &message.fields {
        if field.cardinality == Cardinality::Repeated {
            let mut problem = path.join(".");
            problem.push('.');
            problem.push_str(&field.name);
            path.pop();
            return Some(problem);
        }
        match &field.ty {
            ResolvedType::Bytes | ResolvedType::String => {
                let mut problem = path.join(".");
                problem.push('.');
                problem.push_str(&field.name);
                path.pop();
                return Some(problem);
            }
            ResolvedType::Message { name, .. } => {
                if let Some(child) = messages.get(name.as_str()) {
                    path.push(field.name.clone());
                    if let Some(problem) = retained_ownership_problem(child, messages, path) {
                        path.pop();
                        path.pop();
                        return Some(problem);
                    }
                    path.pop();
                }
            }
            _ => {}
        }
    }
    path.pop();
    None
}

fn check_rpc_role<'a>(
    message: &'a MessageSymbol,
    role: &'static str,
    rpc: &'a RpcBinding,
    retained: &HashMap<u16, RetainedRouteKind>,
    roles: &mut HashMap<u16, (&'a str, &'static str)>,
    span: Span,
    errors: &mut Vec<ProfileSemanticError>,
) {
    if let Some(kind) = retained.get(&message.id) {
        errors.push(ProfileSemanticError::new(
            span,
            format!(
                "RPC {role} message `{}` already has a retained {:?} route",
                message.name, kind
            ),
        ));
    }
    if let Some((other_service, other_role)) = roles.insert(message.id, (&rpc.name.value, role)) {
        errors.push(ProfileSemanticError::new(
            span,
            format!(
                "message `{}` is already the {other_role} of RPC service `{other_service}` and cannot also be the {role} of `{}`",
                message.name, rpc.name.value
            ),
        ));
    }
}

fn resolve_operation_id(
    message: &MessageSymbol,
    mapping: &Spanned<String>,
    property: &str,
    errors: &mut Vec<ProfileSemanticError>,
) -> Option<RpcFieldMapping> {
    let field = resolve_field(message, mapping, errors)?;
    if field.cardinality != Cardinality::Optional || field.ty != ResolvedType::Uint32 {
        errors.push(ProfileSemanticError::new(
            mapping.span,
            format!(
                "RPC property `{property}` must map to an optional uint32 field; `{}.{}` has a different type or cardinality",
                message.name, field.name
            ),
        ));
        return None;
    }
    Some(field_mapping(field))
}

fn resolve_status(
    message: &MessageSymbol,
    mapping: &Spanned<String>,
    schema: &SemanticModel,
    errors: &mut Vec<ProfileSemanticError>,
) -> Option<(RpcFieldMapping, RpcStatusDomain)> {
    let field = resolve_field(message, mapping, errors)?;
    if field.cardinality != Cardinality::Optional {
        errors.push(ProfileSemanticError::new(
            mapping.span,
            format!(
                "RPC response status must be optional; `{}.{}` has a different cardinality",
                message.name, field.name
            ),
        ));
        return None;
    }
    let domain = match &field.ty {
        ResolvedType::Int32 => RpcStatusDomain::Int32,
        ResolvedType::Enum { id, name } => {
            let has_success = schema.declarations.iter().any(|symbol| {
                matches!(symbol, Symbol::Enum(value) if value.id == *id && value.values.iter().any(|variant| variant.number == 0))
            });
            if !has_success {
                errors.push(ProfileSemanticError::new(
                    mapping.span,
                    format!(
                        "RPC status enum `{name}` must declare numeric value zero for application success"
                    ),
                ));
                return None;
            }
            RpcStatusDomain::Enum {
                name: name.clone(),
                id: *id,
            }
        }
        _ => {
            errors.push(ProfileSemanticError::new(
                mapping.span,
                format!(
                    "RPC response status must map to an optional int32 or enum field; `{}.{}` has a different type",
                    message.name, field.name
                ),
            ));
            return None;
        }
    };
    Some((field_mapping(field), domain))
}

fn resolve_field<'a>(
    message: &'a MessageSymbol,
    mapping: &Spanned<String>,
    errors: &mut Vec<ProfileSemanticError>,
) -> Option<&'a FieldSymbol> {
    if let Some(field) = message
        .fields
        .iter()
        .find(|field| field.name == mapping.value)
    {
        return Some(field);
    }
    errors.push(ProfileSemanticError::new(
        mapping.span,
        format!(
            "message `{}` has no field `{}`",
            message.name, mapping.value
        ),
    ));
    None
}

fn field_mapping(field: &FieldSymbol) -> RpcFieldMapping {
    RpcFieldMapping {
        name: field.name.clone(),
        number: field.number,
    }
}
