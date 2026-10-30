// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, binding_profile_identity,
    codec_plan::{CodecPlan, CodecProfile, ValueEncoding},
    compose_binding_profiles, generate_c, generate_runtime_c, parse_binding_profile, parse_schema,
    plan_manifest::generate_binding_plan_manifest,
    profile_semantic::RpcGuarantee,
    rpc_plan::RpcEndpointPlan,
};

const SCHEMA: &str = "version 1;
message Request @id(1) {}
message Response @id(2) { required fixed32 v @id(1); }
message Query @id(3) { required uint32 v @id(1); }
message Large @id(4) { required bytes<256> blob @id(1); }
message Sample @id(5) { required fixed32 n @id(1); }";
const REPLAY: &str = "rpc Set { request=Request; response=Response; }";
const CORRELATE: &str = "rpc Query { request=Query; response=Large; guarantee=correlate; }";

fn schema(source: &str) -> wlc::SemanticModel {
    analyze_schema(&parse_schema(source).unwrap()).unwrap()
}
fn profile(schema: &wlc::SemanticModel, source: &str) -> wlc::BindingProfileModel {
    analyze_binding_profile(
        &parse_binding_profile(&format!("profile version 1; {source}")).unwrap(),
        schema,
    )
    .unwrap()
}

#[test]
fn explicit_defaults_preserve_models_identities_and_all_generated_bytes() {
    let schema = schema(SCHEMA);
    let implicit = profile(
        &schema,
        &format!("{REPLAY} latest Sample {{ delivery=unreliable; }}"),
    );
    let explicit = profile(
        &schema,
        "latest Sample { codec=schema_v1; delivery=unreliable; }
        rpc Set { codec=managed_tagged; response=Response; guarantee=replay; request=Request; }",
    );
    assert_eq!(implicit, explicit);
    assert_eq!(
        binding_profile_identity(&implicit),
        binding_profile_identity(&explicit)
    );
    assert_eq!(
        generate_runtime_c(&schema, &implicit, "demo").unwrap(),
        generate_runtime_c(&schema, &explicit, "demo").unwrap()
    );
    assert_eq!(
        generate_binding_plan_manifest(&schema, &implicit).unwrap(),
        generate_binding_plan_manifest(&schema, &explicit).unwrap()
    );
}

#[test]
fn scalar_profiles_have_full_domain_bounds_and_keep_packed_fixed_width() {
    let schema = schema(
        "version 1; message Values @id(1) {
        required fixed32 scalar @id(16); optional uint16 opt @id(1);
        required packed fixed32 array[2] @id(2); required float32 f @id(3);
        required float64 d @id(4); required bool b @id(5); required int8 s @id(6);
    }",
    );
    for (codec, maximum) in [
        (CodecProfile::SchemaV1, 39),
        (CodecProfile::ManagedTagged, 40),
        (CodecProfile::PositionalV1, 33),
    ] {
        let plan = CodecPlan::new(&schema, codec).unwrap();
        let msg = &plan.messages[&1];
        assert_eq!(msg.max_encoded_size, Some(maximum));
        assert!(msg.has_owned_value());
        assert_eq!(
            msg.fields
                .iter()
                .map(|p| p.field.number)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6, 16]
        );
        assert_eq!(msg.fields[1].encoding, ValueEncoding::BigEndian(4));
        assert_eq!(msg.fields[2].encoding, ValueEncoding::IeeeBigEndian(4));
        assert_eq!(msg.fields[3].encoding, ValueEncoding::IeeeBigEndian(8));
        assert_eq!(
            msg.fields[6].encoding,
            if codec == CodecProfile::SchemaV1 {
                ValueEncoding::BigEndian(4)
            } else {
                ValueEncoding::UnsignedVarint(32)
            }
        );
        assert_eq!(
            msg.presence_bytes,
            usize::from(codec == CodecProfile::PositionalV1)
        );
        if codec == CodecProfile::PositionalV1 {
            assert_eq!(msg.fields[0].presence_bit, Some(0));
            assert!(msg.fields[1..].iter().all(|f| f.presence_bit.is_none()));
            assert!(msg.fields.iter().all(|f| f.wire_type.is_none()));
        }
    }
    // The plan borrows the business schema; no fixed32 -> uint32 mutation.
    let public = generate_c(&schema, "values").unwrap();
    assert!(
        public
            .values_header
            .contains("VALUES_MAX_ENCODED_SIZE UINT64_C(39)")
    );
}

#[test]
fn positional_presence_is_fixed_and_selected_messages_ignore_unrelated_types() {
    let fields = (1..=9)
        .rev()
        .map(|id| format!("optional bool b{id} @id({id});"))
        .collect::<String>();
    let schema = schema(&format!(
        "version 1; message Bits @id(1) {{{fields}}}
        message Other @id(2) {{ repeated string text @id(1); }}"
    ));
    let plan = CodecPlan::selected(&schema, CodecProfile::PositionalV1, [1]).unwrap();
    assert_eq!(plan.messages.len(), 1);
    let msg = &plan.messages[&1];
    assert_eq!(msg.presence_bytes, 2);
    assert_eq!(msg.max_encoded_size, Some(11));
    for (i, field) in msg.fields.iter().enumerate() {
        assert_eq!(field.presence_bit, Some(i));
    }
    assert!(CodecPlan::new(&schema, CodecProfile::PositionalV1).is_err());
    assert!(CodecPlan::selected(&schema, CodecProfile::SchemaV1, [999]).is_err());
}

#[test]
fn mixed_guarantees_trim_history_bounds_and_roles() {
    let schema = schema(SCHEMA);
    let mixed = format!("{REPLAY}{CORRELATE}");
    let model = profile(&schema, &mixed);
    assert_eq!(model.rpc_services[0].guarantee, RpcGuarantee::Correlate);
    let plan = RpcEndpointPlan::new(&schema, &model).unwrap();
    assert_eq!(plan.client_result_methods, ["Query", "Set"]);
    assert_eq!(plan.pending_response_methods, ["Query", "Set"]);
    assert_eq!(plan.replay_methods, ["Set"]);
    assert_eq!(plan.request_snapshot_capacity, Some(19));
    assert_eq!(plan.pending_response_capacity, Some(272));
    assert_eq!(plan.replay_response_capacity, Some(19));
    let correlate = profile(
        &schema,
        &CORRELATE.replace("response=Large", "response=Response"),
    );
    let light = RpcEndpointPlan::new(&schema, &correlate).unwrap();
    assert!(light.replay_methods.is_empty());
    assert_eq!(light.replay_response_capacity, None);
    assert_eq!(light.pending_response_capacity, Some(19));
    for role in ["client", "server"] {
        let profile = profile(&schema, &format!("endpoint {{rpc_role={role};}} {mixed}"));
        let plan = RpcEndpointPlan::new(&schema, &profile).unwrap();
        if role == "client" {
            assert!(plan.pending_response_methods.is_empty());
            assert!(plan.replay_methods.is_empty());
            assert_eq!(plan.pending_response_capacity, None);
            assert_eq!(plan.client_result_methods, ["Query", "Set"]);
            plan.validate_client_results().unwrap();
        } else {
            assert!(plan.client_result_methods.is_empty());
            assert_eq!(plan.request_snapshot_capacity, None);
            plan.validate_client_results().unwrap();
        }
    }
}

#[test]
fn unbounded_correlate_response_does_not_poison_replay_history() {
    let schema = schema(&SCHEMA.replace("bytes<256>", "bytes"));
    let model = profile(&schema, &format!("{REPLAY}{CORRELATE}"));
    let plan = RpcEndpointPlan::new(&schema, &model).unwrap();
    assert_eq!(plan.pending_response_capacity, None);
    assert_eq!(plan.replay_response_capacity, Some(19));
    assert!(
        plan.validate_client_results()
            .unwrap_err()
            .0
            .contains("statically bounded response")
    );
    let manifest = generate_binding_plan_manifest(&schema, &model).unwrap();
    assert!(manifest.contains("\"owned_result_available\": false"));
}

#[test]
fn choices_change_identity_and_compilation_never_silently_falls_back() {
    let schema = schema(SCHEMA);
    let base = profile(&schema, REPLAY);
    let correlate = profile(
        &schema,
        &REPLAY.replace("request=Request;", "request=Request; guarantee=correlate;"),
    );
    let positional = profile(
        &schema,
        &REPLAY.replace("request=Request;", "request=Request; codec=positional_v1;"),
    );
    assert_ne!(
        binding_profile_identity(&base),
        binding_profile_identity(&correlate)
    );
    assert_ne!(
        binding_profile_identity(&base),
        binding_profile_identity(&positional)
    );
    assert_ne!(
        binding_profile_identity(&correlate),
        binding_profile_identity(&positional)
    );
    let light = generate_runtime_c(&schema, &correlate, "demo").unwrap();
    assert!(light.source.contains("wl_rpc_server_begin_correlate"));
    assert!(!light.source.contains("wlc_detail_fingerprint"));
    let positioned = generate_runtime_c(&schema, &positional, "demo").unwrap();
    assert!(positioned.source.contains("positional_decode"));
    assert!(positioned.header.contains("managed-rpc-v4-positional-v1"));
    let route = profile(
        &schema,
        "send Sample {delivery=unreliable; codec=positional_v1;}",
    );
    assert!(
        generate_runtime_c(&schema, &route, "demo")
            .unwrap()
            .header
            .contains("Route codec/profile mismatch")
    );
    assert_ne!(
        binding_profile_identity(&route),
        binding_profile_identity(&profile(&schema, "send Sample {delivery=unreliable;}"))
    );
}

#[test]
fn semantic_errors_cover_unknown_duplicate_wrong_context_and_unsupported_shapes() {
    let schema = schema(SCHEMA);
    for source in [
        "rpc Set {request=Request;response=Response;guarantee=once;}",
        "rpc Set {request=Request;response=Response;codec=schema_v1;}",
        "rpc Set {request=Request;response=Response;codec=unknown;}",
        "send Sample {delivery=unreliable;codec=managed_tagged;}",
        "rpc Set {request=Request;response=Large;codec=positional_v1;}",
    ] {
        let parsed = parse_binding_profile(&format!("profile version 1;{source}")).unwrap();
        assert!(
            analyze_binding_profile(&parsed, &schema).is_err(),
            "{source}"
        );
    }
    for source in [
        "rpc Set {request=Request;response=Response;guarantee=replay;guarantee=correlate;}",
        "rpc Set {request=Request;response=Response;codec=managed_tagged;codec=positional_v1;}",
        "rpc Set {request=Request;response=Response;codec=managed_tagged @delivery(reliable);}",
        "send Sample {delivery=unreliable;codec=schema_v1;codec=positional_v1;}",
        "send Sample {delivery=unreliable;guarantee=replay;}",
        "send Sample {codec=schema_v1;}",
        "send Sample {codec=schema_v1;",
    ] {
        assert!(
            parse_binding_profile(&format!("profile version 1;{source}")).is_err(),
            "{source}"
        );
    }
    for field in [
        "optional string<8> name @id(1);",
        "required bytes<8> name @id(1);",
        "optional Child name @id(1);",
        "repeated uint32 name @id(1);",
    ] {
        let model = self::schema(&format!(
            "version 1; message Child @id(9) {{}} message Values @id(1) {{{field}}}"
        ));
        assert!(
            CodecPlan::new(&model, CodecProfile::PositionalV1)
                .unwrap_err()
                .0
                .contains("Values.name")
        );
    }
}

#[test]
fn route_codec_conflicts_are_rejected_within_and_across_fragments() {
    let schema = schema(SCHEMA);
    let send = "send Sample {delivery=unreliable;codec=positional_v1;}";
    let latest = "latest Sample {delivery=unreliable;}";
    let same = parse_binding_profile(&format!("profile version 1;{send}{latest}")).unwrap();
    assert!(
        analyze_binding_profile(&same, &schema)
            .unwrap_err()
            .errors()
            .iter()
            .any(|e| e.message.contains("conflicting route codecs"))
    );
    let send = profile(&schema, send);
    let latest = profile(&schema, latest);
    assert!(
        compose_binding_profiles(&[send, latest])
            .unwrap_err()
            .0
            .contains("conflicting route codecs across profiles")
    );
    let a = profile(&schema, REPLAY);
    let b = profile(&schema, CORRELATE);
    let first = compose_binding_profiles(&[a.clone(), b.clone()]).unwrap();
    let second = compose_binding_profiles(&[b, a]).unwrap();
    assert_eq!(
        generate_binding_plan_manifest(&schema, &first).unwrap(),
        generate_binding_plan_manifest(&schema, &second).unwrap()
    );
}

#[test]
fn wire_bound_overflow_is_an_error_not_an_unbounded_value() {
    // Eight levels are legal Schema-v1. A shared DAG can still exceed u64
    // when every nested field's full-domain maximum is included.
    let mut source =
        "version 1; message M0 @id(1) {required bytes<65535> payload @id(1);}".to_string();
    for level in 1..=8 {
        source.push_str(&format!("message M{level} @id({}) {{", level + 1));
        for field in 1..=64 {
            source.push_str(&format!("optional M{} f{field} @id({field});", level - 1));
        }
        source.push('}');
    }
    let schema = schema(&source);
    for codec in [CodecProfile::SchemaV1, CodecProfile::ManagedTagged] {
        assert!(
            CodecPlan::new(&schema, codec)
                .unwrap_err()
                .0
                .contains("wire size overflow")
        );
    }
    assert!(
        generate_c(&schema, "demo")
            .unwrap_err()
            .0
            .contains("wire size overflow")
    );
}

#[test]
fn generated_owned_union_alignment_and_arena_overflow_checks_compile_and_run() {
    let schema =
        schema(&SCHEMA.replace("required fixed32 v @id(1);", "required float64 v @id(1);"));
    let model = profile(
        &schema,
        &format!(
            "{REPLAY}{}",
            CORRELATE
                .replace("guarantee=correlate;", "")
                .replace("rpc Query", "rpc Fetch")
        ),
    );
    let c = generate_c(&schema, "demo").unwrap();
    let runtime = generate_runtime_c(&schema, &model, "demo").unwrap();
    assert!(runtime.header.contains("typedef union {\n  large_value_t fetch;\n  response_value_t set;\n} demo_rpc_result_value_t;\n"));
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("demo.h", c.header),
        ("demo_values.h", c.values_header),
        ("demo.c", c.source),
        ("demo_bindings.h", c.bindings_header),
        ("demo_bindings.c", c.bindings_source),
        ("demo_runtime.h", runtime.header),
        ("demo_runtime.c", runtime.source),
    ] {
        fs::write(dir.path().join(name), text).unwrap();
    }
    fs::write(
        dir.path().join("test.c"),
        format!(
            "#include <assert.h>\n#include \"demo_runtime.c\"\n{}",
            include_str!("fixtures/rpc_result_layout.c")
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let binary = dir.path().join("test");
    let mut command = Command::new("cc");
    command
        .args([
            "-std=c11",
            "-O2",
            "-fstrict-aliasing",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
        ])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("test.c"))
        .arg(dir.path().join("demo.c"))
        .arg(dir.path().join("demo_bindings.c"));
    for source in fs::read_dir(root.join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "c"))
    {
        command.arg(source);
    }
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        command.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let output = command.arg("-o").arg(&binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(Command::new(binary).status().unwrap().success());
}

#[test]
fn runtime_names_reject_codec_handler_collisions_and_allow_distinct_prefixes() {
    let schema = schema(SCHEMA);
    let model = profile(&schema, &CORRELATE.replace("guarantee=correlate;", ""));
    assert!(
        generate_runtime_c(&schema, &model, "demo")
            .unwrap_err()
            .0
            .contains("demo_query_handler_fn")
    );
    assert!(wlc::generate_runtime_c_named(&schema, &model, "demo", "other").is_ok());
}

#[test]
fn policy_identities_have_frozen_vectors() {
    let schema = schema(SCHEMA);
    let mut actual = String::new();
    for guarantee in ["replay", "correlate"] {
        for codec in ["managed_tagged", "positional_v1"] {
            let model = profile(
                &schema,
                &format!(
                    "rpc Set {{request=Request;response=Response;guarantee={guarantee};codec={codec};}}"
                ),
            );
            actual.push_str(&format!(
                "{guarantee}/{codec}: 0x{:016x} 0x{:016x}\n",
                wlc::schema_identity(&schema),
                binding_profile_identity(&model)
            ));
        }
    }
    assert_eq!(actual, include_str!("fixtures/rpc_policy_identity.txt"));
}

#[test]
fn mapped_metadata_has_an_explicit_schema_v1_default_and_rejects_wrong_codecs() {
    let schema = schema(
        "version 1; message Request @id(1) {required uint32 call @id(1);} message Response @id(2) {required uint32 call @id(1);required int32 status @id(2);}",
    );
    let source = "rpc Set {request=Request;response=Response;request_operation_id=call;response_operation_id=call;response_status=status;}";
    let implicit = profile(&schema, source);
    let explicit = profile(
        &schema,
        &source.replace(
            "request=Request;",
            "request=Request;codec=schema_v1;guarantee=replay;",
        ),
    );
    assert_eq!(implicit, explicit);
    assert_eq!(implicit.rpc_services[0].codec, CodecProfile::SchemaV1);
    for choice in ["managed_tagged", "positional_v1"] {
        let source = source.replace(
            "request=Request;",
            &format!("request=Request;codec={choice};"),
        );
        assert!(
            analyze_binding_profile(
                &parse_binding_profile(&format!("profile version 1;{source}")).unwrap(),
                &schema
            )
            .is_err()
        );
    }
}

#[test]
fn optional_packed_arrays_use_one_presence_bit_and_retain_a_checked_length_prefix() {
    let schema = schema(
        "version 1; message Arrays @id(1) {packed float32 data[2] @id(16); required packed fixed64 words[2] @id(1);}",
    );
    let plan = CodecPlan::new(&schema, CodecProfile::PositionalV1).unwrap();
    let array = &plan.messages[&1];
    assert_eq!(array.presence_bytes, 1);
    assert_eq!(array.max_encoded_size, Some(1 + 17 + 9));
    assert_eq!(array.fields[0].presence_bit, None);
    assert_eq!(array.fields[1].presence_bit, Some(0));
    assert_eq!(array.fields[1].encoding, ValueEncoding::IeeeBigEndian(4));
}

#[test]
fn result_validation_is_role_aware_and_keeps_member_name_collisions() {
    let schema = schema(&SCHEMA.replace("bytes<256>", "bytes"));
    for role in ["client", "server", "both"] {
        let model = profile(
            &schema,
            &format!(
                "endpoint {{rpc_role={role};}} {}",
                CORRELATE.replace("rpc Query", "rpc Fetch")
            ),
        );
        let plan = RpcEndpointPlan::new(&schema, &model).unwrap();
        let validation = plan.validate_client_results();
        let runtime = generate_runtime_c(&schema, &model, "demo");
        if role == "server" {
            validation.unwrap();
            assert!(
                !runtime
                    .unwrap()
                    .header
                    .contains("} demo_rpc_result_value_t;")
            );
        } else {
            let error = validation.unwrap_err();
            assert!(error.0.contains("statically bounded response"));
            assert!(runtime.unwrap_err().0.ends_with(&error.to_string()));
        }
    }
    let mut model = profile(&schema, &format!("{REPLAY}{CORRELATE}"));
    model.rpc_services[0].name = "switch".into();
    model.rpc_services[1].name = "switch_".into();
    assert!(
        RpcEndpointPlan::new(&schema, &model)
            .unwrap_err()
            .0
            .contains("colliding result member name")
    );
}

#[test]
fn runtime_backend_emits_only_planned_result_members_in_plan_order() {
    let schema = schema(SCHEMA);
    for (methods, expected) in [
        ("".to_owned(), None),
        (
            REPLAY.to_owned(),
            Some("typedef union {\n  response_value_t set;\n} demo_rpc_result_value_t;\n"),
        ),
        (
            format!("{REPLAY}{}", CORRELATE.replace("rpc Query", "rpc Fetch")),
            Some(
                "typedef union {\n  large_value_t fetch;\n  response_value_t set;\n} demo_rpc_result_value_t;\n",
            ),
        ),
    ] {
        for role in ["client", "server", "both"] {
            let model = profile(&schema, &format!("endpoint {{rpc_role={role};}} {methods}"));
            let runtime = generate_runtime_c(&schema, &model, "demo").unwrap();
            if role != "server"
                && let Some(expected) = expected
            {
                assert!(runtime.header.contains(expected));
            } else {
                assert!(!runtime.header.contains("} demo_rpc_result_value_t;"));
            }
        }
    }
}

#[test]
fn rpc_wire_bounds_and_c_storage_limits_remain_distinct() {
    // TLV bytes at this length use one key byte and a three-byte length.
    for (length, body, wire, storage) in [(65518, 65522, 65535, true), (65519, 65523, 65536, false)]
    {
        let schema = schema(&format!(
            "version 1;message Request @id(1) {{}} message Response @id(2) {{required bytes<{length}> data @id(1);}}"
        ));
        for role in ["client", "server", "both"] {
            let model = profile(&schema, &format!("endpoint {{rpc_role={role};}} {REPLAY}"));
            let plan = RpcEndpointPlan::new(&schema, &model).unwrap();
            let method = &plan.methods[0];
            assert_eq!(method.request_body_capacity, Some(0));
            assert_eq!(method.request_capacity, Some(13));
            assert_eq!(method.response_body_capacity, Some(body));
            assert_eq!(method.response_capacity, Some(wire));
            let runtime = generate_runtime_c(&schema, &model, "demo").unwrap();
            assert!(runtime.header.contains(&format!(
                "#define DEMO_RUNTIME_HAS_DEFAULT_STORAGE {}",
                u8::from(storage)
            )));
            assert!(
                runtime
                    .header
                    .contains("#define DEMO_HAS_DEFAULT_ENDPOINT 0")
            );
            assert_eq!(
                runtime
                    .source
                    .contains("config->rpc_server_response_capacity = 65535U;"),
                storage
            );
            assert!(
                !runtime
                    .source
                    .contains("config->rpc_server_response_capacity = 65536U;")
            );
            // Client-only retains the advanced server defaults/declarations.
            assert!(
                runtime
                    .header
                    .contains("uint16_t rpc_server_response_capacity;")
            );
        }
    }
}

#[test]
fn empty_rpc_body_keeps_rejection_and_snapshot_capacity_contracts() {
    let schema = schema("version 1;message Request @id(1) {} message Response @id(2) {}");
    for codec in ["managed_tagged", "positional_v1"] {
        let model = profile(
            &schema,
            &format!("rpc Set {{request=Request;response=Response;codec={codec};}}"),
        );
        let plan = RpcEndpointPlan::new(&schema, &model).unwrap();
        let method = &plan.methods[0];
        assert_eq!(method.request_body_capacity, Some(0));
        assert_eq!(method.response_body_capacity, Some(0));
        assert_eq!(method.request_capacity, Some(13));
        assert_eq!(method.response_capacity, Some(17));
        let runtime = generate_runtime_c(&schema, &model, "demo").unwrap();
        assert!(
            runtime
                .header
                .contains("#define DEMO_ENDPOINT_REQUEST_CAPACITY 13U")
        );
        assert!(
            runtime
                .source
                .contains("config->rpc_server_response_capacity = 17U;")
        );
        assert!(
            runtime
                .header
                .contains("sizeof(wl_rpc_server_cache_slot_t) + 17U")
        );
    }
}

#[test]
fn backend_capacity_lookup_preserves_profile_method_order() {
    let schema = schema(SCHEMA);
    let mut model = profile(
        &schema,
        &format!("{REPLAY}{}", CORRELATE.replace("rpc Query", "rpc Fetch")),
    );
    model.rpc_services.reverse();
    let plan = RpcEndpointPlan::new(&schema, &model).unwrap();
    assert_eq!(
        plan.methods
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["Fetch", "Set"]
    );
    let runtime = generate_runtime_c(&schema, &model, "demo").unwrap();
    let set = runtime.header.find("void *set_user_data;").unwrap();
    let fetch = runtime.header.find("void *fetch_user_data;").unwrap();
    assert!(set < fetch); // Renderer order follows the supplied profile.
    assert!(
        runtime
            .header
            .contains("typedef union {\n  large_value_t fetch;\n  response_value_t set;")
    );
}
