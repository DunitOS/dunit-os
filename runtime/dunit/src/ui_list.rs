//! Raw collection keys use the same controlled selection as host callers.
use super::*;
impl<C: Component> ComponentWindow<C>
where
    C::Message: Clone,
{
    pub fn list_geometry(&self, key: &str) -> Option<ListGeometry> {
        self.renderer.list_geometry(key)
    }
    pub fn list_visible_rows(&self, key: &str) -> Option<usize> {
        self.renderer.list_visible_rows(key)
    }
    pub(super) fn list_event(&mut self, event: &Option<WindowEvent>) -> NativeResult<bool> {
        let Some(WindowEvent::Input(input)) = event else {
            return Ok(false);
        };
        if input.kind != IN_KEY || input.value != 0 || input.y & !3 != 0 {
            return Ok(false);
        }
        if input.y == 0
            && input.x == 0x3e
            && self
                .application
                .table_command(TableCommand::ToggleSort)
                .map_err(native)?
        {
            self.redraw()?;
            return Ok(true);
        }
        if input.y == 0 && input.x == 0x3c && self.application.begin_rename().map_err(native)? {
            self.redraw()?;
            return Ok(true);
        }
        if input.y == 0 {
            let command = match input.x {
                0x4b => Some(TableCommand::PreviousColumn),
                0x4d => Some(TableCommand::NextColumn),
                _ => None,
            };
            if let Some(command) = command {
                if self.application.table_command(command).map_err(native)? {
                    self.redraw()?;
                    return Ok(true);
                }
            }
        }
        if input.y == 0 {
            let tree = match input.x {
                0x4b => Some(TreeCommand::Collapse),
                0x4d => Some(TreeCommand::Expand),
                _ => None,
            };
            if let Some(command) = tree {
                if self.application.tree_command(command).map_err(native)? {
                    self.redraw()?;
                    return Ok(true);
                }
            }
        }
        let command = match input.x {
            0x48 => ListCommand::Previous,
            0x50 => ListCommand::Next,
            0x47 => ListCommand::First,
            0x4f => ListCommand::Last,
            0x49 => ListCommand::PageUp,
            0x51 => ListCommand::PageDown,
            _ => return Ok(false),
        };
        let page = self
            .application
            .focused_key()
            .and_then(|k| self.renderer.list_geometry(k))
            .map_or(1, |g| g.page());
        let consumed = self
            .application
            .list_command(command, input.y as u8, page)
            .map_err(native)?;
        if consumed {
            self.redraw()?;
        }
        Ok(consumed)
    }
}
