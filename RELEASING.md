<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Releasing

Releases are published to [crates.io](https://crates.io/crates/osmviews) by the
`.github/workflows/release.yml` workflow when a `v*` tag is pushed. It uses
crates.io **Trusted Publishing** (short-lived OIDC token, no stored secret) and
attaches **SLSA build provenance** (SLSA v1.0 Build Level 3, via
`actions/attest-build-provenance`) for the packaged `.crate`.

## One-time setup

This has to be done once, by a crate owner, before the workflow can publish.

### 1. Claim the crate name with a manual first publish

Trusted Publishing can only be configured on a crate that already exists, so the
very first version is published by hand:

```sh
cargo login          # paste a token from https://crates.io/settings/tokens
cargo publish        # from a clean checkout of the tagged commit
```

### 2. Create the `release` GitHub environment

Repository → **Settings → Environments → New environment**, name it `release`
(the workflow’s `publish` job references it). Optionally add protection rules —
required reviewers, and “Deployment branches and tags” limited to `v*` tags.

### 3. Configure the trusted publisher on crates.io

On the crate page → **Settings → Trusted Publishing → Add**:

| Field             | Value            |
| ----------------- | ---------------- |
| Publisher         | GitHub Actions   |
| Repository owner  | `brawer`         |
| Repository name   | `osmviews-rs`    |
| Workflow filename | `release.yml`    |
| Environment       | `release`        |

### 4. (Optional) Require Trusted Publishing

Once a trusted publish has succeeded, enable **“Require Trusted Publishing”** in
the crate settings to disable token-based publishing entirely.

## Cutting a release

1. Bump `version` in `Cargo.toml` (and mention notable changes wherever the
   changelog lives).
2. Commit: `git commit -am "chore: release v0.1.2"`.
3. Tag and push:

   ```sh
   git tag v0.1.2
   git push remo main --follow-tags
   ```

The workflow then: runs the full test suite including the `#[ignore]`d tests
against the freshly downloaded dataset, checks the tag matches `Cargo.toml`,
packages the crate, generates and signs provenance, publishes to crates.io, and
creates a GitHub release with the `.crate` attached.

## Verifying provenance

```sh
gh attestation verify osmviews-0.1.2.crate --repo brawer/osmviews-rs
```
