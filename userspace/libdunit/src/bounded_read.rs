use alloc::vec::Vec;
use crate::{EFBIG, EIO};

pub(crate) fn read(cap: usize, mut transfer: impl FnMut(&mut [u8]) -> isize) -> Result<Vec<u8>, isize> {
    let mut data = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        // One extra byte distinguishes exact-cap EOF from an oversized file.
        let remaining = cap - data.len();
        let count = chunk.len().min(remaining.saturating_add(1));
        let received = transfer(&mut chunk[..count]);
        if received < 0 {
            return Err(received);
        }
        let received = received as usize;
        if received > count {
            return Err(EIO);
        }
        if received == 0 {
            return Ok(data);
        }
        if received > remaining {
            return Err(EFBIG);
        }
        data.extend_from_slice(&chunk[..received]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(bytes: &[u8], cap: usize, step: usize) -> Result<Vec<u8>, isize> {
        let mut position = 0;
        read(cap, |buffer| {
            let count = buffer.len().min(step).min(bytes.len() - position);
            buffer[..count].copy_from_slice(&bytes[position..position + count]);
            position += count;
            count as isize
        })
    }

    #[test]
    fn accepts_complete_exact_cap_and_empty_files_with_short_reads() {
        let bytes = [7; 4097];
        assert_eq!(source(&bytes, bytes.len(), 3).unwrap(), bytes);
        assert_eq!(source(&[], 0, 3).unwrap(), []);
    }

    #[test]
    fn rejects_cap_plus_one_instead_of_returning_a_prefix() {
        assert_eq!(source(&[7; 4097], 4096, 4096), Err(EFBIG));
        assert_eq!(source(&[1], 0, 3), Err(EFBIG));
        let valid_prefix = b"Button { fill: #ffffff; }";
        let mut bytes = valid_prefix.to_vec();
        bytes.extend_from_slice(b"invalid tail");
        assert_eq!(source(&bytes, valid_prefix.len(), 3), Err(EFBIG));
    }

    #[test]
    fn preserves_a_read_error_after_successful_data() {
        let mut calls = 0;
        assert_eq!(read(10, |buffer| {
            calls += 1;
            if calls == 1 { buffer[0] = 7; 1 } else { EIO }
        }), Err(EIO));
        assert_eq!(calls, 2);
    }

    #[test]
    fn rejects_an_invalid_transfer_count() {
        assert_eq!(read(2, |buffer| buffer.len() as isize + 1), Err(EIO));
    }
}
