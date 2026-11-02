//! Input geometry and painting. Editing remains in the widget layer.
use crate::{CaretAffinity, PreparedInput, ScrollArea, Surface, TextPosition, VisualLines};
use alloc::string::String;
use dunit_style::Style;
use dunit_text::Font;
use dunit_ui::Rect;
use dunit_widgets::ScrollAxis;

/// A render snapshot; offsets refer to UTF-8 byte boundaries in `value`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInput {
    pub spinbox: Option<SpinboxPaint>,
    pub value: String,
    pub placeholder: String,
    pub password: bool,
    pub multiline: bool,
    pub soft_wrap: bool,
    pub affinity: CaretAffinity,
    pub cursor: usize,
    pub anchor: usize,
    pub focused: bool,
    pub caret_visible: bool,
    pub scroll_x: f32,
    pub scroll_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpinboxPaint {
    pub increase: bool,
    pub decrease: bool,
    pub pressed: Option<dunit_widgets::spinbox::SpinButton>,
}

/// Content box shared by painting, cursor fitting and pointer hit testing.
pub fn input_content_rect(style: &Style, rect: Rect) -> Rect {
    let b = style.number("border-width").unwrap_or(0.).max(0.);
    let [t, r, bottom, l] = style.padding().unwrap_or([0.; 4]);
    Rect {
        x: rect.x + b + l,
        y: rect.y + b + t,
        w: (rect.w - 2. * b - l - r).max(0.),
        h: (rect.h - 2. * b - t - bottom).max(0.),
    }
}
fn finite_scroll(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.)
    } else {
        0.
    }
}
fn scroll_axis(prior: f32, visible: f32, limit: f32) -> ScrollAxis {
    let visible = finite_scroll(visible);
    let limit = finite_scroll(limit);
    let content = (visible as f64 + limit as f64).min(f32::MAX as f64) as f32;
    let mut axis = ScrollAxis {
        offset: prior,
        ..ScrollAxis::default()
    };
    axis.set_extents(visible, content);
    axis
}
fn fit_axis(prior: f32, pos: f32, visible: f32, limit: f32) -> f32 {
    // The caller reserves the caret dimensions from the usable viewport, so
    // revealing its leading-edge point keeps the entire caret in view.
    let mut axis = scroll_axis(prior, visible, limit);
    axis.reveal(pos, pos);
    axis.offset
}
impl TextInput {
    /// Text viewport; multiline fields reserve both scrollbar gutters even
    /// when overflow auto-hides the bars, keeping text layout stable.
    pub fn content_rect(&self, style: &Style, rect: Rect) -> Rect {
        let mut content = input_content_rect(style, rect);
        if self.spinbox.is_some() {
            content.w = (content.w - self.spinbox_rects(style, rect)[0].w).max(0.);
        }
        if self.multiline {
            let gutter = style
                .number("scrollbar-width")
                .filter(|v| v.is_finite())
                .unwrap_or(0.)
                .max(0.);
            content.w = (content.w - gutter).max(0.);
            content.h = (content.h - gutter).max(0.);
        }
        content
    }
    /// Shared button boxes for painting and pointer hit testing.
    pub fn spinbox_rects(&self, style: &Style, rect: Rect) -> [Rect; 2] {
        let area = input_content_rect(style, rect);
        let w = style
            .number("spinbox-button-width")
            .unwrap_or(0.)
            .max(0.)
            .min(area.w);
        let up = Rect {
            x: area.x + area.w - w,
            y: area.y,
            w,
            h: area.h / 2.,
        };
        [
            up,
            Rect {
                y: up.y + up.h,
                ..up
            },
        ]
    }
    pub fn spinbox_button_at(
        &self,
        style: &Style,
        rect: Rect,
        x: f32,
        y: f32,
    ) -> Option<dunit_widgets::spinbox::SpinButton> {
        self.spinbox.as_ref()?;
        let boxes = self.spinbox_rects(style, rect);
        for (r, button) in boxes.into_iter().zip([
            dunit_widgets::spinbox::SpinButton::Increase,
            dunit_widgets::spinbox::SpinButton::Decrease,
        ]) {
            if x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h {
                return Some(button);
            }
        }
        None
    }
    fn paint_spinbox(&self, style: &Style, rect: Rect, surface: &mut Surface) {
        let Some(spin) = &self.spinbox else {
            return;
        };
        use dunit_widgets::spinbox::SpinButton;
        for (r, button, enabled) in [
            (
                self.spinbox_rects(style, rect)[0],
                SpinButton::Increase,
                spin.increase,
            ),
            (
                self.spinbox_rects(style, rect)[1],
                SpinButton::Decrease,
                spin.decrease,
            ),
        ] {
            surface.with_clip(r, |surface| {
                let bg = style.color(if spin.pressed == Some(button) {
                    "spinbox-pressed-color"
                } else {
                    "spinbox-button-color"
                });
                if let Some(bg) = bg {
                    surface.fill_rect(r.x, r.y, r.w, r.h, bg);
                }
                let color = style.color(if enabled {
                    "color"
                } else {
                    "spinbox-disabled-color"
                });
                let size = style
                    .number("spinbox-mark-size")
                    .unwrap_or(0.)
                    .max(0.)
                    .min(r.w)
                    .min(r.h);
                let stroke = style
                    .number("spinbox-mark-width")
                    .unwrap_or(0.)
                    .max(0.)
                    .min(size);
                if let Some(color) = color {
                    surface.fill_rect(
                        r.x + (r.w - size) / 2.,
                        r.y + (r.h - stroke) / 2.,
                        size,
                        stroke,
                        color,
                    );
                    if button == SpinButton::Increase {
                        surface.fill_rect(
                            r.x + (r.w - stroke) / 2.,
                            r.y + (r.h - size) / 2.,
                            stroke,
                            size,
                            color,
                        );
                    }
                }
            });
        }
    }
    /// Actual text extents (excluding placeholder), including insertion caret
    /// width so dragging the horizontal bar can reveal the full final caret.
    pub fn scroll_axes(&self, font: &Font, style: &Style, rect: Rect) -> (ScrollAxis, ScrollAxis) {
        let lines = self.visual_lines(font, style, rect);
        self.scroll_axes_with_lines(font, style, rect, &lines)
    }
    /// Extents using geometry prepared for the same value/font/style/width.
    pub fn scroll_axes_with_lines(
        &self,
        font: &Font,
        style: &Style,
        rect: Rect,
        lines: &VisualLines,
    ) -> (ScrollAxis, ScrollAxis) {
        let content = self.content_rect(style, rect);
        let px = style.number("font-size").unwrap_or(0.);
        let text_width = lines.max_width();
        let mut x = ScrollAxis {
            offset: self.scroll_x,
            ..ScrollAxis::default()
        };
        x.set_extents(
            content.w,
            text_width + style.number("caret-width").unwrap_or(0.).max(0.),
        );
        let mut y = ScrollAxis {
            offset: self.scroll_y,
            ..ScrollAxis::default()
        };
        y.set_extents(
            content.h,
            if self.multiline {
                lines.lines.len() as f32 * font.line_metrics(px).line_height()
            } else {
                0.
            },
        );
        (x, y)
    }
    pub fn scroll_area(&self, font: &Font, style: &Style, rect: Rect) -> ScrollArea {
        let (x, y) = self.scroll_axes(font, style, rect);
        ScrollArea::from_axes(
            self.content_rect(style, rect),
            style,
            self.multiline.then_some(&x),
            self.multiline.then_some(&y),
        )
    }

    fn boundary(&self, index: usize) -> usize {
        let mut i = index.min(self.value.len());
        while !self.value.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
    fn advance(&self, font: &Font, px: f32, ch: char) -> f32 {
        let ch = if self.password { '•' } else { ch };
        font.glyph_metrics(font.glyph_index(ch)).advance as f32 * font.scale_for_px(px)
    }
    fn paint_width(&self, font: &Font, px: f32, text: &str, limit: f32) -> f32 {
        let mut width = 0.;
        for ch in text.chars() {
            if width > limit {
                break;
            }
            width += self.advance(font, px, ch);
        }
        width
    }
    /// The same grapheme-aware line map drives all geometry consumers.
    pub fn visual_lines(&self, font: &Font, style: &Style, rect: Rect) -> VisualLines {
        let content = self.content_rect(style, rect);
        let width = (content.w - style.number("caret-width").unwrap_or(0.).max(0.)).max(0.);
        VisualLines::new(
            &self.value,
            font,
            style.number("font-size").unwrap_or(0.),
            width,
            self.multiline,
            self.soft_wrap,
            self.password,
        )
    }
    /// Prepare once after final layout. Value geometry is shared by every
    /// consumer; a distinct placeholder map exists only while it is displayed.
    pub fn prepare(&self, font: &Font, style: &Style, rect: Rect) -> PreparedInput {
        let value = self.visual_lines(font, style, rect);
        let placeholder = (self.value.is_empty() && !self.placeholder.is_empty()).then(|| {
            let content = self.content_rect(style, rect);
            VisualLines::new(
                &self.placeholder,
                font,
                style.number("font-size").unwrap_or(0.),
                (content.w - style.number("caret-width").unwrap_or(0.).max(0.)).max(0.),
                self.multiline,
                self.soft_wrap,
                false,
            )
        });
        PreparedInput { value, placeholder }
    }
    fn vertical_limit(&self, font: &Font, style: &Style, rect: Rect) -> f32 {
        if !self.multiline {
            return 0.;
        }
        (self.visual_lines(font, style, rect).lines.len() as f32
            * font
                .line_metrics(style.number("font-size").unwrap_or(0.))
                .line_height()
            - self.content_rect(style, rect).h)
            .max(0.)
    }
    /// Preserve the prior viewport while keeping the insertion caret visible.
    pub fn fit_scroll(&mut self, font: &Font, style: &Style, rect: Rect) {
        let lines = self.visual_lines(font, style, rect);
        self.fit_scroll_with_lines(font, style, rect, &lines);
    }
    /// Fit using the same immutable map that will be painted in this snapshot.
    pub fn fit_scroll_with_lines(
        &mut self,
        font: &Font,
        style: &Style,
        rect: Rect,
        lines: &VisualLines,
    ) {
        let content = self.content_rect(style, rect);
        let px = style.number("font-size").unwrap_or(0.);
        let width = (content.w - style.number("caret-width").unwrap_or(0.).max(0.)).max(0.);
        let total = lines.max_width();
        let (x, row) = lines.position(TextPosition {
            index: self.boundary(self.cursor),
            affinity: self.affinity,
        });
        self.scroll_x = fit_axis(self.scroll_x, x, width, total - width);
        let height = font.line_metrics(px).line_height();
        self.scroll_y = if self.multiline {
            fit_axis(
                self.scroll_y,
                row as f32 * height,
                (content.h - height).max(0.),
                (lines.lines.len() as f32 * height - content.h).max(0.),
            )
        } else {
            0.
        };
    }
    /// Move the viewport by logical lines without fitting it back to the caret.
    pub fn scroll_lines(&mut self, font: &Font, style: &Style, rect: Rect, lines: f32) -> bool {
        if !lines.is_finite() {
            return false;
        }
        let height = font
            .line_metrics(style.number("font-size").unwrap_or(0.))
            .line_height();
        let mut axis = scroll_axis(
            self.scroll_y,
            self.content_rect(style, rect).h,
            self.vertical_limit(font, style, rect),
        );
        let delta = (lines as f64 * finite_scroll(height) as f64)
            .clamp(-(f32::MAX as f64), f32::MAX as f64) as f32;
        axis.move_by(delta);
        let changed = axis.offset != self.scroll_y;
        self.scroll_y = axis.offset;
        changed
    }
    /// Nearest scalar boundary on the first logical line (single-line API).
    pub fn hit_index(&self, font: &Font, style: &Style, rect: Rect, x: f32) -> usize {
        self.hit_index_at(
            font,
            style,
            rect,
            x,
            self.content_rect(style, rect).y - self.scroll_y,
        )
    }
    /// Map both pointer axes through the shared visual-line viewport.
    pub fn hit_position_at(
        &self,
        font: &Font,
        style: &Style,
        rect: Rect,
        x: f32,
        y: f32,
    ) -> TextPosition {
        let lines = self.visual_lines(font, style, rect);
        self.hit_position_with_lines(font, style, rect, x, y, &lines)
    }
    pub fn hit_position_with_lines(
        &self,
        font: &Font,
        style: &Style,
        rect: Rect,
        x: f32,
        y: f32,
        lines: &VisualLines,
    ) -> TextPosition {
        let px = style.number("font-size").unwrap_or(0.);
        let content = self.content_rect(style, rect);
        let height = font.line_metrics(px).line_height();
        let row = if self.multiline && height > 0. {
            ((y - content.y + self.scroll_y).max(0.) / height) as usize
        } else {
            0
        };
        lines.hit(row, x - content.x + self.scroll_x)
    }
    pub fn hit_index_at(&self, font: &Font, style: &Style, rect: Rect, x: f32, y: f32) -> usize {
        self.hit_position_at(font, style, rect, x, y).index
    }
    /// Pixel-column navigation over the same visual rows used by painting.
    /// The returned goal is retained across repeated vertical movements.
    pub fn visual_navigation(
        &self,
        font: &Font,
        style: &Style,
        rect: Rect,
        command: dunit_widgets::text_input::EditCommand,
        goal: Option<f32>,
    ) -> Option<(TextPosition, Option<f32>)> {
        if !self.multiline || !self.soft_wrap {
            return None;
        }
        let lines = self.visual_lines(font, style, rect);
        self.visual_navigation_with_lines(command, goal, &lines)
    }
    pub fn visual_navigation_with_lines(
        &self,
        command: dunit_widgets::text_input::EditCommand,
        goal: Option<f32>,
        lines: &VisualLines,
    ) -> Option<(TextPosition, Option<f32>)> {
        use dunit_widgets::text_input::EditCommand::*;
        if !self.multiline || !self.soft_wrap {
            return None;
        }
        let (x, row) = lines.position(TextPosition {
            index: self.cursor,
            affinity: self.affinity,
        });
        Some(match command {
            Home { .. } => (
                TextPosition {
                    index: lines.lines[row].start,
                    affinity: CaretAffinity::Downstream,
                },
                None,
            ),
            End { .. } => (
                TextPosition {
                    index: lines.lines[row].end,
                    affinity: if lines.lines[row].soft_break {
                        CaretAffinity::Upstream
                    } else {
                        CaretAffinity::Downstream
                    },
                },
                None,
            ),
            Up { .. } | Down { .. } => {
                let goal = goal.unwrap_or(x);
                let target = if matches!(command, Up { .. }) {
                    row.saturating_sub(1)
                } else {
                    (row + 1).min(lines.lines.len() - 1)
                };
                (lines.hit(target, goal), Some(goal))
            }
            _ => return None,
        })
    }
    pub(crate) fn paint(&self, font: &Font, style: &Style, rect: Rect, surface: &mut Surface) {
        let prepared = self.prepare(font, style, rect);
        self.paint_prepared(font, style, rect, surface, &prepared);
    }
    pub(crate) fn paint_prepared(
        &self,
        font: &Font,
        style: &Style,
        rect: Rect,
        surface: &mut Surface,
        prepared: &PreparedInput,
    ) {
        self.paint_spinbox(style, rect, surface);
        let content = self.content_rect(style, rect);
        let px = style.number("font-size").unwrap_or(0.);
        let metrics = font.line_metrics(px);
        let height = metrics.line_height();
        if height <= 0. || content.w <= 0. || content.h <= 0. {
            return;
        }
        let top = if self.multiline {
            content.y - self.scroll_y
        } else {
            content.y + (content.h - height).max(0.) / 2.
        };
        let (y_min, y_max) = font.vertical_bounds_px(px);
        // Logical row boxes and actual glyph ink can overlap different rows.
        // Keep both extents: selection follows logical boxes while tall accents
        // and deep descenders remain visible across viewport boundaries.
        // The two-pixel guard covers raster padding and integer placement.
        let row_ink_top = (metrics.ascent - y_max - 2.).min(0.);
        let row_ink_bottom = (metrics.ascent - y_min + 2.).max(height);
        let first = if self.multiline {
            ((finite_scroll(self.scroll_y) - row_ink_bottom).max(0.) / height) as usize
        } else {
            0
        };
        let placeholder = self.value.is_empty();
        let text = if placeholder {
            &self.placeholder
        } else {
            &self.value
        };
        let ink = style.color(if placeholder {
            "placeholder-color"
        } else {
            "color"
        });
        let origin = content.x - self.scroll_x;
        let selection_start = self.boundary(self.anchor.min(self.cursor));
        let selection_end = self.boundary(self.anchor.max(self.cursor));
        let caret_width = style.number("caret-width").unwrap_or(0.).max(0.);
        let cursor = self.boundary(self.cursor);
        let caret_lines = prepared.lines();
        let lines = prepared.placeholder.as_ref().unwrap_or(caret_lines);
        let (caret_x, caret_row) = caret_lines.position(TextPosition {
            index: cursor,
            affinity: self.affinity,
        });
        let horizontal_limit = self.scroll_x + content.w;
        let (ink_min, ink_max) = font.horizontal_bounds_px(px);
        // Raster::render pads the outline by one pixel; integer placement and
        // floor/ceil can each move the bound by a fractional pixel. Two pixels
        // conservatively cover that quantization, independent of theme policy.
        let ink_min = ink_min - 2.;
        let ink_max = ink_max + 2.;
        surface.with_clip(content, |surface| {
            for (row, visual) in lines.lines.iter().enumerate() {
                let start = visual.start;
                let end = visual.end;
                let line = &text[start..end];
                if row < first {
                    continue;
                }
                let y = top + row as f32 * height;
                if y + row_ink_top >= content.y + content.h {
                    break;
                }
                if !placeholder
                    && self.focused
                    && selection_start < selection_end
                    && selection_start <= end
                    && selection_end > start
                {
                    if let Some(color) = style.color("selection-color") {
                        let left_index = selection_start.saturating_sub(start).min(line.len());
                        let right_index = selection_end.saturating_sub(start).min(line.len());
                        let left = origin
                            + self.paint_width(font, px, &line[..left_index], horizontal_limit);
                        let right = if self.multiline
                            && selection_end > end
                            && end < self.value.len()
                        {
                            (origin
                                + self.paint_width(font, px, line, horizontal_limit)
                                + caret_width)
                                .max(content.x + content.w)
                        } else {
                            origin
                                + self.paint_width(font, px, &line[..right_index], horizontal_limit)
                        };
                        surface.fill_rect(left, y, right - left, height, color);
                    }
                }
                if let Some(color) = ink {
                    let mut pen = origin;
                    for ch in line.chars() {
                        if pen + ink_min >= content.x + content.w {
                            break;
                        }
                        let displayed = if self.password && !placeholder {
                            '•'
                        } else {
                            ch
                        };
                        let glyph = font.glyph_index(displayed);
                        let advance =
                            font.glyph_metrics(glyph).advance as f32 * font.scale_for_px(px);
                        if pen + ink_max >= content.x {
                            if let Some(bitmap) = surface.rasterize(font, glyph, px) {
                                let round = |v: f32| {
                                    if v < 0. {
                                        (v - 0.5) as i32
                                    } else {
                                        (v + 0.5) as i32
                                    }
                                };
                                surface.blit_glyph(
                                    &bitmap,
                                    round(pen) + bitmap.left,
                                    round(y + metrics.ascent) - bitmap.top,
                                    color,
                                );
                            }
                        }
                        pen += advance;
                    }
                }
            }
            let caret_y = top + caret_row as f32 * height;
            if self.focused
                && self.caret_visible
                && caret_y < content.y + content.h
                && caret_y + height > content.y
            {
                if let Some(color) = style.color("caret-color") {
                    surface.fill_rect(
                        origin + caret_x,
                        top + caret_row as f32 * height,
                        caret_width,
                        height,
                        color,
                    );
                }
            }
        });
    }
}
