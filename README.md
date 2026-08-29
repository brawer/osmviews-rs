<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# osmviews

Rust client for [OSMViews](https://osmviews.toolforge.org), a world-wide ranking
of geographic locations by how much they are looked at on OpenStreetMap-based
maps. See the [main project](https://github.com/brawer/osmviews) for background.

OSMViews aggregates a year of OpenStreetMap map-tile access logs into a single
raster covering the whole planet. This crate reads a copy of that raster from
local disk and answers point queries.

## Usage

```rust
use osmviews::OsmViews;

let osmviews = OsmViews::open("osmviews.tiff").unwrap();

// rank() returns a value from 0.0 (nobody looks here) to 1.0 (one of the
// most-viewed places on the planet). Coordinates are WGS84 degrees, lon then lat.
println!("Tokyo, Shibuya:      {:.3}", osmviews.rank(139.7013,  35.6586));
println!("Zürich, Altstetten:  {:.3}", osmviews.rank(  8.4889,  47.3915));
println!("Ushuaia:             {:.3}", osmviews.rank(-68.3030, -54.8019));
```

The crate does **not** download anything. Fetch the dataset (~594 MB, regenerated
weekly) from `osmviews::DOWNLOAD_URL` however you like, then pass the path to
`OsmViews::open`.

`OsmViews` is `Send + Sync` and every query takes `&self`, so a single instance
can be shared across threads. Decoded tiles are kept in a small LRU cache
(configurable via `open_with_cache_capacity`), so queries clustered in one region
stay fast. `metrics()` returns counters (cache hit rate, decode time, …) worth
logging at the end of a long run.

## Minimal dependencies

Two small crates: [`memmap2`](https://crates.io/crates/memmap2) and
[`miniz_oxide`](https://crates.io/crates/miniz_oxide) (for the tiles’ DEFLATE
compression). The TIFF header parsing and the map projection are done in-crate.

## Minimum supported Rust version

Rust 1.85. Raising the MSRV is a semver-minor change.

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). The design and
its rationale are written up in [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md).

## Sponsoring

This crate and the [OSMViews](https://github.com/brawer/osmviews) pipeline behind
it are maintained by [Sascha Brawer](https://github.com/brawer) as a volunteer
effort. If your project relies on them, please consider sponsoring continued
maintenance and future development via
[GitHub Sponsors](https://github.com/sponsors/brawer).

## License

MIT — see [LICENSE](LICENSE).
