// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! Test against the real ~594 MB OSMViews dataset.
//!
//! `#[ignore]`d by default. Provide the file via the `OSMVIEWS_TIFF` environment
//! variable or by dropping `osmviews.tiff` in the repository root, then:
//!
//! ```sh
//! cargo test --test online -- --ignored
//! ```
//!
//! Assertions check only relative order and coarse thresholds, never absolute
//! values: the dataset is regenerated weekly and drifts.

use std::path::PathBuf;

use osmviews::OsmViews;

fn dataset_path() -> Option<PathBuf> {
    if let Ok(from_env) = std::env::var("OSMVIEWS_TIFF") {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }
    let in_repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("osmviews.tiff");
    in_repo.is_file().then_some(in_repo)
}

#[test]
#[ignore = "needs the ~594 MB osmviews.tiff (set OSMVIEWS_TIFF or drop it in the repo root)"]
fn ranks_reflect_how_the_planet_is_viewed() {
    let Some(path) = dataset_path() else {
        eprintln!("skipping: no OSMViews dataset (OSMVIEWS_TIFF unset, ./osmviews.tiff absent)");
        return;
    };
    let osmviews = OsmViews::open(&path).expect("open real dataset");
    let rank = |lon, lat| osmviews.rank(lon, lat);

    // (lon, lat)
    let london_centre = rank(-0.1281, 51.5080); // Trafalgar Square
    let london_inner = rank(-0.0553, 51.5452); // Hackney
    let london_outer = rank(0.1730, 51.6217); // Havering-atte-Bower
    let bern_centre = rank(7.4474, 46.9480);
    let ushuaia = rank(-68.3030, -54.8019);

    // Cross-region ordering of city centres.
    assert!(
        london_centre > bern_centre && bern_centre > ushuaia,
        "expected London {london_centre} > Bern {bern_centre} > Ushuaia {ushuaia}"
    );

    // The dataset's ~150 m resolution resolves the fall-off across one city.
    assert!(
        london_centre > london_inner && london_inner > london_outer,
        "expected London {london_centre} > {london_inner} > {london_outer} centre to edge"
    );

    // Remote / empty places: well below any inhabited point, but not necessarily
    // exactly zero, and not ordered against each other.
    for (name, value) in [
        ("Sahara", rank(13.0, 23.0)),
        ("remote S Pacific", rank(-140.0, -30.0)),
        ("Birdsville", rank(139.3508, -25.8975)),
    ] {
        assert!(value < 0.1, "{name} = {value}, expected < 0.1");
        assert!(
            value < ushuaia,
            "{name} = {value}, expected below Ushuaia {ushuaia}"
        );
    }

    // Poles and non-finite inputs.
    assert_eq!(rank(0.0, 90.0), 0.0);
    assert_eq!(rank(0.0, -90.0), 0.0);
    assert_eq!(rank(f64::NAN, 0.0), 0.0);

    // Null Island is a known bad-geocode hotspot and sits at the planetary max.
    assert!(rank(0.0, 0.0) > 0.9, "Null Island = {}", rank(0.0, 0.0));

    let m = osmviews.metrics();
    assert_eq!(m.out_of_range, 3); // the two poles + the NaN
    assert!(m.tiles_decoded >= 1);
}
