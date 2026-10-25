//! Pure shell geometry shared by hit testing and painting.
pub type Rect = (i32, i32, i32, i32);
pub fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.0.max(b.0);
    let y = a.1.max(b.1);
    (
        x,
        y,
        (a.0 + a.2).min(b.0 + b.2).saturating_sub(x).max(0),
        (a.1 + a.3).min(b.1 + b.3).saturating_sub(y).max(0),
    )
}
pub fn item_rect(r: Rect, horizontal: bool, index: usize, count: usize, gap: i32) -> Rect {
    if count == 0 || index >= count {
        return (r.0, r.1, 0, 0);
    }
    let n = count as i32;
    let i = index as i32;
    let length = if horizontal { r.2 } else { r.3 }.max(0);
    let gap = if n > 1 {
        gap.max(0).min(length / (n - 1))
    } else {
        0
    };
    let usable = (length - gap * (n - 1)).max(0);
    // Distribute remainder pixels rather than losing the final item at the edge.
    let start = usable * i / n + i * gap;
    let end = usable * (i + 1) / n + i * gap;
    if horizontal {
        (r.0 + start, r.1, end - start, r.3.max(0))
    } else {
        (r.0, r.1 + start, r.2.max(0), end - start)
    }
}
pub fn popup(anchor: Rect, width: i32, height: i32, count: usize, presentation: &str) -> Rect {
    let n = count.max(1) as i32;
    let (w, h) = match presentation {
        "list" => (220, n * 40 + 16),
        "fan" => (80 + (n - 1) * 20 + 16, 80 + (n - 1) * 28 + 16),
        _ => (n.min(4) * 64 + 16, ((n + 3) / 4) * 64 + 16),
    };
    let w = w.min(width.max(1));
    let h = h.min(height.max(1));
    let x = (anchor.0 + anchor.2 / 2 - w / 2).clamp(0, (width - w).max(0));
    let y = if anchor.1 >= h {
        anchor.1 - h
    } else {
        anchor.1 + anchor.3
    }
    .clamp(0, (height - h).max(0));
    (x, y, w, h)
}
pub fn group_cell(r: Rect, index: usize, count: usize, presentation: &str) -> Rect {
    if count == 0 || index >= count {
        return (r.0, r.1, 0, 0);
    }
    let pad = 8.min(r.2.min(r.3).max(0) / 2);
    let inner = (
        r.0 + pad,
        r.1 + pad,
        (r.2 - pad * 2).max(0),
        (r.3 - pad * 2).max(0),
    );
    if presentation == "list" {
        return item_rect(inner, false, index, count, 0);
    }
    if presentation == "fan" {
        let size = 80.min(inner.2).min(inner.3).max(0);
        let steps = count.saturating_sub(1).max(1) as i32;
        return (
            inner.0 + (inner.2 - size) * index as i32 / steps,
            inner.1 + (inner.3 - size) * index as i32 / steps,
            size,
            size,
        );
    }
    let columns = count.min(4).max(1);
    let rows = count.div_ceil(columns);
    let row = item_rect(inner, false, index / columns, rows, 0);
    item_rect(row, true, index % columns, columns, 0)
}
/// Integer Bhaskara sine approximation. Unlike polygon interpolation, the hand
/// keeps approximately constant length while moving between clock markers.
fn sine(phase: u64, period: u64) -> i32 {
    let p = period.max(1);
    let phase = phase % p;
    let negative = phase >= p / 2;
    let x = ((phase * 2048 / p) % 1024) as i64;
    let product = x * (1024 - x);
    let value = 16 * product * 1024 / (5 * 1024 * 1024 - 4 * product);
    if negative {
        -(value as i32)
    } else {
        value as i32
    }
}
pub fn hand_endpoint(radius: i32, phase: u64, period: u64) -> (i32, i32) {
    let p = period.max(4);
    let x = sine(phase, p);
    let y = -sine((phase % p + p / 4) % p, p);
    (x * radius / 1024, y * radius / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inside(a: Rect, b: Rect) -> bool {
        a.0 >= b.0 && a.1 >= b.1 && a.0 + a.2 <= b.0 + b.2 && a.1 + a.3 <= b.1 + b.3
    }
    #[test]
    fn items_stay_clipped_with_large_gaps_and_preserve_remainder() {
        let r = (5, 9, 101, 23);
        for gap in [0, 2, 80, 16384] {
            for i in 0..7 {
                assert!(inside(item_rect(r, true, i, 7, gap), r));
            }
        }
        let last = item_rect(r, true, 6, 7, 2);
        assert_eq!(last.0 + last.2, 106);
        assert_eq!(item_rect(r, true, 0, 0, 0).2, 0);
    }
    #[test]
    fn popup_cells_fit_small_displays_for_every_presentation() {
        for presentation in ["grid", "list", "fan"] {
            for n in [1, 4, 5, 32] {
                let r = popup((300, 220, 20, 20), 320, 240, n, presentation);
                assert!(inside(r, (0, 0, 320, 240)));
                for i in 0..n {
                    assert!(inside(group_cell(r, i, n, presentation), r));
                }
            }
        }
    }
    #[test]
    fn fan_has_distinct_overlapping_cards() {
        let r = popup((100, 100, 40, 40), 800, 600, 5, "fan");
        let a = group_cell(r, 0, 5, "fan");
        let b = group_cell(r, 1, 5, "fan");
        assert!(b.0 > a.0 && b.1 > a.1);
        assert!(intersect(a, b).2 > 0);
    }
    #[test]
    fn intersection_removes_offscreen_left_and_top_parts() {
        assert_eq!(
            intersect((-20, -10, 30, 20), (0, 0, 100, 100)),
            (0, 0, 10, 10)
        );
        assert_eq!(intersect((110, 0, 10, 10), (0, 0, 100, 100)).2, 0);
    }
    #[test]
    fn clock_hands_keep_radius_and_move_between_second_ticks() {
        assert_eq!(hand_endpoint(100, 0, 60000), (0, -100));
        assert_eq!(hand_endpoint(100, 15000, 60000), (100, 0));
        assert_ne!(hand_endpoint(100, 0, 60000), hand_endpoint(100, 500, 60000));
        for phase in (0..60000).step_by(500) {
            let (x, y) = hand_endpoint(100, phase, 60000);
            assert!((9700..=10200).contains(&(x * x + y * y)));
        }
    }
}
