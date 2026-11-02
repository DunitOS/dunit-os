//! Visual text navigation uses the last rendered line map and editor identity.
use super::*;
use crate::{Application, Component, EditCommand};
use dunit_render::TextPosition;
impl Renderer {
    pub fn input_position_at(&self, key: &str, x: f32, y: f32) -> Option<TextPosition> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        Some(p.inputs[id.0 as usize].as_ref()?.hit_position_with_lines(
            &self.font,
            &p.styles[id.0 as usize],
            p.layout.rect(id),
            x,
            y,
            p.input_layouts[id.0 as usize].as_ref()?.lines(),
        ))
    }
    /// Route a raw editing command. Wrapped vertical/line movement uses pixels
    /// and visual affinity; all text mutations still use the shared editor.
    /// A stale render snapshot falls back to logical editor navigation.
    pub fn edit_input<C: Component>(
        &self,
        app: &mut Application<C>,
        command: EditCommand,
    ) -> crate::Result<bool> {
        let Some(key) = app.focused_input_key().map(String::from) else {
            return app.edit(command);
        };
        let target = (|| {
            let v = app.view.find(&key)?;
            let binding = v.input.as_ref()?;
            if !binding.options.soft_wrap {
                return None;
            }
            let p = self.previous.as_ref()?;
            let id = p.tree.by_name(&key)?;
            let input = p.inputs[id.0 as usize].as_ref()?;
            if input.value != binding.editor.text()
                || input.cursor != binding.editor.cursor()
                || input.affinity != binding.affinity
            {
                return None;
            }
            input.visual_navigation_with_lines(
                command,
                binding.visual_goal,
                p.input_layouts[id.0 as usize].as_ref()?.lines(),
            )
        })();
        if let Some((position, goal)) = target {
            let extend = match command {
                EditCommand::Up { extend }
                | EditCommand::Down { extend }
                | EditCommand::Home { extend }
                | EditCommand::End { extend } => extend,
                _ => false,
            };
            return Ok(app.place_visual_input(&key, position, extend, goal));
        }
        app.edit(command)
    }
}
