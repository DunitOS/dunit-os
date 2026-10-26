//! Authoritative committed image metadata for native transports; no wire changes.
use crate::wire::{Rect, MAX_DAMAGE_RECTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    rects: [Rect; MAX_DAMAGE_RECTS as usize],
    count: usize,
    /// Empty ATTACH damage requests conservative full updates (legacy clients).
    pub continuous: bool,
}
impl Damage {
    pub const fn full() -> Self {
        Self {
            rects: [Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }; MAX_DAMAGE_RECTS as usize],
            count: 0,
            continuous: true,
        }
    }
    pub fn from_validated(rects: &[Rect]) -> Self {
        let mut damage = Self::full();
        // Server validation establishes this bound before calling us.
        damage.count = rects.len().min(MAX_DAMAGE_RECTS as usize);
        damage.rects[..damage.count].copy_from_slice(&rects[..damage.count]);
        damage.continuous = rects.is_empty();
        damage
    }
    pub fn force_full(&mut self) {
        self.count = 0;
    }
    /// Empty means full, including first map and size/scale transitions.
    pub fn rects(&self) -> &[Rect] {
        &self.rects[..self.count]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub buffer: u64,
    pub commit: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub offset: u64,
    pub format: u32,
    pub damage: Damage,
}

impl Frame {
    /// Snapshot accepted pixels into a packed CPU texture. Bounds are checked
    /// before any mutation. Bytes outside declared damage retain their previous
    /// value; callers force full on first image/geometry changes.
    pub fn copy_into(&self, source: &[u8], target: &mut [u32], force_full: bool) -> Option<u64> {
        let w = self.width as usize;
        let h = self.height as usize;
        let stride = self.stride as usize;
        let offset = usize::try_from(self.offset).ok()?;
        if w == 0 || h == 0 || stride < w.checked_mul(4)? || target.len() != w.checked_mul(h)? {
            return None;
        }
        let end = offset
            .checked_add((h - 1).checked_mul(stride)?)?
            .checked_add(w.checked_mul(4)?)?;
        if end > source.len() {
            return None;
        }
        let full = [Rect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        }];
        let rects = if force_full || self.damage.rects().is_empty() {
            &full[..]
        } else {
            self.damage.rects()
        };
        for r in rects {
            if r.x < 0
                || r.y < 0
                || r.w == 0
                || r.h == 0
                || r.x as u64 + r.w as u64 > self.width as u64
                || r.y as u64 + r.h as u64 > self.height as u64
            {
                return None;
            }
        }
        let mut copied = 0;
        for r in rects {
            for y in r.y as usize..r.y as usize + r.h as usize {
                for x in r.x as usize..r.x as usize + r.w as usize {
                    let at = offset + y * stride + x * 4;
                    target[y * w + x] = u32::from_le_bytes(source[at..at + 4].try_into().ok()?);
                    copied += 1;
                }
            }
        }
        Some(copied)
    }
}
