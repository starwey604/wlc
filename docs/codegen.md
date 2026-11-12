# Generate codecs and inspect a deployment

[中文](codegen-cn.md) · [Typed endpoint](static-kernel.md) · [SDK](sdk.md)

Use WLC 0.13.0 / contract 0.13 with matching Wirelink 0.13.0. From a directory
containing your schema, run:

```sh
wlc validate device.wl
wlc compile device.wl --out-dir generated-codec
wlc compile device.wl --profile device.bind.wl --out-dir generated-endpoint
wlc plan device.wl --profile device.bind.wl
wlc identity device.wl --profile device.bind.wl
wlc dependencies device.wl
wlc module-name device.wl
```

Without a profile, `compile` emits `<module>.h`, `_values.h`, `.c` and a manifest.
The public codec uses `wirelink/codec.h`; it does not bring an endpoint into your firmware.
With a profile, it also emits the [typed kernel](static-kernel.md) and its selected
connector. `compile-kernel` is an alias of `compile --profile`.

## Borrow views or retain owned values

Public Schema-v1 codecs preserve integer widths, unknown signed enum values,
IEEE floating-point bits, optional presence and defaults, bounded UTF-8/NUL,
bytes, fixed packed arrays, nested messages and repeated borrowed views.
Repeated fields use caller-provided pointer/count/capacity storage. Unknown fields
are validated and skipped. See [schema and wire rules](https://github.com/starwey604/wirelink/blob/main/docs/schema-v1.md).

Finite types get `<message>_value_t` in `_values.h`. String/bytes storage is inline,
and nested values own their contents. `*_value_from_view` copies borrowed input;
`*_value_to_view` borrows the owned value. Check conversion results. Bounds are
encoded bytes, and a string's extra terminating NUL is not encoded. `*_VALUE_SIZE`
is target `sizeof`, not maximum wire length. Unbounded types have `*_HAS_VALUE=0`.

## Validate composition and evolution

Imports resolve relative to the importing schema. `dependencies` lists canonical
transitive inputs; `module-name` prints the exact C filename/symbol prefix,
including keyword escaping. Repeating `--profile` composes declarations with
conflict checks, not overrides. Profile version 2 must select a connector explicitly.

Use `validate --previous previous.wl` or `compile --previous previous.wl` to check
schema evolution. Identity is exact, not compatibility-aware. `plan` reports actual
four-call/four-reply kernel resources, bounded TX storage and selected codec field
facts. Identity and FNV artifact digests serve diagnostics, not authentication.

## Integrate generation into a build

The installed Wirelink CMake package provides `wirelink_wlc_generate(TARGET ...
SCHEMA ... PROFILE ...)` for endpoints and `wirelink_wlc_generate_codec` for codecs.
Set `WIRELINK_WLC_EXECUTABLE` to the matching local compiler; imports and profiles
become build/reconfigure dependencies. See [build commands](https://github.com/starwey604/wirelink/blob/main/docs/development.md).

Use a dedicated output directory per deployment. An incompatible artifact set,
including a stream connector left by another deployment, requires a fresh directory.
Keep provenance with the generated files. Changing templates requires rebuilding WLC
before regeneration because templates are compiled into the tool.

For generator changes, exercise public byte vectors and malformed-input cases as
well as generated-C compilation/execution. A changed snapshot alone does not establish
wire correctness.
