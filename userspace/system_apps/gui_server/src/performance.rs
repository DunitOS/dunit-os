//! Opt-in aggregate diagnostics, never a per-event serial trace.
//! Guest time is PIT-derived (10ms currently): interval sums are useful, but a
//! short syscall with IF masked can report zero. Host input-to-result timing
//! must accompany these counters; stage clocks are not hardware timestamps.
const REPORT_FRAMES: u64 = 128;
const STAGES: usize = 7;
#[derive(Clone, Copy)]
pub enum Stage {
    Work,
    Pump,
    Input,
    Compose,
    Present,
    FrameDone,
    Sleep,
}
#[derive(Default)]
struct Sample {
    frames: u64,
    elapsed: [u64; STAGES],
    max_work: u64,
    max_sleep: u64,
    pointer: u64,
    wheel: u64,
    lost: u64,
    keys: u64,
    ipc_rx: u64,
    ipc_qmax: u64,
    ready_max: u64,
    full: u64,
    partial: u64,
    blocked: u64,
    present_calls: u64,
    present_pixels: u64,
    clock_failures: u64,
}
pub struct Profiler {
    enabled: bool,
    sequence: u64,
    sample: Sample,
}
impl Profiler {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            sequence: 0,
            sample: Sample::default(),
        }
    }
    pub fn configure(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.enabled = enabled;
            self.sample = Sample::default();
        }
    }
    fn record(&mut self, stage: Stage, start: Option<u64>, end: Option<u64>) -> u64 {
        if !self.enabled {
            return 0;
        }
        match start.zip(end).and_then(|(a, b)| b.checked_sub(a)) {
            Some(delta) => {
                self.sample.elapsed[stage as usize] =
                    self.sample.elapsed[stage as usize].saturating_add(delta);
                match stage {
                    Stage::Work => self.sample.max_work = self.sample.max_work.max(delta),
                    Stage::Sleep => self.sample.max_sleep = self.sample.max_sleep.max(delta),
                    _ => {}
                }
                delta
            }
            None => {
                self.sample.clock_failures += 1;
                0
            }
        }
    }
    pub fn pointer(&mut self, wheel: i32, lost: bool) {
        if self.enabled {
            self.sample.pointer += 1;
            self.sample.wheel += u64::from(wheel != 0);
            self.sample.lost += u64::from(lost);
        }
    }
    pub fn key(&mut self) {
        if self.enabled {
            self.sample.keys += 1;
        }
    }
    pub fn ipc_packet(&mut self) {
        if self.enabled {
            self.sample.ipc_rx += 1;
        }
    }
    pub fn composition(&mut self, partial: bool, blocked: bool) {
        if self.enabled {
            self.sample.partial += u64::from(partial);
            self.sample.full += u64::from(!partial);
            self.sample.blocked += u64::from(blocked);
        }
    }
    fn presentation(&mut self, pixels: u64) {
        if self.enabled {
            self.sample.present_calls += 1;
            self.sample.present_pixels = self.sample.present_pixels.saturating_add(pixels);
        }
    }
    fn queue_sample(&mut self, queued: u64, ready: u64) {
        if self.enabled {
            self.sample.ipc_qmax = self.sample.ipc_qmax.max(queued);
            self.sample.ready_max = self.sample.ready_max.max(ready);
        }
    }
    fn frame_complete(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.sample.frames += 1;
        self.sample.frames >= REPORT_FRAMES
    }
    fn reset(&mut self) {
        self.sequence += 1;
        self.sample = Sample::default();
    }
}
#[cfg(not(test))]
impl Profiler {
    /// Disabled instrumentation performs no clock syscall.
    pub fn stamp(&self) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        let mut time = [0u64; 2];
        if libdunit::syscall2(
            libdunit::SYSCALL_CLOCK_GET_TIME,
            1,
            time.as_mut_ptr() as usize,
        ) < 0
        {
            return None;
        }
        time[0]
            .checked_mul(1000)
            .and_then(|ms| ms.checked_add(time[1] / 1_000_000))
    }
    pub fn finish(&mut self, stage: Stage, start: Option<u64>) {
        if self.enabled {
            self.record(stage, start, self.stamp());
        }
    }
    /// Composition includes partial presents; subtract the separately measured
    /// present intervals so compose_ms and present_ms are not double-counted.
    pub fn begin_compose(&self) -> (Option<u64>, u64) {
        (self.stamp(), self.sample.elapsed[Stage::Present as usize])
    }
    pub fn finish_compose(&mut self, mark: (Option<u64>, u64)) {
        if !self.enabled {
            return;
        }
        let total = self.record(Stage::Compose, mark.0, self.stamp());
        let present = self.sample.elapsed[Stage::Present as usize].saturating_sub(mark.1);
        self.sample.elapsed[Stage::Compose as usize] -= total.min(present);
    }
    pub fn present(&mut self, start: Option<u64>, pixels: u64) {
        if self.enabled {
            self.finish(Stage::Present, start);
            self.presentation(pixels);
        }
    }
    /// Snapshots are global IPC backlog, sampled at frame boundaries, not an
    /// exact per-client queue peak. Pointer queue depth is not exposed by ABI.
    pub fn queues(&mut self) {
        if !self.enabled {
            return;
        }
        let mut stats = libdunit::SystemStats::default();
        if libdunit::get_system_stats(&mut stats) >= 0 {
            self.queue_sample(stats.ipc_queued_messages, stats.process_ready);
        }
    }
    pub fn frame(&mut self) {
        if !self.frame_complete() {
            return;
        }
        self.emit();
    }
    fn emit(&mut self) {
        let s = &self.sample;
        libdunit::println(&alloc::format!(
            "[PERF-SERVER] seq={} frames={} work_ms={} pump_ms={} input_ms={} compose_ms={} present_ms={} done_ms={} sleep_ms={} max_work_ms={} max_sleep_ms={} pointer={} wheel={} lost={} keys={} ipc_rx={} ipc_qmax={} ready_max={} full={} partial={} blocked={} present_calls={} present_pixels={} clock_failures={}",
            self.sequence,s.frames,s.elapsed[Stage::Work as usize],s.elapsed[Stage::Pump as usize],s.elapsed[Stage::Input as usize],s.elapsed[Stage::Compose as usize],s.elapsed[Stage::Present as usize],s.elapsed[Stage::FrameDone as usize],s.elapsed[Stage::Sleep as usize],s.max_work,s.max_sleep,s.pointer,s.wheel,s.lost,s.keys,s.ipc_rx,s.ipc_qmax,s.ready_max,s.full,s.partial,s.blocked,s.present_calls,s.present_pixels,s.clock_failures));
        self.reset();
    }
    pub fn flush(&mut self) {
        if self.enabled && self.sample.frames > 0 {
            self.emit();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_collection_has_no_activity_or_ready_report() {
        let mut p = Profiler::new(false);
        p.pointer(2, true);
        p.presentation(80);
        p.key();
        p.ipc_packet();
        p.composition(false, true);
        p.queue_sample(9, 2);
        p.record(Stage::Work, Some(0), Some(100));
        for _ in 0..REPORT_FRAMES * 2 {
            assert!(!p.frame_complete());
        }
        assert_eq!((p.sample.present_calls, p.sample.present_pixels), (0, 0));
        assert_eq!(
            (
                p.sample.pointer,
                p.sample.keys,
                p.sample.elapsed[0],
                p.sample.ipc_qmax
            ),
            (0, 0, 0, 0)
        );
    }
    #[test]
    fn window_keeps_all_event_counts_and_separate_maxima_until_the_bound() {
        let mut p = Profiler::new(true);
        p.pointer(1, false);
        p.pointer(-1, true);
        p.key();
        p.key();
        p.ipc_packet();
        p.composition(false, true);
        p.composition(true, false);
        p.presentation(200);
        p.presentation(30);
        p.queue_sample(8, 3);
        p.queue_sample(2, 6);
        p.record(Stage::Work, Some(0), Some(30));
        p.record(Stage::Work, Some(30), Some(40));
        p.record(Stage::Sleep, Some(40), Some(60));
        for _ in 0..REPORT_FRAMES - 1 {
            assert!(!p.frame_complete());
        }
        assert!(p.frame_complete());
        assert_eq!(p.sample.pointer, 2);
        assert_eq!((p.sample.present_calls, p.sample.present_pixels), (2, 230));
        assert_eq!(p.sample.wheel, 2);
        assert_eq!(p.sample.lost, 1);
        assert_eq!(
            (p.sample.full, p.sample.partial, p.sample.blocked),
            (1, 1, 1)
        );
        assert_eq!((p.sample.ipc_qmax, p.sample.ready_max), (8, 6));
        assert_eq!(
            (p.sample.elapsed[0], p.sample.max_work, p.sample.max_sleep),
            (40, 30, 20)
        );
        p.reset();
        assert_eq!(p.sequence, 1);
        assert_eq!(p.sample.frames, 0);
        assert_eq!(p.sample.pointer, 0);
    }
    #[test]
    fn invalid_clock_is_reported_and_enabling_starts_a_clean_window() {
        let mut p = Profiler::new(true);
        p.record(Stage::Input, None, Some(20));
        p.record(Stage::Present, Some(20), Some(10));
        assert_eq!(p.sample.clock_failures, 2);
        assert_eq!(p.sample.elapsed[Stage::Present as usize], 0);
        p.configure(false);
        assert_eq!(p.sample.clock_failures, 0);
        p.configure(true);
        p.record(Stage::Pump, Some(10), Some(20));
        assert_eq!(p.sample.elapsed[Stage::Pump as usize], 10);
        p.record(Stage::Compose, Some(20), Some(20));
        p.record(Stage::FrameDone, Some(20), Some(20));
    }
}
