// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! A client for [OSMViews](https://osmviews.toolforge.org), a world-wide ranking
//! of geographic locations by how much they are looked at on OpenStreetMap-based
//! maps.
//!
//! OSMViews aggregates a year of OpenStreetMap map-tile access logs into a single
//! raster covering the whole planet. This crate reads a copy of that raster from
//! local disk and answers point queries:
//!
//! ```no_run
//! let osmviews = osmviews::OsmViews::open("osmviews.tiff")?;
//!
//! // A value from 0.0 (nobody looks here) to 1.0 (one of the most-viewed places).
//! let central_london = osmviews.rank(-0.1276, 51.5072);
//! let outback        = osmviews.rank(139.3508, -25.8975);
//! assert!(central_london > outback);
//! # Ok::<(), osmviews::Error>(())
//! ```
//!
//! The crate does not download anything: fetch the raster from [`DOWNLOAD_URL`]
//! (regenerated weekly, ~594 MB) however you like, then hand [`OsmViews::open`]
//! the path.
//!
//! [`OsmViews`] is [`Send`] + [`Sync`] and every query takes `&self`, so one
//! instance can be shared across threads.

mod cache;
mod projection;
mod tiff;

use std::fmt;
use std::fs::File;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use memmap2::Mmap;

use cache::Cache;
use tiff::{Header, TileOffset};

/// Where the OSMViews raster is published.
///
/// This crate never downloads anything itself, but exposing the URL as a
/// constant means a change of hosting is a version bump here rather than a
/// string to hunt down in every caller. The file behind it is regenerated
/// weekly and is roughly 594 MB.
pub const DOWNLOAD_URL: &str = "https://osmviews.toolforge.org/download/osmviews.tiff";

/// Decoded-tile cache capacity used by [`OsmViews::open`], in tiles. Each tile is
/// a fixed 256 KiB, so this is about 16 MiB.
const DEFAULT_CACHE_TILES: usize = 64;

const TILE_PIXELS: usize = 256 * 256;
const TILE_BYTES: usize = TILE_PIXELS * 4;

/// A read-only view of a downloaded OSMViews raster.
pub struct OsmViews {
    // SAFETY invariant: `mmap` must not be written through, and the backing file
    // must not change on disk while this value is alive (see `open`).
    mmap: Mmap,
    header: Header,
    cache: Mutex<Cache>,
}

impl OsmViews {
    /// Opens a downloaded OSMViews GeoTIFF from local disk.
    ///
    /// The file is memory-mapped and must not be modified or truncated for as
    /// long as the returned `OsmViews` is alive.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be opened or mapped, and
    /// [`Error::Format`] if it is not a readable OSMViews raster.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<OsmViews, Error> {
        Self::open_with_cache_capacity(path, DEFAULT_CACHE_TILES)
    }

    /// Like [`OsmViews::open`], but with an explicit decoded-tile cache capacity
    /// (in tiles). A capacity of `0` disables caching.
    ///
    /// Queries that fall in roughly the same region reuse cached tiles, so a
    /// larger cache helps workloads with spread-out spatial locality, at the cost
    /// of memory (256 KiB per tile).
    ///
    /// # Errors
    ///
    /// As for [`OsmViews::open`].
    pub fn open_with_cache_capacity<P: AsRef<Path>>(
        path: P,
        cache_tiles: usize,
    ) -> Result<OsmViews, Error> {
        let file = File::open(path)?;
        // SAFETY: the mapping is read-only and never written through. The
        // remaining hazard is the file being truncated or overwritten by another
        // process while it is mapped, which could raise SIGBUS; the crate's
        // contract (documented on `open`) is that the file stays unchanged for
        // the lifetime of this value.
        let mmap = unsafe { Mmap::map(&file)? };
        let header = tiff::parse(&mmap)?;
        Ok(OsmViews {
            mmap,
            header,
            cache: Mutex::new(Cache::new(cache_tiles)),
        })
    }

    /// How much the location at `lon`/`lat` (WGS84 degrees) is looked at on
    /// OpenStreetMap-based maps.
    ///
    /// The result runs from `0.0` (effectively never) to `1.0` (among the
    /// most-viewed places on the planet), derived from a year of OpenStreetMap
    /// tile-access logs. Locations near the poles, beyond the map’s coverage,
    /// return `0.0`. Longitude wraps, so `181.0` and `-179.0` are the same place.
    #[must_use]
    pub fn rank(&self, lon: f64, lat: f64) -> f64 {
        let Some((x, y)) = projection::project(lon, lat, self.header.size) else {
            self.cache.lock().unwrap().record_out_of_range();
            return 0.0;
        };

        let x = x as usize;
        let y = y as usize;
        let across = self.header.tiles_across as usize;
        let grid_index = (y >> 8) * across + (x >> 8);
        let pixel = (y & 255) * 256 + (x & 255);
        let offset = TileOffset(self.header.tile_offsets.get(&self.mmap, grid_index));

        if let Some(value) = self.cache.lock().unwrap().lookup(offset, pixel) {
            return self.scale(value);
        }

        let len = self.header.tile_byte_counts.get(&self.mmap, grid_index) as usize;
        let started = Instant::now();
        let Some(raw) = self.mmap.get(offset.0 as usize..offset.0 as usize + len) else {
            return 0.0;
        };
        let Some(tile) = self.decode(raw) else {
            return 0.0;
        };
        let elapsed = started.elapsed();
        let value = tile.get(pixel).copied().unwrap_or(0.0);
        self.cache.lock().unwrap().insert(offset, tile, elapsed);
        self.scale(value)
    }

    /// A snapshot of internal counters, meant to be logged once at the end of a
    /// long-running job.
    #[must_use]
    pub fn metrics(&self) -> Metrics {
        self.cache.lock().unwrap().metrics()
    }

    fn decode(&self, raw: &[u8]) -> Option<Box<[f32]>> {
        let bytes = if self.header.compression == 8 {
            // Cap the output at one tile. `raw` is a zero-copy slice of the mmap,
            // so a crafted blob can be a few KB yet inflate to gigabytes; the
            // `bytes.len()` check below runs too late to stop that allocation.
            match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(raw, TILE_BYTES + 1) {
                Ok(v) => v,
                Err(_) => return None,
            }
        } else {
            raw.to_vec()
        };
        if bytes.len() != TILE_BYTES {
            return None;
        }
        let mut out = vec![0.0f32; TILE_PIXELS].into_boxed_slice();
        for (slot, chunk) in out.iter_mut().zip(bytes.chunks_exact(4)) {
            *slot = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        Some(out)
    }

    fn scale(&self, value: f32) -> f64 {
        let max = self.header.max_value;
        if !value.is_finite() || max <= 0.0 || max.is_nan() {
            return 0.0;
        }
        (f64::from(value) / max).clamp(0.0, 1.0)
    }
}

/// Diagnostic counters from an [`OsmViews`] instance.
///
/// Every field except the `tiles_cached` / `tile_cache_capacity` gauges counts
/// monotonically since `open`. The cache’s memory footprint is
/// `tiles_cached * 256 KiB`.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Total calls to [`OsmViews::rank`].
    pub queries: u64,
    /// Calls whose coordinates fell outside the covered area (returned `0.0`).
    pub out_of_range: u64,
    /// Tile lookups served from the cache.
    pub tile_cache_hits: u64,
    /// Tile lookups that missed and triggered a decode.
    pub tile_cache_misses: u64,
    /// Tiles actually decoded. Exceeds `tile_cache_misses` only when threads
    /// race to decode the same tile.
    pub tiles_decoded: u64,
    /// Cache entries dropped to make room. Large relative to the capacity means
    /// the cache is too small for the workload.
    pub tile_cache_evictions: u64,
    /// Tiles currently held in the cache.
    pub tiles_cached: usize,
    /// Configured cache capacity, in tiles.
    pub tile_cache_capacity: usize,
    /// Cumulative wall-clock time spent reading and decompressing tiles.
    pub decode_time: Duration,
}

impl Metrics {
    /// The fraction of tile lookups served from cache, or `0.0` before the first
    /// lookup.
    #[must_use]
    pub fn tile_cache_hit_rate(&self) -> f64 {
        let lookups = self.tile_cache_hits + self.tile_cache_misses;
        if lookups == 0 {
            0.0
        } else {
            self.tile_cache_hits as f64 / lookups as f64
        }
    }
}

/// An error from [`OsmViews::open`].
#[derive(Debug)]
pub enum Error {
    /// The file could not be opened or memory-mapped.
    Io(std::io::Error),
    /// The file is not a TIFF, or not an OSMViews raster this crate can read.
    Format(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(msg) => write!(f, "not a readable OSMViews raster: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Format(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osmviews_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<OsmViews>();
        assert_send_sync::<Metrics>();
        assert_send_sync::<Error>();
    }

    #[test]
    fn download_url_points_at_a_tiff() {
        assert!(DOWNLOAD_URL.starts_with("https://"));
        assert!(DOWNLOAD_URL.ends_with(".tiff"));
    }
}
