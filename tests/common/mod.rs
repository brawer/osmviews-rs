// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! Helpers shared by the integration tests: a builder for minimal-but-real
//! OSMViews-shaped GeoTIFFs, and a self-deleting temp file.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of IFD entries [`build_tiff`] writes. Exposed so tests can locate the
/// out-of-line arrays that follow the IFD.
pub const ENTRY_COUNT: usize = 13;

/// File offset of the `TileOffsets` array in a [`build_tiff`] file.
pub const TILE_OFFSETS_POS: usize = 8 + 2 + ENTRY_COUNT * 12 + 4;
/// File offset of the `TileByteCounts` array in a [`build_tiff`] file.
pub const TILE_BYTE_COUNTS_POS: usize = TILE_OFFSETS_POS + 16;

const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_FLOAT: u16 = 11;

const TILE: usize = 256 * 256;

/// A decoded tile is exactly `TILE` 32-bit floats = 256 KiB. Matches
/// `tiff::TILE_BYTES`.
pub const TILE_BYTES: usize = TILE * 4;

fn ifd_entry(tag: u16, typ: u16, count: u32, value: [u8; 4]) -> [u8; 12] {
    let mut e = [0u8; 12];
    e[0..2].copy_from_slice(&tag.to_le_bytes());
    e[2..4].copy_from_slice(&typ.to_le_bytes());
    e[4..8].copy_from_slice(&count.to_le_bytes());
    e[8..12].copy_from_slice(&value);
    e
}

fn short(v: u16) -> [u8; 4] {
    let b = v.to_le_bytes();
    [b[0], b[1], 0, 0]
}

/// Builds a 512×512 single-level GeoTIFF laid out like the real OSMViews file:
/// four 256×256 tiles, out-of-line `TileOffsets`/`TileByteCounts`, 32-bit float
/// samples, `SMaxSampleValue = max_value`.
///
/// `tile_values[g]` is the uniform value of grid tile `g` (order: top-left,
/// top-right, bottom-left, bottom-right). Tiles with an equal value share one
/// compressed blob and therefore one file offset, exercising the dedup cache.
///
/// `compression` is `8` (zlib) or `1` (none).
///
/// The `GDAL_METADATA` tag carries the histogram the pipeline would write for
/// these tiles (see [`histogram_of`]).
pub fn build_tiff(compression: u16, tile_values: [f32; 4], max_value: f32) -> Vec<u8> {
    let xml = histogram_xml(&histogram_of(tile_values));
    build_tiff_with_metadata(compression, tile_values, max_value, Some(&xml))
}

/// Pixel counts per histogram bucket for a [`build_tiff`] raster, bucketed the
/// way the pipeline does it: bucket `floor(v * 16)` for `v > 0`, else 0.
pub fn histogram_of(tile_values: [f32; 4]) -> Vec<u64> {
    let mut counts = vec![0u64];
    for v in tile_values {
        let bucket = if v > 0.0 {
            (f64::from(v) / 0.0625) as usize
        } else {
            0
        };
        if bucket >= counts.len() {
            counts.resize(bucket + 1, 0);
        }
        counts[bucket] += TILE as u64;
    }
    counts
}

/// The `GDAL_METADATA` XML that the OSMViews pipeline writes for a histogram
/// with these bucket counts.
pub fn histogram_xml(counts: &[u64]) -> String {
    let mut s = String::from(
        r#"<GDALMetadata><Item name="DEFAULT_RASTER_ATTRIBUTE_TABLE" sample="0" role="rat"><GDALRasterAttributeTable Row0Min="0" BinSize="0.0625" tableType="athematic"><FieldDefn index="0"><Name>min</Name><Type>1</Type><Usage>3</Usage></FieldDefn><FieldDefn index="1"><Name>max</Name><Type>1</Type><Usage>4</Usage></FieldDefn><FieldDefn index="2"><Name>count</Name><Type>1</Type><Usage>1</Usage></FieldDefn>"#,
    );
    for (i, count) in counts.iter().enumerate() {
        s.push_str(&format!(
            r#"<Row index="{i}"><F>{}</F><F>{}</F><F>{count}</F></Row>"#,
            i as f64 * 0.0625,
            (i + 1) as f64 * 0.0625,
        ));
    }
    s.push_str("</GDALRasterAttributeTable></Item></GDALMetadata>");
    s
}

/// Like [`build_tiff`], but with a caller-supplied `GDAL_METADATA` string, or
/// none at all.
pub fn build_tiff_with_metadata(
    compression: u16,
    tile_values: [f32; 4],
    max_value: f32,
    gdal_metadata: Option<&str>,
) -> Vec<u8> {
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    let mut blob_bits: Vec<u32> = Vec::new();
    let mut grid_to_blob = [0usize; 4];
    for (g, &value) in tile_values.iter().enumerate() {
        let idx = blob_bits
            .iter()
            .position(|b| *b == value.to_bits())
            .unwrap_or_else(|| {
                let raw: Vec<u8> = std::iter::repeat_n(value.to_le_bytes(), TILE)
                    .flatten()
                    .collect();
                let blob = match compression {
                    8 => miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
                    _ => raw,
                };
                blobs.push(blob);
                blob_bits.push(value.to_bits());
                blobs.len() - 1
            });
        grid_to_blob[g] = idx;
    }

    let mut blob_pos = Vec::with_capacity(blobs.len());
    let mut cursor = TILE_BYTE_COUNTS_POS + 16;
    for blob in &blobs {
        blob_pos.push(cursor as u32);
        cursor += blob.len();
    }
    let tile_offsets: [u32; 4] = std::array::from_fn(|g| blob_pos[grid_to_blob[g]]);
    let tile_byte_counts: [u32; 4] = std::array::from_fn(|g| blobs[grid_to_blob[g]].len() as u32);

    let blob_refs: Vec<&[u8]> = blobs.iter().map(Vec::as_slice).collect();
    assemble(
        compression,
        max_value,
        tile_offsets,
        tile_byte_counts,
        &blob_refs,
        gdal_metadata,
    )
}

/// Builds a [`build_tiff`]-shaped raster whose four grid tiles all point at one
/// caller-supplied stored blob. Lets a test hand the decode path a tile it would
/// never compress itself — an over-large blob, or a small zlib stream that
/// inflates past one tile.
pub fn build_tiff_single_blob(compression: u16, blob: &[u8], max_value: f32) -> Vec<u8> {
    let pos = (TILE_BYTE_COUNTS_POS + 16) as u32;
    assemble(
        compression,
        max_value,
        [pos; 4],
        [blob.len() as u32; 4],
        &[blob],
        Some(&histogram_xml(&[4 * TILE as u64])),
    )
}

fn assemble(
    compression: u16,
    max_value: f32,
    tile_offsets: [u32; 4],
    tile_byte_counts: [u32; 4],
    blobs: &[&[u8]],
    gdal_metadata: Option<&str>,
) -> Vec<u8> {
    // The metadata string, NUL-terminated, goes after the tile blobs.
    let metadata_pos = TILE_BYTE_COUNTS_POS + 16 + blobs.iter().map(|b| b.len()).sum::<usize>();
    let metadata = gdal_metadata.map(|m| {
        let mut bytes = m.as_bytes().to_vec();
        bytes.push(0);
        bytes
    });
    // Without metadata, keep the IFD the same size by writing an unknown
    // private tag, which readers must skip.
    let last_entry = match &metadata {
        Some(bytes) => ifd_entry(
            42112,
            TYPE_ASCII,
            bytes.len() as u32,
            (metadata_pos as u32).to_le_bytes(),
        ),
        None => ifd_entry(65000, TYPE_SHORT, 1, short(0)),
    };
    let entries = [
        ifd_entry(256, TYPE_LONG, 1, 512u32.to_le_bytes()),
        ifd_entry(257, TYPE_LONG, 1, 512u32.to_le_bytes()),
        ifd_entry(258, TYPE_SHORT, 1, short(32)),
        ifd_entry(259, TYPE_SHORT, 1, short(compression)),
        ifd_entry(277, TYPE_SHORT, 1, short(1)),
        ifd_entry(284, TYPE_SHORT, 1, short(1)),
        ifd_entry(322, TYPE_SHORT, 1, short(256)),
        ifd_entry(323, TYPE_SHORT, 1, short(256)),
        ifd_entry(324, TYPE_LONG, 4, (TILE_OFFSETS_POS as u32).to_le_bytes()),
        ifd_entry(
            325,
            TYPE_LONG,
            4,
            (TILE_BYTE_COUNTS_POS as u32).to_le_bytes(),
        ),
        ifd_entry(339, TYPE_SHORT, 1, short(3)),
        ifd_entry(341, TYPE_FLOAT, 1, max_value.to_le_bytes()),
        last_entry,
    ];
    assert_eq!(entries.len(), ENTRY_COUNT);

    let mut buf = Vec::new();
    buf.extend_from_slice(b"II");
    buf.extend_from_slice(&42u16.to_le_bytes());
    buf.extend_from_slice(&8u32.to_le_bytes());
    buf.extend_from_slice(&(ENTRY_COUNT as u16).to_le_bytes());
    for e in &entries {
        buf.extend_from_slice(e);
    }
    buf.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    assert_eq!(buf.len(), TILE_OFFSETS_POS);
    for v in tile_offsets {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in tile_byte_counts {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for blob in blobs {
        buf.extend_from_slice(blob);
    }
    assert_eq!(buf.len(), metadata_pos);
    if let Some(bytes) = metadata {
        buf.extend_from_slice(&bytes);
    }
    buf
}

/// A temp file that deletes itself on drop.
pub struct TempTiff {
    pub path: PathBuf,
}

impl TempTiff {
    pub fn new(bytes: &[u8]) -> TempTiff {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "osmviews-test-{}-{}.tiff",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).expect("write temp fixture");
        TempTiff { path }
    }
}

impl Drop for TempTiff {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
