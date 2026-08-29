// SPDX-FileCopyrightText: 2026 Sascha Brawer <sascha@brawer.ch>
// SPDX-License-Identifier: MIT

//! A small LRU cache of decoded tiles, plus the diagnostic counters.
//!
//! Both live behind the single `Mutex` that `rank()` already takes for every
//! lookup, so the counters need no atomics and add nothing to the hot path
//! beyond a few integer increments under a lock that is held anyway.

use std::collections::HashMap;
use std::time::Duration;

use crate::Metrics;
use crate::tiff::TileOffset;

struct Entry {
    data: Box<[f32]>,
    /// Value of `tick` at the most recent access; smallest = least recently used.
    used: u64,
}

pub(crate) struct Cache {
    capacity: usize,
    tick: u64,
    entries: HashMap<TileOffset, Entry>,

    queries: u64,
    out_of_range: u64,
    hits: u64,
    misses: u64,
    tiles_decoded: u64,
    evictions: u64,
    decode_nanos: u64,
}

impl Cache {
    pub(crate) fn new(capacity: usize) -> Cache {
        Cache {
            capacity,
            tick: 0,
            entries: HashMap::new(),
            queries: 0,
            out_of_range: 0,
            hits: 0,
            misses: 0,
            tiles_decoded: 0,
            evictions: 0,
            decode_nanos: 0,
        }
    }

    /// Records a `rank()` call whose coordinates fell outside the covered area.
    pub(crate) fn record_out_of_range(&mut self) {
        self.queries += 1;
        self.out_of_range += 1;
    }

    /// Records a `rank()` call and returns the requested pixel if its tile is
    /// cached.
    pub(crate) fn lookup(&mut self, off: TileOffset, pixel: usize) -> Option<f32> {
        self.queries += 1;
        match self.entries.get_mut(&off) {
            Some(entry) => {
                self.tick += 1;
                entry.used = self.tick;
                self.hits += 1;
                entry.data.get(pixel).copied()
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Inserts a freshly decoded tile, evicting the least recently used entry if
    /// the cache is full.
    pub(crate) fn insert(&mut self, off: TileOffset, data: Box<[f32]>, decode: Duration) {
        self.tiles_decoded += 1;
        self.decode_nanos = self
            .decode_nanos
            .saturating_add(u64::try_from(decode.as_nanos()).unwrap_or(u64::MAX));

        if self.capacity == 0 {
            return;
        }
        if !self.entries.contains_key(&off) && self.entries.len() >= self.capacity {
            if let Some(&victim) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k)
            {
                self.entries.remove(&victim);
                self.evictions += 1;
            }
        }
        self.tick += 1;
        self.entries.insert(
            off,
            Entry {
                data,
                used: self.tick,
            },
        );
    }

    pub(crate) fn metrics(&self) -> Metrics {
        Metrics {
            queries: self.queries,
            out_of_range: self.out_of_range,
            tile_cache_hits: self.hits,
            tile_cache_misses: self.misses,
            tiles_decoded: self.tiles_decoded,
            tile_cache_evictions: self.evictions,
            tiles_cached: self.entries.len(),
            tile_cache_capacity: self.capacity,
            decode_time: Duration::from_nanos(self.decode_nanos),
        }
    }
}
