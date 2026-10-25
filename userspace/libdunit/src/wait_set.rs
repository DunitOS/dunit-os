//! Safe, non-consuming native wait-set interface. Readiness is level-triggered.
pub use dunit_wait_abi::*;

/// Wait for interests, returning every ready record in registration order.
/// `timeout_ns` is relative; zero snapshots, INFINITE has no overall deadline.
/// Cancellation uses an ordinary message to a registered own-inbox interest.
pub fn wait(interests: &[Interest], output: &mut [Event], timeout_ns: u64) -> Result<usize, isize> {
    if interests.len() > MAX_INTERESTS
        || output.len() < interests.len()
        || output.len() > MAX_INTERESTS
    {
        return Err(crate::EINVAL);
    }
    let result = crate::syscall6(
        crate::SYSCALL_WAIT_SET_WAIT,
        interests.as_ptr() as usize,
        interests.len(),
        output.as_mut_ptr() as usize,
        output.len(),
        timeout_ns as usize,
        0,
    );
    if result < 0 {
        Err(result)
    } else {
        Ok(result as usize)
    }
}
