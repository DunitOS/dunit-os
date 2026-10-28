//! Single-line input geometry and painting. Editing remains in the widget layer.
use crate::Surface;
use alloc::string::String;
use dunit_style::Style;
use dunit_text::Font;
use dunit_ui::Rect;

/// A render snapshot; offsets refer to UTF-8 byte boundaries in `value`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInput {
    pub value: String,
    pub placeholder: String,
    pub password: bool,
    pub cursor: usize,
    pub anchor: usize,
    pub focused: bool,
    pub caret_visible: bool,
    pub scroll_x: f32,
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
    fn position(&self, font: &Font, px: f32, index: usize) -> f32 {
        self.value[..self.boundary(index)]
            .chars()
            .map(|ch| self.advance(font, px, ch))
            .sum()
    }
    /// Preserve the prior viewport while keeping the insertion caret visible.
    pub fn fit_scroll(&mut self, font: &Font, style: &Style, rect: Rect) {
        let content = input_content_rect(style, rect);
        let px = style.number("font-size").unwrap_or(0.);
        let caret = style.number("caret-width").unwrap_or(0.).max(0.);
        let width = (content.w - caret).max(0.);
        let total = self.position(font, px, self.value.len());
        let cursor = self.position(font, px, self.cursor);
        let mut scroll = if self.scroll_x.is_finite() {
            self.scroll_x.max(0.)
        } else {
            0.
        };
        if cursor < scroll {
            scroll = cursor;
        }
        if cursor > scroll + width {
            scroll = cursor - width;
        }
        self.scroll_x = scroll.min((total - width).max(0.));
    }
    /// Nearest scalar boundary, in the displayed (possibly masked) line.
    pub fn hit_index(&self, font: &Font, style: &Style, rect: Rect, x: f32) -> usize {
        let px = style.number("font-size").unwrap_or(0.);
        let target = x - input_content_rect(style, rect).x + self.scroll_x;
        let mut pen = 0.;
        for (index, ch) in self.value.char_indices() {
            let next = pen + self.advance(font, px, ch);
            if target < (pen + next) / 2. {
                return index;
            }
            pen = next;
        }
        self.value.len()
    }
    pub(crate) fn paint(&self, font: &Font, style: &Style, rect: Rect, surface: &mut Surface) {
        let content = input_content_rect(style, rect);
        let px = style.number("font-size").unwrap_or(0.);
        let metrics = font.line_metrics(px);
        let top = content.y + (content.h - metrics.line_height()).max(0.) / 2.;
        let baseline = top + metrics.ascent;
        let start = self.position(font, px, self.anchor.min(self.cursor));
        let end = self.position(font, px, self.anchor.max(self.cursor));
        let origin = content.x - self.scroll_x;
        surface.with_clip(content, |surface| {
            if self.focused && start != end {
                if let Some(color) = style.color("selection-color") {
                    surface.fill_rect(
                        origin + start,
                        top,
                        end - start,
                        metrics.line_height(),
                        color,
                    );
                }
            }
            let placeholder = self.value.is_empty();
            let color = style.color(if placeholder {
                "placeholder-color"
            } else {
                "color"
            });
            if let Some(color) = color {
                let text = if placeholder {
                    &self.placeholder
                } else {
                    &self.value
                };
                let mut pen = origin;
                for ch in text.chars() {
                    let displayed = if self.password && !placeholder {
                        '•'
                    } else {
                        ch
                    };
                    let glyph = font.glyph_index(displayed);
                    if let Some(bitmap) = font.rasterize(glyph, px) {
                        // Signed rounding is essential for partially visible scrolled glyphs.
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
                            round(baseline) - bitmap.top,
                            color,
                        );
                    }
                    pen += font.glyph_metrics(glyph).advance as f32 * font.scale_for_px(px);
                }
            }
            if self.focused && self.caret_visible {
                if let Some(color) = style.color("caret-color") {
                    surface.fill_rect(
                        origin + self.position(font, px, self.cursor),
                        top,
                        style.number("caret-width").unwrap_or(0.).max(0.),
                        metrics.line_height(),
                        color,
                    );
                }
            }
        });
    }
}
