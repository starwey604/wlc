use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

fn compile(schema: &str, body: &str) -> String {
    let model = analyze_schema(&parse_schema(schema).unwrap()).unwrap();
    let codec = generate_c(&model, "plan").unwrap();
    let source = codec.source.clone();
    let dir = tempdir().unwrap();
    for (file, text) in [
        ("plan.h", codec.header),
        ("plan_values.h", codec.values_header),
        ("plan.c", codec.source),
        ("test.c", body.into()),
    ] {
        fs::write(dir.path().join(file), text).unwrap();
    }
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let mut cc = Command::new("cc");
    cc.args([
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
    ]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let binary = dir.path().join("test");
    let result = cc
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("test.c"))
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(binary).output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    source
}

#[test]
fn adaptive_lookup_matches_linear_for_every_field_number() {
    let mut schema = "version 1;".to_string();
    for (id, name, count, stride) in [
        (1, "Few", 8, 1),
        (2, "Dense", 32, 1),
        (3, "Sparse", 64, 997),
        (4, "FewSparse", 8, 997),
    ] {
        schema.push_str(&format!("message {name} @id({id}) {{"));
        // Deliberately unsorted source declarations; semantic order is canonical.
        for i in (0..count).rev() {
            schema.push_str(&format!("optional uint32 f{i} @id({});", 33 + i * stride));
        }
        schema.push('}');
    }
    schema.push_str("message Empty @id(5) {} message Top @id(6) { optional uint32 x @id(65535); }");
    compile(
        &schema,
        r#"
#include <assert.h>
#include "plan.c"
int main(void) {
  const wlc_desc_t *descs[] = {
    &few_desc, &dense_desc, &sparse_desc, &few_sparse_desc, &empty_desc, &top_desc
  };
  assert(few_desc.lookup == WLC_LOOKUP_LINEAR);
  assert(dense_desc.lookup == WLC_LOOKUP_DENSE);
  assert(sparse_desc.lookup == WLC_LOOKUP_BINARY);
  assert(few_sparse_desc.lookup == WLC_LOOKUP_LINEAR);
  for (size_t d = 0; d < sizeof(descs) / sizeof(descs[0]); ++d) {
    for (uint32_t n = 0; n <= UINT16_MAX; ++n) {
      const wlc_field_t *expected = NULL;
      for (size_t i = 0; i < descs[d]->count; ++i)
        if (descs[d]->fields[i].number == n) { expected = &descs[d]->fields[i]; break; }
      assert(wlc_find_field(descs[d], (uint16_t)n) == expected);
    }
  }
  return 0;
}
"#,
    );
}

#[test]
fn fixed_array_specialization_matches_generic_codec_and_failure_state() {
    // All messages specialized: generic helpers must also stay warning-clean
    // when no ordinary public wrapper references them.
    compile(
        r#"version 1;
message Packed @id(1) { packed float32 values[30] @id(1); }
message Wide @id(2) { required packed float64 values[64] @id(65535); }
message Fixed @id(3) { packed fixed32 values[128] @id(21); }
message One @id(4) { required packed fixed64 values[1] @id(16); }
"#,
        include_str!("fixtures/codec_plan.c"),
    );
}

#[test]
fn cursor_matches_frozen_linear_decoder_including_failure_state() {
    let mut schema = "version 1;".to_string();
    let mut calls = String::new();
    let mut id = 1;
    for count in [2, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64] {
        for stride in [1, 997] {
            let name = format!("n{count}s{stride}");
            schema.push_str(&format!("message {name} @id({id}) {{"));
            id += 1;
            for i in (0..count).rev() {
                let card = if i % 5 == 0 { "required" } else { "optional" };
                schema.push_str(&format!("{card} uint32 f{i} @id({});", 1 + i * stride));
            }
            schema.push('}');
            calls.push_str(&format!("  TEST({name});\n"));
        }
    }
    schema.push_str(
        "message Empty @id(40) {}\n\
         message Child @id(41) { required uint32 x @id(1); optional uint32 y @id(16); }\n\
         message Mixed @id(42) { required Child child @id(1); repeated uint32 samples @id(2);\n\
         repeated Child children @id(3); packed float32 values[3] @id(16);\n\
         optional string<16> text @id(2048); optional uint64 last @id(65535); }\n\
         message ReferenceSigned @id(43) { optional int32 x @id(1); optional int64 y @id(2); }",
    );
    compile(
        &schema,
        &format!(
            "#include <assert.h>\n#include \"plan.c\"\n{}\n{}\nint main(void) {{\n{}  test_mixed();\n  test_key_bounds();\n  empty_t a, b;\n  assert(wlc_decode(&empty_desc, NULL, 0, &a) == reference_decode(&empty_desc, NULL, 0, &b));\n  return 0;\n}}\n",
            include_str!("fixtures/codec_lookup_reference.c.in"),
            include_str!("fixtures/codec_cursor.c.in"),
            calls,
        ),
    );
}

#[test]
fn precomputed_keys_match_canonical_varints_at_length_boundaries() {
    let mut schema = "version 1;".to_string();
    let mut descs = Vec::new();
    for (i, ty) in [
        "uint32",
        "fixed64",
        "string<8>",
        "fixed32",
        "packed float32",
    ]
    .iter()
    .enumerate()
    {
        let name = format!("key{i}");
        descs.push(format!("&{name}_desc"));
        schema.push_str(&format!("message {name} @id({}) {{", i + 1));
        for number in [1, 15, 16, 2047, 2048, 65535] {
            let field = if i == 4 {
                format!("{ty} f{number}[2]")
            } else {
                format!("optional {ty} f{number}")
            };
            schema.push_str(&format!("{field} @id({number});"));
        }
        schema.push('}');
    }
    compile(
        &schema,
        &format!(
            r#"
#include <assert.h>
#include "plan.c"
int main(void) {{
  const wlc_desc_t *descs[] = {{ {} }};
  const unsigned wires[] = {{ 0, 1, 2, 5, 2 }};
  if (sizeof(void *) == 8) assert(sizeof(wlc_field_t) == 96);
  if (sizeof(void *) == 4) assert(sizeof(wlc_field_t) == 64);
  for (size_t d = 0; d < 5; ++d) {{
    for (size_t i = 0; i < descs[d]->count; ++i) {{
      const wlc_field_t *f = &descs[d]->fields[i];
      uint64_t key = ((uint64_t)f->number << 3) | wires[d];
      uint8_t bytes[10], actual[10], *p = bytes, *q = actual;
      wlc_putv(&p, key); wlc_put_key(&q, f);
      assert((size_t)(p - bytes) == f->key_size && q - actual == p - bytes);
      assert(memcmp(bytes, f->key, f->key_size) == 0);
      assert(memcmp(bytes, actual, f->key_size) == 0);
      assert(f->wire == wires[d]);
      wlc_hash_state_t a = {{17, 0, WL_CODEC_OK}}, b = a;
      wlc_hash_putv(&a, key); wlc_hash_put_key(&b, f);
      assert(a.hash == b.hash && a.length == b.length && a.status == b.status);
    }}
  }}
  return 0;
}}
"#,
            descs.join(", ")
        ),
    );
}

#[test]
fn final_field_hint_preserves_repeated_and_duplicate_semantics() {
    // The frozen lookup oracle references UTF-8/fixed/packed helpers even though
    // Single and Tail do not; include those types in this oracle-only schema.
    compile(
        "version 1;\n\
         message Single @id(1) { optional uint32 value @id(1); }\n\
         message Tail @id(2) { required uint32 first @id(1); repeated uint32 values @id(2); }\n\
         message ReferenceHelpers @id(3) { optional string text @id(1); packed fixed32 values[1] @id(2); optional int32 x @id(3); optional int64 y @id(4); }",
        &format!(
            r#"
#include <assert.h>
#include "plan.c"
{}
int main(void) {{
  const uint8_t singleton[] = {{8, 1, 0x78, 42, 8, 2}};
  single_t a, b;
  for (size_t n = 0; n <= sizeof(singleton); ++n) {{
    memset(&a, 0xA5, sizeof(a)); memset(&b, 0xA5, sizeof(b));
    int ar = single_decode(singleton, n, &a);
    int br = reference_decode(&single_desc, singleton, n, &b);
    assert(ar == br && memcmp(&a, &b, sizeof(a)) == 0);
  }}
  assert(single_decode(singleton, sizeof(singleton), &a) == WL_CODEC_ERR_DUPLICATE_FIELD);
  /* Last repeated field first, unknown between repeats, then a descending
   * field and the last repeated field again. Test every truncation/capacity. */
  const uint8_t repeated[] = {{16, 1, 0x78, 42, 16, 2, 8, 3, 16, 4}};
  for (size_t cap = 0; cap <= 3; ++cap) {{
    for (size_t n = 0; n <= sizeof(repeated); ++n) {{
      tail_t x, y; uint32_t xs[3], ys[3];
      memset(&x, 0xA5, sizeof(x)); memset(&y, 0xA5, sizeof(y));
      memset(xs, 0xA5, sizeof(xs)); memset(ys, 0xA5, sizeof(ys));
      x.values = cap ? xs : NULL; y.values = cap ? ys : NULL;
      x.values_capacity = y.values_capacity = cap;
      int xr = tail_decode(repeated, n, &x);
      int yr = reference_decode(&tail_desc, repeated, n, &y);
      x.values = y.values = NULL;
      assert(xr == yr && memcmp(&x, &y, sizeof(x)) == 0);
      assert(memcmp(xs, ys, sizeof(xs)) == 0);
    }}
  }}
  return 0;
}}
"#,
            include_str!("fixtures/codec_lookup_reference.c.in"),
        ),
    );
}

#[test]
fn schema_type_subsets_preserve_wire_bytes_validation_and_fingerprints() {
    // Each case generates its own translation unit: a combined scalar matrix
    // cannot detect helpers accidentally depending on an absent type branch.
    let cases = [
        ("bool", "BOOL", "value.v = true;", "8, 1", ""),
        ("uint8", "U8", "value.v = 129;", "8, 129, 1", ""),
        ("uint16", "U16", "value.v = 129;", "8, 129, 1", ""),
        ("uint32", "U32", "value.v = 129;", "8, 129, 1", ""),
        ("uint64", "U64", "value.v = 129;", "8, 129, 1", ""),
        ("int8", "I8", "value.v = -64;", "8, 127", ""),
        ("int16", "I16", "value.v = -64;", "8, 127", ""),
        ("int32", "I32", "value.v = -64;", "8, 127", ""),
        ("int64", "I64", "value.v = -64;", "8, 127", ""),
        (
            "fixed32",
            "F32",
            "value.v = UINT32_C(0x80000001);",
            "13, 128, 0, 0, 1",
            "",
        ),
        (
            "fixed64",
            "F64",
            "value.v = UINT64_C(0x8000000000000001);",
            "9, 128, 0, 0, 0, 0, 0, 0, 1",
            "",
        ),
        (
            "float32",
            "FLOAT32",
            "value.v = -1.5f;",
            "13, 191, 192, 0, 0",
            "",
        ),
        (
            "float64",
            "FLOAT64",
            "value.v = -1.5;",
            "9, 191, 248, 0, 0, 0, 0, 0, 0",
            "",
        ),
        (
            "bytes<8>",
            "BYTES",
            "const uint8_t b[] = {0, 255}; value.v = (wl_codec_bytes_t){b, 2};",
            "10, 2, 0, 255",
            "",
        ),
        (
            "string<8>",
            "STRING",
            "value.v = (wl_codec_string_t){\"电\", 3};",
            "10, 3, 231, 148, 181",
            "",
        ),
        (
            "Mode",
            "ENUM",
            "value.v = -1;",
            "8, 1",
            "enum Mode @id(2) { OFF = 0; NEG = -1; }",
        ),
        (
            "Child",
            "MESSAGE",
            "value.v.has_x = true; value.v.x = 129;",
            "10, 3, 8, 129, 1",
            "message Child @id(2) { required uint16 x @id(1); }",
        ),
    ];
    let kinds = [
        "BOOL", "U8", "U16", "U32", "U64", "I8", "I16", "I32", "I64", "F32", "F64", "FLOAT32",
        "FLOAT64", "BYTES", "STRING", "ENUM", "MESSAGE",
    ];
    for (ty, kind, init, bytes, extra) in cases {
        let schema =
            format!("version 1; {extra} message Sample @id(1) {{ required {ty} v @id(1); }}");
        let source = compile(
            &schema,
            &format!(
                r#"
#include <assert.h>
#include "plan.c"
static uint64_t reference_hash(const uint8_t *bytes, size_t n) {{
  uint64_t h = 12345;
  for (size_t i = 0; i < n; ++i) h = (h ^ bytes[i]) * UINT64_C(0x100000001b3);
  return h;
}}
int main(void) {{
  const uint8_t expected[] = {{{bytes}}};
  /* All legal unknown wire types must remain supported, regardless of schema. */
  const uint8_t unknown[] = {{16, 129, 1, 25, 0,0,0,0,0,0,0,1, 34, 2, 255,0, 45, 0,0,0,1}};
  sample_t value, decoded;
  sample_clear(&value); value.has_v = true; {init}
  uint8_t encoded[64], input[128]; size_t n;
  assert(sample_encode(&value, encoded, sizeof(encoded), &n) == WL_CODEC_OK);
  assert(n == sizeof(expected) && memcmp(encoded, expected, n) == 0);
  assert(sample_decode(NULL, 0, &decoded) == WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
  for (size_t i = 1; i < sizeof(expected); ++i)
    assert(sample_decode(expected, i, &decoded) != WL_CODEC_OK);
  for (unsigned order = 0; order != 2; ++order) {{
    memcpy(input, order ? unknown : expected, order ? sizeof(unknown) : sizeof(expected));
    memcpy(input + (order ? sizeof(unknown) : sizeof(expected)), order ? expected : unknown,
           order ? sizeof(expected) : sizeof(unknown));
    assert(sample_decode(input, sizeof(expected) + sizeof(unknown), &decoded) == WL_CODEC_OK);
    uint64_t hash = 12345; size_t length = SIZE_MAX;
    assert(sample_wlc_detail_fingerprint(&decoded, &hash, &length) == WL_CODEC_OK);
    assert(length == sizeof(expected) && hash == reference_hash(expected, sizeof(expected)));
  }}
  memcpy(input, expected, sizeof(expected)); memcpy(input + sizeof(expected), expected, sizeof(expected));
  assert(sample_decode(input, 2 * sizeof(expected), &decoded) == WL_CODEC_ERR_DUPLICATE_FIELD);
  memcpy(input, expected, sizeof(expected)); input[0] = expected[0] == 8 ? 13 : 8;
  assert(sample_decode(input, sizeof(expected), &decoded) == WL_CODEC_ERR_WIRE_TYPE);
  const uint8_t noncanonical[] = {{16, 128, 0}};
  assert(sample_decode(noncanonical, sizeof(noncanonical), &decoded) == WL_CODEC_ERR_MALFORMED);
  return 0;
}}
"#
            ),
        );
        for candidate in kinds {
            let present = candidate == kind || (kind == "MESSAGE" && candidate == "U16");
            assert_eq!(
                source.contains(&format!("case WLC_{candidate}:")),
                present,
                "{ty}: {candidate}"
            );
        }
        assert!(!source.contains("WLC_IF"));
        assert!(!source.contains("static wl_codec_status_t wlc_read_packed("));
        assert_eq!(source.contains("static bool wlc_utf8("), kind == "STRING");
    }
}

#[test]
fn empty_schema_engine_is_warning_clean_and_keeps_unknown_field_validation() {
    let source = compile(
        "version 1; message Empty @id(1) {}",
        r#"
#include <assert.h>
#include "plan.c"
int main(void) {
  empty_t view; empty_value_t value;
  empty_value_clear(&value);
  assert(empty_value_to_view(&value, &view) == WL_CODEC_OK && view._empty == 0);
  const uint8_t unknown[] = {8, 1, 17, 0,0,0,0,0,0,0,0, 26, 1, 255, 37, 0,0,0,0};
  assert(empty_decode(unknown, sizeof(unknown), &view) == WL_CODEC_OK);
  uint64_t hash = 12345; size_t length = SIZE_MAX;
  assert(empty_wlc_detail_fingerprint(&view, &hash, &length) == WL_CODEC_OK);
  assert(hash == 12345 && length == 0);
  const uint8_t truncated[] = {17, 0};
  assert(empty_decode(truncated, sizeof(truncated), &view) == WL_CODEC_ERR_MALFORMED);
  assert(empty_encoded_size(&view) == 0);
  return 0;
}
"#,
    );
    assert!(!source.contains("case WLC_"));
    assert!(!source.contains("static bool wlc_utf8("));
    assert!(!source.contains("static wl_codec_status_t wlc_read_fixed("));
    assert!(!source.contains("static inline void wlc_putv("));
}

#[test]
fn packed_type_subsets_keep_specialized_and_generic_paths_equivalent() {
    for (ty, kind, bytes) in [
        ("fixed32", "F32", "10, 8, 128,0,0,1, 127,192,0,1"),
        ("float32", "FLOAT32", "10, 8, 128,0,0,1, 127,192,0,1"),
        (
            "fixed64",
            "F64",
            "10, 16, 128,0,0,0,0,0,0,1, 127,248,0,0,0,0,0,1",
        ),
        (
            "float64",
            "FLOAT64",
            "10, 16, 128,0,0,0,0,0,0,1, 127,248,0,0,0,0,0,1",
        ),
    ] {
        let source = compile(
            &format!("version 1; message Sample @id(1) {{ required packed {ty} v[2] @id(1); }}"),
            &format!(
                r#"
#include <assert.h>
#include "plan.c"
int main(void) {{
  /* Keep floating representations including a NaN payload byte-for-byte. */
  const uint8_t expected[] = {{{bytes}}};
  sample_t value, generic;
  assert(sample_decode(expected, sizeof(expected), &value) == WL_CODEC_OK);
  assert(wlc_decode(&sample_desc, expected, sizeof(expected), &generic) == WL_CODEC_OK);
  assert(value.has_v && generic.has_v && memcmp(value.v, generic.v, sizeof(value.v)) == 0);
  uint8_t encoded[64], reference[64]; size_t n, m;
  assert(sample_encode(&value, encoded, sizeof(encoded), &n) == WL_CODEC_OK);
  assert(wlc_encode(&sample_desc, &generic, reference, sizeof(reference), &m) == WL_CODEC_OK);
  assert(n == sizeof(expected) && n == m && memcmp(encoded, expected, n) == 0 && memcmp(encoded, reference, n) == 0);
  uint64_t hash = 12345, want = hash; size_t length;
  for (size_t i = 0; i < n; ++i) want = (want ^ expected[i]) * UINT64_C(0x100000001b3);
  assert(sample_wlc_detail_fingerprint(&value, &hash, &length) == WL_CODEC_OK && hash == want && length == n);
  for (size_t i = 0; i < sizeof(expected); ++i)
    assert(sample_decode(expected, i, &value) == wlc_decode(&sample_desc, expected, i, &generic));
  return 0;
}}
"#
            ),
        );
        for candidate in ["F32", "F64", "FLOAT32", "FLOAT64"] {
            assert_eq!(
                source.contains(&format!("case WLC_{candidate}:")),
                candidate == kind
            );
        }
    }
}
