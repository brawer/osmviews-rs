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
    /// `cum[k]..cum[k + 1]` is the quantile range of bucket `k` among the
    /// pixels above bucket 0. `cum[0] == cum[1] == 0`, so bucket 0 maps to the
    /// empty range at zero. Length is the number of buckets plus one.
    cum: Box<[f64]>,
    /// Share of all pixels that fall in bucket 0.
    zero_fraction: f64,
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

        // Pixel totals stay far below 2^53, so these f64 sums are exact.
        let total: f64 = counts.iter().sum();
        if total <= 0.0 {
            return Err(err("histogram counts no pixels"));
        }
        let nonzero = total - counts[0];
        let mut cum = Vec::with_capacity(counts.len() + 1);
        cum.push(0.0);
        cum.push(0.0);
        let mut below = 0.0;
        for &count in &counts[1..] {
            below += count;
            cum.push(if nonzero > 0.0 { below / nonzero } else { 0.0 });
        }

        Ok(Histogram {
            row0_min,
            bin_size,
            cum: cum.into_boxed_slice(),
            zero_fraction: counts[0] / total,
        })
    }

    /// Share of all pixels that fall in bucket 0.
    pub(crate) fn zero_fraction(&self) -> f64 {
        self.zero_fraction
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

    /// Quantile range of `value` among the pixels above bucket 0.
    pub(crate) fn range(&self, value: f64) -> Range<f64> {
        let (bucket, _) = self.locate(value);
        self.cum[bucket]..self.cum[bucket + 1]
    }

    /// Like [`Histogram::range`], but a single number interpolated linearly
    /// within the bucket.
    pub(crate) fn interpolate(&self, value: f64) -> f64 {
        let (bucket, t) = self.locate(value);
        let (lo, hi) = (self.cum[bucket], self.cum[bucket + 1]);
        lo + t * (hi - lo)
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

    #[test]
    fn parses_the_pipeline_format() {
        // 4e9 exceeds i32, as bucket 0 does in the real file.
        let h = Histogram::parse(&xml(&[4_000_000_000, 1, 3])).unwrap();
        assert!((h.zero_fraction() - 4e9 / (4e9 + 4.0)).abs() < 1e-12);
        assert_eq!(h.range(0.0), 0.0..0.0);
        assert_eq!(h.range(0.0625), 0.0..0.25);
        assert_eq!(h.range(0.125), 0.25..1.0);
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
    fn interpolates_within_a_bucket() {
        let h = Histogram::parse(&xml(&[10, 1, 1, 2])).unwrap();
        assert_eq!(h.interpolate(0.0), 0.0);
        assert_eq!(h.interpolate(0.0625), 0.0);
        assert_eq!(h.interpolate(0.09375), 0.125);
        assert_eq!(h.interpolate(0.21875), 0.75);
        assert_eq!(h.interpolate(100.0), 1.0);
    }

    #[test]
    fn only_bucket_zero_gives_empty_ranges() {
        let h = Histogram::parse(&xml(&[7])).unwrap();
        assert_eq!(h.zero_fraction(), 1.0);
        assert_eq!(h.range(3.0), 0.0..0.0);
        assert_eq!(h.interpolate(3.0), 0.0);
    }

    #[test]
    fn finds_the_count_column_by_usage() {
        let s = r#"<GDALRasterAttributeTable BinSize="1" Row0Min="0"><FieldDefn index="0"><Name>n</Name><Type>1</Type><Usage>1</Usage></FieldDefn><FieldDefn index="1"><Name>min</Name><Type>1</Type><Usage>3</Usage></FieldDefn><Row index="0"><F>2</F><F>0</F></Row><Row index="1"><F>6</F><F>1</F></Row></GDALRasterAttributeTable>"#;
        let h = Histogram::parse(s).unwrap();
        assert_eq!(h.zero_fraction(), 0.25);
        assert_eq!(h.range(1.5), 0.0..1.0);
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
