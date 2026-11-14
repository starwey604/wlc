# Rehearse and promote a compiler release

Run a rehearsal from the frozen compiler branch, passing the exact Wirelink
candidate. `SOURCE` must be that branch's commit; Wirelink's
`.github/release-pair.json` must name it.

```sh
gh workflow run release.yml --repo starwey604/wlc --ref "$COMPILER_BRANCH" \
  -f source_ref="$SOURCE" -f wirelink_ref="$WIRELINK_SOURCE" -F dry_run=true
```

The Release workflow shares ordinary CI's Rust/generated-C verification. It
builds five archives with Rust 1.96.1 and locked dependencies, extracts each,
checks version/contract/API, compares checked-in generated artifacts and runs
profile-v2 packed-float RPC plus installed C/C++/COBS/keyword consumers against
the paired core. `wlc-build.json` and `wlc-release.json` record both source SHAs.

After the WLC and Wirelink rehearsals both pass, promote the successful WLC run:

```sh
gh workflow run release.yml --repo starwey604/wlc --ref "$COMPILER_BRANCH" \
  -f source_ref="$SOURCE" -f wirelink_ref="$WIRELINK_SOURCE" \
  -f tag=v0.13.0 -f promote_run="$WLC_RUN" -F dry_run=false
```

Promotion downloads `wlc-release-bundle` from that successful run and verifies
source, pair, inventory and `SHA256SUMS`. It creates the missing tag at the
verified source and publishes those exact assets without rebuilding. Repeated
promotion of identical assets succeeds without changing the release; different
existing assets fail and are never overwritten. Keep the rehearsal artifacts
available until both releases are published.

Tag-push publication and manual rebuilding of an existing tag remain available.
A tag push uses the qualified API baseline; paired releases use the explicit
Wirelink candidate and promotion above to record the final core commit.
For an existing tag, `tag=v0.13.0` alone retains the publication entry point;
`dry_run=true` makes it a non-publishing rebuild. A rebuild that differs from
already-published assets is rejected rather than uploaded with `--clobber`.

Consumers can download the matching [release archive](https://github.com/starwey604/wlc/releases/tag/v0.13.0)
without Rust. Verify its archive against `SHA256SUMS` before extraction. The
Wirelink release's provenance must name this compiler commit and the same core
commit. Continue with the [paired release procedure](https://github.com/starwey604/wirelink/blob/main/docs/releasing-cn.md).
