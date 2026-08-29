// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! Integration tests that need no network: they build small OSMViews-shaped
//! GeoTIFFs in a temp file and exercise `open` / `rank` / `metrics` end to end.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{TILE_OFFSETS_POS, TempTiff, build_tiff};
use osmviews::{Error, OsmViews};

// Points that land in a known grid tile of a `build_tiff` raster (size 512, two
// tiles per axis). Western hemisphere -> left column, northern -> top row.
const IN_TILE_TOP_LEFT: (f64, f64) = (-90.0, 45.0);
const IN_TILE_TOP_RIGHT: (f64, f64) = (90.0, 45.0);
const IN_TILE_BOTTOM_LEFT: (f64, f64) = (-90.0, -45.0);
const IN_TILE_BOTTOM_RIGHT: (f64, f64) = (90.0, -45.0);

#[test]
fn ranks_scale_against_the_planetary_maximum() {
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 10.0, 10.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();

    assert!((osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1) - 0.3).abs() < 1e-6);
    assert_eq!(osmviews.rank(IN_TILE_TOP_RIGHT.0, IN_TILE_TOP_RIGHT.1), 0.0);
    assert_eq!(
        osmviews.rank(IN_TILE_BOTTOM_LEFT.0, IN_TILE_BOTTOM_LEFT.1),
        1.0
    );
    // Null Island falls in the bottom-right tile of this raster.
    assert_eq!(osmviews.rank(0.0, 0.0), 1.0);
    // Beyond the Web Mercator latitude limit.
    assert_eq!(osmviews.rank(0.0, 89.0), 0.0);
}

#[test]
fn ranks_clamp_to_the_unit_interval() {
    // Declared maximum is 10.0, but the tiles carry a value well above it and
    // one below zero.
    let fixture = TempTiff::new(&build_tiff(8, [25.0, -4.0, 10.0, 6.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();

    assert_eq!(osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1), 1.0); // 25.0 -> clamped
    assert_eq!(osmviews.rank(IN_TILE_TOP_RIGHT.0, IN_TILE_TOP_RIGHT.1), 0.0); // -4.0 -> clamped
    assert_eq!(
        osmviews.rank(IN_TILE_BOTTOM_LEFT.0, IN_TILE_BOTTOM_LEFT.1),
        1.0
    ); // exactly the max
    assert!((osmviews.rank(IN_TILE_BOTTOM_RIGHT.0, IN_TILE_BOTTOM_RIGHT.1) - 0.6).abs() < 1e-6);
}

#[test]
fn shared_tiles_use_one_cache_entry() {
    // Bottom-left and bottom-right tiles have the same value -> one blob.
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 7.0, 7.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();

    let _ = osmviews.rank(IN_TILE_BOTTOM_LEFT.0, IN_TILE_BOTTOM_LEFT.1);
    let _ = osmviews.rank(IN_TILE_BOTTOM_RIGHT.0, IN_TILE_BOTTOM_RIGHT.1);

    let m = osmviews.metrics();
    assert_eq!(m.tiles_cached, 1);
    assert_eq!(m.tiles_decoded, 1);
    assert_eq!(m.tile_cache_misses, 1);
    assert_eq!(m.tile_cache_hits, 1);
}

#[test]
fn uncompressed_tiles_are_supported() {
    let fixture = TempTiff::new(&build_tiff(1, [5.0, 5.0, 5.0, 5.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    assert!((osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1) - 0.5).abs() < 1e-6);
}

#[test]
fn metrics_track_queries_evictions_and_hit_rate() {
    let fixture = TempTiff::new(&build_tiff(8, [1.0, 2.0, 3.0, 4.0], 10.0));
    let osmviews = OsmViews::open_with_cache_capacity(&fixture.path, 2).unwrap();

    let _ = osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1);
    let _ = osmviews.rank(IN_TILE_TOP_RIGHT.0, IN_TILE_TOP_RIGHT.1);
    let _ = osmviews.rank(IN_TILE_BOTTOM_LEFT.0, IN_TILE_BOTTOM_LEFT.1);
    let _ = osmviews.rank(IN_TILE_BOTTOM_RIGHT.0, IN_TILE_BOTTOM_RIGHT.1);
    let _ = osmviews.rank(0.0, 89.0); // out of range

    let m = osmviews.metrics();
    assert_eq!(m.queries, 5);
    assert_eq!(m.out_of_range, 1);
    assert_eq!(m.tile_cache_misses, 4);
    assert_eq!(m.tile_cache_hits, 0);
    assert_eq!(m.tiles_cached, 2);
    assert_eq!(m.tile_cache_capacity, 2);
    assert_eq!(m.tile_cache_evictions, 2);
    assert!(m.decode_time > Duration::ZERO);
    assert_eq!(m.tile_cache_hit_rate(), 0.0);
}

#[test]
fn disabled_cache_still_answers_and_never_stores() {
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 7.0, 7.0], 10.0));
    let osmviews = OsmViews::open_with_cache_capacity(&fixture.path, 0).unwrap();

    assert!((osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1) - 0.3).abs() < 1e-6);
    assert!((osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1) - 0.3).abs() < 1e-6);

    let m = osmviews.metrics();
    assert_eq!(m.tiles_cached, 0);
    assert_eq!(m.tiles_decoded, 2);
    assert_eq!(m.tile_cache_hits, 0);
}

#[test]
fn rejects_non_tiff() {
    let fixture = TempTiff::new(b"this is definitely not a TIFF file, sorry");
    assert!(matches!(
        OsmViews::open(&fixture.path),
        Err(Error::Format(_))
    ));
}

#[test]
fn rejects_big_endian() {
    let mut bytes = build_tiff(8, [1.0; 4], 10.0);
    bytes[0] = b'M';
    bytes[1] = b'M';
    let fixture = TempTiff::new(&bytes);
    assert!(matches!(
        OsmViews::open(&fixture.path),
        Err(Error::Format(_))
    ));
}

#[test]
fn rejects_bigtiff_version() {
    let mut bytes = build_tiff(8, [1.0; 4], 10.0);
    bytes[2] = 43;
    bytes[3] = 0;
    let fixture = TempTiff::new(&bytes);
    assert!(matches!(
        OsmViews::open(&fixture.path),
        Err(Error::Format(_))
    ));
}

#[test]
fn rejects_truncated_file() {
    let bytes = build_tiff(8, [1.0, 2.0, 3.0, 4.0], 10.0);
    let fixture = TempTiff::new(&bytes[..bytes.len() / 2]);
    assert!(matches!(
        OsmViews::open(&fixture.path),
        Err(Error::Format(_))
    ));
}

#[test]
fn rejects_tile_offset_past_end_of_file() {
    let mut bytes = build_tiff(8, [1.0, 2.0, 3.0, 4.0], 10.0);
    bytes[TILE_OFFSETS_POS..TILE_OFFSETS_POS + 4].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
    let fixture = TempTiff::new(&bytes);
    assert!(matches!(
        OsmViews::open(&fixture.path),
        Err(Error::Format(_))
    ));
}

#[test]
fn shared_across_threads() {
    let fixture = TempTiff::new(&build_tiff(8, [1.0, 2.0, 3.0, 4.0], 10.0));
    let osmviews = Arc::new(OsmViews::open_with_cache_capacity(&fixture.path, 4).unwrap());

    let mut handles = Vec::new();
    for thread in 0..8u32 {
        let osmviews = Arc::clone(&osmviews);
        handles.push(std::thread::spawn(move || {
            let mut sum = 0.0;
            for i in 0..2000 {
                let lon = -90.0 + f64::from(thread) + f64::from(i % 5) * 0.01;
                let lat = if i % 2 == 0 { 45.0 } else { -45.0 };
                sum += osmviews.rank(lon, lat);
            }
            sum
        }));
    }
    for handle in handles {
        assert!(handle.join().unwrap() >= 0.0);
    }

    let m = osmviews.metrics();
    assert_eq!(m.queries, 8 * 2000);
    assert_eq!(m.out_of_range, 0);
    assert_eq!(
        m.tile_cache_hits + m.tile_cache_misses,
        m.queries - m.out_of_range
    );
    assert!(m.tiles_decoded >= m.tile_cache_misses);
}
