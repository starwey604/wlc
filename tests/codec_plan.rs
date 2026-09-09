use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

fn compile(schema: &str, body: &str) {
    let model = analyze_schema(&parse_schema(schema).unwrap()).unwrap();
    let codec = generate_c(&model, "plan").unwrap();
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
         optional string<16> text @id(2048); optional uint64 last @id(65535); }",
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
