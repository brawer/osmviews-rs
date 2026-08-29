<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Releasing

Releases are published to [crates.io](https://crates.io/crates/osmviews) by the
`.github/workflows/release.yml` workflow when a `v*` tag is pushed. It uses
crates.io **Trusted Publishing** (short-lived OIDC token, no stored secret) and
attaches **SLSA build provenance** (SLSA v1.0 Build Level 3, via
`actions/attest-build-provenance`) for the packaged `.crate`. The trusted
publisher, the `release` GitHub environment, and the `v*` tag protection rule are
already configured.

## Choosing the version number

Follow [Semantic Versioning](https://semver.org), as Cargo does when resolving
`^` version requirements. The public API is everything reachable from the crate
root: `OsmViews` and its methods, `Metrics` and its fields, the `Error` enum and
its variants, `DOWNLOAD_URL`, and the documented behaviour of `rank`.

While the crate is `0.x` (pre-1.0), Cargo treats a bump of the **minor** as
breaking and a bump of the **patch** as compatible:

| Bump                | When                                                                                                                                                                   |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0.x.0` → `0.x.(y+1)` | Backwards-compatible: bug fixes, new `pub` items, more permissive behaviour, doc changes, dependency bumps that don’t change our API.                                    |
| `0.x.0` → `0.(x+1).0` | Anything a downstream crate could notice at compile time or that changes observed results: removing/renaming a `pub` item, changing a signature, adding an `Error` variant or a `Metrics` field, tightening input handling, **or raising the MSRV**. |

After `1.0.0` the usual rules apply: **major** for breaking changes, **minor**
for backwards-compatible additions, **patch** for backwards-compatible fixes.

When in doubt, `cargo semver-checks check-release` catches most accidental API
breaks.

## Cutting a release

1. Pick the new version per the rules above and set `version` in `Cargo.toml`.
2. Commit: `git commit -am "chore: release v0.1.2"`.
3. Tag and push:

   ```sh
   git tag v0.1.2
   git push remo main --follow-tags
   ```

The workflow then runs the full test suite including the `#[ignore]`d tests
against the freshly downloaded dataset, checks the tag matches `Cargo.toml`,
waits for a maintainer to approve the `release` environment, packages the crate,
generates and signs provenance, publishes to crates.io, and creates a GitHub
release. Its notes are generated from the merged PRs, grouped by label per
`.github/release.yml` (labels are applied automatically from each PR’s
Conventional Commits title).

## Verifying provenance

```sh
gh attestation verify osmviews-0.1.2.crate --repo brawer/osmviews-rs
```
