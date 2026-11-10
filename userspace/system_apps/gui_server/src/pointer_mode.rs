//! Pure seat policy; kernel records remain device facts.
use gui_protocol_v1::control::PointerMode;
#[derive(Default)]
pub struct State {
    pub owner: Option<u32>,
    pub mode: PointerMode,
    pub position: (i32, i32),
    pub relative: bool,
    projected: bool,
}
impl State {
    pub fn release(&mut self) -> Option<u32> {
        let owner = self.owner.take();
        self.mode = PointerMode::Normal;
        owner
    }
    pub fn request(
        &mut self,
        pid: u32,
        focused: Option<u32>,
        mode: PointerMode,
    ) -> Result<(), isize> {
        if mode == PointerMode::Normal {
            if self.owner.is_some() && self.owner != Some(pid) {
                return Err(libdunit::EACCES);
            }
            self.release();
            return Ok(());
        }
        if focused != Some(pid) {
            return Err(libdunit::EACCES);
        }
        if self.owner.is_some() && self.owner != Some(pid) {
            return Err(libdunit::EBUSY);
        }
        if mode == PointerMode::Locked && !self.relative {
            return Err(libdunit::EOPNOTSUPP);
        }
        self.owner = Some(pid);
        self.mode = mode;
        self.projected = true;
        Ok(())
    }
    pub fn reconcile(&mut self, focused: Option<u32>) -> Option<u32> {
        if self.owner.is_some() && self.owner != focused {
            self.release()
        } else {
            None
        }
    }
    pub fn sample(
        &mut self,
        absolute: (i32, i32),
        delta: (i32, i32),
        relative: bool,
        bounds: (i32, i32, i32, i32),
    ) -> (i32, i32) {
        self.relative = relative;
        if self.mode != PointerMode::Locked {
            self.position = if relative && self.projected {
                (
                    self.position.0.saturating_add(delta.0),
                    self.position.1.saturating_add(delta.1),
                )
            } else {
                absolute
            };
        }
        self.bound(bounds);
        self.position
    }
    pub fn bound(&mut self, (x, y, w, h): (i32, i32, i32, i32)) {
        self.position.0 = self.position.0.clamp(x, x.saturating_add(w.max(1) - 1));
        self.position.1 = self.position.1.clamp(y, y.saturating_add(h.max(1) - 1));
    }
}
