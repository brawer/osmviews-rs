<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Changelog

Notable changes to the `osmviews` crate. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning follows
[Semantic Versioning](https://semver.org) — see
[RELEASING.md](RELEASING.md#choosing-the-version-number) for how the `0.x`
version is chosen.

The [GitHub releases page](https://github.com/brawer/osmviews-rs/releases) has
the full per-release list of merged pull requests.

## [Unreleased]

## [0.1.1] - 2026-08-29

### Changed

- Minimum `miniz_oxide` is now 0.9 (was 0.8). No effect on this crate’s API or
  MSRV.
- Documentation only: added crates.io / docs.rs / CI / OpenSSF Scorecard badges
  and a sponsoring section to the README.

## [0.1.0] - 2026-08-29

Initial release.

### Added

- `OsmViews::open` and `OsmViews::open_with_cache_capacity` — read a downloaded
  OSMViews GeoTIFF from local disk (memory-mapped, with a small LRU cache of
  decoded tiles).
- `OsmViews::rank(lon, lat) -> f64` — a `0.0..=1.0` score for how much a location
  is looked at on OpenStreetMap-based maps. Longitude wraps around the globe;
  latitudes past the Web Mercator limit return `0.0`.
- `OsmViews::metrics()` and the `Metrics` type — cache hit rate, decode time,
  evictions, and other diagnostics for long-running jobs.
- `DOWNLOAD_URL` — the published location of the dataset.

[Unreleased]: https://github.com/brawer/osmviews-rs/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/brawer/osmviews-rs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/brawer/osmviews-rs/releases/tag/v0.1.0
