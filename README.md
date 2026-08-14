# wlc

`wlc` is the Wirelink schema compiler. This baseline implements parsing and
front-end validation only; semantic type resolution, IR, and C code generation
are subsequent milestones.

## Schema grammar

```text
schema      = "version" positive-integer ";" declaration+
declaration = message | enum
message     = "message" identifier "=" positive-integer "{" field* "}"
field       = ("optional" | "repeated") identifier identifier "="
              positive-integer ("[" "default" "=" literal "]")? ";"
enum        = "enum" identifier "=" positive-integer "{" enum-value+ "}"
enum-value  = identifier "=" integer ";"
literal     = integer | string | "true" | "false"
```

Line comments start with `//`. `version` must be the first declaration and is
currently a positive integer. Message and enum identifiers share one global
namespace and one non-zero 16-bit ID namespace. Field IDs are non-zero 16-bit
numbers, unique within their message. Enum names and values are unique within
their enum. Valid built-in field types are `bool`, `string`, `int32`, `uint32`,
`int64`, and `uint64`; user-defined type names are accepted for later semantic
resolution.

`optional` fields may specify one default. Defaults must match the built-in
type; user-defined (enum) defaults currently use an integer literal.
`repeated` fields cannot have defaults.

## Usage

```sh
cargo run -- path/to/schema.wl
cargo test
```

Diagnostics use `line:column: message` and the process exits non-zero on an
invalid schema.
