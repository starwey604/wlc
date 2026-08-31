# wlc

`wlc` is the Wirelink schema compiler. It parses and validates `.wl` schemas,
checks a revision against its predecessor, and generates allocation-free C11
payload codecs.

## Schema grammar

```text
schema         = "version" positive-integer ";" item+
item           = declaration | reservation
declaration    = message | enum
reservation    = "reserved" positive-integer ";"
message        = "message" identifier "=" positive-integer
                 "{" (field | reservation)* "}"
field          = optional-field | repeated-field | packed-field
optional-field = "optional" type identifier "=" positive-integer
                 ("[" "default" "=" literal "]")? ";"
repeated-field = "repeated" type identifier "=" positive-integer ";"
packed-field   = "packed" packed-type identifier "[" positive-integer "]"
                 "=" positive-integer ";"
packed-type    = "float32" | "float64" | "fixed32" | "fixed64"
enum           = "enum" identifier "=" positive-integer
                 "{" enum-value (enum-value | enum-reservation)* "}"
enum-value     = identifier "=" integer ";"
enum-reservation = "reserved" integer ";"
literal        = integer | string | "true" | "false"
```

For example, a six-axis control vector is declared as:

```wl
message JointControl = 16 {
  packed float32 position[6] = 1;
  packed float32 velocity[6] = 2;
}
```

Line comments start with `//`. `version` must be first and positive. Message
and enum identifiers share one global namespace and one nonzero 16-bit ID
namespace. Field IDs are nonzero 16-bit numbers and unique within a message.
Packed element counts are in `1..=65535`.

The built-in types are `bool`, `bytes`, `string`, `int32`, `uint32`, `int64`,
`uint64`, `fixed32`, `fixed64`, `float32`, and `float64`. `float32` and
`float64` generate C `float` and `double`. Generated headers enforce 4/8-byte
IEEE-754 binary32/binary64 value formats with compile-time assertions. Codecs
move floating-point bits through `memcpy`; they never use aliasing casts or
numeric conversions, so signed zero, infinities, and NaN payload bits round
trip exactly.

Optional scalar defaults must match their type. Explicit floating-point
defaults are intentionally not accepted until the schema has a canonical,
locale-independent floating literal syntax; absent floats clear to positive
zero. `bytes`, repeated fields, and packed fields cannot have defaults.

## Wire rules

The encoder emits fields in field-number order. Each field starts with an
unsigned LEB128 key `(field_number << 3) | wire_type`. Integers retain their
existing varint representation. `fixed32` and `float32` use wire type 5 and
four big-endian bytes; `fixed64` and `float64` use wire type 1 and eight
big-endian bytes. Adding float support does not alter any existing scalar or
repeated wire bytes.

A packed declaration is one optional field occurrence, represented in C by a
presence flag and an inline array:

```c
bool has_position;
float position[6];
```

On the wire it always uses type 2 and contains one length prefix followed by
exactly `count` fixed-width, big-endian elements. A decoder rejects duplicate
occurrences, a wrong wire type, or any byte length other than
`count * sizeof(element)`. There is no caller pointer, count, capacity, heap
allocation, or per-element tag. Thus `packed float32 values[30] = 7;` encodes
as 122 bytes when present: one-byte tag, one-byte length (`120`), and 120 data
bytes.

Ordinary `repeated` fields keep their previous representation: a caller-owned
pointer/count/capacity in C and one complete tag/value pair per element.

## Semantic and compatibility rules

All declaration and field numbers are explicit allocations; `wlc` never
auto-assigns IDs. Semantic analysis resolves every field type and rejects
unknown types, recursive or overly deep messages, invalid defaults, and
non-fixed-width packed element types. The public semantic model is sorted by
IDs and field numbers, so source reordering does not change generated output.

`reserved N;` permanently reserves a removed declaration ID, field number, or
enum value at its respective scope. Compatibility checks reject ID, name,
type, and cardinality changes or removal without a reservation. A packed
field's element type and fixed count are both part of its wire identity;
changing either is incompatible. Existing reservations must remain reserved.

The library API is `parse_schema()`, `analyze_schema()`,
`check_compatibility()`, and `generate_c()`.

## CLI usage

```sh
# Validate one schema.
cargo run -- validate path/to/schema.wl

# Validate compatibility and generate schema.h/schema.c.
cargo run -- compile path/to/schema.wl \
  --previous path/to/previous.wl \
  --out-dir generated

cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Diagnostics use `line:column: message`; at the CLI boundary `miette` renders a
source snippet at the invalid token.

## Dependency policy

The compiler deliberately keeps one dependency per concern:

| Crate | Role |
| --- | --- |
| `miette` | Source-aware, terminal-friendly diagnostics. |
| `thiserror` | Typed library errors. |
| `clap` | Declarative `compile` and `validate` CLI. |
| `heck` | Stable generated C symbol conversion. |
| `insta` | Reviewed generator golden snapshots. |
| `proptest` | Parser robustness and scalar-boundary properties. |
| `assert_cmd`, `tempfile` | Isolated CLI and generated-C tests. |

The hand-written parser keeps the grammar explicit. Generated C is emitted by
a small self-contained runtime and depends only on `wirelink/codec.h` for
status and borrowed byte/string types.
