<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

<!--
Title this PR in Conventional Commits style, e.g. "fix: clamp longitude at the
antimeridian". CI checks the title, and it becomes the squash-merge commit
message. Add "!" (e.g. "feat!: ...") for a breaking change.
-->

## What and why



## Checklist

- [ ] `cargo test` passes (and new behaviour has a test)
- [ ] `cargo fmt` and `cargo clippy --all-targets -- -D warnings` are clean
- [ ] Public API changes are documented and noted as breaking if they are
- [ ] Sources for any adapted code or data are credited
