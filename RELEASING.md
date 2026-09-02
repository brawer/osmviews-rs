<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Releasing

Releases are automated with
[release-please](https://github.com/googleapis/release-please) and published to
[crates.io](https://crates.io/crates/osmviews) by GitHub Actions.

## How it works

1. Every pull request has a [Conventional Commits](https://www.conventionalcommits.org)
   title (`feat:`, `fix:`, `docs:`, `perf:`, `refactor:`, `test:`, `build:`,
   `ci:`, `chore:`). PRs are squash-merged, so the title becomes the commit on
   `main`. A CI check enforces this.
2. `.github/workflows/release-please.yml` watches `main` and keeps a single open
   **“chore(main): release x.y.z”** pull request. It bumps `version` in
   `Cargo.toml` and `Cargo.lock`, updates `CHANGELOG.md` from the commit history,
   and updates `.release-please-manifest.json`. It runs as the account-wide
   **`brawer-release-bot`** GitHub App (see [One-time setup](#one-time-setup)) —
   a workflow run started by the built-in `GITHUB_TOKEN` cannot itself start
   further workflow runs, so the App identity is what lets the release PR’s CI
   run and the tag launch `release.yml`.
3. Review that PR and squash-merge it when you want to cut the release.
   release-please then pushes the `vX.Y.Z` tag and creates the GitHub release.
4. The tag push launches `release.yml` automatically. (To re-run it:
   `gh workflow run release.yml --ref vX.Y.Z`.) It then:
   - runs the full test suite, including the otherwise-`#[ignore]`d tests,
     against the freshly downloaded dataset;
   - checks the tag matches `Cargo.toml`;
   - packages the crate with `cargo package --locked`;
   - attests the `.crate`’s **SLSA build provenance** with
     `actions/attest-build-provenance` (Sigstore-signed, keyed to the artifact
     digest, kept in this repo’s attestation store — not attached to the
     release);
   - **waits for a maintainer to approve the `release` deployment** (the
     environment has a required reviewer and is limited to `v*` tags), then
     publishes to crates.io via **Trusted Publishing** (short-lived OIDC token,
     no stored secret).

## Choosing the version number

release-please picks the bump from the commit types since the last release:
`fix:` → patch, `feat:` → minor, and a `!` after the type or a `BREAKING CHANGE:`
footer → a breaking bump. While the crate is `0.x` (pre-1.0),
`bump-minor-pre-major` maps a breaking change to a **minor** bump (`0.1.z` →
`0.2.0`) and everything else to a **patch** bump — the same rule Cargo uses when
resolving `^` requirements.

Only `feat:`, `fix:` and `perf:` commits cut a release (and appear in
`CHANGELOG.md`). `docs:`, `refactor:`, `test:`, `build:`, `ci:` and `chore:`
(including Dependabot’s `chore(deps):`) are silent — they ride along with the
next real release. Use `Release-As:` if you need to ship one of those alone.

The public API is everything reachable from the crate root: `OsmViews` and its
methods, `Metrics` and its fields, the `Error` enum and its variants,
`DOWNLOAD_URL`, and the documented behaviour of `rank`.

| Bump                  | When                                                                                                                                                                                                                                            |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0.x.0` → `0.x.(y+1)` | Backwards-compatible: bug fixes, new `pub` items, more permissive behaviour, doc changes, dependency bumps that don’t change our API.                                                                                                              |
| `0.x.0` → `0.(x+1).0` | Anything a downstream crate could notice at compile time or that changes observed results: removing/renaming a `pub` item, changing a signature, adding an `Error` variant or a `Metrics` field, tightening input handling, **or raising the MSRV**. |

After `1.0.0` the usual rules apply: **major** for breaking changes, **minor**
for backwards-compatible additions, **patch** for backwards-compatible fixes.

To force a specific version, put `Release-As: 1.0.0` in a commit body.
`cargo semver-checks check-release` catches most accidental API breaks.

## One-time setup

Already configured on this repository (listed here in case it needs rebuilding):

- **crates.io Trusted Publisher** for `osmviews`: owner `brawer`, repository
  `osmviews-rs`, workflow `release.yml`, environment `release`.
- **GitHub `release` environment**: required reviewer, deployments limited to
  `v*` tags.
- The `main` ruleset requires the `ci` and `validate PR title` checks and
  squash-only merges.
- **`brawer-release-bot` GitHub App** — an account-wide App shared with the
  other `osmviews` repositories. `release-please.yml` authenticates as this App
  so its PR and tag can trigger CI. To rebuild it:
  1. Create the App at <https://github.com/settings/apps/new> (a personal App is
     fine). Homepage URL: the repo URL. Uncheck **Webhook → Active**.
     **Repository permissions**: `Contents: Read and write`,
     `Pull requests: Read and write`; nothing else.
  2. On the App page: **Generate a private key** (downloads a `.pem`), and note
     the **Client ID** (shown near the top, `Iv23…`).
  3. **Install App** → select the `osmviews` repositories.
  4. In the repo, **Settings → Secrets and variables → Actions**: add a
     **variable** `RELEASE_PLEASE_APP_CLIENT_ID` (the Client ID) and a **secret**
     `RELEASE_PLEASE_APP_PRIVATE_KEY` (the full `.pem` contents). Until the
     variable exists, the `release-please` workflow is skipped.
  5. Delete the local `.pem`. To rotate, generate a new key and update the
     secret; App tokens themselves are short-lived and auto-refreshed per run.

## Verifying a release

```sh
# Download the exact published .crate and verify its SLSA build provenance:
curl -fSL -o osmviews-X.Y.Z.crate \
  https://crates.io/api/v1/crates/osmviews/X.Y.Z/download
gh attestation verify osmviews-X.Y.Z.crate --repo brawer/osmviews-rs

# The GitHub release's own attestation (tag, commit) — once immutable releases
# are enabled on the repo:
gh release verify vX.Y.Z --repo brawer/osmviews-rs
```
