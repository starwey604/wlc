// SPDX-License-Identifier: Apache-2.0
use crate::codegen::{c_identifier, type_name, upper_snake};
use crate::profile_semantic::{BindingProfileModel, RetainedRouteKind};
use std::collections::HashMap;
use std::fmt::Write;

pub(super) struct RuntimeDefaultCapacities {
    rpc_response: Option<u16>,
    request_bounds: Vec<(String, Option<u16>)>,
}

impl RuntimeDefaultCapacities {
    pub(super) fn has_storage(&self) -> bool {
        self.rpc_response.is_some()
            && self
                .request_bounds
                .iter()
                .all(|(_, capacity)| capacity.is_some())
    }
}

pub(super) fn runtime_default_capacities(
    maxima: &HashMap<u16, Option<u64>>,
    profile: &BindingProfileModel,
) -> RuntimeDefaultCapacities {
    let mut rpc_response = Some(1_u16);
    let mut request_bounds = Vec::new();

    for service in &profile.rpc_services {
        let request_capacity = maxima
            .get(&service.request_id)
            .copied()
            .flatten()
            .map(|capacity| capacity.max(1))
            .and_then(|capacity| u16::try_from(capacity).ok());
        let response_capacity = maxima
            .get(&service.response_id)
            .copied()
            .flatten()
            .map(|capacity| (capacity + service.metadata_size()).max(1))
            .and_then(|capacity| u16::try_from(capacity).ok());
        rpc_response = match (rpc_response, response_capacity) {
            (Some(current), Some(capacity)) => Some(current.max(capacity)),
            _ => None,
        };
        request_bounds.push((c_identifier(&service.name), request_capacity));
    }
    RuntimeDefaultCapacities {
        rpc_response,
        request_bounds,
    }
}

pub(super) fn default_storage_terms(
    profile: &BindingProfileModel,
    capacities: &RuntimeDefaultCapacities,
    prefix: &str,
) -> Vec<String> {
    let padding = format!("({prefix}_RUNTIME_DEFAULT_STORAGE_ALIGNMENT - 1U)");
    let mut terms = Vec::new();
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => terms.push(format!(
                "({padding} + ((sizeof({message}_t) + {padding}) * WL_LATEST_SLOT_COUNT))"
            )),
            RetainedRouteKind::Fifo => {
                terms.push(format!("({padding} + sizeof({message}_t) + {padding})"))
            }
        }
    }
    if !profile.rpc_services.is_empty() {
        terms.push(format!("({padding} + sizeof(wl_rpc_client_slot_t))"));
        terms.push(format!("{}U", capacities.rpc_response.unwrap()));
        terms.push(format!(
            "({padding} + sizeof(wl_rpc_server_pending_slot_t))"
        ));
        terms.push(format!("({padding} + sizeof(wl_rpc_server_cache_slot_t))"));
        terms.push(format!("{}U", capacities.rpc_response.unwrap()));
        // The canonical sink retains only a fingerprint, never a byte arena.
    }
    terms
}

pub(super) fn emit_assembly_header(
    output: &mut String,
    maxima: &HashMap<u16, Option<u64>>,
    profile: &BindingProfileModel,
    module: &str,
) {
    output.push_str(concat!(
        "/* Static runtime assembly. requirements() validates every sizing field and\n",
        " * reports the exact caller-owned byte storage needed by init(). Configuration\n",
        " * and storage descriptors may be temporary; instance and storage must outlive\n",
        " * all runtime activity and must not be copied after successful initialization. */\n",
        "typedef struct {\n",
        "  uint8_t _reserved;\n",
    ));
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => {
                writeln!(output, "  uint32_t {message}_latest_initial_generation;").unwrap();
            }
            RetainedRouteKind::Fifo => {
                writeln!(output, "  uint32_t {message}_fifo_capacity;").unwrap();
            }
        }
    }
    if !profile.rpc_services.is_empty() {
        output.push_str(concat!(
            "  uint8_t rpc_client_enabled;\n",
            "  uint16_t rpc_client_slot_count;\n",
            "  uint16_t rpc_client_response_capacity;\n",
            "  uint32_t rpc_client_next_operation_id;\n",
            "  uint8_t rpc_server_enabled;\n",
            "  uint16_t rpc_server_pending_slot_count;\n",
            "  uint16_t rpc_server_cache_slot_count;\n",
            "  uint16_t rpc_server_response_capacity;\n",
            "  uint32_t rpc_server_pending_timeout_ms;\n",
            "  uint32_t rpc_server_cache_ttl_ms;\n",
            "  wl_rpc_cache_policy_t rpc_server_cache_policy;\n",
        ));
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            writeln!(
                output,
                "  {module}_{service_name}_rpc_request_handler_fn {service_name}_request_handler;"
            )
            .unwrap();
            writeln!(output, "  void *{service_name}_user_data;").unwrap();
        }
    }
    writeln!(output, "}} {module}_runtime_config_t;\n").unwrap();
    let default_capacities = runtime_default_capacities(maxima, profile);
    let prefix = upper_snake(module);
    writeln!(
        output,
        "#define {prefix}_RUNTIME_HAS_DEFAULT_STORAGE {}",
        u8::from(default_capacities.has_storage())
    )
    .unwrap();
    if default_capacities.has_storage() {
        output.push_str("typedef union {\n  uint8_t byte;\n");
        for route in &profile.retained_routes {
            let message = type_name(&route.message_name);
            let kind = match route.kind {
                RetainedRouteKind::Latest => "latest",
                RetainedRouteKind::Fifo => "fifo",
            };
            writeln!(output, "  {message}_t {message}_{kind};").unwrap();
        }
        if !profile.rpc_services.is_empty() {
            output.push_str(
                "  wl_rpc_client_slot_t rpc_client_slot;\n  wl_rpc_server_pending_slot_t rpc_server_pending_slot;\n  wl_rpc_server_cache_slot_t rpc_server_cache_slot;\n",
            );
        }
        writeln!(
            output,
            "}} {module}_runtime_default_storage_alignment_t;\n\n#if defined(__cplusplus)\n#define {prefix}_RUNTIME_DEFAULT_STORAGE_ALIGNMENT alignof({module}_runtime_default_storage_alignment_t)\n#elif defined(_MSC_VER)\n#define {prefix}_RUNTIME_DEFAULT_STORAGE_ALIGNMENT __alignof({module}_runtime_default_storage_alignment_t)\n#else\n#define {prefix}_RUNTIME_DEFAULT_STORAGE_ALIGNMENT _Alignof({module}_runtime_default_storage_alignment_t)\n#endif"
        )
        .unwrap();
        writeln!(
            output,
            "#define {prefix}_RUNTIME_DEFAULT_STORAGE_CAPACITY \\"
        )
        .unwrap();
        output.push_str("  (1U");
        for term in default_storage_terms(profile, &default_capacities, &prefix) {
            output.push_str(" + \\\n   ");
            output.push_str(&term);
        }
        output.push_str(")\n\ntypedef union {\n  ");
        write!(
            output,
            "{module}_runtime_default_storage_alignment_t alignment;\n  uint8_t bytes["
        )
        .unwrap();
        write!(output, "{prefix}_RUNTIME_DEFAULT_STORAGE_CAPACITY").unwrap();
        writeln!(output, "];\n}} {module}_runtime_default_storage_t;\n").unwrap();
    }
    // C++ anonymous unions may contain data members, not type declarations.
    // Define the member types first; the shared layout and field paths stay identical.
    for service in &profile.rpc_services {
        let name = c_identifier(&service.name);
        let request = type_name(&service.request_name);
        let response = type_name(&service.response_name);
        writeln!(output, "typedef union {{ {request}_t request; {response}_t response; }} {module}_runtime_{name}_decode_detail_t;").unwrap();
    }
    write!(
        output,
        "typedef struct {{\n  size_t storage_size;\n  size_t storage_alignment;\n}} {module}_runtime_requirements_t;\n\ntypedef struct {{\n  void *data;\n  size_t size;\n}} {module}_runtime_storage_t;\n\ntypedef struct {{\n  {module}_runtime_t runtime;\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let (ty, kind) = match route.kind {
            RetainedRouteKind::Latest => ("wl_latest_t", "latest"),
            RetainedRouteKind::Fifo => ("wl_fifo_t", "fifo"),
        };
        writeln!(output, "  {ty} {message}_{kind};").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        output.push_str("  wl_rpc_client_t rpc_client;\n  wl_rpc_server_t rpc_server;\n");
        if profile
            .rpc_services
            .iter()
            .any(|service| !service.is_managed())
        {
            writeln!(
                output,
                "  {module}_runtime_rpc_encode_scratch_t rpc_encode_scratch;"
            )
            .unwrap();
        }
        // Unbounded repeated fields carry caller-configured backing pointers.
        // Keep those per-service objects stable; bounded messages have no such
        // persistent decode configuration and can safely share a union.
        let share_decode = default_capacities.has_storage();
        if share_decode {
            output.push_str(
                "  /* One dispatch at a time: bounded services share decode scratch.\n   * Views are callback-scoped; deferred work must copy its input. */\n  union {\n",
            );
        } else {
            output.push_str("  /* Preserve per-service caller-configured decode backing. */\n");
        }
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            writeln!(
                output,
                "    {module}_runtime_{service_name}_decode_detail_t {service_name}_scratch;"
            )
            .unwrap();
        }
        if share_decode {
            output.push_str("  };\n");
        }
    }
    writeln!(
        output,
        "}} {module}_runtime_instance_t;\n\ntypedef int32_t {module}_runtime_init_issue_t;\nenum {{\n  {prefix}_RUNTIME_INIT_OK = 0,\n  {prefix}_RUNTIME_INIT_NULL_ARGUMENT,\n  {prefix}_RUNTIME_INIT_ROLE_ENABLE,\n  {prefix}_RUNTIME_INIT_RETAINED_CAPACITY,\n  {prefix}_RUNTIME_INIT_RPC_CLIENT_CAPACITY,\n  {prefix}_RUNTIME_INIT_RPC_SERVER_CAPACITY,\n  {prefix}_RUNTIME_INIT_RPC_TIMEOUT,\n  {prefix}_RUNTIME_INIT_RPC_CACHE_POLICY,\n  {prefix}_RUNTIME_INIT_LAYOUT_OVERFLOW,\n  {prefix}_RUNTIME_INIT_STORAGE_TOO_SMALL,\n  {prefix}_RUNTIME_INIT_STORAGE_NULL,\n  {prefix}_RUNTIME_INIT_STORAGE_ALIGNMENT,\n  {prefix}_RUNTIME_INIT_STORAGE_OVERLAP,\n  {prefix}_RUNTIME_INIT_COMPONENT\n}};\n\ntypedef struct {{\n  {module}_runtime_init_issue_t issue;\n  const char *field;\n  size_t required;\n  size_t provided;\n}} {module}_runtime_init_diagnostic_t;\n\nconst char *{module}_runtime_init_issue_str({module}_runtime_init_issue_t issue);\n\n/* Mechanical defaults use one FIFO/RPC slot, generation/operation ID one,\n * bounded encoded maxima, disabled roles, zero timeouts, and reject-new cache.\n * Override policy fields after this call. */\nwl_err_t {module}_runtime_config_defaults({module}_runtime_config_t *config);\n"
    )
    .unwrap();
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "wl_err_t {module}_runtime_config_enable_client({module}_runtime_config_t *config);\nwl_err_t {module}_runtime_config_enable_server({module}_runtime_config_t *config);\n"
        )
        .unwrap();
    }
    if default_capacities.has_storage() {
        writeln!(
            output,
            "{module}_runtime_storage_t {module}_runtime_default_storage_descriptor({module}_runtime_default_storage_t *storage);"
        )
        .unwrap();
    }
    write!(
        output,
        "int {module}_runtime_requirements(const {module}_runtime_config_t *config, {module}_runtime_requirements_t *out_requirements);\n/* Checked initialization reports the exact rejected field and capacity values. */\nint {module}_runtime_init_checked({module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage, {module}_runtime_init_diagnostic_t *out_diagnostic);\nint {module}_runtime_init({module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage);\n"
    )
    .unwrap();
}

pub(super) fn emit_config_defaults(
    output: &mut String,
    maxima: &HashMap<u16, Option<u64>>,
    profile: &BindingProfileModel,
    module: &str,
) {
    let capacities = runtime_default_capacities(maxima, profile);
    write!(
        output,
        "wl_err_t {module}_runtime_config_defaults({module}_runtime_config_t *config) {{\n  if (config == NULL) return WL_ERR_INVALID_ARG;\n  memset(config, 0, sizeof(*config));\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => writeln!(
                output,
                "  config->{message}_latest_initial_generation = 1U;"
            )
            .unwrap(),
            RetainedRouteKind::Fifo => {
                writeln!(output, "  config->{message}_fifo_capacity = 1U;").unwrap()
            }
        }
    }
    if !profile.rpc_services.is_empty() {
        output.push_str(
            "  config->rpc_client_slot_count = 1U;\n  config->rpc_client_next_operation_id = 1U;\n  config->rpc_server_pending_slot_count = 1U;\n  config->rpc_server_cache_slot_count = 1U;\n  config->rpc_server_cache_policy = WL_RPC_CACHE_REJECT_NEW;\n",
        );
        if let Some(response_capacity) = capacities.rpc_response {
            writeln!(
                output,
                "  config->rpc_client_response_capacity = {}U;\n  config->rpc_server_response_capacity = {}U;",
                response_capacity, response_capacity
            )
            .unwrap();
        }
    }
    output.push_str("  return WL_OK;\n}\n\n");

    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "wl_err_t {module}_runtime_config_enable_client({module}_runtime_config_t *config) {{\n  if (config == NULL) return WL_ERR_INVALID_ARG;\n  if (config->rpc_client_slot_count == 0U || config->rpc_client_response_capacity == 0U) return WL_ERR_NOT_SUPPORTED;\n  config->rpc_client_enabled = 1U;\n  return WL_OK;\n}}\n\nwl_err_t {module}_runtime_config_enable_server({module}_runtime_config_t *config) {{\n  if (config == NULL) return WL_ERR_INVALID_ARG;\n  if (config->rpc_server_pending_slot_count == 0U || config->rpc_server_cache_slot_count == 0U || config->rpc_server_response_capacity == 0U) return WL_ERR_NOT_SUPPORTED;\n"
        )
        .unwrap();
        output.push_str("  config->rpc_server_enabled = 1U;\n  return WL_OK;\n}\n\n");
    }
    if capacities.has_storage() {
        write!(
            output,
            "{module}_runtime_storage_t {module}_runtime_default_storage_descriptor({module}_runtime_default_storage_t *storage) {{\n  {module}_runtime_storage_t descriptor = {{0}};\n  if (storage != NULL) {{\n    descriptor.data = storage->bytes;\n    descriptor.size = sizeof(storage->bytes);\n  }}\n  return descriptor;\n}}\n\n"
        )
        .unwrap();
    }
}

pub(super) fn emit_assembly_source(
    output: &mut String,
    maxima: &HashMap<u16, Option<u64>>,
    profile: &BindingProfileModel,
    module: &str,
) {
    emit_config_defaults(output, maxima, profile, module);
    let prefix = upper_snake(module);
    let has_components = !profile.retained_routes.is_empty() || !profile.rpc_services.is_empty();
    let result_declaration = if has_components {
        "  int result;\n"
    } else {
        ""
    };
    write!(
        output,
        "const char *{module}_runtime_init_issue_str({module}_runtime_init_issue_t issue) {{\n  switch (issue) {{\n    case {prefix}_RUNTIME_INIT_OK: return \"ok\";\n    case {prefix}_RUNTIME_INIT_NULL_ARGUMENT: return \"null argument\";\n    case {prefix}_RUNTIME_INIT_ROLE_ENABLE: return \"role enable must be zero or one\";\n    case {prefix}_RUNTIME_INIT_RETAINED_CAPACITY: return \"retained capacity is zero\";\n    case {prefix}_RUNTIME_INIT_RPC_CLIENT_CAPACITY: return \"RPC client capacity is zero\";\n    case {prefix}_RUNTIME_INIT_RPC_SERVER_CAPACITY: return \"RPC server capacity is zero\";\n    case {prefix}_RUNTIME_INIT_RPC_TIMEOUT: return \"RPC timeout exceeds wrap-safe range\";\n    case {prefix}_RUNTIME_INIT_RPC_CACHE_POLICY: return \"unknown RPC cache policy\";\n    case {prefix}_RUNTIME_INIT_LAYOUT_OVERFLOW: return \"runtime layout size overflow\";\n    case {prefix}_RUNTIME_INIT_STORAGE_TOO_SMALL: return \"runtime storage is too small\";\n    case {prefix}_RUNTIME_INIT_STORAGE_NULL: return \"runtime storage data is null\";\n    case {prefix}_RUNTIME_INIT_STORAGE_ALIGNMENT: return \"runtime storage is misaligned\";\n    case {prefix}_RUNTIME_INIT_STORAGE_OVERLAP: return \"runtime storage overlaps the instance\";\n    case {prefix}_RUNTIME_INIT_COMPONENT: return \"runtime component initialization failed\";\n    default: return \"unknown runtime initialization issue\";\n  }}\n}}\n\nstatic int {module}_runtime_init_failure({module}_runtime_init_diagnostic_t *diagnostic, {module}_runtime_init_issue_t issue, const char *field, size_t required, size_t provided, int result) {{\n  if (diagnostic != NULL) {{\n    diagnostic->issue = issue;\n    diagnostic->field = field;\n    diagnostic->required = required;\n    diagnostic->provided = provided;\n  }}\n  return result;\n}}\n\n"
    )
    .unwrap();
    write!(
        output,
        "typedef struct {{\n  uint8_t *base;\n  size_t size;\n  size_t offset;\n}} {module}_runtime_storage_cursor_t;\n\ntypedef struct {{\n"
    )
    .unwrap();
    if !has_components {
        output.push_str("  uint8_t _reserved;\n");
    }
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        writeln!(output, "  void *{message}_{kind}_storage;").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        output.push_str(concat!(
            "  void *rpc_client_slots;\n",
            "  void *rpc_client_responses;\n",
            "  size_t rpc_client_responses_size;\n",
            "  void *rpc_server_pending_slots;\n",
            "  void *rpc_server_cache_slots;\n",
            "  void *rpc_server_responses;\n",
            "  size_t rpc_server_responses_size;\n",
        ));
    }
    write!(
        output,
        "}} {module}_runtime_layout_t;\n\nstatic inline int {module}_runtime_storage_region({module}_runtime_storage_cursor_t *cursor, size_t alignment, size_t count, size_t element_size, void **out_data, size_t *out_size) {{\n  size_t aligned;\n  size_t region_size;\n  if (cursor == NULL || alignment == 0U || (alignment & (alignment - 1U)) != 0U) return WL_ERR_INVALID_ARG;\n  if (out_data != NULL) *out_data = NULL;\n  if (out_size != NULL) *out_size = 0U;\n  if (count != 0U && element_size > SIZE_MAX / count) return WL_ERR_INVALID_ARG;\n  region_size = count * element_size;\n  if (cursor->offset > SIZE_MAX - (alignment - 1U)) return WL_ERR_INVALID_ARG;\n  aligned = (cursor->offset + (alignment - 1U)) & ~(alignment - 1U);\n  if (region_size > SIZE_MAX - aligned) return WL_ERR_INVALID_ARG;\n  if (aligned + region_size > cursor->size) return WL_ERR_BUF_TOO_SMALL;\n  if (out_data != NULL && cursor->base != NULL) *out_data = cursor->base + aligned;\n  if (out_size != NULL) *out_size = region_size;\n  cursor->offset = aligned + region_size;\n  return WL_OK;\n}}\n\nstatic int {module}_runtime_layout(const {module}_runtime_config_t *config, uint8_t *base, size_t size, {module}_runtime_layout_t *out_layout, {module}_runtime_requirements_t *out_requirements) {{\n  {module}_runtime_storage_cursor_t cursor = {{base, size, 0U}};\n  size_t alignment = 1U;\n{result_declaration}  if (out_layout != NULL) memset(out_layout, 0, sizeof(*out_layout));\n  if (out_requirements != NULL) memset(out_requirements, 0, sizeof(*out_requirements));\n  if (config == NULL) return WL_ERR_INVALID_ARG;\n"
    )
    .unwrap();

    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => {
                write!(
                    output,
                    "  {{\n    const wl_latest_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_latest_initial_generation}};\n    wl_latest_requirements_t route_requirements;\n    result = wl_latest_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) return result;\n    if (alignment < _Alignof({message}_t)) alignment = _Alignof({message}_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof({message}_t), 1U, route_requirements.storage_size, out_layout == NULL ? NULL : &out_layout->{message}_latest_storage, NULL);\n    if (result != WL_OK) return result;\n  }}\n"
                )
                .unwrap();
            }
            RetainedRouteKind::Fifo => {
                write!(
                    output,
                    "  {{\n    const wl_fifo_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_fifo_capacity}};\n    wl_fifo_requirements_t route_requirements;\n    result = wl_fifo_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) return result;\n    if (alignment < _Alignof({message}_t)) alignment = _Alignof({message}_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof({message}_t), 1U, route_requirements.storage_size, out_layout == NULL ? NULL : &out_layout->{message}_fifo_storage, NULL);\n    if (result != WL_OK) return result;\n  }}\n"
                )
                .unwrap();
            }
        }
    }

    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (config->rpc_client_enabled > 1U || config->rpc_server_enabled > 1U) return WL_ERR_INVALID_ARG;\n  if (config->rpc_client_enabled != 0U) {{\n    if (config->rpc_client_slot_count == 0U || config->rpc_client_response_capacity == 0U) return WL_ERR_INVALID_ARG;\n    if (alignment < _Alignof(wl_rpc_client_slot_t)) alignment = _Alignof(wl_rpc_client_slot_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof(wl_rpc_client_slot_t), config->rpc_client_slot_count, sizeof(wl_rpc_client_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_client_slots, NULL);\n    if (result != WL_OK) return result;\n    result = {module}_runtime_storage_region(&cursor, 1U, config->rpc_client_slot_count, config->rpc_client_response_capacity, out_layout == NULL ? NULL : &out_layout->rpc_client_responses, out_layout == NULL ? NULL : &out_layout->rpc_client_responses_size);\n    if (result != WL_OK) return result;\n  }}\n  if (config->rpc_server_enabled != 0U) {{\n    if (config->rpc_server_pending_slot_count == 0U || config->rpc_server_cache_slot_count == 0U || config->rpc_server_response_capacity == 0U) return WL_ERR_INVALID_ARG;\n    if ((config->rpc_server_pending_timeout_ms != 0U && config->rpc_server_pending_timeout_ms >= UINT32_C(0x80000000)) || (config->rpc_server_cache_ttl_ms != 0U && config->rpc_server_cache_ttl_ms >= UINT32_C(0x80000000))) return WL_ERR_INVALID_ARG;\n    if (config->rpc_server_cache_policy != WL_RPC_CACHE_REJECT_NEW && config->rpc_server_cache_policy != WL_RPC_CACHE_EVICT_OLDEST) return WL_ERR_INVALID_ARG;\n    if (alignment < _Alignof(wl_rpc_server_pending_slot_t)) alignment = _Alignof(wl_rpc_server_pending_slot_t);\n    if (alignment < _Alignof(wl_rpc_server_cache_slot_t)) alignment = _Alignof(wl_rpc_server_cache_slot_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof(wl_rpc_server_pending_slot_t), config->rpc_server_pending_slot_count, sizeof(wl_rpc_server_pending_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_server_pending_slots, NULL);\n    if (result != WL_OK) return result;\n    result = {module}_runtime_storage_region(&cursor, _Alignof(wl_rpc_server_cache_slot_t), config->rpc_server_cache_slot_count, sizeof(wl_rpc_server_cache_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_server_cache_slots, NULL);\n    if (result != WL_OK) return result;\n    result = {module}_runtime_storage_region(&cursor, 1U, config->rpc_server_cache_slot_count, config->rpc_server_response_capacity, out_layout == NULL ? NULL : &out_layout->rpc_server_responses, out_layout == NULL ? NULL : &out_layout->rpc_server_responses_size);\n    if (result != WL_OK) return result;\n"
        )
        .unwrap();
        output.push_str("  }\n");
    }
    write!(
        output,
        "  if (out_requirements != NULL) {{\n    out_requirements->storage_size = cursor.offset;\n    out_requirements->storage_alignment = alignment;\n  }}\n  return WL_OK;\n}}\n\nint {module}_runtime_requirements(const {module}_runtime_config_t *config, {module}_runtime_requirements_t *out_requirements) {{\n  {module}_runtime_config_t config_copy;\n  if (config == NULL || out_requirements == NULL) return WL_ERR_INVALID_ARG;\n  config_copy = *config;\n  *out_requirements = ({module}_runtime_requirements_t){{0}};\n  return {module}_runtime_layout(&config_copy, NULL, SIZE_MAX, NULL, out_requirements);\n}}\n\nstatic int {module}_runtime_init_validate(const {module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage, {module}_runtime_requirements_t *requirements, {module}_runtime_init_diagnostic_t *diagnostic) {{\n  uintptr_t instance_address;\n  uintptr_t storage_address;\n  int result;\n  if (diagnostic != NULL) memset(diagnostic, 0, sizeof(*diagnostic));\n  if (instance == NULL || config == NULL || storage == NULL || requirements == NULL || diagnostic == NULL)\n    return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_NULL_ARGUMENT, instance == NULL ? \"instance\" : config == NULL ? \"config\" : storage == NULL ? \"storage\" : requirements == NULL ? \"requirements\" : \"diagnostic\", 1U, 0U, WL_ERR_INVALID_ARG);\n"
    )
    .unwrap();

    for route in &profile.retained_routes {
        if route.kind == RetainedRouteKind::Fifo {
            let message = type_name(&route.message_name);
            writeln!(
                output,
                "  if (config->{message}_fifo_capacity == 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RETAINED_CAPACITY, \"{message}_fifo_capacity\", 1U, 0U, WL_ERR_INVALID_ARG);"
            )
            .unwrap();
        }
    }
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (config->rpc_client_enabled > 1U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_ROLE_ENABLE, \"rpc_client_enabled\", 1U, config->rpc_client_enabled, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled > 1U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_ROLE_ENABLE, \"rpc_server_enabled\", 1U, config->rpc_server_enabled, WL_ERR_INVALID_ARG);\n  if (config->rpc_client_enabled != 0U && config->rpc_client_slot_count == 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_CLIENT_CAPACITY, \"rpc_client_slot_count\", 1U, 0U, WL_ERR_INVALID_ARG);\n  if (config->rpc_client_enabled != 0U && config->rpc_client_response_capacity == 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_CLIENT_CAPACITY, \"rpc_client_response_capacity\", 1U, 0U, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled != 0U && config->rpc_server_pending_slot_count == 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_SERVER_CAPACITY, \"rpc_server_pending_slot_count\", 1U, 0U, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled != 0U && config->rpc_server_cache_slot_count == 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_SERVER_CAPACITY, \"rpc_server_cache_slot_count\", 1U, 0U, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled != 0U && config->rpc_server_response_capacity == 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_SERVER_CAPACITY, \"rpc_server_response_capacity\", 1U, 0U, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled != 0U && config->rpc_server_pending_timeout_ms >= UINT32_C(0x80000000)) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_TIMEOUT, \"rpc_server_pending_timeout_ms\", UINT32_C(0x7fffffff), config->rpc_server_pending_timeout_ms, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled != 0U && config->rpc_server_cache_ttl_ms >= UINT32_C(0x80000000)) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_TIMEOUT, \"rpc_server_cache_ttl_ms\", UINT32_C(0x7fffffff), config->rpc_server_cache_ttl_ms, WL_ERR_INVALID_ARG);\n  if (config->rpc_server_enabled != 0U && config->rpc_server_cache_policy != WL_RPC_CACHE_REJECT_NEW && config->rpc_server_cache_policy != WL_RPC_CACHE_EVICT_OLDEST) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_RPC_CACHE_POLICY, \"rpc_server_cache_policy\", 0U, (size_t)config->rpc_server_cache_policy, WL_ERR_INVALID_ARG);\n"
        )
        .unwrap();
    }
    write!(
        output,
        "  result = {module}_runtime_requirements(config, requirements);\n  if (result != WL_OK) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_LAYOUT_OVERFLOW, \"config\", 0U, 0U, result);\n  if (storage->size < requirements->storage_size) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_STORAGE_TOO_SMALL, \"storage.size\", requirements->storage_size, storage->size, WL_ERR_BUF_TOO_SMALL);\n  if (requirements->storage_size != 0U) {{\n    if (storage->data == NULL) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_STORAGE_NULL, \"storage.data\", requirements->storage_size, 0U, WL_ERR_INVALID_ARG);\n    if (((uintptr_t)storage->data & (requirements->storage_alignment - 1U)) != 0U) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_STORAGE_ALIGNMENT, \"storage.data\", requirements->storage_alignment, (size_t)((uintptr_t)storage->data & (requirements->storage_alignment - 1U)), WL_ERR_INVALID_ARG);\n    instance_address = (uintptr_t)(const void *)instance;\n    storage_address = (uintptr_t)storage->data;\n    if ((storage_address <= instance_address && instance_address - storage_address < requirements->storage_size) || (instance_address < storage_address && storage_address - instance_address < sizeof(*instance))) return {module}_runtime_init_failure(diagnostic, {prefix}_RUNTIME_INIT_STORAGE_OVERLAP, \"storage.data\", requirements->storage_size, storage->size, WL_ERR_INVALID_ARG);\n  }}\n  return WL_OK;\n}}\n\nint {module}_runtime_init({module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage) {{\n  {module}_runtime_config_t config_copy;\n  {module}_runtime_storage_t storage_copy;\n  {module}_runtime_requirements_t requirements;\n  {module}_runtime_layout_t layout;\n  uintptr_t instance_address;\n  uintptr_t storage_address;\n  int result;\n  if (instance == NULL || config == NULL || storage == NULL) return WL_ERR_INVALID_ARG;\n  config_copy = *config;\n  storage_copy = *storage;\n  config = &config_copy;\n  storage = &storage_copy;\n  result = {module}_runtime_requirements(config, &requirements);\n  if (result != WL_OK) return result;\n  if (storage->size < requirements.storage_size) return WL_ERR_BUF_TOO_SMALL;\n  if (requirements.storage_size != 0U) {{\n    if (storage->data == NULL || ((uintptr_t)storage->data & (requirements.storage_alignment - 1U)) != 0U) return WL_ERR_INVALID_ARG;\n    instance_address = (uintptr_t)(void *)instance;\n    storage_address = (uintptr_t)storage->data;\n    if ((storage_address <= instance_address && instance_address - storage_address < requirements.storage_size) || (instance_address < storage_address && storage_address - instance_address < sizeof(*instance))) return WL_ERR_INVALID_ARG;\n  }}\n  result = {module}_runtime_layout(config, (uint8_t *)storage->data, storage->size, &layout, NULL);\n  if (result != WL_OK) return result;\n  memset(instance, 0, sizeof(*instance));\n"
    )
    .unwrap();

    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => {
                write!(
                    output,
                    "  {{\n    const wl_latest_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_latest_initial_generation}};\n    wl_latest_requirements_t route_requirements;\n    wl_latest_storage_t route_storage;\n    result = wl_latest_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) goto init_failed;\n    route_storage.data = layout.{message}_latest_storage;\n    route_storage.size = route_requirements.storage_size;\n    result = wl_latest_init(&instance->{message}_latest, &route_config, &route_storage);\n    if (result != WL_OK) goto init_failed;\n    instance->runtime.{message}_latest = &instance->{message}_latest;\n  }}\n"
                )
                .unwrap();
            }
            RetainedRouteKind::Fifo => {
                write!(
                    output,
                    "  {{\n    const wl_fifo_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_fifo_capacity}};\n    wl_fifo_requirements_t route_requirements;\n    wl_fifo_storage_t route_storage;\n    result = wl_fifo_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) goto init_failed;\n    route_storage.data = layout.{message}_fifo_storage;\n    route_storage.size = route_requirements.storage_size;\n    result = wl_fifo_init(&instance->{message}_fifo, &route_config, &route_storage);\n    if (result != WL_OK) goto init_failed;\n    instance->runtime.{message}_fifo = &instance->{message}_fifo;\n  }}\n"
                )
                .unwrap();
            }
        }
    }

    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (config->rpc_client_enabled != 0U) {{\n    const wl_rpc_client_config_t client_config = {{\n      (wl_rpc_client_slot_t *)layout.rpc_client_slots,\n      config->rpc_client_slot_count,\n      (uint8_t *)layout.rpc_client_responses,\n      layout.rpc_client_responses_size,\n      config->rpc_client_response_capacity,\n      config->rpc_client_next_operation_id\n    }};\n    if (wl_rpc_client_init(&instance->rpc_client, &client_config) != WL_RPC_OK) {{\n      result = WL_ERR_INVALID_ARG;\n      goto init_failed;\n    }}\n    instance->runtime.rpc_client = &instance->rpc_client;\n  }}\n  if (config->rpc_server_enabled != 0U) {{\n    const wl_rpc_server_config_t server_config = {{\n      (wl_rpc_server_pending_slot_t *)layout.rpc_server_pending_slots,\n      config->rpc_server_pending_slot_count,\n      (wl_rpc_server_cache_slot_t *)layout.rpc_server_cache_slots,\n      config->rpc_server_cache_slot_count,\n      (uint8_t *)layout.rpc_server_responses,\n      layout.rpc_server_responses_size,\n      config->rpc_server_response_capacity,\n      config->rpc_server_pending_timeout_ms,\n      config->rpc_server_cache_ttl_ms,\n      config->rpc_server_cache_policy\n    }};\n    if (wl_rpc_server_init(&instance->rpc_server, &server_config) != WL_RPC_OK) {{\n      result = WL_ERR_INVALID_ARG;\n      goto init_failed;\n    }}\n    instance->runtime.rpc_server = &instance->rpc_server;\n  }}\n"
        )
        .unwrap();
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            write!(
                output,
                "  if (config->rpc_server_enabled != 0U) {{\n    instance->runtime.{service_name}.request_scratch = &instance->{service_name}_scratch.request;\n    instance->runtime.{service_name}.request_handler = config->{service_name}_request_handler;\n    instance->runtime.{service_name}.user_data = config->{service_name}_user_data;\n  }}\n  if (config->rpc_client_enabled != 0U) instance->runtime.{service_name}.response_scratch = &instance->{service_name}_scratch.response;\n"
            )
            .unwrap();
        }
        if profile
            .rpc_services
            .iter()
            .any(|service| !service.is_managed())
        {
            writeln!(
            output,
            "  if (config->rpc_client_enabled != 0U || config->rpc_server_enabled != 0U) instance->runtime.rpc_encode_scratch = &instance->rpc_encode_scratch;"
        )
        .unwrap();
        }
    }
    output.push_str("  return WL_OK;\n");
    if has_components {
        output.push_str(
            "\ninit_failed:\n  memset(instance, 0, sizeof(*instance));\n  return result;\n",
        );
    }
    write!(
        output,
        "}}\n\nint {module}_runtime_init_checked({module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage, {module}_runtime_init_diagnostic_t *out_diagnostic) {{\n  {module}_runtime_requirements_t requirements;\n  int result = {module}_runtime_init_validate(instance, config, storage, &requirements, out_diagnostic);\n  if (result != WL_OK) return result;\n  result = {module}_runtime_init(instance, config, storage);\n  if (result != WL_OK) return {module}_runtime_init_failure(out_diagnostic, {prefix}_RUNTIME_INIT_COMPONENT, \"component\", 0U, 0U, result);\n  return WL_OK;\n}}\n\n"
    )
    .unwrap();
}
