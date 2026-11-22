//! Bounded stream operations for IRQ-safe tables. Capacity is prepared by the
//! caller outside its guard, then rechecked against the current ring contents.
use alloc::collections::VecDeque;

/// Returns the accepted byte count or the capacity to prepare. Failure leaves
/// the ring untouched. On growth, `spare` receives the replaced allocation.
pub(crate) fn push(
    ring: &mut VecDeque<u8>,
    data: &[u8],
    limit: usize,
    spare: &mut VecDeque<u8>,
) -> Result<usize, usize> {
    let count = (limit - ring.len()).min(data.len());
    let required = ring.len() + count;
    if required > ring.capacity() {
        if required > spare.capacity() {
            return Err((ring.capacity() * 2).max(required).min(limit));
        }
        let (head, tail) = ring.as_slices();
        spare.extend(head);
        spare.extend(tail);
        core::mem::swap(ring, spare);
    }
    ring.extend(&data[..count]);
    Ok(count)
}

pub(crate) fn pop(ring: &mut VecDeque<u8>, out: &mut [u8]) -> usize {
    let count = ring.len().min(out.len());
    let (head, tail) = ring.as_slices();
    let head_len = head.len().min(count);
    out[..head_len].copy_from_slice(&head[..head_len]);
    out[head_len..count].copy_from_slice(&tail[..count - head_len]);
    drop(ring.drain(..count));
    count
}
