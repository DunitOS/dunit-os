//! Immutable input geometry prepared for one resolved text/font/style/width.
use crate::VisualLines;
/// Shared by caret fitting, extents, hit testing and painting in one snapshot.
/// Rebuild when text, placeholder, font, font size, content width, wrapping or
/// password mode changes. Cursor, selection, focus and scroll do not affect it.
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
