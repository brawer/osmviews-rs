<!--
SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
SPDX-License-Identifier: MIT
-->

# Technical design: the `osmviews` Rust crate

## Objective

Provide a small, native-feeling Rust library that, given a locally available copy
of the OSMViews raster, answers:

> How much is the location at this longitude/latitude looked at on
> OpenStreetMap-based maps, on a scale from 0.0 to 1.0?

The library reads a file path and nothing else — no network, no configuration, no
large dependency tree.

## Background

[OSMViews](https://osmviews.toolforge.org) is a weekly pipeline
([brawer/osmviews](https://github.com/brawer/osmviews)) that aggregates roughly a
year of OpenStreetMap map-tile access logs into a single Cloud-Optimized GeoTIFF.
Each pixel holds a 32-bit float: the density of map views for that patch of the
planet. The file is about 594 MB and covers the whole world at ~150 m resolution.

Being able to score a location by “how much do people look here” is a useful
signal wherever geographic results need to be ranked by real-world importance.
The immediate consumer is [alltheplaces/osm-diffs](https://github.com/alltheplaces/osm-diffs/),
which ranks suggested OpenStreetMap edits: a suggested fix in central London
should be surfaced before one in the Australian outback, all else being equal.
OSMViews is one of several signals feeding that ranking.

A [Python client](https://github.com/brawer/osmviews-py) already exists. This
crate is a fresh Rust design rather than a port; the two share only the data
format.

## Design

**Input.** `OsmViews::open(path)` memory-maps the file
([`memmap2`](https://crates.io/crates/memmap2)) and parses its header. Mapping
avoids copying the ~8 MB of tile-index tables into the process and lets the OS
page in only the tiles a workload actually touches. The cost is a documented
contract: the file must not change on disk while an `OsmViews` is alive.

**TIFF parsing.** Hand-written, in `src/tiff.rs`, ~300 lines. It reads only IFD 0
(the full-resolution level; the file also carries reduced-resolution pyramid
levels that this crate never needs) and validates that the file is the exact
shape the OSMViews pipeline produces — little-endian classic TIFF, 256×256 tiles,
single-band 32-bit float, DEFLATE or no compression, no predictor. Anything else
is rejected with an error. A general TIFF crate would be far more code and
surface area for a format we fully control.

**Projection.** The OSMViews grid is Web Mercator (EPSG:3857) and lines up
exactly with the standard “slippy map” tile scheme, so mapping longitude/latitude
to a pixel is about ten lines of arithmetic in `src/projection.rs` — no
projection library.

**Decompression.** Tiles are zlib-compressed, so the crate depends on
[`miniz_oxide`](https://crates.io/crates/miniz_oxide) for `inflate`. This is the
one genuinely algorithmic dependency; re-implementing DEFLATE would be a lot of
tricky code with real correctness risk.

**Tile cache.** Decoding a tile is the expensive step, so decoded tiles are kept
in a small LRU cache (default 64 tiles ≈ 16 MB, configurable). The cache is
**keyed by the tile’s byte offset in the file, not by its grid position**,
because the raster is sparse: its ~1 million grid positions resolve to only about
100 000 distinct tiles, and two “empty” tiles alone back most of the oceans.
Keying by offset means a sweep across open water occupies a single cache entry
instead of evicting everything useful.

**Concurrency.** `rank()` takes `&self`. The memory map is read-only and `Sync`;
the only shared mutable state is the cache, behind one `Mutex`. That lock is
dropped during the decode of a missed tile, so concurrent readers don’t
serialize on slow work; the worst case is two threads briefly decoding the same
tile and producing identical results. `OsmViews` is therefore `Send + Sync` and
one instance can serve many threads.

**Output.** `rank()` returns `f64` in `0.0..=1.0`. Internally this is the raw
sample divided by the raster’s embedded planetary maximum, clamped; that scaling
is an implementation detail and not part of the API contract.

**Observability.** `metrics()` returns a snapshot of counters — query count,
cache hit rate, evictions, cumulative decode time — meant to be logged once at
the end of a long-running job. The counters live under the cache mutex that every
query already takes, so they add nothing measurable to the hot path.

## Non-goals

- **Downloading or refreshing** the dataset. Callers fetch the file themselves.
- **On-demand tile loading over the network.** The file genuinely is a
  Cloud-Optimized GeoTIFF and per-tile HTTP range requests would be feasible, but
  this crate targets pipelines that download the whole file up front. On-demand
  loading (e.g. to support a WebAssembly target) is a plausible future extension,
  deliberately left out for now — contributions welcome.
- **Writing GeoTIFFs**, reading other coordinate reference systems, or reading
  rasters other than OSMViews.
- **Exposing raw pixel values** or a dataset date (the file carries no date).

## Security

Supply-chain posture is deliberately small:

- **Minimal dependency surface**: two direct dependencies (`memmap2`,
  `miniz_oxide`), and their transitive set is tiny.
- **`cargo-deny`** runs in CI — weekly and on any change to `Cargo.*` — to catch
  known advisories and unwanted licenses. Dependabot proposes dependency and
  GitHub Actions updates as a single aggregated pull request each month.
- **Releases** publish to crates.io via Trusted Publishing (short-lived OIDC
  token, no stored secret) and carry SLSA v1.0 Build Level 3 provenance for the
  packaged `.crate`. See `RELEASING.md`.
- **One `unsafe` block**, the `Mmap::map` call, with its safety contract
  documented at the call site and on `OsmViews::open`.
- All header parsing is bounds-checked, and a corrupt file is rejected at
  `open()` so that `rank()` cannot panic on bad data.
