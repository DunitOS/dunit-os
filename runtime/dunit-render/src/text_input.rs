//! Input geometry and painting. Editing remains in the widget layer.
use crate::Surface;
use alloc::string::String;
use dunit_style::Style;
use dunit_text::Font;
use dunit_ui::Rect;
use dunit_widgets::ScrollAxis;

/// A render snapshot; offsets refer to UTF-8 byte boundaries in `value`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInput {
    pub value: String,
    pub placeholder: String,
    pub password: bool,
    pub multiline: bool,
    pub cursor: usize,
    pub anchor: usize,
    pub focused: bool,
    pub caret_visible: bool,
    pub scroll_x: f32,
    pub scroll_y: f32,
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
    fn width(&self, font: &Font, px: f32, text: &str) -> f32 {
        text.chars().map(|ch| self.advance(font, px, ch)).sum()
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
    fn cursor_position(&self, font: &Font, px: f32) -> (f32, usize) {
        let prefix = &self.value[..self.boundary(self.cursor)];
        if self.multiline {
            let row = prefix.bytes().filter(|b| *b == b'\n').count();
            (
                self.width(font, px, prefix.rsplit('\n').next().unwrap_or("")),
                row,
            )
        } else {
            (self.width(font, px, prefix), 0)
        }
    }
    fn rows(&self) -> usize {
        if self.multiline {
            self.value.bytes().filter(|b| *b == b'\n').count() + 1
        } else {
            1
        }
    }
    fn vertical_limit(&self, font: &Font, style: &Style, rect: Rect) -> f32 {
        if !self.multiline {
            return 0.;
        }
        (self.rows() as f32
            * font
                .line_metrics(style.number("font-size").unwrap_or(0.))
                .line_height()
            - input_content_rect(style, rect).h)
            .max(0.)
    }
    /// Preserve the prior viewport while keeping the insertion caret visible.
    pub fn fit_scroll(&mut self, font: &Font, style: &Style, rect: Rect) {
        let content = input_content_rect(style, rect);
        let px = style.number("font-size").unwrap_or(0.);
        let width = (content.w - style.number("caret-width").unwrap_or(0.).max(0.)).max(0.);
        let total = if self.multiline {
            self.value
                .split('\n')
                .map(|line| self.width(font, px, line))
                .fold(0., f32::max)
        } else {
            self.width(font, px, &self.value)
        };
        let (x, row) = self.cursor_position(font, px);
        self.scroll_x = fit_axis(self.scroll_x, x, width, total - width);
        let height = font.line_metrics(px).line_height();
        self.scroll_y = if self.multiline {
            fit_axis(
                self.scroll_y,
                row as f32 * height,
                (content.h - height).max(0.),
                self.vertical_limit(font, style, rect),
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
            input_content_rect(style, rect).h,
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
            input_content_rect(style, rect).y - self.scroll_y,
        )
    }
    /// Map both pointer axes through the current logical-line viewport.
    pub fn hit_index_at(&self, font: &Font, style: &Style, rect: Rect, x: f32, y: f32) -> usize {
        let px = style.number("font-size").unwrap_or(0.);
        let content = input_content_rect(style, rect);
        let height = font.line_metrics(px).line_height();
        let row = if self.multiline && height > 0. {
            ((y - content.y + self.scroll_y).max(0.) / height) as usize
        } else {
            0
        };
        let (start, line) = if self.multiline {
            let mut start = 0;
            let mut found = (0, "");
            for (n, line) in self.value.split('\n').enumerate() {
                found = (start, line);
                if n >= row {
                    break;
                }
                start += line.len() + 1;
            }
            found
        } else {
            (0, self.value.as_str())
        };
        let target = x - content.x + self.scroll_x;
        let mut pen = 0.;
        for (index, ch) in line.char_indices() {
            let next = pen + self.advance(font, px, ch);
            if target < (pen + next) / 2. {
                return start + index;
            }
            pen = next;
        }
        start + line.len()
    }
    pub(crate) fn paint(&self, font: &Font, style: &Style, rect: Rect, surface: &mut Surface) {
        let content = input_content_rect(style, rect);
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
        let prefix = &self.value[..cursor];
        let caret_row = if self.multiline {
            prefix.bytes().filter(|b| *b == b'\n').count()
        } else {
            0
        };
        let caret_prefix = if self.multiline {
            prefix.rsplit('\n').next().unwrap_or("")
        } else {
            prefix
        };
        let horizontal_limit = self.scroll_x + content.w;
        let (ink_min, ink_max) = font.horizontal_bounds_px(px);
        // Raster::render pads the outline by one pixel; integer placement and
        // floor/ceil can each move the bound by a fractional pixel. Two pixels
        // conservatively cover that quantization, independent of theme policy.
        let ink_min = ink_min - 2.;
        let ink_max = ink_max + 2.;
        surface.with_clip(content, |surface| {
            let mut start = 0;
            // split scans bytes in skipped lines, but performs no glyph work there.
            for (row, line) in text.split(|ch| self.multiline && ch == '\n').enumerate() {
                let end = start + line.len();
                if row < first {
                    start = end + 1;
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
                            if let Some(bitmap) = font.rasterize(glyph, px) {
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
                start = end + 1;
            }
            let caret_y = top + caret_row as f32 * height;
            if self.focused
                && self.caret_visible
                && caret_y < content.y + content.h
                && caret_y + height > content.y
            {
                let caret_x = self.paint_width(font, px, caret_prefix, horizontal_limit);
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
