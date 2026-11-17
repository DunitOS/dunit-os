//! Pointer captures keep the original geometry through client resize replies.
type Rect = (i32, i32, i32, i32);
type Point = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}
pub fn resize_edges(rect: Rect, point: Point, grip: i32) -> Option<Edges> {
    let (x, y, w, h) = rect;
    let (px, py) = point;
    if w <= 0 || h <= 0 || px < x || py < y || px >= x + w || py >= y + h {
        return None;
    }
    let grip = grip.max(1).min((w / 2).max(1)).min((h / 2).max(1));
    let edges = Edges {
        left: px < x + grip,
        right: px >= x + w - grip,
        top: py < y + grip,
        bottom: py >= y + h - grip,
    };
    (edges.left || edges.right || edges.top || edges.bottom).then_some(edges)
}

#[derive(Clone, Copy, Debug)]
pub enum Capture {
    Move {
        window: usize,
        offset: Point,
    },
    Resize {
        window: usize,
        origin: Rect,
        pointer: Point,
        edges: Edges,
    },
}
impl Capture {
    pub fn window(self) -> usize {
        match self {
            Self::Move { window, .. } | Self::Resize { window, .. } => window,
        }
    }
    pub fn remove_window(capture: &mut Option<Self>, removed: usize) {
        if let Some(current) = capture {
            let window = match current {
                Self::Move { window, .. } | Self::Resize { window, .. } => window,
            };
            if *window == removed {
                *capture = None;
            } else if *window > removed {
                *window -= 1;
            }
        }
    }
    pub fn geometry(self, current: Rect, point: Point, bounds: Rect, minimum: Point) -> Rect {
        let (bx, by, bw, bh) = bounds;
        let bw = bw.max(1);
        let bh = bh.max(1);
        match self {
            Self::Move { offset, .. } => (
                (point.0 - offset.0).clamp(bx, (bx + bw - current.2).max(bx)),
                (point.1 - offset.1).clamp(by, (by + bh - current.3).max(by)),
                current.2,
                current.3,
            ),
            Self::Resize {
                origin,
                pointer,
                edges,
                ..
            } => {
                let mw = minimum.0.max(1).min(bw.max(1));
                let mh = minimum.1.max(1).min(bh.max(1));
                let mut x = origin.0.clamp(bx, (bx + bw - mw).max(bx));
                let mut y = origin.1.clamp(by, (by + bh - mh).max(by));
                let mut right = (origin.0 + origin.2).clamp(x + mw, (bx + bw).max(x + mw));
                let mut bottom = (origin.1 + origin.3).clamp(y + mh, (by + bh).max(y + mh));
                let dx = point.0.saturating_sub(pointer.0);
                let dy = point.1.saturating_sub(pointer.1);
                if edges.left {
                    x = origin.0.saturating_add(dx).clamp(bx, right - mw);
                }
                if edges.right {
                    right = (origin.0 + origin.2)
                        .saturating_add(dx)
                        .clamp(x + mw, bx + bw);
                }
                if edges.top {
                    y = origin.1.saturating_add(dy).clamp(by, bottom - mh);
                }
                if edges.bottom {
                    bottom = (origin.1 + origin.3)
                        .saturating_add(dy)
                        .clamp(y + mh, by + bh);
                }
                (x, y, right - x, bottom - y)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_edge_and_corner_keeps_the_opposite_edges_anchored() {
        let origin = (100, 100, 300, 200);
        for point in [
            (100, 150),
            (399, 150),
            (200, 100),
            (200, 299),
            (100, 100),
            (399, 100),
            (100, 299),
            (399, 299),
        ] {
            let edges = resize_edges(origin, point, 8).unwrap();
            let capture = Capture::Resize {
                window: 0,
                origin,
                pointer: point,
                edges,
            };
            let result = capture.geometry(
                origin,
                (point.0 + 30, point.1 + 20),
                (0, 0, 800, 600),
                (80, 60),
            );
            if !edges.left {
                assert_eq!(result.0, origin.0);
            }
            if !edges.right {
                assert_eq!(result.0 + result.2, origin.0 + origin.2);
            }
            if !edges.top {
                assert_eq!(result.1, origin.1);
            }
            if !edges.bottom {
                assert_eq!(result.1 + result.3, origin.1 + origin.3);
            }
            assert_ne!(result, origin);
        }
        assert_eq!(resize_edges(origin, (200, 200), 8), None);
        assert_eq!(resize_edges(origin, (99, 150), 8), None);
    }
    #[test]
    fn resize_uses_the_capture_instead_of_drifting_after_client_replies() {
        let origin = (100, 100, 300, 200);
        let pointer = (399, 299);
        let capture = Capture::Resize {
            window: 2,
            origin,
            pointer,
            edges: resize_edges(origin, pointer, 8).unwrap(),
        };
        let bounds = (10, 20, 600, 400);
        let large = capture.geometry(origin, (10000, 10000), bounds, (80, 60));
        assert_eq!(large, (100, 100, 510, 320));
        let small = capture.geometry(large, (-1000, -1000), bounds, (80, 60));
        assert_eq!(small, (100, 100, 80, 60));
        assert_eq!(capture.geometry(small, pointer, bounds, (80, 60)), origin);
    }
    #[test]
    fn small_displays_and_oversized_move_keep_a_reachable_title_bar() {
        let capture = Capture::Move {
            window: 0,
            offset: (5, 5),
        };
        assert_eq!(
            capture.geometry((0, 0, 1000, 800), (-100, -100), (30, 20, 100, 80), (80, 60)),
            (30, 20, 1000, 800)
        );
        let origin = (0, 0, 100, 100);
        let capture = Capture::Resize {
            window: 0,
            origin,
            pointer: (99, 99),
            edges: resize_edges(origin, (99, 99), 8).unwrap(),
        };
        assert_eq!(
            capture.geometry(origin, (900, 900), (0, 0, 40, 30), (80, 60)),
            (0, 0, 40, 30)
        );
    }
    #[test]
    fn removal_cancels_its_capture_and_repairs_indices_above_it() {
        let mut capture = Some(Capture::Move {
            window: 3,
            offset: (1, 2),
        });
        Capture::remove_window(&mut capture, 1);
        assert_eq!(capture.unwrap().window(), 2);
        Capture::remove_window(&mut capture, 2);
        assert!(capture.is_none());
    }
}
