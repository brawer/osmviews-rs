// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! Dependency-free micro-benchmarks. `#[ignore]`d; run with:
//!
//! ```sh
//! cargo test --release --test bench -- --ignored --nocapture
//! ```

mod common;

use std::time::Instant;

use common::{TempTiff, build_tiff};
use osmviews::OsmViews;

#[test]
#[ignore = "benchmark; run with --release --nocapture"]
fn bench_rank_cached() {
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 7.0, 7.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    let _ = osmviews.rank(-90.0, 45.0); // warm the tile

    let iterations = 10_000_000u64;
    let start = Instant::now();
    let mut acc = 0.0f64;
    for i in 0..iterations {
        // Sweep within the (already cached) top-left tile so nothing folds away.
        let lon = -120.0 + f64::from((i % 60) as u32);
        acc += osmviews.rank(lon, 45.0);
    }
    let elapsed = start.elapsed();

    let m = osmviews.metrics();
    eprintln!(
        "rank(), cached tile: {:.1} ns/call ({iterations} calls, acc={acc:.1}, hit rate {:.5})",
        elapsed.as_nanos() as f64 / iterations as f64,
        m.tile_cache_hit_rate(),
    );
    assert!(m.tile_cache_hit_rate() > 0.99);
}

#[test]
#[ignore = "benchmark; run with --release --nocapture"]
fn bench_rank_uncached() {
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 7.0, 7.0], 10.0));
    let osmviews = OsmViews::open_with_cache_capacity(&fixture.path, 0).unwrap();

    let iterations = 200_000u64;
    let start = Instant::now();
    let mut acc = 0.0f64;
    for _ in 0..iterations {
        acc += osmviews.rank(-90.0, 45.0);
    }
    let elapsed = start.elapsed();

    eprintln!(
        "rank(), re-decoding every call: {:.0} ns/call ({iterations} calls, acc={acc:.1})",
        elapsed.as_nanos() as f64 / iterations as f64,
    );
}

#[test]
#[ignore = "benchmark; run with --release --nocapture"]
fn bench_quantile() {
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 7.0, 7.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();
    let _ = osmviews.quantile(-90.0, 45.0); // warm the tile

    let iterations = 10_000_000u64;
    let start = Instant::now();
    let mut acc = 0.0f64;
    for i in 0..iterations {
        let lon = -120.0 + f64::from((i % 60) as u32);
        acc += osmviews.quantile(lon, 45.0).end;
    }
    let elapsed = start.elapsed();

    eprintln!(
        "quantile(), cached tile: {:.1} ns/call ({iterations} calls, acc={acc:.1})",
        elapsed.as_nanos() as f64 / iterations as f64,
    );
}

#[test]
#[ignore = "benchmark; run with --release --nocapture"]
fn bench_rank_quantile() {
    let fixture = TempTiff::new(&build_tiff(8, [3.0, 0.0, 7.0, 7.0], 10.0));
    let osmviews = OsmViews::open(&fixture.path).unwrap();

    let iterations = 100_000_000u64;
    let start = Instant::now();
    let mut acc = 0.0f64;
    for i in 0..iterations {
        // Vary the input so the lookup can't be hoisted out of the loop.
        let rank = f64::from((i % 1000) as u32) / 1000.0;
        acc += osmviews.rank_quantile(std::hint::black_box(rank)).end;
    }
    let elapsed = start.elapsed();

    eprintln!(
        "rank_quantile(): {:.1} ns/call ({iterations} calls, acc={acc:.1})",
        elapsed.as_nanos() as f64 / iterations as f64,
    );
}
