// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! Helpers shared by the integration tests: a builder for minimal-but-real
//! OSMViews-shaped GeoTIFFs, and a self-deleting temp file.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of IFD entries [`build_tiff`] writes. Exposed so tests can locate the
/// out-of-line arrays that follow the IFD.
pub const ENTRY_COUNT: usize = 12;

/// File offset of the `TileOffsets` array in a [`build_tiff`] file.
pub const TILE_OFFSETS_POS: usize = 8 + 2 + ENTRY_COUNT * 12 + 4;
/// File offset of the `TileByteCounts` array in a [`build_tiff`] file.
pub const TILE_BYTE_COUNTS_POS: usize = TILE_OFFSETS_POS + 16;

const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_FLOAT: u16 = 11;

const TILE: usize = 256 * 256;

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
pub fn build_tiff(compression: u16, tile_values: [f32; 4], max_value: f32) -> Vec<u8> {
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
    ];
    assert_eq!(entries.len(), ENTRY_COUNT);

    let mut buf = Vec::with_capacity(cursor);
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
    for blob in &blobs {
        buf.extend_from_slice(blob);
    }
    assert_eq!(buf.len(), cursor);
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
