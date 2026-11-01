//! Only visible rows are retained and painted. Geometry is shared with hit tests.
use crate::Surface;
use alloc::{string::String, vec::Vec};
use dunit_style::Style;
use dunit_text::Font;
use dunit_widgets::selection::ListGeometry;
#[derive(Debug, Clone, PartialEq)]
pub struct ListRowPaint {
    pub index: usize,
    pub text: String,
    pub selected: bool,
    pub current: bool,
    pub disabled: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ListPaint {
    pub geometry: ListGeometry,
    pub focused: bool,
    pub rows: Vec<ListRowPaint>,
}
impl ListPaint {
    pub(crate) fn paint(&self, style: &Style, font: &Font, surface: &mut Surface) {
        surface.with_clip(self.geometry.viewport, |s| {
            for row in &self.rows {
                let r = dunit_ui::Rect {
                    x: self.geometry.viewport.x,
                    y: self.geometry.viewport.y + row.index as f32 * self.geometry.row_height
                        - self.geometry.offset,
                    w: self.geometry.viewport.w,
                    h: self.geometry.row_height,
                };
                if row.selected {
                    if let Some(color) = style.color("list-selection-background") {
                        s.fill_rect(r.x, r.y, r.w, r.h, color);
                    }
                }
                if self.focused && row.current {
                    if let Some(color) = style.color("list-current-color") {
                        s.stroke_rect(
                            r.x,
                            r.y,
                            r.w,
                            r.h,
                            style.number("border-width").unwrap_or(0.),
                            color,
                        );
                    }
                }
                let mut text_style = style.clone();
                let ink = if row.disabled {
                    style.get("list-disabled-color")
                } else if row.selected {
                    style.get("list-selection-color")
                } else {
                    style.get("color")
                };
                if let Some(ink) = ink {
                    text_style.set("color", ink.clone());
                }
                super::paint::paint_text(&text_style, r, 0., &row.text, font, s, true);
            }
        });
    }
}
