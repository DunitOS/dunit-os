//! Animation progress uses elapsed monotonic milliseconds, independent of frames.
pub fn progress(now: u64, start: u64, duration_ms: u32, ease_out: bool) -> u32 {
    let elapsed = now.saturating_sub(start);
    let linear = if duration_ms == 0 || elapsed >= duration_ms as u64 {
        255
    } else {
        (elapsed * 255 / duration_ms as u64) as u32
    };
    if ease_out { 255 - (255 - linear) * (255 - linear) / 255 } else { linear }
}

#[cfg(test)]
mod tests {
    use super::progress;
    #[test]
    fn dropped_frames_do_not_extend_duration() {
        assert_eq!(progress(1000, 1000, 140, false), 0);
        assert_eq!(progress(1070, 1000, 140, false), 127);
        assert_eq!(progress(1200, 1000, 140, false), 255);
        assert_eq!(progress(1000, 1000, 0, true), 255);
        assert_eq!(progress(999, 1000, 140, false), 0);
        assert_eq!(progress(u64::MAX, 0, 2000, true), 255);
    }
    #[test]
    fn easing_preserves_endpoints_and_is_monotonic() {
        let mut previous = 0;
        for elapsed in 0..=180 {
            let value = progress(elapsed, 0, 180, true);
            assert!(value >= previous && value <= 255);
            previous = value;
        }
        assert_eq!(progress(0, 0, 180, true), 0);
        assert_eq!(previous, 255);
    }
}
