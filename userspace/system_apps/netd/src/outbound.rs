//! One bounded transmit backlog. A slow device never stalls RX or lease timers.
use alloc::{vec, vec::Vec};
use dunit::{errno, Error, Result};
use dunit_packet_abi::{encode_record, record_len, Records, MAX_BATCH_LEN, MAX_RECORDS};
use netd::{wire_error, Queue};

pub struct Outbound {
    bytes: Vec<u8>,
    used: usize,
    records: usize,
}

impl Outbound {
    pub fn new() -> Self {
        Self {
            bytes: vec![0; MAX_BATCH_LEN],
            used: 0,
            records: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    pub fn clear(&mut self) {
        self.used = 0;
        self.records = 0;
    }

    /// A full backlog drops subsequent frames. DHCP keeps its own retry timer.
    pub fn push(&mut self, frame: &[u8]) -> Result<()> {
        self.try_push(frame).map(|_| ())
    }

    pub fn try_push(&mut self, frame: &[u8]) -> Result<bool> {
        let length = record_len(frame.len()).map_err(wire_error)?;
        if self.records == MAX_RECORDS || length > self.bytes.len() - self.used {
            return Ok(false);
        }
        self.used += encode_record(frame, &mut self.bytes[self.used..]).map_err(wire_error)?;
        self.records += 1;
        Ok(true)
    }

    pub fn flush(&mut self, queue: &Queue) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        match queue.try_send(&self.bytes[..self.used]) {
            Ok(count) if (1..=self.used).contains(&count) => {
                let accepted = Records::new(&self.bytes[..count])
                    .map_err(wire_error)?
                    .count();
                self.bytes.copy_within(count..self.used, 0);
                self.used -= count;
                self.records -= accepted;
                Ok(())
            }
            Ok(_) => Err(Error(errno::EIO)),
            Err(Error(errno::EAGAIN)) => Ok(()),
            Err(error) => Err(error),
        }
    }
}
