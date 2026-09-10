# RPC runtime Flash optimization

2026-09-10, WLC 0.7.0-dev / generated ABI 32. No public layout, codec, schema
identity, binding identity, wire framing, or RPC metadata change.

## Changes

- Generate request/response receive cases only for enabled endpoint roles.
  An omitted role still recognizes its message IDs, reports the same
  delivery-mismatch or missing-route result, and releases the RX event.
- Share managed request header/session validation and server admission. Decode
  and fingerprint validation still precede peer observation, so malformed
  business payloads cannot evict another peer's pending work or replay cache.
- Share managed response preparation, metadata encoding, payload encoding, and
  cache commit. Thin service-specific adapters retain ordinary C type checking.
  Borrowed and owned response encoding remain distinct. Token owner,
  incarnation, and service identity checks are preserved before cache mutation.
- Do not add allocation, persistent scratch, or function-pointer casts. Do not
  force compiler optimization attributes or enable `-Os`.

Mapped RPC wire handling remains on its existing path; role pruning applies to
both mapped and managed services. `both` remains the default role. Advanced
public helper declarations/layouts are unchanged; manual setup must respect the
profile capabilities just as checked initialization already requires.

## Willow measurement

Consumer: Ragtime_Firmwares `c7039fe`, FCI `4f30689`, Wirelink `e180aa8`.
Board: dm_mc02/stm32h723xx. Toolchain: Zephyr SDK 1.0.1, GCC 14.3.0.
Compare WLC `18b830a` with this implementation using identical `-O2 + LTO`
configuration. Ruckig, logging, fault capture, and application features are unchanged.

| Measurement | Before | After |
| --- | ---: | ---: |
| Entire application Flash (`_flash_used` and `.bin`) | 402,340 B | 383,124 B |
| Application RAM | 126,128 B | 126,128 B |
| `fci_device_runtime_dispatch_event` | 40,600 B | 23,334 B |
| Approximate FCI named-symbol attribution including codec/descriptors | 80,486 B | 61,704 B |

Whole-image reduction: **19,216 B / 18.77 KiB / 4.78%**. LTO attribution includes
inlined callees and excludes unnamed strings/padding, so the whole image is the
authoritative savings figure. The dispatcher alone is not the whole runtime.

The image now fits below the nominal 384 KiB slot by 10,092 B. This is not a
signed MCUboot image, and additional update logic, image header, signature TLVs,
and trailer still require space. No Ruckig or compiler-policy changes are included.

## Regression gates

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
WLC_TEST_SANITIZE=1 cargo test --test managed_rpc --test endpoint_layout \
  --test shared_scratch --test direct_routes --test async_endpoint
cargo test --test rpc_flash -- --nocapture
```

`rpc_flash` uses `arm-none-eabi-gcc` when installed, at Cortex-M7 `-O2` without
LTO. The 24-service fixture measures 19,662 B on this machine; its 24 KiB budget
counts dispatch plus extracted admission/completion helpers and typed wrappers.
The test explicitly reports a skip when that toolchain is absent. It does not
replace the real application's linked-image measurement.

Verified locally: WLC full suite (148 tests), focused generated-C ASan/UBSan
suites (15 tests), formatting and Clippy, FCI tests (12/12), and libflorid Release
tests in both regenerated and checked-in snapshot modes (7/7 each). FCI includes
arm/upgrade/dual/device host and firmware recipe compilation. The compact artifact
snapshots change only the managed runtime C artifact, not its public headers or
the business codec. These checks do not establish hardware latency or throughput;
no new hardware-performance claim is made for this change.
