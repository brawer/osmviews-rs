<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to the `osmviews` crate are recorded here. From v0.1.3 on
this file is maintained by
[release-please](https://github.com/googleapis/release-please) from the
Conventional Commit history. Versioning follows
[Semantic Versioning](https://semver.org); while the crate is pre-1.0 a bump of
the **minor** version may be breaking — see
[RELEASING.md](RELEASING.md#choosing-the-version-number).

## [0.1.3](https://github.com/brawer/osmviews-rs/compare/v0.1.2...v0.1.3) (2026-09-02)


### 🐞 Fixes

* bound tile decompression to one tile's worth of output ([#17](https://github.com/brawer/osmviews-rs/issues/17)) ([ecddf1a](https://github.com/brawer/osmviews-rs/commit/ecddf1a1ef3310add0b1c1e762bf506b4d3a44d6))

## [0.1.2](https://github.com/brawer/osmviews-rs/compare/v0.1.1...v0.1.2) (2026-08-30)

### Changed

- Documentation only: shorter README usage example, and a Performance section
  with rough timings and a spatial-ordering tip.

## [0.1.1](https://github.com/brawer/osmviews-rs/compare/v0.1.0...v0.1.1) (2026-08-29)

### Changed

- Minimum `miniz_oxide` is now 0.9 (was 0.8). No effect on this crate’s API or
  MSRV.
- Documentation only: added crates.io / docs.rs / CI / OpenSSF Scorecard badges
  and a sponsoring section to the README.

## [0.1.0](https://github.com/brawer/osmviews-rs/releases/tag/v0.1.0) (2026-08-29)

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
