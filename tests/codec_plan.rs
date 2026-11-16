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
    let mut cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()));
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
        .arg(root.join("src/codec.c"))
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
            let ty = if count <= 8 && i == 0 {
                "string<8>"
            } else {
                "uint32"
            };
            schema.push_str(&format!("optional {ty} f{i} @id({});", 33 + i * stride));
        }
        schema.push('}');
    }
    schema.push_str(
        "message Empty @id(5) {} message Top @id(6) { optional string<8> x @id(65535); }",
    );
    compile(
        &schema,
        r#"
#include <assert.h>
#include "plan.c"
int main(void) {
  const wl_codec_desc_t *descs[] = {
    &few_desc, &dense_desc, &sparse_desc, &few_sparse_desc, &empty_desc, &top_desc
  };
  assert(few_desc.lookup == WL_CODEC_LOOKUP_LINEAR);
  assert(dense_desc.lookup == WL_CODEC_LOOKUP_DENSE);
  assert(sparse_desc.lookup == WL_CODEC_LOOKUP_BINARY);
  assert(few_sparse_desc.lookup == WL_CODEC_LOOKUP_LINEAR);
  for (size_t d = 0; d < sizeof(descs) / sizeof(descs[0]); ++d) {
    for (uint32_t n = 0; n <= UINT16_MAX; ++n) {
      const wl_codec_field_t *expected = NULL;
      for (size_t i = 0; i < descs[d]->field_count; ++i)
        if (descs[d]->fields[i].number == n) { expected = &descs[d]->fields[i]; break; }
      assert(wl_codec_find_field(descs[d], (uint16_t)n) == expected);
    }
  }
  return 0;
}
"#,
    );
}

#[test]
fn fixed_array_specialization_matches_generic_codec_and_failure_state() {
    let source = compile(
        r#"version 1;
message Packed @id(1) { optional packed float32 values[30] @id(1); }
message Wide @id(2) { required packed float64 values[64] @id(65535); }
message Fixed @id(3) { optional packed fixed32 values[128] @id(21); }
message One @id(4) { required packed fixed64 values[1] @id(16); }
"#,
        include_str!("fixtures/codec_plan.c"),
    );
    assert!(source.contains("packed_desc"));
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
                let ty = if count <= 8 && i == count - 1 {
                    "string<8>"
                } else {
                    "uint32"
                };
                schema.push_str(&format!("{card} {ty} f{i} @id({});", 1 + i * stride));
            }
            schema.push('}');
            calls.push_str(&format!("  TEST({name});\n"));
        }
    }
    schema.push_str(
        "message Empty @id(40) {}\n\
         message Child @id(41) { required uint32 x @id(1); optional uint32 y @id(16); }\n\
         message Mixed @id(42) { required Child child @id(1); repeated uint32 samples @id(2);\n\
         repeated Child children @id(3); optional packed float32 values[3] @id(16);\n\
         optional string<16> text @id(2048); optional uint64 last @id(65535); }\n\
         message ReferenceSigned @id(43) { optional int32 x @id(1); optional int64 y @id(2); }",
    );
    compile(
        &schema,
        &format!(
            "#include <assert.h>\n#include \"plan.c\"\n{}\n{}\nint main(void) {{\n{}  test_mixed();\n  test_key_bounds();\n  empty_t a, b, scratch;\n  assert(wl_codec_decode(&empty_desc, NULL, 0, &a, &scratch) == reference_decode(&empty_desc, NULL, 0, &b));\n  return 0;\n}}\n",
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
        "optional packed float32",
    ]
    .iter()
    .enumerate()
    {
        let name = format!("key{i}");
        descs.push(format!("&{name}_desc"));
        schema.push_str(&format!("message {name} @id({}) {{", i + 1));
        for number in [1, 15, 16, 2047, 2048, 20001, 20002, 20003, 65535] {
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
  const wl_codec_desc_t *descs[] = {{ {} }};
  const unsigned wires[] = {{ 0, 1, 2, 5, 2 }};
  assert(sizeof(wl_codec_field_t) <= 64);
  for (size_t d = 0; d < 5; ++d) {{
    for (size_t i = 0; i < descs[d]->field_count; ++i) {{
      const wl_codec_field_t *f = &descs[d]->fields[i];
      uint64_t key = ((uint64_t)f->number << 3) | wires[d];
      uint8_t bytes[10], *p = bytes;
      p = wl_codec_putv(p, key);
      assert((size_t)(p - bytes) == f->key_len);
      assert(memcmp(bytes, f->key, f->key_len) == 0);
      assert(f->wire_type == wires[d]);
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
    compile(
        "version 1;\n\
         message Single @id(1) { optional uint32 value @id(1); }\n\
         message Tail @id(2) { required uint32 first @id(1); repeated uint32 values @id(2); }",
        &format!(
            r#"
#include <assert.h>
#include "plan.c"
{}
int main(void) {{
  const uint8_t singleton[] = {{8, 1, 0x78, 42, 8, 2}};
  single_t a;
  assert(single_decode(singleton, 0, &a) == WL_CODEC_OK && !a.has_value);
  assert(single_decode(singleton, 2, &a) == WL_CODEC_OK && a.has_value && a.value == 1);
  assert(single_decode(singleton, 4, &a) == WL_CODEC_OK && a.has_value && a.value == 1);
  assert(single_decode(singleton, 1, &a) == WL_CODEC_ERR_MALFORMED);
  assert(single_decode(singleton, 5, &a) == WL_CODEC_ERR_DUPLICATE_FIELD);
  assert(single_decode(singleton, sizeof(singleton), &a) == WL_CODEC_ERR_DUPLICATE_FIELD);
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
            "13, 1, 0, 0, 128",
            "",
        ),
        (
            "fixed64",
            "F64",
            "value.v = UINT64_C(0x8000000000000001);",
            "9, 1, 0, 0, 0, 0, 0, 0, 128",
            "",
        ),
        (
            "float32",
            "FLOAT32",
            "value.v = -1.5f;",
            "13, 0, 0, 192, 191",
            "",
        ),
        (
            "float64",
            "FLOAT64",
            "value.v = -1.5;",
            "9, 0, 0, 0, 0, 0, 0, 248, 191",
            "",
        ),
        (
            "bytes<8>",
            "BYTES",
            "value.v.length = 2; value.v.data[0] = 0; value.v.data[1] = 255;",
            "10, 2, 0, 255",
            "",
        ),
        (
            "string<8>",
            "STRING",
            "value.v.length = 3; memcpy(value.v.data, \"\\xe7\\x94\\xb5\", 3);",
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
            "value.v.x = 129;",
            "10, 3, 8, 129, 1",
            "message Child @id(2) { required uint16 x @id(1); }",
        ),
    ];
    let _kinds = [
        "BOOL", "U8", "U16", "U32", "U64", "I8", "I16", "I32", "I64", "F32", "F64", "FLOAT32",
        "FLOAT64", "BYTES", "STRING", "ENUM", "MESSAGE",
    ];
    for (ty, _kind, init, bytes, extra) in cases {
        let schema =
            format!("version 1; {extra} message Sample @id(1) {{ required {ty} v @id(1); }}");
        let _source = compile(
            &schema,
            &format!(
                r#"
#include <assert.h>
#include "plan.c"
int main(void) {{
  const uint8_t expected[] = {{{bytes}}};
  /* All legal unknown wire types must remain supported, regardless of schema. */
  const uint8_t unknown[] = {{16, 129, 1, 25, 0,0,0,0,0,0,0,1, 34, 2, 255,0, 45, 0,0,0,1}};
  sample_t value, decoded, canary;
  sample_clear(&value); {init}
  uint8_t encoded[64], input[128], reencoded[64]; size_t n = 0, m = 0;
  assert(sample_encode(&value, encoded, sizeof(encoded), &n) == WL_CODEC_OK);
  assert(n == sizeof(expected) && memcmp(encoded, expected, n) == 0);
  memset(&canary, 0xa5, sizeof(canary)); decoded = canary;
  assert(sample_decode(NULL, 0, &decoded) == WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
  assert(memcmp(&decoded, &canary, sizeof(decoded)) == 0);
  for (size_t i = 1; i < sizeof(expected); ++i) {{
    decoded = canary;
    assert(sample_decode(expected, i, &decoded) != WL_CODEC_OK);
    assert(memcmp(&decoded, &canary, sizeof(decoded)) == 0);
  }}
  for (unsigned order = 0; order != 2; ++order) {{
    memcpy(input, order ? unknown : expected, order ? sizeof(unknown) : sizeof(expected));
    memcpy(input + (order ? sizeof(unknown) : sizeof(expected)), order ? expected : unknown,
           order ? sizeof(expected) : sizeof(unknown));
    assert(sample_decode(input, sizeof(expected) + sizeof(unknown), &decoded) == WL_CODEC_OK);
    assert(sample_encode(&decoded, reencoded, sizeof(reencoded), &m) == WL_CODEC_OK);
    assert(m == sizeof(expected) && memcmp(reencoded, expected, m) == 0);
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
  empty_t value;
  empty_clear(&value);
  assert(value._empty == 0);
  const uint8_t unknown[] = {8, 1, 17, 0,0,0,0,0,0,0,0, 26, 1, 255, 37, 0,0,0,0};
  assert(empty_decode(unknown, sizeof(unknown), &value) == WL_CODEC_OK);
  const uint8_t truncated[] = {17, 0};
  assert(empty_decode(truncated, sizeof(truncated), &value) == WL_CODEC_ERR_MALFORMED);
  assert(empty_encoded_size(&value) == 0);
  return 0;
}
"#,
    );
    assert!(!source.contains("WLC_FINGERPRINT"));
}

#[test]
fn packed_type_subsets_keep_specialized_and_generic_paths_equivalent() {
    for (ty, _kind, bytes) in [
        ("fixed32", "F32", "10, 8, 1,0,0,128, 1,0,192,127"),
        ("float32", "FLOAT32", "10, 8, 1,0,0,128, 1,0,192,127"),
        (
            "fixed64",
            "F64",
            "10, 16, 1,0,0,0,0,0,0,128, 1,0,0,0,0,0,248,127",
        ),
        (
            "float64",
            "FLOAT64",
            "10, 16, 1,0,0,0,0,0,0,128, 1,0,0,0,0,0,248,127",
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
  sample_t value, canary;
  assert(sample_decode(expected, sizeof(expected), &value) == WL_CODEC_OK);
  uint8_t encoded[64]; size_t n = 0;
  assert(sample_encode(&value, encoded, sizeof(encoded), &n) == WL_CODEC_OK);
  assert(n == sizeof(expected) && memcmp(encoded, expected, n) == 0);
  memset(&canary, 0xa5, sizeof(canary));
  for (size_t i = 0; i < sizeof(expected); ++i) {{
    sample_t tmp = canary;
    assert(sample_decode(expected, i, &tmp) != WL_CODEC_OK);
    assert(memcmp(&tmp, &canary, sizeof(tmp)) == 0);
  }}
  return 0;
}}
"#
            ),
        );
        assert!(source.contains("sample_desc"));
    }
}

#[test]
fn signed_value_extremes_and_open_enums_keep_canonical_wire_without_repeated_storage() {
    let source = compile(
        "version 1; enum Code @id(2) { Zero = 0; } message SignedValue @id(1) { required int32 a @id(1); required int64 b @id(2); required Code code @id(3); }",
        r#"#include <assert.h>
#include <limits.h>
#include <string.h>
#include "plan.c"
int main(void) {
  const int32_t narrow[] = {INT32_MIN, INT32_MAX, -1, 0, 1};
  const int64_t wide[] = {INT64_MIN, INT64_MAX, -1, 0, 1};
  const uint8_t minimum[] = {
    8, 255, 255, 255, 255, 15,
    16, 255, 255, 255, 255, 255, 255, 255, 255, 255, 1,
    24, 255, 255, 255, 255, 15
  };
  for (size_t i = 0; i < sizeof(narrow) / sizeof(narrow[0]); ++i) {
    signed_value_t value = {0}, decoded = {0};
    uint8_t bytes[64];
    size_t length = 0;
    value.a = narrow[i]; value.b = wide[i]; value.code = narrow[i];
    assert(signed_value_encode(&value, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
    if (i == 0) assert(length == sizeof(minimum) && memcmp(bytes, minimum, length) == 0);
    assert(signed_value_decode(bytes, length, &decoded) == WL_CODEC_OK);
    assert(decoded.a == value.a && decoded.b == value.b && decoded.code == value.code);
  }
  return 0;
}"#,
    );
    assert!(!source.contains("signed_value_desc"));
}

#[test]
fn all_scalar_shapes_match_shared_codec_next_to_larger_span_views() {
    let source = compile(
        "version 1; enum Mode @id(1) { ZERO = 0; } message Small @id(2) { required bool flag @id(1); required int8 a @id(2); optional int16 b @id(3) [default = -32768]; required int32 c @id(4); optional int64 d @id(5); required uint64 e @id(6); optional Mode mode @id(7); } message Span @id(3) { required bytes<64> data @id(1); }",
        r#"#include <assert.h>
#include <limits.h>
#include <string.h>
#include "plan.c"
int main(void) {
  const int64_t values[] = {INT64_MIN, INT64_MAX, 0, -1, 1};
  for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
    small_t value = {0}, decoded = {0}, canary;
    uint8_t direct[128], reencoded[128];
    size_t length = 0, reencoded_length = 0;
    value.has_b = value.has_d = value.has_mode = true;
    value.flag = i % 2 != 0; value.a = (int8_t)values[i]; value.b = (int16_t)values[i];
    value.c = (int32_t)values[i]; value.d = values[i];
    value.e = (uint64_t)values[i]; value.mode = (int32_t)values[i];
    assert(small_encode(&value, direct, sizeof(direct), &length) == WL_CODEC_OK);
    assert(small_decode(direct, length, &decoded) == WL_CODEC_OK);
    assert(decoded.flag == value.flag && decoded.a == value.a && decoded.b == value.b &&
           decoded.c == value.c && decoded.d == value.d && decoded.e == value.e &&
           decoded.mode == value.mode);
    assert(small_encode(&decoded, reencoded, sizeof(reencoded), &reencoded_length) == WL_CODEC_OK);
    assert(length == reencoded_length && memcmp(direct, reencoded, length) == 0);
    memset(&canary, 0xa5, sizeof(canary));
    for (size_t n = 0; n < length; ++n) {
      small_t tmp = canary;
      if (small_decode(direct, n, &tmp) != WL_CODEC_OK) {
        assert(memcmp(&tmp, &canary, sizeof(tmp)) == 0);
      }
    }
    for (size_t n = 0; n < length; ++n) {
      uint8_t saved = direct[n];
      for (unsigned b = 0; b < 256; ++b) {
        direct[n] = (uint8_t)b;
        small_t tmp = canary;
        if (small_decode(direct, length, &tmp) != WL_CODEC_OK) {
          assert(memcmp(&tmp, &canary, sizeof(tmp)) == 0);
        }
      }
      direct[n] = saved;
    }
  }
  return 0;
}"#,
    );
    assert!(source.contains("wlc_scalar_shape_0_decode"));
    assert!(!source.contains("small_desc"));
    assert!(!source.contains("return WL_CODEC_OK;\n  return WL_CODEC_ERR_INVALID_VALUE;"));
}

#[test]
fn required_bitmap_covers_256_sparse_fields_without_presence_members() {
    let mut schema = "version 1; message Wide @id(1) {".to_owned();
    for i in (0..256).rev() {
        schema.push_str(&format!("uint8 f{i} @id({});", 1 + i * 256));
    }
    schema.push('}');
    compile(
        &schema,
        r#"#include <assert.h>
#include "plan.c"
_Static_assert(sizeof(wide_t) == 256U, "required fields carry values only");
int main(void) {
  wide_t input = {0}, out, before;
  uint8_t wire[WIDE_MAX_ENCODED_SIZE + 2U]; size_t n;
  input.f0 = 42U; input.f255 = 255U;
  assert(wide_encode(&input, wire, sizeof(wire), &n) == WL_CODEC_OK);
  assert(wide_decode(wire, n, &out) == WL_CODEC_OK);
  assert(out.f0 == 42U && out.f255 == 255U);
  memset(&out, 0xA5, sizeof(out)); before = out;
  /* Last field is a three-byte key and a two-byte uint8 value. */
  assert(wide_decode(wire, n - 5U, &out) == WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
  assert(memcmp(&out, &before, sizeof(out)) == 0);
  wire[n] = 8U; wire[n + 1U] = 42U;
  assert(wide_decode(wire, n + 2U, &out) == WL_CODEC_ERR_DUPLICATE_FIELD);
  assert(memcmp(&out, &before, sizeof(out)) == 0);
  return 0;
}
"#,
    );
    schema.pop();
    schema.push_str("uint8 overflow @id(65535); }");
    assert!(analyze_schema(&parse_schema(&schema).unwrap()).is_err());
}
