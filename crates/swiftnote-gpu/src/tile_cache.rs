use std::collections::{HashMap, VecDeque};

use swiftnote_core::{PageId, Rect};

/// Physical tile dimension in pixels (`512x512`).
pub const TILE_SIZE_PX: u32 = 512;

/// Default VRAM ceiling for cached page tiles (`256 MiB`).
pub const DEFAULT_VRAM_BUDGET_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub page_id: PageId,
    pub tile_x: u32,
    pub tile_y: u32,
    /// Quantized physical scale factor (`round(zoom * display_scale * 1000.0)`).
    pub scale_milli: u32,
}

impl TileKey {
    #[inline]
    pub fn scale_milli_from(zoom: f64, display_scale: f32) -> u32 {
        let s = (zoom * f64::from(display_scale)).clamp(0.05, 32.0);
        (s * 1000.0).round().max(1.0) as u32
    }

    #[inline]
    pub fn tile_page_rect(
        self,
        page_width_pt: f64,
        page_height_pt: f64,
    ) -> Option<(Rect, u32, u32)> {
        let scale = f64::from(self.scale_milli) / 1000.0;
        let tile_pt = f64::from(TILE_SIZE_PX) / scale;
        let min_x = f64::from(self.tile_x) * tile_pt;
        let min_y = f64::from(self.tile_y) * tile_pt;
        if min_x >= page_width_pt || min_y >= page_height_pt {
            return None;
        }
        let w_px = ((page_width_pt - min_x) * scale)
            .round()
            .clamp(1.0, f64::from(TILE_SIZE_PX)) as u32;
        let h_px = ((page_height_pt - min_y) * scale)
            .round()
            .clamp(1.0, f64::from(TILE_SIZE_PX)) as u32;
        let max_x = min_x + f64::from(w_px) / scale;
        let max_y = min_y + f64::from(h_px) / scale;
        Some((Rect::new(min_x, min_y, max_x, max_y), w_px, h_px))
    }
}

pub fn visible_tile_keys(
    page_id: PageId,
    page_w_pt: f64,
    page_h_pt: f64,
    vis_page_rect: Rect,
    scale_milli: u32,
) -> Vec<TileKey> {
    if vis_page_rect.is_empty() || page_w_pt <= 0.0 || page_h_pt <= 0.0 {
        return Vec::new();
    }
    let scale = f64::from(scale_milli) / 1000.0;
    let tile_pt = f64::from(TILE_SIZE_PX) / scale;
    let max_cols = ((page_w_pt / tile_pt).ceil() as u32).max(1);
    let max_rows = ((page_h_pt / tile_pt).ceil() as u32).max(1);

    let x0 = (vis_page_rect.min_x.max(0.0) / tile_pt).floor() as u32;
    let y0 = (vis_page_rect.min_y.max(0.0) / tile_pt).floor() as u32;
    let x1 = ((vis_page_rect.max_x.min(page_w_pt) / tile_pt).ceil() as u32)
        .saturating_sub(1)
        .min(max_cols - 1);
    let y1 = ((vis_page_rect.max_y.min(page_h_pt) / tile_pt).ceil() as u32)
        .saturating_sub(1)
        .min(max_rows - 1);

    if x0 > x1 || y0 > y1 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1)) as usize);
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            out.push(TileKey {
                page_id,
                tile_x: tx,
                tile_y: ty,
                scale_milli,
            });
        }
    }
    out
}

#[derive(Debug)]
struct CacheEntry<T> {
    payload: T,
    byte_size: usize,
    rendered_gen: u64,
}

/// Generation-aware bounded LRU cache preventing stale renders from clearing newer dirty flags.
#[derive(Debug)]
pub struct GpuTileCache<T> {
    budget_bytes: usize,
    total_bytes: usize,
    entries: HashMap<TileKey, CacheEntry<T>>,
    lru_order: VecDeque<TileKey>,
    dirty_gen: HashMap<TileKey, u64>,
    global_gen: u64,
}

impl<T> Default for GpuTileCache<T> {
    fn default() -> Self {
        Self::new(DEFAULT_VRAM_BUDGET_BYTES)
    }
}

impl<T> GpuTileCache<T> {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            budget_bytes: budget_bytes.max(1),
            total_bytes: 0,
            entries: HashMap::new(),
            lru_order: VecDeque::new(),
            dirty_gen: HashMap::new(),
            global_gen: 1,
        }
    }

    #[inline]
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn required_gen(&self, key: TileKey) -> u64 {
        self.dirty_gen.get(&key).copied().unwrap_or(0)
    }

    pub fn is_dirty(&self, key: TileKey) -> bool {
        let req = self.required_gen(key);
        match self.entries.get(&key) {
            None => true,
            Some(entry) => entry.rendered_gen < req,
        }
    }

    pub fn mark_dirty(&mut self, key: TileKey) -> u64 {
        self.global_gen = self.global_gen.saturating_add(1);
        self.dirty_gen.insert(key, self.global_gen);
        self.global_gen
    }

    pub fn invalidate_page(&mut self, page_id: PageId) {
        self.global_gen = self.global_gen.saturating_add(1);
        let keys: Vec<TileKey> = self
            .entries
            .keys()
            .copied()
            .filter(|k| k.page_id == page_id)
            .collect();
        for k in keys {
            self.dirty_gen.insert(k, self.global_gen);
        }
    }

    pub fn invalidate_rect(&mut self, page_id: PageId, rect_pt: Rect) {
        if rect_pt.is_empty() {
            return;
        }
        self.global_gen = self.global_gen.saturating_add(1);
        let keys: Vec<TileKey> = self
            .entries
            .keys()
            .copied()
            .filter(|k| {
                if k.page_id != page_id {
                    return false;
                }
                let scale = f64::from(k.scale_milli) / 1000.0;
                let tile_pt = f64::from(TILE_SIZE_PX) / scale;
                let min_x = f64::from(k.tile_x) * tile_pt;
                let min_y = f64::from(k.tile_y) * tile_pt;
                let tile_rect = Rect::new(min_x, min_y, min_x + tile_pt, min_y + tile_pt);
                tile_rect.intersects(rect_pt)
            })
            .collect();
        for k in keys {
            self.dirty_gen.insert(k, self.global_gen);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.lru_order.clear();
        self.dirty_gen.clear();
        self.total_bytes = 0;
    }

    pub fn get_mut(&mut self, key: TileKey) -> Option<&mut T> {
        if !self.entries.contains_key(&key) {
            return None;
        }
        self.touch_lru(key);
        self.entries.get_mut(&key).map(|e| &mut e.payload)
    }

    pub fn get(&self, key: TileKey) -> Option<&T> {
        self.entries.get(&key).map(|e| &e.payload)
    }

    /// Inserts or replaces a rendered tile; rejects stale renders older than `required_gen`.
    pub fn insert(
        &mut self,
        key: TileKey,
        payload: T,
        byte_size: usize,
        rendered_gen: u64,
    ) -> bool {
        let req = self.required_gen(key);
        if rendered_gen < req {
            return false;
        }

        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.byte_size);
        } else {
            self.lru_order.push_back(key);
        }
        self.touch_lru(key);

        self.total_bytes = self.total_bytes.saturating_add(byte_size);
        self.entries.insert(
            key,
            CacheEntry {
                payload,
                byte_size,
                rendered_gen,
            },
        );
        if req > 0 && rendered_gen >= req {
            self.dirty_gen.remove(&key);
        }
        self.evict_over_budget(key);
        true
    }

    fn touch_lru(&mut self, key: TileKey) {
        if let Some(pos) = self.lru_order.iter().position(|k| *k == key) {
            self.lru_order.remove(pos);
        }
        self.lru_order.push_back(key);
    }

    fn evict_over_budget(&mut self, keep_key: TileKey) {
        while self.total_bytes > self.budget_bytes && self.entries.len() > 1 {
            let Some(oldest) = self.lru_order.pop_front() else {
                break;
            };
            if oldest == keep_key {
                self.lru_order.push_back(oldest);
                if self.entries.len() <= 1 {
                    break;
                }
                continue;
            }
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.byte_size);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn lru_cache_enforces_budget_and_generation_invalidation() {
        // 1 MiB budget, 400 KiB per tile => holds at most 2 tiles
        let mut cache: GpuTileCache<u32> = GpuTileCache::new(1024 * 1024);
        let k1 = TileKey {
            page_id: PageId(1),
            tile_x: 0,
            tile_y: 0,
            scale_milli: 1000,
        };
        let k2 = TileKey { tile_x: 1, ..k1 };
        let k3 = TileKey { tile_x: 2, ..k1 };

        let tile_bytes = 400 * 1024;
        let g1 = cache.mark_dirty(k1);
        assert!(cache.insert(k1, 10, tile_bytes, g1));
        assert!(!cache.is_dirty(k1));

        let g2 = cache.mark_dirty(k2);
        assert!(cache.insert(k2, 20, tile_bytes, g2));

        // Touch k1 so k2 becomes least-recently-used
        assert_eq!(cache.get_mut(k1).copied(), Some(10));

        // Simulate stale k1 invalidation race: mark dirty (g_new), try inserting old g1 => rejected!
        let g_new = cache.mark_dirty(k1);
        assert!(cache.is_dirty(k1));
        assert!(!cache.insert(k1, 99, tile_bytes, g1));
        assert!(cache.is_dirty(k1));
        assert!(cache.insert(k1, 11, tile_bytes, g_new));
        assert!(!cache.is_dirty(k1));

        // Insert k3 => evicts k2 (LRU), keeps k1 and k3 within <= 1 MiB
        let g3 = cache.mark_dirty(k3);
        assert!(cache.insert(k3, 30, tile_bytes, g3));
        assert!(cache.total_bytes() <= 1024 * 1024);
        assert!(cache.get(k2).is_none());
        assert_eq!(cache.get(k1).copied(), Some(11));
        assert_eq!(cache.get(k3).copied(), Some(30));
    }

    #[test]
    fn tile_page_rect_snaps_without_oob() {
        let key = TileKey {
            page_id: PageId(1),
            tile_x: 1,
            tile_y: 1,
            scale_milli: 1000,
        };
        let (rect, w_px, h_px) = key.tile_page_rect(595.28, 841.89).unwrap();
        assert_eq!(w_px, 83);
        assert_eq!(h_px, 330);
        assert!((rect.min_x - 512.0).abs() < 1e-6);
        assert!((rect.min_y - 512.0).abs() < 1e-6);
    }
}
