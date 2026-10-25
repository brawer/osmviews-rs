// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! Integration tests that need no network: they build small OSMViews-shaped
//! GeoTIFFs in a temp file and exercise `open` / `rank` / `metrics` end to end.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    TILE_BYTES, TILE_OFFSETS_POS, TempTiff, build_tiff, build_tiff_single_blob,
    build_tiff_with_metadata, histogram_xml,
};
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
fn zip_bomb_tile_is_treated_as_empty_not_inflated() {
    // A tiny zlib stream that inflates to 4 MiB — 16× a tile. The stored blob is
    // a few KB (well under the parse-time cap), so `open()` accepts the file;
    // the danger is `rank()` inflating it in full.
    let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; TILE_BYTES * 16], 9);
    assert!(
        bomb.len() < TILE_BYTES,
        "the point of the test is a small blob; got {} bytes",
        bomb.len()
    );

    let fixture = TempTiff::new(&build_tiff_single_blob(8, &bomb, 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();

    // The tile decodes to more than one tile's worth of data, so it is rejected
    // and the query falls back to 0.0 rather than allocating gigabytes.
    assert_eq!(osmviews.rank(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1), 0.0);
    assert_eq!(
        osmviews.rank(IN_TILE_BOTTOM_RIGHT.0, IN_TILE_BOTTOM_RIGHT.1),
        0.0
    );
    assert_eq!(osmviews.metrics().tiles_cached, 0);
}

#[test]
fn rejects_tile_blob_larger_than_a_tile_could_compress_to() {
    // Stored size alone is implausible for a 256 KiB tile; rejected at parse
    // time, before the uncompressed path would `to_vec()` it.
    let oversized = vec![0u8; TILE_BYTES + 8192];
    let fixture = TempTiff::new(&build_tiff_single_blob(1, &oversized, 10.0));
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

fn assert_close(got: f64, want: f64) {
    assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
}

#[test]
fn quantiles_come_from_the_embedded_histogram() {
    // Buckets: 0.0 -> 0, 0.0625 -> 1 (on its lower edge), 0.1249 -> 1 (just
    // under the next edge), 0.5 -> 8 (the top bucket). So bucket 0 holds one
    // tile, bucket 1 two and bucket 8 one; the non-zero population is three
    // tiles.
    let fixture = TempTiff::new(&build_tiff(8, [0.0, 0.0625, 0.1249, 0.5], 0.5));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    let (tl, tr, bl, br) = (
        IN_TILE_TOP_LEFT,
        IN_TILE_TOP_RIGHT,
        IN_TILE_BOTTOM_LEFT,
        IN_TILE_BOTTOM_RIGHT,
    );

    assert_close(osmviews.zero_fraction(), 0.25);

    // Bucket 0 is outside the reference population.
    assert_eq!(osmviews.quantile(tl.0, tl.1), 0.0..0.0);
    assert_eq!(osmviews.quantile_interpolated(tl.0, tl.1), 0.0);

    // Both ends of bucket 1 share one range.
    for (lon, lat) in [tr, bl] {
        let q = osmviews.quantile(lon, lat);
        assert_close(q.start, 0.0);
        assert_close(q.end, 2.0 / 3.0);
    }
    assert_close(osmviews.quantile_interpolated(tr.0, tr.1), 0.0);
    assert_close(
        osmviews.quantile_interpolated(bl.0, bl.1),
        (f64::from(0.1249f32) * 16.0 - 1.0) * 2.0 / 3.0,
    );

    // The top bucket reaches 1.0.
    let q = osmviews.quantile(br.0, br.1);
    assert_close(q.start, 2.0 / 3.0);
    assert_close(q.end, 1.0);

    // Out of range behaves like an unviewed pixel.
    assert_eq!(osmviews.quantile(0.0, 89.0), 0.0..0.0);
}

#[test]
fn rank_quantile_matches_the_location_lookup() {
    let fixture = TempTiff::new(&build_tiff(8, [0.0, 0.0625, 0.1249, 0.5], 0.5));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    for (lon, lat) in [
        IN_TILE_TOP_LEFT,
        IN_TILE_TOP_RIGHT,
        IN_TILE_BOTTOM_LEFT,
        IN_TILE_BOTTOM_RIGHT,
    ] {
        let rank = osmviews.rank(lon, lat);
        assert_eq!(osmviews.rank_quantile(rank), osmviews.quantile(lon, lat));
        assert_eq!(
            osmviews.rank_quantile_interpolated(rank),
            osmviews.quantile_interpolated(lon, lat)
        );
    }

    // Clamped and degenerate inputs.
    assert_eq!(osmviews.rank_quantile(1.0), osmviews.rank_quantile(7.0));
    assert_close(osmviews.rank_quantile(1.0).end, 1.0);
    assert_eq!(osmviews.rank_quantile(0.0), 0.0..0.0);
    assert_eq!(osmviews.rank_quantile(-1.0), 0.0..0.0);
    assert_eq!(osmviews.rank_quantile(f64::NAN), 0.0..0.0);
}

#[test]
fn rank_quantile_keeps_values_on_bucket_edges() {
    // `rank = value / max` loses exactness for most maxima; the lookup must
    // still put a value that sits exactly on a bucket edge into that bucket,
    // not the one below.
    for max in [0.7f32, 3.3, 19.13, 19.1875] {
        for edge in [1.0f32, 2.0, 3.0, 5.0, 7.0, 11.0] {
            let v = edge / 16.0;
            if v >= max {
                continue;
            }
            let fixture = TempTiff::new(&build_tiff(8, [v, v - 0.01, max, 0.0], max));
            let osmviews = OsmViews::open(&fixture.path).unwrap();
            let (lon, lat) = IN_TILE_TOP_LEFT;
            let rank = osmviews.rank(lon, lat);
            assert_eq!(
                osmviews.rank_quantile(rank),
                osmviews.quantile(lon, lat),
                "max {max}, value {v}"
            );
        }
    }
}

#[test]
fn quantiles_rise_with_rank() {
    let fixture = TempTiff::new(&build_tiff(8, [0.3, 1.0, 2.2, 4.0], 4.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    let mut last = osmviews.rank_quantile_interpolated(0.0);
    for i in 1..=1000 {
        let q = osmviews.rank_quantile_interpolated(f64::from(i) / 1000.0);
        assert!(q >= last, "not monotonic at rank {i}/1000");
        let range = osmviews.rank_quantile(f64::from(i) / 1000.0);
        assert!(range.start <= q && q <= range.end);
        last = q;
    }
    // The maximum, 4.0, sits on the lower edge of the top bucket.
    assert_close(last, 0.75);
}

#[test]
fn rejects_file_without_histogram() {
    let fixture = TempTiff::new(&build_tiff_with_metadata(
        8,
        [1.0, 2.0, 3.0, 4.0],
        10.0,
        None,
    ));
    assert!(matches!(
        OsmViews::open(&fixture.path),
        Err(Error::Format(_))
    ));
}

#[test]
fn rejects_unusable_histogram() {
    for metadata in [
        "<GDALMetadata></GDALMetadata>".to_string(),
        histogram_xml(&[]),
        histogram_xml(&[0, 0, 0]),
        histogram_xml(&[3, 4]).replace("BinSize=\"0.0625\"", ""),
    ] {
        let fixture = TempTiff::new(&build_tiff_with_metadata(
            8,
            [1.0, 2.0, 3.0, 4.0],
            10.0,
            Some(&metadata),
        ));
        assert!(
            matches!(OsmViews::open(&fixture.path), Err(Error::Format(_))),
            "accepted {metadata}"
        );
    }
}

#[test]
fn quantile_lookups_count_as_queries() {
    let fixture = TempTiff::new(&build_tiff(8, [1.0, 2.0, 3.0, 4.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    let _ = osmviews.quantile(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1);
    let _ = osmviews.quantile_interpolated(IN_TILE_TOP_LEFT.0, IN_TILE_TOP_LEFT.1);
    let _ = osmviews.rank_quantile(0.5);
    let m = osmviews.metrics();
    assert_eq!(m.queries, 2);
    assert_eq!(m.tile_cache_hits, 1);
}
