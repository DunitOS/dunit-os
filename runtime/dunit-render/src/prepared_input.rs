//! Immutable input geometry prepared for one resolved text/font/style/width.
use crate::VisualLines;
/// Shared by caret fitting, extents, hit testing and painting. Multiline edits
/// can retain the complete leading lines while font and wrapping geometry match.
/// Cursor, selection, focus and scroll do not affect the map.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedInput {
    pub(crate) value: VisualLines,
    pub(crate) placeholder: Option<VisualLines>,
}
impl PreparedInput {
    pub fn lines(&self) -> &VisualLines {
        &self.value
    }
}
