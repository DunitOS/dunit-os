//! Shared client-surface popup placement. Coordinates never escape the viewport.
use dunit_ui::Rect;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Below,
    Side,
    Point,
}
pub fn place(
    anchor: Rect,
    width: f32,
    height: f32,
    viewport: Rect,
    placement: Placement,
    gap: f32,
) -> Rect {
    let w = width.max(0.).min(viewport.w.max(0.));
    let h = height.max(0.).min(viewport.h.max(0.));
    let (mut x, mut y) = match placement {
        Placement::Point => (anchor.x, anchor.y),
        Placement::Side => {
            let right = anchor.x + anchor.w + gap;
            let left = anchor.x - gap - w;
            (
                if right + w <= viewport.x + viewport.w {
                    right
                } else {
                    left
                },
                anchor.y,
            )
        }
        Placement::Below => {
            let below = anchor.y + anchor.h + gap;
            let above = anchor.y - gap - h;
            (
                anchor.x,
                if below + h <= viewport.y + viewport.h
                    || anchor.y - viewport.y < viewport.y + viewport.h - (anchor.y + anchor.h)
                {
                    below
                } else {
                    above
                },
            )
        }
    };
    x = x.max(viewport.x).min(viewport.x + (viewport.w - w).max(0.));
    y = y.max(viewport.y).min(viewport.y + (viewport.h - h).max(0.));
    Rect { x, y, w, h }
}
