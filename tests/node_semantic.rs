// SPDX-License-Identifier: Apache-2.0
use std::fs;
use wlc::node::{Framing, Handlers};
use wlc::{analyze_nodes, analyze_schema, generate_node_c, parse_schema};
const MESSAGES: &str = "message Req @id(1) { uint32 value @id(1); } message Resp @id(2) { uint32 value @id(1); } message Tick @id(3) { uint32 value @id(1); }";
fn nodes(tail: &str) -> Result<Vec<wlc::Node>, String> {
    let ast = parse_schema(&format!("{MESSAGES} {tail}")).map_err(|e| e.to_string())?;
    let model = analyze_schema(&ast).map_err(|e| e.to_string())?;
    analyze_nodes(&ast, &model).map_err(|e| e.to_string())
}
#[test]
fn defaults_and_explicit_configuration_have_the_same_model() {
    let a = nodes("rpc Add(Req)->Resp; node sensor { send Tick; recv Tick; call Add; serve Add; }")
        .unwrap();
    let b=nodes("node sensor { serve Add(Req)->Resp; call Add; recv Tick @codec(schema_v1); send Tick; rx_slots=2; timeout_ms=500; framing=native; codec=schema_v1; handlers=static; }").unwrap();
    assert_eq!(a, b);
    assert_eq!(a[0].framing, Framing::Native);
    assert_eq!(a[0].handlers, Handlers::Static);
    assert_eq!(wlc::node_identity(&a[0]), wlc::node_identity(&b[0]));
    let mut changed = a[0].clone();
    for slots in [4, 8] {
        let configured = nodes(&format!("node n {{ rx_slots={slots}; }}")).unwrap();
        assert_eq!(configured[0].rx_slots, slots);
    }
    changed.rx_slots = 4;
    assert_ne!(wlc::node_identity(&a[0]), wlc::node_identity(&changed));
    changed = a[0].clone();
    changed.timeout_ms = 501;
    assert_ne!(wlc::node_identity(&a[0]), wlc::node_identity(&changed));
}
#[test]
fn invalid_nodes_fail_before_generation() {
    for tail in [
        "node n { framing=magic; }",
        "node n { codec=magic; }",
        "node n { io=magic; }",
        "node n { handlers=magic; }",
        "node n { rx_slots=0; }",
        "node n { rx_slots=3; }",
        "node n { rx_slots=16; }",
        "node n { timeout_ms=0; }",
        "node n { timeout_ms=2147483648; }",
        "node n { timeout_ms=1; timeout_ms=1; }",
        "node n { send Missing; }",
        "node n { call Missing; }",
        "node n { recv Tick; recv Tick; }",
        "node n { send Tick; send Tick; }",
        "node n { call Add(Req)->Resp; recv Req; }",
        "node n { serve Add(Req)->Req; }",
        "rpc Add(Req)->Resp; rpc Add(Req)->Resp; node n {}",
        "rpc Add(Req)->Resp; node n { call Add(Req)->Tick; }",
        "node n { call Add(Req)->Resp; call Other(Req)->Tick; }",
        "node n { recv Tick = switch; }",
        "node n { recv Tick = wl_invalid; }",
        "node n { handlers=dynamic; recv Tick = custom; }",
        "node n { send Tick = custom; }",
        "node n { call Add(Req)->Resp @codec(schema_v1); }",
        "node n { send Tick @delivery(reliable); }",
        "node n { delivery = reliable; }",
        "node n {} node n {}",
        "node ReadValue {} node read_value {}",
        "node n { call Add(Req) > Resp; }",
        "profile version 2;",
    ] {
        assert!(nodes(tail).is_err(), "accepted {tail}");
    }
}
#[test]
fn bounds_include_rpc_metadata_and_received_nested_repeated() {
    for (size, valid) in [(2037, true), (2038, false)] {
        // bytes length prefix = 2, key = 1, RPC metadata = 8.
        let source = format!(
            "message Req @id(1) {{bytes<{size}> data @id(1);}} message Resp @id(2) {{}} node n {{call F(Req)->Resp;}}"
        );
        let ast = parse_schema(&source).unwrap();
        let model = analyze_schema(&ast).unwrap();
        assert_eq!(analyze_nodes(&ast, &model).is_ok(), valid);
    }
    for source in [
        "message A @id(1){string value @id(1);} node n {send A;}",
        "message A @id(1){repeated uint32 value @id(1);} message B @id(2){A child @id(1);} node n {recv B;}",
    ] {
        let ast = parse_schema(source).unwrap();
        let model = analyze_schema(&ast).unwrap();
        assert!(analyze_nodes(&ast, &model).is_err());
    }
}
#[test]
fn default_required_and_optional_packed_are_distinct() {
    let ast=parse_schema("message A @id(1){uint32 value @id(1); packed float32 values[2] @id(2); optional packed float32 maybe[2] @id(3);}").unwrap();
    assert_eq!(ast.version.value, 1);
    let wlc::ast::Declaration::Message(m) = &ast.declarations[0] else {
        panic!()
    };
    assert_eq!(m.fields[0].cardinality, wlc::ast::Cardinality::Required);
    assert_eq!(
        m.fields[1].cardinality,
        wlc::ast::Cardinality::RequiredPacked(2)
    );
    assert_eq!(m.fields[2].cardinality, wlc::ast::Cardinality::Packed(2));
}
#[test]
fn imported_optional_versions_compose_rpc_and_nodes() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("types.wl"), MESSAGES).unwrap();
    fs::write(
        dir.path().join("rpc.wl"),
        "import \"types.wl\"; rpc Add(Req)->Resp;",
    )
    .unwrap();
    fs::write(
        dir.path().join("main.wl"),
        "import \"rpc.wl\"; import \"types.wl\"; node client {call Add;} node server {serve Add;}",
    )
    .unwrap();
    let loaded = wlc::load_schema(&dir.path().join("main.wl")).unwrap();
    assert_eq!(loaded.dependencies.len(), 3);
    let model = analyze_schema(&loaded.schema).unwrap();
    let nodes = analyze_nodes(&loaded.schema, &model).unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(wlc::node::select_nodes(&nodes, None, false).is_err());
    assert_eq!(
        wlc::node::select_nodes(&nodes, Some("client"), false)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        wlc::node::select_nodes(&nodes, None, true).unwrap().len(),
        2
    );
}
#[test]
fn c_namespace_collisions_are_rejected_before_emission() {
    for source in [
        "message NNode @id(1){} node n {}",
        "message M @id(1){} node n {recv M = m_t;}",
        "message M @id(1){} node n {recv M = n_route;}",
        "message M @id(1){string<2> name @id(1);} node n {recv M = m_set_name;}",
        "message M @id(1){} enum E @id(2){N_RX_FRAME_MAX=1;} node n {}",
        "message M @id(1){} node n {recv M = m_decode;}",
    ] {
        let ast = parse_schema(source).unwrap();
        let model = analyze_schema(&ast).unwrap();
        let nodes = analyze_nodes(&ast, &model).unwrap();
        assert!(
            generate_node_c(&model, &nodes, "test").is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn custom_c_symbols_are_preserved_and_local_members_do_not_collide() {
    let ast = parse_schema(&format!(
        "{MESSAGES} node Sensor {{ recv Tick = MyReceive; }}"
    ))
    .unwrap();
    let model = analyze_schema(&ast).unwrap();
    let nodes = analyze_nodes(&ast, &model).unwrap();
    assert_eq!(
        wlc::node::select_nodes(&nodes, Some("Sensor"), false)
            .unwrap()
            .len(),
        1
    );
    let generated = generate_node_c(&model, &nodes, "test").unwrap();
    assert!(generated.files["test.c"].contains("MyReceive(self, &message)"));
    for tail in [
        "node n { handlers=dynamic; recv Tick; serve Tick(Req)->Resp; }",
        "node n { serve ReadValue(Req)->Resp = first; serve read_value(Tick)->Other = second; }",
    ] {
        let ast = parse_schema(&format!("{MESSAGES} message Other @id(4){{}} {tail}")).unwrap();
        let model = analyze_schema(&ast).unwrap();
        let nodes = analyze_nodes(&ast, &model).unwrap();
        assert!(generate_node_c(&model, &nodes, "test").is_err());
    }
}

#[test]
fn pure_codec_plan_keeps_field_resource_facts() {
    let ast = parse_schema(
        "message M @id(1){bytes<9> data @id(1); optional packed float32 values[2] @id(2);}",
    )
    .unwrap();
    let model = analyze_schema(&ast).unwrap();
    let plan = wlc::plan_manifest::generate_node_plan_manifest(&model, &[]).unwrap();
    assert!(plan.contains("\"message_id\": 1"));
    assert!(plan.contains("\"max_length\":9"));
    assert!(plan.contains("\"packed_count\":2"));
    assert!(plan.contains("\"wire_type\":2"));
}
