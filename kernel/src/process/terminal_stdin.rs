/// One terminal line, retained until the foreground process consumes it fully.
pub(super) struct TerminalStdin {
    owner: Option<u64>,
    buffer: [u8; 256],
    len: usize,
    ready: bool,
}

impl TerminalStdin {
    pub(super) const fn new() -> Self {
        Self {
            owner: None,
            buffer: [0; 256],
            len: 0,
            ready: false,
        }
    }

    pub(super) fn clear(&mut self) {
        self.owner = None;
        self.len = 0;
        self.ready = false;
    }

    /// A competing reader retries after the current owner drains the line.
    pub(super) fn request(&mut self, pid: u64) {
        if self.owner.is_none() {
            self.owner = Some(pid);
        }
    }

    pub(super) fn waiting_pid(&self) -> Option<u64> {
        self.owner.filter(|_| !self.ready)
    }

    pub(super) fn provide(&mut self, pid: u64, data: &[u8]) -> bool {
        if self.owner != Some(pid) || self.ready {
            return false;
        }
        self.len = data.len().min(self.buffer.len());
        self.buffer[..self.len].copy_from_slice(&data[..self.len]);
        self.ready = true;
        true
    }

    pub(super) fn is_ready(&self, pid: u64) -> bool {
        self.owner == Some(pid) && self.ready
    }

    pub(super) fn take(&mut self, pid: u64, out: &mut [u8]) -> Option<usize> {
        if !self.is_ready(pid) {
            return None;
        }
        let read = self.len.min(out.len());
        out[..read].copy_from_slice(&self.buffer[..read]);
        self.buffer.copy_within(read..self.len, 0);
        self.len -= read;
        if self.len == 0 {
            self.ready = false;
            self.owner = None;
        }
        Some(read)
    }
}
