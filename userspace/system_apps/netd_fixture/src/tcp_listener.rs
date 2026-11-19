//! Drive two fixture-only accepted streams without global socket authority.
use super::*;
use dunit_net_core::tcp::{
    table::{ListenerId, StreamId, Table},
    State, StreamError,
};

const PORT: u16 = 8080;

struct EchoStream {
    id: StreamId,
    pending: [u8; 128],
    start: usize,
    end: usize,
    sent: usize,
    shutdown: bool,
}

impl EchoStream {
    fn step(&mut self, table: &mut Table) -> Result<bool> {
        let ready = table.readiness(self.id).map_err(tcp_error)?;
        require(!ready.error, "passive TCP stream error")?;
        if self.start == self.end && !self.shutdown && ready.readable {
            self.start = 0;
            self.end = table.read(self.id, &mut self.pending).map_err(tcp_error)?;
            if self.end == 0 {
                table.shutdown_write(self.id).map_err(tcp_error)?;
                self.shutdown = true;
            }
        }
        if self.start < self.end && ready.writable {
            let count = table.send(self.id, &self.pending[self.start..self.end]).map_err(tcp_error)?;
            self.start += count;
            self.sent += count;
        }
        if table.state(self.id).map_err(tcp_error)? != State::Closed {
            return Ok(false);
        }
        require(self.shutdown && self.sent != 0, "passive TCP stream retired before echo/EOF")?;
        table.close(self.id).map_err(tcp_error)?;
        Ok(true)
    }
}

pub struct Echoes {
    listener: ListenerId,
    streams: [Option<EchoStream>; 2],
    accepted: usize,
    closed: usize,
}

impl Echoes {
    pub fn listen(table: &mut Table) -> Result<Self> {
        Ok(Self {
            listener: table.listen(PORT, 2).map_err(tcp_error)?,
            streams: [None, None],
            accepted: 0,
            closed: 0,
        })
    }

    pub fn step(&mut self, table: &mut Table) -> Result<()> {
        while table.listener_ready(self.listener).map_err(tcp_error)? {
            require(self.accepted < self.streams.len(), "unexpected third TCP accept")?;
            let id = table.accept(self.listener).map_err(tcp_error)?;
            self.streams[self.accepted] = Some(EchoStream {
                id, pending: [0; 128], start: 0, end: 0, sent: 0, shutdown: false,
            });
            self.accepted += 1;
        }
        for slot in &mut self.streams {
            if let Some(stream) = slot {
                if stream.step(table)? {
                    *slot = None;
                    self.closed += 1;
                }
            }
        }
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.closed == self.streams.len()
    }

    pub fn close(self, table: &mut Table) -> Result<()> {
        table.close_listener(self.listener).map_err(tcp_error)?;
        dunit::println!("[NETD-FIXTURE] TCP listener echoed two interleaved streams and closed")
    }
}

pub fn tcp_error(_: StreamError) -> Error {
    Error(errno::EIO)
}
