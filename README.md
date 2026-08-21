# wlc

`wlc` is the Wirelink schema compiler. This baseline implements parsing and
front-end validation only; semantic type resolution, IR, and C code generation
are subsequent milestones.

## Schema grammar

```text
schema      = "version" positive-integer ";" item+
item        = declaration | reservation
declaration = message | enum
reservation = "reserved" positive-integer ";"
message     = "message" identifier "=" positive-integer "{" (field | reservation)* "}"
field       = ("optional" | "repeated") identifier identifier "="
              positive-integer ("[" "default" "=" literal "]")? ";"
enum        = "enum" identifier "=" positive-integer "{" enum-value (enum-value | enum-reservation)* "}"
enum-value  = identifier "=" integer ";"
enum-reservation = "reserved" integer ";"
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

## Semantic and compatibility rules

All declaration and field numbers are explicit allocations: `wlc` never
auto-assigns IDs. Semantic analysis resolves every field type to a built-in,
message, or enum and rejects unknown types, message defaults, and enum defaults
that do not name an existing numeric enum value. Its public model sorts
declarations by global ID, fields by field number, and enum values by value;
the result is therefore stable when source declarations are reordered.

`reserved N;` at schema scope permanently reserves a removed message or enum
ID. The same statement inside a message permanently reserves a field number;
inside an enum it reserves an enum value. Compatibility checks compare a prior
semantic model to a current model and reject ID/name/type/cardinality changes,
or removal without a corresponding reservation. Existing reservations must
remain reserved. Default changes are not wire changes and are allowed.

The current library API is `analyze_schema(&Schema)` followed by
`check_compatibility(&previous, &current)`. The future `wlc validate` command
will expose the latter through a version-baseline file.

## Usage

```sh
cargo run -- path/to/schema.wl
cargo test
```

Diagnostics use `line:column: message` and the process exits non-zero on an
invalid schema. At the CLI boundary, `miette` renders a source snippet and
points at the invalid token.

## Dependency policy

The compiler deliberately keeps one dependency per concern:

| Crate | Role | Adoption point |
| --- | --- | --- |
| `miette` | Source-aware, terminal-friendly diagnostics. | Used now by the CLI. |
| `thiserror` | Typed library errors without exposing implementation details. | Used now for parser errors. |
| `clap` | Declarative CLI and the future `compile` / `validate` subcommands. | Used now for argument parsing. |
| `heck` | Stable snake-case and macro-case conversion for generated C symbols. | Used now by code generation. |
| `insta` | Reviewed golden snapshots for generated C headers and sources. | Development dependency; enabled when codegen lands. |
| `proptest` | Property tests for parser robustness and scalar default boundaries. | Development dependency; used now. |
| `assert_cmd`, `tempfile` | Isolated end-to-end tests of the CLI and its generated output. | Development dependencies; used now. |

We will not add a parser generator, general templating engine, or `anyhow` at
this stage. The hand-written parser preserves exact grammar control and works
with `miette`; generated C will be written by a small, testable emitter.
