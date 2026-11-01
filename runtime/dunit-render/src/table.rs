//! Virtual table paint payload: fixed header and clipped visible cells.
use crate::Surface;
use alloc::{string::String, vec::Vec};
use dunit_style::Style;
use dunit_text::Font;
use dunit_ui::Rect;
#[derive(Debug, Clone, PartialEq)]
pub struct TablePaint {
    pub header: Rect,
    pub columns: Vec<String>,
    pub widths: Vec<f32>,
    pub offset: f32,
    pub current_column: usize,
    pub sort: Option<(usize, bool)>,
}
impl TablePaint {
    pub fn header_at(&self, x: f32, y: f32) -> Option<usize> {
        dunit_widgets::table::column_at(self.header, &self.widths, self.offset, x, y)
    }
    pub fn cell_at(&self, viewport: Rect, x: f32, y: f32) -> Option<usize> {
        dunit_widgets::table::column_at(viewport, &self.widths, self.offset, x, y)
    }
    pub(crate) fn header(&self, style: &Style, font: &Font, s: &mut Surface) {
        s.with_clip(self.header, |s| {
            if let Some(color) = style.color("table-header-background") {
                s.fill_rect(
                    self.header.x,
                    self.header.y,
                    self.header.w,
                    self.header.h,
                    color,
                );
            }
            for (i, label) in self.columns.iter().enumerate() {
                let Some(rect) =
                    dunit_widgets::table::column_rect(self.header, &self.widths, self.offset, i)
                else {
                    continue;
                };
                if rect.x >= self.header.x + self.header.w || rect.x + rect.w <= self.header.x {
                    continue;
                }
                let mut label = label.clone();
                if let Some((column, ascending)) = self.sort {
                    if column == i {
                        if let Some(mark) = style.keyword(if ascending {
                            "table-sort-asc-mark"
                        } else {
                            "table-sort-desc-mark"
                        }) {
                            label.push(' ');
                            label.push_str(mark);
                        }
                    }
                }
                s.with_clip(rect, |s| {
                    super::paint::paint_text(style, rect, 0., &label, font, s, true)
                });
                if let Some(color) = style.color("border-color") {
                    s.stroke_rect(
                        rect.x,
                        rect.y,
                        rect.w,
                        rect.h,
                        style.number("border-width").unwrap_or(0.),
                        color,
                    );
                }
            }
        });
    }
    pub(crate) fn row(
        &self,
        rect: Rect,
        cells: &[String],
        style: &Style,
        font: &Font,
        s: &mut Surface,
        current: bool,
    ) {
        for (i, text) in cells.iter().enumerate() {
            let Some(cell) = dunit_widgets::table::column_rect(rect, &self.widths, self.offset, i)
            else {
                continue;
            };
            if cell.x >= rect.x + rect.w || cell.x + cell.w <= rect.x {
                continue;
            }
            s.with_clip(cell, |s| {
                super::paint::paint_text(style, cell, 0., text, font, s, true)
            });
            if let Some(color) = style.color("border-color") {
                s.stroke_rect(
                    cell.x,
                    cell.y,
                    cell.w,
                    cell.h,
                    style.number("border-width").unwrap_or(0.),
                    color,
                );
            }
            if current && i == self.current_column {
                if let Some(color) = style.color("list-current-color") {
                    s.stroke_rect(
                        cell.x,
                        cell.y,
                        cell.w,
                        cell.h,
                        style.number("border-width").unwrap_or(0.),
                        color,
                    );
                }
            }
        }
    }
}
