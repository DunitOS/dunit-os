//! One byte-preserving visual-line map for text painting and navigation.
use alloc::{sync::Arc, vec::Vec};
use dunit_text::Font;
use unicode_segmentation::UnicodeSegmentation;

/// A soft line break has two visual caret positions at the same byte offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaretAffinity {
    Upstream,
    #[default]
    Downstream,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextPosition {
    pub index: usize,
    pub affinity: CaretAffinity,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VisualLine {
    pub start: usize,
    pub end: usize,
    pub width: f32,
    pub soft_break: bool,
    // Byte boundaries and their exact unkerned glyph advances. This is the
    // same advance model used by TextInput painting, including password masks.
    stops: Arc<Vec<(usize, f32)>>,
}
impl VisualLine {
    pub fn x(&self, index: usize) -> f32 {
        let end = self.stops.partition_point(|(byte, _)| *byte <= index);
        if end == 0 { 0. } else { self.stops[end - 1].1 }
    }
    pub fn hit(&self, x: f32) -> TextPosition {
        for pair in self.stops.windows(2) {
            if x < (pair[0].1 + pair[1].1) / 2. {
                return TextPosition {
                    index: pair[0].0,
                    affinity: CaretAffinity::Downstream,
                };
            }
        }
        TextPosition {
            index: self.end,
            affinity: if self.soft_break {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct VisualLines {
    pub lines: Vec<VisualLine>,
}
struct Cluster {
    start: usize,
    end: usize,
    width: f32,
    whitespace: bool,
}
impl VisualLines {
    pub fn new(
        value: &str,
        font: &Font,
        px: f32,
        width: f32,
        multiline: bool,
        wrap: bool,
        password: bool,
    ) -> Self {
        Self::build(value, font, px, width, multiline, wrap, password, &[])
    }
    /// Retain complete logical lines whose bytes and following newline match.
    /// The caller has already checked the font and line layout parameters.
    pub(crate) fn with_previous(
        value: &str,
        font: &Font,
        px: f32,
        width: f32,
        wrap: bool,
        password: bool,
        previous_value: &str,
        previous: &Self,
    ) -> Self {
        let common = value
            .bytes()
            .zip(previous_value.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let prefix = value.as_bytes()[..common]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(&[][..], |newline| {
                let rows = previous.lines.partition_point(|line| line.end <= newline);
                &previous.lines[..rows]
            });
        Self::build(value, font, px, width, true, wrap, password, prefix)
    }
    fn build(
        value: &str,
        font: &Font,
        px: f32,
        width: f32,
        multiline: bool,
        wrap: bool,
        password: bool,
        prefix: &[VisualLine],
    ) -> Self {
        let width = if width.is_finite() { width.max(0.) } else { 0. };
        let scale = font.scale_for_px(px);
        let measure = |ch| font.glyph_metrics(font.glyph_index(ch)).advance as f32 * scale;
        // The same font and scale apply to this entire map. Repeated ASCII
        // scalars need one cmap/metrics lookup, even across hard line breaks.
        let mut ascii = [None; 128];
        let mask = password.then(|| measure('•'));
        let mut cluster_width = |cluster: &str| {
            cluster.chars().map(|ch| {
                if let Some(advance) = mask {
                    advance
                } else if ch.is_ascii() {
                    *ascii[ch as usize].get_or_insert_with(|| measure(ch))
                } else {
                    measure(ch)
                }
            }).sum::<f32>()
        };
        let mut lines = Vec::with_capacity(prefix.len() + 1);
        lines.extend_from_slice(prefix);
        let mut clusters = Vec::new();
        let mut logical_start = prefix.last().map_or(0, |line| line.end + 1);
        let wrap = wrap && multiline;
        for logical in value[logical_start..].split(|ch| multiline && ch == '\n') {
            if !wrap {
                let mut stops = alloc::vec![(logical_start, 0.)];
                let mut x = 0.;
                for (byte, cluster) in logical.grapheme_indices(true) {
                    x += cluster_width(cluster);
                    stops.push((logical_start + byte + cluster.len(), x));
                }
                lines.push(VisualLine {
                    start: logical_start,
                    end: logical_start + logical.len(),
                    width: x,
                    soft_break: false,
                    stops: Arc::new(stops),
                });
                logical_start += logical.len() + 1;
                continue;
            }

            clusters.clear();
            clusters.extend(logical
                .grapheme_indices(true)
                .map(|(i, g)| Cluster {
                    start: logical_start + i,
                    end: logical_start + i + g.len(),
                    width: cluster_width(g),
                    whitespace: g.chars().all(char::is_whitespace),
                }));
            if clusters.is_empty() {
                lines.push(VisualLine {
                    start: logical_start,
                    end: logical_start,
                    width: 0.,
                    soft_break: false,
                    stops: Arc::new(alloc::vec![(logical_start, 0.)]),
                });
            }
            let mut start = 0;
            while start < clusters.len() {
                let mut end = start;
                let mut used = 0.;
                let mut word_break = None;
                while end < clusters.len() {
                    let cluster = &clusters[end];
                    if end > start && used + cluster.width > width {
                        break;
                    }
                    used += cluster.width;
                    end += 1;
                    if cluster.whitespace {
                        word_break = Some(end);
                    }
                }
                if end < clusters.len() {
                    if let Some(at) = word_break.filter(|at| *at > start) {
                        end = at;
                    }
                }
                let mut stops = alloc::vec![(clusters[start].start, 0.)];
                let mut x = 0.;
                for cluster in &clusters[start..end] {
                    x += cluster.width;
                    stops.push((cluster.end, x));
                }
                lines.push(VisualLine {
                    start: clusters[start].start,
                    end: clusters[end - 1].end,
                    width: x,
                    soft_break: end < clusters.len(),
                    stops: Arc::new(stops),
                });
                start = end;
            }
            logical_start += logical.len() + 1;
        }
        Self { lines }
    }
    pub fn row(&self, position: TextPosition) -> usize {
        for (i, line) in self.lines.iter().enumerate() {
            if position.index < line.end
                || position.index == line.end
                    && (!line.soft_break || position.affinity == CaretAffinity::Upstream)
            {
                return i;
            }
        }
        self.lines.len().saturating_sub(1)
    }
    pub fn position(&self, position: TextPosition) -> (f32, usize) {
        let row = self.row(position);
        (self.lines[row].x(position.index), row)
    }
    pub fn hit(&self, row: usize, x: f32) -> TextPosition {
        self.lines[row.min(self.lines.len() - 1)].hit(x)
    }
    pub fn max_width(&self) -> f32 {
        self.lines.iter().map(|l| l.width).fold(0., f32::max)
    }
}
