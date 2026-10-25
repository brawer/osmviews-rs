// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! The pixel-value histogram that the OSMViews pipeline embeds in the
//! GeoTIFF's `GDAL_METADATA` tag (42112), and quantile lookups against it.
//!
//! The tag holds a small XML document with a band-1 Raster Attribute Table:
//!
//! ```xml
//! <GDALMetadata>
//!   <Item name="DEFAULT_RASTER_ATTRIBUTE_TABLE" sample="0" role="rat">
//!     <GDALRasterAttributeTable Row0Min="0" BinSize="0.0625" tableType="athematic">
//!       <FieldDefn index="0"><Name>min</Name><Type>1</Type><Usage>3</Usage></FieldDefn>
//!       <FieldDefn index="1"><Name>max</Name><Type>1</Type><Usage>4</Usage></FieldDefn>
//!       <FieldDefn index="2"><Name>count</Name><Type>1</Type><Usage>1</Usage></FieldDefn>
//!       <Row index="0"><F>0</F><F>0.0625</F><F>37581782296</F></Row>
//!       ...
//! ```
//!
//! Bucket `k` covers `[Row0Min + k·BinSize, Row0Min + (k+1)·BinSize)`. We read
//! only `Row0Min`, `BinSize` and the column whose GDAL usage is `PixelCount`
//! (`1`). The parser is hand-written for exactly this shape, to keep the crate
//! free of an XML dependency; anything else is rejected.

use std::ops::Range;

use crate::Error;

/// GDAL's `GFU_PixelCount` field usage code.
const USAGE_PIXEL_COUNT: &str = "1";

/// A parsed histogram, reduced to what quantile lookups need.
pub(crate) struct Histogram {
    row0_min: f64,
    bin_size: f64,
    /// `cum[k]..cum[k + 1]` is the quantile range of bucket `k`: the share of
    /// all views that fall in lower buckets, up to the share in lower-or-equal
    /// buckets. Bucket 0 carries no weight, so `cum[0] == cum[1] == 0`. Length
    /// is the number of buckets plus one.
    cum: Box<[f64]>,
}

impl Histogram {
    /// Parses the `GDAL_METADATA` XML written by the OSMViews pipeline.
    pub(crate) fn parse(xml: &str) -> Result<Histogram, Error> {
        let (rat_attrs, rat_body) = children(xml, "GDALRasterAttributeTable")?
            .into_iter()
            .next()
            .ok_or(err("GDAL_METADATA has no raster attribute table"))?;

        let row0_min = attr(rat_attrs, "Row0Min")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or(err("histogram lacks a valid Row0Min"))?;
        let bin_size = attr(rat_attrs, "BinSize")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or(err("histogram lacks a valid BinSize"))?;

        let fields = children(rat_body, "FieldDefn")?;
        let mut count_column = None;
        for (i, (_, defn)) in fields.iter().enumerate() {
            let usage = children(defn, "Usage")?;
            if usage.first().map(|(_, text)| text.trim()) == Some(USAGE_PIXEL_COUNT) {
                if count_column.is_some() {
                    return Err(err("histogram has more than one pixel-count column"));
                }
                count_column = Some(i);
            }
        }
        let count_column = count_column.ok_or(err("histogram has no pixel-count column"))?;

        let rows = children(rat_body, "Row")?;
        if rows.is_empty() {
            return Err(err("histogram has no rows"));
        }
        let mut counts = Vec::with_capacity(rows.len());
        for (i, (row_attrs, row_body)) in rows.iter().enumerate() {
            if attr(row_attrs, "index").and_then(|v| v.parse::<usize>().ok()) != Some(i) {
                return Err(err("histogram rows are not numbered 0, 1, 2, …"));
            }
            let cells = children(row_body, "F")?;
            if cells.len() != fields.len() {
                return Err(err("histogram row has the wrong number of fields"));
            }
            let count = cells[count_column]
                .1
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|c| c.is_finite() && *c >= 0.0 && c.fract() == 0.0)
                .ok_or(err("histogram pixel count is not a non-negative integer"))?;
            counts.push(count);
        }

        if counts.iter().sum::<f64>() <= 0.0 {
            return Err(err("histogram counts no pixels"));
        }

        // Weight each bucket by the views it holds: its pixel count times the
        // mean view density of a pixel in it. Pixel values are
        // `ln(1 + density)`; taking them as uniform within a bucket, the mean
        // density over `[a, b]` is `(views(b) - views(a)) / (b - a)`, with
        // `views` from [`Histogram::views`]. Bucket 0 is left out: its
        // density rounds to zero, and most of its pixels are exactly zero.
        let mut cum = Vec::with_capacity(counts.len() + 1);
        cum.push(0.0);
        cum.push(0.0);
        let mut below = 0.0;
        for (k, &count) in counts.iter().enumerate().skip(1) {
            let (a, b) = Self::bounds(row0_min, bin_size, k);
            below += count * (Self::views(b) - Self::views(a)) / bin_size;
            cum.push(below);
        }
        if !below.is_finite() {
            return Err(err("histogram values are out of range"));
        }
        if below > 0.0 {
            for c in &mut cum {
                *c /= below;
            }
        }

        Ok(Histogram {
            row0_min,
            bin_size,
            cum: cum.into_boxed_slice(),
        })
    }

    /// Value range `[a, b)` of bucket `k`, in `ln(1 + density)`.
    fn bounds(row0_min: f64, bin_size: f64, k: usize) -> (f64, f64) {
        let a = row0_min + k as f64 * bin_size;
        (a, a + bin_size)
    }

    /// Antiderivative of `exp(v) - 1`, the view density at pixel value `v`.
    fn views(v: f64) -> f64 {
        v.exp_m1() - v
    }

    /// Bucket of `value`, as the pipeline assigns it, together with the
    /// value's position inside that bucket (`0.0..=1.0`). Non-finite and
    /// below-range values land in bucket 0; values past the last bucket are
    /// clamped to the top of the last bucket.
    fn locate(&self, value: f64) -> (usize, f64) {
        let pos = (value - self.row0_min) / self.bin_size;
        if pos.is_nan() || pos < 1.0 {
            return (0, 0.0);
        }
        let last = self.cum.len() - 2;
        let bucket = pos.floor();
        if bucket >= last as f64 {
            return (last, (pos - last as f64).min(1.0));
        }
        (bucket as usize, pos - bucket)
    }

    /// View-weighted quantile range of the bucket that `value` falls in.
    pub(crate) fn range(&self, value: f64) -> Range<f64> {
        let (bucket, _) = self.locate(value);
        self.cum[bucket]..self.cum[bucket + 1]
    }

    /// Like [`Histogram::range`], but a single number: the share of views
    /// below `value`, taking pixel values as uniform within the bucket.
    pub(crate) fn interpolate(&self, value: f64) -> f64 {
        let (bucket, t) = self.locate(value);
        let (lo, hi) = (self.cum[bucket], self.cum[bucket + 1]);
        if bucket == 0 || hi <= lo {
            return lo;
        }
        let (a, b) = Self::bounds(self.row0_min, self.bin_size, bucket);
        let v = a + t * self.bin_size;
        let within = (Self::views(v) - Self::views(a)) / (Self::views(b) - Self::views(a));
        lo + within.clamp(0.0, 1.0) * (hi - lo)
    }
}

fn err(msg: &'static str) -> Error {
    Error::Format(msg)
}

/// Value of attribute `name` in the attribute text of an XML start tag.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    loop {
        let i = rest.find(name)?;
        let before = rest[..i].chars().next_back();
        let after = &rest[i + name.len()..];
        if before.is_some_and(char::is_whitespace) {
            if let Some(quoted) = after.trim_start().strip_prefix('=') {
                let quoted = quoted.trim_start();
                let quote = quoted.chars().next().filter(|c| *c == '"' || *c == '\'')?;
                let value = &quoted[1..];
                return value.find(quote).map(|end| &value[..end]);
            }
        }
        rest = after;
    }
}

/// All elements called `name` in `xml`, in document order, as pairs of
/// (attribute text, inner text). Elements of the same name must not nest,
/// which holds for everything we read.
fn children<'a>(xml: &'a str, name: &str) -> Result<Vec<(&'a str, &'a str)>, Error> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        if !after.starts_with(|c: char| c == '>' || c == '/' || c.is_whitespace()) {
            // A longer name with the same prefix, such as <FieldDefn> for <F>.
            rest = after;
            continue;
        }
        let gt = after.find('>').ok_or(err("malformed GDAL_METADATA"))?;
        let attrs = &after[..gt];
        let body = &after[gt + 1..];
        if let Some(attrs) = attrs.strip_suffix('/') {
            out.push((attrs, ""));
            rest = body;
            continue;
        }
        let end = body.find(&close).ok_or(err("malformed GDAL_METADATA"))?;
        out.push((attrs, &body[..end]));
        rest = &body[end + close.len()..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xml(rows: &[u64]) -> String {
        let mut s = String::from(
            r#"<GDALMetadata><Item name="DEFAULT_RASTER_ATTRIBUTE_TABLE" sample="0" role="rat"><GDALRasterAttributeTable Row0Min="0" BinSize="0.0625" tableType="athematic"><FieldDefn index="0"><Name>min</Name><Type>1</Type><Usage>3</Usage></FieldDefn><FieldDefn index="1"><Name>max</Name><Type>1</Type><Usage>4</Usage></FieldDefn><FieldDefn index="2"><Name>count</Name><Type>1</Type><Usage>1</Usage></FieldDefn>"#,
        );
        for (i, c) in rows.iter().enumerate() {
            s.push_str(&format!(
                r#"<Row index="{i}"><F>{}</F><F>{}</F><F>{c}</F></Row>"#,
                i as f64 / 16.0,
                (i + 1) as f64 / 16.0
            ));
        }
        s.push_str("</GDALRasterAttributeTable></Item></GDALMetadata>");
        s
    }

    /// Views held by `count` pixels spread evenly over bucket `k`, by
    /// numerical integration: independent of the closed form in `parse`.
    fn bucket_views(k: usize, count: f64) -> f64 {
        let steps = 10_000;
        let a = k as f64 / 16.0;
        let sum: f64 = (0..steps)
            .map(|i| (a + (i as f64 + 0.5) / (16.0 * steps as f64)).exp_m1())
            .sum();
        count * sum / steps as f64
    }

    fn assert_close(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
    }

    #[test]
    fn parses_the_pipeline_format() {
        // 4e9 exceeds i32, as bucket 0 does in the real file. Bucket 0 carries
        // no weight however large it is.
        let h = Histogram::parse(&xml(&[4_000_000_000, 1, 3])).unwrap();
        let (v1, v2) = (bucket_views(1, 1.0), bucket_views(2, 3.0));
        assert_eq!(h.range(0.0), 0.0..0.0);
        let r = h.range(0.0625);
        assert_close(r.start, 0.0);
        assert_close(r.end, v1 / (v1 + v2));
        let r = h.range(0.125);
        assert_close(r.start, v1 / (v1 + v2));
        assert_close(r.end, 1.0);
    }

    #[test]
    fn weights_buckets_by_views() {
        // Equal pixel counts, but the higher bucket holds more views.
        let h = Histogram::parse(&xml(&[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 5,
        ]))
        .unwrap();
        let (lo, hi) = (bucket_views(16, 5.0), bucket_views(18, 5.0));
        let r = h.range(1.0);
        assert_close(r.end, lo / (lo + hi));
        assert!(r.end < 0.5);
        assert_eq!(h.range(1.0625), h.range(1.1));
        assert_close(h.range(1.0625).start, r.end);
        assert_close(h.range(1.0625).end, r.end);
    }

    #[test]
    fn locates_buckets_like_the_pipeline() {
        let h = Histogram::parse(&xml(&[10, 1, 1, 2])).unwrap();
        assert_eq!(h.locate(f64::NAN), (0, 0.0));
        assert_eq!(h.locate(-3.0), (0, 0.0));
        assert_eq!(h.locate(0.0624), (0, 0.0));
        assert_eq!(h.locate(0.0625), (1, 0.0));
        assert_eq!(h.locate(0.09375), (1, 0.5));
        assert_eq!(h.locate(0.1875), (3, 0.0));
        assert_eq!(h.locate(0.21875), (3, 0.5));
        // Past the last bucket: clamped to its top.
        assert_eq!(h.locate(5.0), (3, 1.0));
        assert_eq!(h.locate(f64::INFINITY), (3, 1.0));
    }

    #[test]
    fn interpolates_by_views_within_a_bucket() {
        let h = Histogram::parse(&xml(&[10, 1, 1, 2])).unwrap();
        let v = [
            0.0,
            bucket_views(1, 1.0),
            bucket_views(2, 1.0),
            bucket_views(3, 2.0),
        ];
        let total: f64 = v.iter().sum();
        assert_eq!(h.interpolate(0.0), 0.0);
        assert_eq!(h.interpolate(0.03), 0.0);
        assert_close(h.interpolate(0.0625), 0.0);
        assert_close(h.interpolate(0.125), v[1] / total);
        assert_close(h.interpolate(100.0), 1.0);

        // Halfway through bucket 1 in value is under halfway in views,
        // because density grows with the value. Near zero, density is about
        // linear in the value, so the lower half holds about
        // (1.5² - 1²) / (2² - 1²) = 0.417 of the bucket's views.
        let mid = h.interpolate(0.09375) / (v[1] / total);
        assert!((mid - 0.417).abs() < 0.01, "{mid}");

        // Monotonic, and inside the bucket's range.
        let mut last = 0.0;
        for i in 0..=400 {
            let value = f64::from(i) / 1600.0;
            let q = h.interpolate(value);
            let r = h.range(value);
            assert!(q >= last && r.start <= q && q <= r.end, "at {value}");
            last = q;
        }
    }

    #[test]
    fn only_bucket_zero_gives_empty_ranges() {
        let h = Histogram::parse(&xml(&[7])).unwrap();
        assert_eq!(h.range(3.0), 0.0..0.0);
        assert_eq!(h.interpolate(3.0), 0.0);
        let h = Histogram::parse(&xml(&[7, 0, 0])).unwrap();
        assert_eq!(h.range(0.1), 0.0..0.0);
        assert_eq!(h.interpolate(0.1), 0.0);
    }

    #[test]
    fn finds_the_count_column_by_usage() {
        let s = r#"<GDALRasterAttributeTable BinSize="1" Row0Min="0"><FieldDefn index="0"><Name>n</Name><Type>1</Type><Usage>1</Usage></FieldDefn><FieldDefn index="1"><Name>min</Name><Type>1</Type><Usage>3</Usage></FieldDefn><Row index="0"><F>2</F><F>0</F></Row><Row index="1"><F>6</F><F>1</F></Row><Row index="2"><F>0</F><F>2</F></Row></GDALRasterAttributeTable>"#;
        let h = Histogram::parse(s).unwrap();
        assert_eq!(h.range(1.5), 0.0..1.0);
        assert_eq!(h.range(2.5), 1.0..1.0);
    }

    #[test]
    fn rejects_malformed_tables() {
        let good = xml(&[1, 2]);
        for bad in [
            String::new(),
            "<GDALMetadata></GDALMetadata>".to_string(),
            good.replace(r#"BinSize="0.0625""#, ""),
            good.replace(r#"BinSize="0.0625""#, r#"BinSize="0""#),
            good.replace(r#"Row0Min="0""#, r#"Row0Min="nan""#),
            good.replace("<Usage>1</Usage>", "<Usage>0</Usage>"),
            good.replace("<Usage>3</Usage>", "<Usage>1</Usage>"),
            good.replace(r#"<Row index="1">"#, r#"<Row index="7">"#),
            good.replace("<F>2</F>", "<F>-2</F>"),
            good.replace("<F>2</F>", "<F>2.5</F>"),
            good.replace("<F>2</F>", ""),
            good.replace("</GDALRasterAttributeTable>", ""),
            xml(&[]),
            xml(&[0, 0]),
        ] {
            assert!(Histogram::parse(&bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn reads_attributes() {
        let a = r#" Row0Min="0" BinSize = '0.5' xBinSize="9""#;
        assert_eq!(attr(a, "BinSize"), Some("0.5"));
        assert_eq!(attr(a, "Row0Min"), Some("0"));
        assert_eq!(attr(a, "Missing"), None);
        assert_eq!(attr(r#" xBinSize="9""#, "BinSize"), None);
    }
}
