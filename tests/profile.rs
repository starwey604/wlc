use wlc::{analyze_schema, parse_schema, schema_identity};
#[test]
fn schema_identity_distinguishes_required_cardinality() {
    let optional = analyze_schema(
        &parse_schema("version 1; message Value = 1 { optional uint32 field = 1; }").unwrap(),
    )
    .unwrap();
    let required = analyze_schema(
        &parse_schema("version 1; message Value = 1 { required uint32 field = 1; }").unwrap(),
    )
    .unwrap();
    assert_ne!(schema_identity(&optional), schema_identity(&required));
}

#[test]
fn schema_identity_distinguishes_narrow_integer_types() {
    let identities = ["uint8", "uint16", "int8", "int16"]
        .into_iter()
        .map(|ty| {
            let source = format!("version 1; message Value = 1 {{ optional {ty} field = 1; }}");
            let model = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
            schema_identity(&model)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 4);
}

#[test]
fn schema_identity_distinguishes_bounded_lengths_without_renumbering_unbounded_types() {
    let identities = ["string", "string<31>", "string<32>", "bytes<31>"]
        .into_iter()
        .map(|ty| {
            let source = format!("version 1; message Value = 1 {{ optional {ty} field = 1; }}");
            let model = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
            schema_identity(&model)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 4);
}
