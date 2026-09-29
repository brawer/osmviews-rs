<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# osmviews

[![crates.io](https://img.shields.io/crates/v/osmviews.svg)](https://crates.io/crates/osmviews)
[![docs.rs](https://img.shields.io/docsrs/osmviews)](https://docs.rs/osmviews)
[![CI](https://github.com/brawer/osmviews-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/brawer/osmviews-rs/actions/workflows/ci.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/brawer/osmviews-rs/badge)](https://scorecard.dev/viewer/?uri=github.com/brawer/osmviews-rs)
[![REUSE status](https://api.reuse.software/badge/github.com/brawer/osmviews-rs)](https://api.reuse.software/info/github.com/brawer/osmviews-rs)

Rust client for [OSMViews](https://osmviews.brawer.ch), a world-wide ranking
of geographic locations by how much they are looked at on OpenStreetMap-based
maps. See the [main project](https://github.com/brawer/osmviews) for background.

OSMViews aggregates a year of OpenStreetMap map-tile access logs into a single
raster covering the whole planet. This crate reads a copy of that raster from
local disk and answers point queries.

## Usage

```rust
use osmviews::OsmViews;

let osmviews = OsmViews::open("osmviews.tiff").unwrap();

// rank() is 0.0 (nobody looks here) to 1.0 (one of the most-viewed places on
// Earth). Coordinates are WGS84 degrees, lon then lat; values drift weekly.
let shibuya    = osmviews.rank(139.7013,  35.6586); // Tokyo, Shibuya     ~0.69
let altstetten = osmviews.rank(  8.4889,  47.3915); // Zürich, Altstetten ~0.66
let ushuaia    = osmviews.rank(-68.3030, -54.8019); // Ushuaia            ~0.56
let sahara     = osmviews.rank( 13.0000,  23.0000); // Sahara             ~0.00
assert!(shibuya > altstetten && altstetten > ushuaia && ushuaia > sahara);
```

The crate does **not** download anything. The dataset (~594 MB) is rebuilt
weekly under a new dated file name, so there is no single download link. Instead,
fetch the small [data package](https://datapackage.org) descriptor at
`osmviews::DATAPACKAGE_URL`. It names the current file and its SHA-256. Download
that file however you like, then pass the path to `OsmViews::open`. For example,
with [`ureq`](https://crates.io/crates/ureq) (feature `json`),
[`serde_json`](https://crates.io/crates/serde_json) and
[`sha2`](https://crates.io/crates/sha2):

```rust
use std::fs::File;
use std::io::{Read, Write};

use sha2::{Digest, Sha256};

let package: serde_json::Value = ureq::get(osmviews::DATAPACKAGE_URL)
    .call()?
    .body_mut()
    .read_json()?;
let raster = package["resources"]
    .as_array()
    .and_then(|r| r.iter().find(|r| r["name"] == "osmviews"))
    .ok_or("no osmviews resource in data package")?;
let (base, _) = osmviews::DATAPACKAGE_URL.rsplit_once('/').unwrap();
let url = format!("{base}/{}", raster["path"].as_str().ok_or("no path")?);

// Download and hash in one pass.
let mut body = ureq::get(&url).call()?.into_body().into_reader();
let mut file = File::create("osmviews.tiff")?;
let mut hasher = Sha256::new();
let mut buf = vec![0; 1 << 16];
loop {
    let n = body.read(&mut buf)?;
    if n == 0 {
        break;
    }
    hasher.update(&buf[..n]);
    file.write_all(&buf[..n])?;
}
let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
if raster["hash"] != format!("sha256:{hex}") {
    return Err(format!("checksum mismatch for {url}").into());
}
```

The descriptor’s `version` is the date of the build, so you can tell which
week’s views you have.

`osmviews::DOWNLOAD_URL` is deprecated. The link it names stops working after
2026-12-10, and the constant will be removed in 0.2.0.

`OsmViews` is `Send + Sync` and every query takes `&self`, so a single instance
can be shared across threads. Decoded tiles are kept in a small LRU cache
(configurable via `open_with_cache_capacity`), so queries clustered in one region
stay fast. `metrics()` returns counters (cache hit rate, decode time, …) worth
logging at the end of a long run.

## Performance

Rough numbers on an Apple M5 (from `tests/bench.rs`): `rank()` returns in ~40 ns
when the tile is already cached and ~70 µs on a miss that has to read and inflate
one. Each decoded tile is 256 KiB; the default LRU holds 64 of them (~16 MiB),
and the GeoTIFF is memory-mapped rather than read onto the heap. For bulk
lookups, submit points in roughly spatial order (e.g. sorted by tile or by
S2 cell ID) so neighbouring queries reuse cached tiles.

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

The repository is [REUSE](https://reuse.software) compliant: every file declares
its copyright and licensing, in an SPDX header or via [`REUSE.toml`](REUSE.toml).
The sole exception to MIT is `CODE_OF_CONDUCT.md`, adapted from the Contributor
Covenant 3.0 (CC-BY-SA-4.0).
