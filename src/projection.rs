// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! WGS84 longitude/latitude → pixel coordinate in the OSMViews raster.
//!
//! The OSMViews GeoTIFF is stored in Web Mercator (EPSG:3857) and its pixel grid
//! lines up exactly with the standard “slippy map” tile scheme at the zoom level
//! whose world width in pixels equals the raster width. So the mapping is the
//! well-known slippy-tile math, done once here in a few lines rather than pulling
//! in a projection crate.

use std::f64::consts::PI;

/// The latitude beyond which Web Mercator is not defined. Locations at or past
/// this latitude (in either hemisphere) have no data.
pub(crate) const MAX_LAT: f64 = 85.051_128_779_806_59;

/// Maps `lon`/`lat` (WGS84 degrees) to an `(x, y)` pixel in a `size` × `size`
/// raster that spans the whole Web Mercator world square.
///
/// Longitude is treated as periodic, so `182.0` is the same meridian as `-178.0`.
/// Returns `None` when the inputs are not finite or the latitude is outside the
/// Web Mercator range.
pub(crate) fn project(lon: f64, lat: f64, size: u32) -> Option<(u32, u32)> {
    if !lon.is_finite() || !lat.is_finite() || lat.abs() >= MAX_LAT {
        return None;
    }
    let n = f64::from(size);
    let x = (lon + 180.0).rem_euclid(360.0) / 360.0 * n;
    let y = (1.0 - lat.to_radians().tan().asinh() / PI) / 2.0 * n;
    let max = f64::from(size - 1);
    Some((
        (x.floor().clamp(0.0, max)) as u32,
        (y.floor().clamp(0.0, max)) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: u32 = 262_144;

    /// Independent reference implementation of the slippy-tile northing,
    /// written the “textbook” way with `ln(tan + sec)` instead of `asinh(tan)`.
    fn reference(lon: f64, lat: f64, size: u32) -> (u32, u32) {
        let n = f64::from(size);
        let lat_rad = lat.to_radians();
        let x = (lon + 180.0) / 360.0 * n;
        let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / PI) / 2.0 * n;
        (x.floor() as u32, y.floor() as u32)
    }

    #[test]
    fn null_island_is_the_centre_pixel() {
        assert_eq!(project(0.0, 0.0, SIZE), Some((SIZE / 2, SIZE / 2)));
    }

    #[test]
    fn antimeridian_is_the_left_edge() {
        // -180, +180 and -180 ± full turns all name the same meridian.
        assert_eq!(project(-180.0, 0.0, SIZE).unwrap().0, 0);
        assert_eq!(project(180.0, 0.0, SIZE).unwrap().0, 0);
        assert_eq!(project(-540.0, 0.0, SIZE).unwrap().0, 0);
    }

    #[test]
    fn longitude_wraps_around_the_globe() {
        for lon in [-178.0_f64, 0.0, 90.0, 179.9] {
            assert_eq!(
                project(lon, 20.0, SIZE),
                project(lon + 360.0, 20.0, SIZE),
                "lon {lon} vs {}",
                lon + 360.0
            );
            assert_eq!(
                project(lon, 20.0, SIZE),
                project(lon - 360.0, 20.0, SIZE),
                "lon {lon} vs {}",
                lon - 360.0
            );
        }
        // 182° east is 178° west.
        assert_eq!(project(182.0, 0.0, SIZE), project(-178.0, 0.0, SIZE));
    }

    #[test]
    fn out_of_range_latitudes_and_non_finite_inputs() {
        assert_eq!(project(0.0, 85.06, SIZE), None);
        assert_eq!(project(0.0, -85.06, SIZE), None);
        assert_eq!(project(0.0, 90.0, SIZE), None);
        assert_eq!(project(0.0, -90.0, SIZE), None);
        assert_eq!(project(f64::NAN, 0.0, SIZE), None);
        assert_eq!(project(0.0, f64::INFINITY, SIZE), None);
    }

    #[test]
    fn matches_reference_in_every_quadrant() {
        // (name, lat, lon, expected pixel) — expected values computed offline.
        let cases = [
            ("Tokyo", 35.6586, 139.7016, (232_799_u32, 103_246_u32)),
            ("New York", 40.7128, -74.0060, (77_182, 98_561)),
            ("Sydney", -33.8688, 151.2093, (241_179, 157_310)),
            ("Buenos Aires", -34.6037, -58.3816, (88_559, 157_957)),
        ];
        for (name, lat, lon, expected) in cases {
            assert_eq!(project(lon, lat, SIZE), Some(expected), "{name}");
            assert_eq!(reference(lon, lat, SIZE), expected, "{name} (reference)");
        }
    }
}
