use super::Renderer;
use crate::TextClickConfig;
impl Renderer {
    /// Policy from the last successfully painted DSS state. Missing values
    /// disable multi-click classification while preserving ordinary selection.
    pub fn input_click_config(&self, key: &str) -> Option<TextClickConfig> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        p.inputs[id.0 as usize].as_ref()?;
        let style = &p.styles[id.0 as usize];
        let interval = style.get("input-multiclick-interval")?.as_duration()?;
        let distance = style.number("input-multiclick-distance")?;
        if !interval.is_finite() || interval < 0. || !distance.is_finite() || distance < 0. {
            return None;
        }
        Some(TextClickConfig {
            interval_ms: interval as u64,
            distance,
        })
    }
}
