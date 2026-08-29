<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Contributing 👋

Thanks for looking! This is a small, focused crate and contributions of every
size are welcome — a typo fix, a clearer doc sentence, a missing test case, a bug
report, or a new feature. No contribution is too small. 🙂

## Getting set up 🛠️

```sh
cargo build
cargo test
cargo fmt
cargo clippy --all-targets
```

CI requires `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
to be clean, and runs the test suite on both stable Rust and the crate’s minimum
supported version (Rust 1.85).

## Running the tests against the real dataset 🌍

Most tests build tiny synthetic GeoTIFFs and need nothing extra. The end-to-end
test in `tests/online.rs` runs against the real ~594 MB dataset and is `#[ignore]`d
by default. To run it, fetch the file and point the test at it:

```sh
curl -L -o osmviews.tiff https://osmviews.toolforge.org/download/osmviews.tiff
OSMVIEWS_TIFF="$PWD/osmviews.tiff" cargo test -- --ignored
```

`osmviews.tiff` in the repository root is picked up automatically (and is
git-ignored), so `cargo test -- --ignored` works once the file is there.

## Running the micro-benchmarks 📈

```sh
cargo test --release --test bench -- --ignored --nocapture
```

These print `ns/call` numbers for a cache hit and for the re-decode path; they
are informational, not pass/fail.

## Commit and PR style 📝

We use [Conventional Commits](https://www.conventionalcommits.org) for pull
request titles (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `perf:`,
`build:`, `chore:`, `ci:`), and CI checks the PR title. PRs are squash-merged, so
the title becomes the commit message on `main`.

Please keep changes focused, add tests for behaviour changes, and credit any
sources you adapt code or data from.

## Reporting issues and asking questions 🤝

Open an issue on GitHub. For anything sensitive, or to report a Code of Conduct
concern, email Sascha (sascha@brawer.ch). By contributing you agree that your
work is licensed under the [MIT License](LICENSE), and to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md).
