// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! A deliberately tiny, read-only parser for the one TIFF layout that the
//! OSMViews pipeline produces.
//!
//! We only look at IFD 0 (the full-resolution level; the file also carries
//! reduced-resolution pyramid levels in later IFDs, which we never need). We
//! reject anything that does not match the expected shape rather than trying to
//! be a general TIFF reader.

use crate::Error;

/// Byte offset of a tile’s compressed data within the file.
///
/// This is the key of the decoded-tile cache. The OSMViews GeoTIFF is sparse:
/// large uniform areas (oceans, deserts) are encoded once and referenced from
/// many tile-grid positions, so keying the cache by offset collapses all those
/// positions onto a single entry.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TileOffset(pub(crate) u64);

const TAG_IMAGE_WIDTH: u16 = 256;
const TAG_IMAGE_LENGTH: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_PLANAR_CONFIG: u16 = 284;
const TAG_PREDICTOR: u16 = 317;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_LENGTH: u16 = 323;
const TAG_TILE_OFFSETS: u16 = 324;
const TAG_TILE_BYTE_COUNTS: u16 = 325;
const TAG_SAMPLE_FORMAT: u16 = 339;
const TAG_MAX_SAMPLE_VALUE: u16 = 341;

const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_FLOAT: u16 = 11;
const TYPE_DOUBLE: u16 = 12;

const TILE_SIDE: u32 = 256;

/// Everything `rank()` needs from the file header.
pub(crate) struct Header {
    /// Raster width in pixels (equal to the height).
    pub size: u32,
    /// Number of tiles along one axis (`size / 256`).
    pub tiles_across: u32,
    /// TIFF compression tag: `1` (none) or `8` (zlib DEFLATE).
    pub compression: u16,
    /// Highest sample value anywhere in the raster (`SMaxSampleValue`).
    pub max_value: f64,
    pub tile_offsets: TileTable,
    pub tile_byte_counts: TileTable,
}

#[derive(Copy, Clone)]
enum Elem {
    Short,
    Long,
}

impl Elem {
    fn size(self) -> usize {
        match self {
            Elem::Short => 2,
            Elem::Long => 4,
        }
    }

    fn from_type(t: u16) -> Option<Elem> {
        match t {
            TYPE_SHORT => Some(Elem::Short),
            TYPE_LONG => Some(Elem::Long),
            _ => None,
        }
    }
}

/// A `TileOffsets` or `TileByteCounts` array, kept as a position into the
/// memory-mapped file (never copied out).
pub(crate) struct TileTable {
    pos: usize,
    elem: Elem,
}

impl TileTable {
    /// Reads the `i`-th entry. `i` must be within the tile grid and the array’s
    /// extent was bounds-checked against the file in [`parse`].
    pub(crate) fn get(&self, data: &[u8], i: usize) -> u64 {
        let at = self.pos + i * self.elem.size();
        match self.elem {
            Elem::Short => u64::from(u16::from_le_bytes([data[at], data[at + 1]])),
            Elem::Long => u64::from(u32::from_le_bytes([
                data[at],
                data[at + 1],
                data[at + 2],
                data[at + 3],
            ])),
        }
    }
}

/// Raw form of one IFD entry, before we know which tag we care about.
struct Entry {
    typ: u16,
    count: u32,
    /// The 4-byte value/offset field, verbatim.
    value: [u8; 4],
}

impl Entry {
    /// Interprets the entry as a single SHORT or LONG stored inline.
    fn scalar_u32(&self) -> Option<u32> {
        if self.count != 1 {
            return None;
        }
        match self.typ {
            TYPE_SHORT => Some(u32::from(u16::from_le_bytes([
                self.value[0],
                self.value[1],
            ]))),
            TYPE_LONG => Some(u32::from_le_bytes(self.value)),
            _ => None,
        }
    }

    /// Interprets the entry as a single FLOAT (inline) or DOUBLE (out of line).
    fn scalar_f64(&self, data: &[u8]) -> Option<f64> {
        if self.count != 1 {
            return None;
        }
        match self.typ {
            TYPE_FLOAT => Some(f64::from(f32::from_le_bytes(self.value))),
            TYPE_DOUBLE => {
                let at = u32::from_le_bytes(self.value) as usize;
                let bytes = data.get(at..at + 8)?;
                Some(f64::from_le_bytes(bytes.try_into().ok()?))
            }
            _ => None,
        }
    }
}

fn err(msg: &'static str) -> Error {
    Error::Format(msg)
}

fn read_u16(data: &[u8], at: usize) -> Result<u16, Error> {
    let bytes = data.get(at..at + 2).ok_or(err("truncated file"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], at: usize) -> Result<u32, Error> {
    let bytes = data.get(at..at + 4).ok_or(err("truncated file"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Parses and validates the header of an OSMViews GeoTIFF held in `data`
/// (typically a memory map of the whole file).
pub(crate) fn parse(data: &[u8]) -> Result<Header, Error> {
    if data.len() < 8 {
        return Err(err("not a TIFF (file too short)"));
    }
    match &data[0..2] {
        b"II" => {}
        b"MM" => return Err(err("big-endian TIFF is not supported")),
        _ => return Err(err("not a TIFF")),
    }
    match read_u16(data, 2)? {
        42 => {}
        43 => return Err(err("BigTIFF is not supported")),
        _ => return Err(err("not a TIFF (unrecognized version)")),
    }

    let ifd = read_u32(data, 4)? as usize;
    let n = read_u16(data, ifd)? as usize;
    // Entry table plus the trailing 4-byte "next IFD" pointer.
    let entries_end = ifd
        .checked_add(2)
        .and_then(|v| v.checked_add(n.checked_mul(12)?))
        .and_then(|v| v.checked_add(4))
        .ok_or(err("bad IFD"))?;
    if entries_end > data.len() {
        return Err(err("truncated IFD"));
    }

    let mut width = None;
    let mut length = None;
    let mut bits = None;
    let mut compression = None;
    let mut samples = None;
    let mut planar = None;
    let mut predictor = None;
    let mut tile_w = None;
    let mut tile_h = None;
    let mut sample_format = None;
    let mut max_value = None;
    let mut tile_offsets = None;
    let mut tile_byte_counts = None;

    for i in 0..n {
        let at = ifd + 2 + i * 12;
        let tag = u16::from_le_bytes([data[at], data[at + 1]]);
        let entry = Entry {
            typ: u16::from_le_bytes([data[at + 2], data[at + 3]]),
            count: u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]),
            value: [data[at + 8], data[at + 9], data[at + 10], data[at + 11]],
        };
        match tag {
            TAG_IMAGE_WIDTH => width = entry.scalar_u32(),
            TAG_IMAGE_LENGTH => length = entry.scalar_u32(),
            TAG_BITS_PER_SAMPLE => bits = entry.scalar_u32(),
            TAG_COMPRESSION => compression = entry.scalar_u32(),
            TAG_SAMPLES_PER_PIXEL => samples = entry.scalar_u32(),
            TAG_PLANAR_CONFIG => planar = entry.scalar_u32(),
            TAG_PREDICTOR => predictor = entry.scalar_u32(),
            TAG_TILE_WIDTH => tile_w = entry.scalar_u32(),
            TAG_TILE_LENGTH => tile_h = entry.scalar_u32(),
            TAG_SAMPLE_FORMAT => sample_format = entry.scalar_u32(),
            TAG_MAX_SAMPLE_VALUE => max_value = entry.scalar_f64(data),
            TAG_TILE_OFFSETS => tile_offsets = Some(entry),
            TAG_TILE_BYTE_COUNTS => tile_byte_counts = Some(entry),
            _ => {}
        }
    }

    let size = match (width, length) {
        (Some(w), Some(h)) if w == h => w,
        (Some(_), Some(_)) => return Err(err("raster is not square")),
        _ => return Err(err("missing image dimensions")),
    };
    if size < TILE_SIDE || !size.is_power_of_two() {
        return Err(err("unexpected raster size"));
    }
    if tile_w != Some(TILE_SIDE) || tile_h != Some(TILE_SIDE) {
        return Err(err("unexpected tile size"));
    }
    if bits != Some(32) || sample_format != Some(3) {
        return Err(err("samples are not 32-bit float"));
    }
    if samples != Some(1) {
        return Err(err("expected a single sample per pixel"));
    }
    if planar != Some(1) {
        return Err(err("unexpected planar configuration"));
    }
    let compression = match compression {
        Some(c @ (1 | 8)) => c as u16,
        _ => return Err(err("unsupported compression")),
    };
    if !matches!(predictor, None | Some(1)) {
        return Err(err("TIFF predictor is not supported"));
    }
    let max_value = match max_value {
        Some(v) if v.is_finite() => v,
        Some(_) => return Err(err("SMaxSampleValue is not finite")),
        None => return Err(err("missing SMaxSampleValue")),
    };

    let tiles_across = size / TILE_SIDE;
    let grid = (tiles_across as usize)
        .checked_mul(tiles_across as usize)
        .filter(|g| *g <= u32::MAX as usize)
        .ok_or(err("raster has too many tiles"))?;

    let tile_offsets = tile_table(data, tile_offsets.ok_or(err("missing TileOffsets"))?, grid)?;
    let tile_byte_counts = tile_table(
        data,
        tile_byte_counts.ok_or(err("missing TileByteCounts"))?,
        grid,
    )?;

    // One sequential pass so that a corrupt file is rejected here rather than
    // making `rank()` fallible. Touched pages can be evicted afterwards.
    for i in 0..grid {
        let off = tile_offsets.get(data, i) as usize;
        let len = tile_byte_counts.get(data, i) as usize;
        if len == 0 || off < 8 {
            return Err(err("invalid tile entry"));
        }
        if off.checked_add(len).is_none_or(|end| end > data.len()) {
            return Err(err("tile data extends past end of file"));
        }
    }

    Ok(Header {
        size,
        tiles_across,
        compression,
        max_value,
        tile_offsets,
        tile_byte_counts,
    })
}

fn tile_table(data: &[u8], entry: Entry, grid: usize) -> Result<TileTable, Error> {
    let elem = Elem::from_type(entry.typ).ok_or(err("unexpected tile-table element type"))?;
    if entry.count as usize != grid {
        return Err(err("tile-table length does not match the tile grid"));
    }
    let total = grid
        .checked_mul(elem.size())
        .ok_or(err("tile table too large"))?;
    if total <= 4 {
        // Would be stored inline in the entry; the OSMViews pipeline never emits
        // a single-tile image, so we don't implement that path.
        return Err(err("inline tile table not supported"));
    }
    let pos = u32::from_le_bytes(entry.value) as usize;
    if pos.checked_add(total).is_none_or(|end| end > data.len()) {
        return Err(err("tile table extends past end of file"));
    }
    Ok(TileTable { pos, elem })
}
