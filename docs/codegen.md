# Generate codecs, nodes, and inspect plans

[中文](codegen-cn.md) · [Typed nodes](static-endpoint.md) · [SDK](sdk.md)

Use WLC 0.14.0 / codegen contract 0.14 with matching Wirelink 0.14.0. Messages, RPCs, and node roles live in a single `.wl` schema:

```sh
wlc validate device.wl [--previous old.wl]
wlc compile device.wl --out-dir generated-codec                # pure codec when schema has no nodes
wlc compile device.wl --node server --out-dir generated-server # single node role
wlc compile device.wl --all-nodes --out-dir generated-loopback # in-process loopback
wlc plan device.wl --node server
wlc identity device.wl
wlc dependencies device.wl
wlc module-name device.wl
```

`compile` emits one pair `<stem>.h` / `<stem>.c` and `<stem>_manifest.json` (the legacy `_values.h`, `_kernel.h`, `.bind.wl`, `--profile`, and `compile-kernel` entries are removed). Zero nodes emit pure codecs; a single node is selected automatically; multiple nodes require `--node <name>` or `--all-nodes`.

## Default-required fields, borrowed views, and bounded storage

- **Default-required fields**: Unqualified fields and bare `packed` fixed arrays are required by default and omit `has_<field>`; only `optional` and `optional packed` emit presence members. Decoders still reject inputs missing any required field. Migrate legacy optional packed arrays explicitly to `optional packed`.
- **Bounded vs. borrowed storage**: Bounded `string<N>` / `bytes<N>` store data inline in the message struct with checked setters. Unbounded `string` / `bytes` and `repeated` use caller-owned borrowed views/backing in standalone codecs; node routes require bounded frames.
- **Wire rules**: See [Wire v2 format](https://github.com/starwey604/wirelink/blob/main/docs/wire-format.md).

## Validate composition and evolution

Imports resolve relative to the importing schema. `dependencies` prints canonical transitive inputs; `module-name` prints the sanitized C module stem.

Use `validate --previous previous.wl` or `compile --previous previous.wl` to check schema evolution compatibility. `identity` prints exact schema/node hashes, and `plan` reports node routes, frame bounds, storage sizing, and codec plans for diagnostics.

## Integrate generation into CMake

Wirelink provides `wirelink_wlc_generate(TARGET ... SCHEMA ... [NODE name | ALL_NODES] [PREVIOUS old.wl])` for node targets and `wirelink_wlc_generate_codec(TARGET ... SCHEMA ...)` for codec-only targets. Set `WIRELINK_WLC_EXECUTABLE` to the matching local 0.14 compiler; transitive imports are tracked as build dependencies. See [development checks](https://github.com/starwey604/wirelink/blob/main/docs/development.md).

Use a dedicated output directory per deployment so stale generated ABI files are never mixed. Rebuild WLC after changing codegen templates before regenerating C or SDK outputs, and validate generated code with wire vectors, malformed inputs, and compiled C consumers.
