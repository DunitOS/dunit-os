/// One terminal line, retained until the foreground process consumes it fully.
pub(super) struct TerminalStdin {
    buffer: [u8; 256],
    len: usize,
    ready: bool,
}

impl TerminalStdin {
    pub(super) const fn new() -> Self {
        Self {
            buffer: [0; 256],
            len: 0,
            ready: false,
        }
    }

    pub(super) fn clear(&mut self) {
        self.len = 0;
        self.ready = false;
    }

    pub(super) fn provide(&mut self, data: &[u8]) {
        self.len = data.len().min(self.buffer.len());
        self.buffer[..self.len].copy_from_slice(&data[..self.len]);
        self.ready = true;
    }

    pub(super) fn is_ready(&self) -> bool {
        self.ready
    }

    pub(super) fn take(&mut self, out: &mut [u8]) -> Option<usize> {
        if !self.ready {
            return None;
        }
        let read = self.len.min(out.len());
        out[..read].copy_from_slice(&self.buffer[..read]);
        self.buffer.copy_within(read..self.len, 0);
        self.len -= read;
        if self.len == 0 {
            self.ready = false;
        }
        Some(read)
    }
}
