//! Opt-in client-side diagnostics. These are elapsed monotonic durations, not
//! CPU cycles or physical input-to-display latency. The current kernel clock is
//! tick-granular; IRQ-masked work can be undercounted. Aggregate a sustained burst
//! and compare the same accelerator, workload and instrumentation setting.
use crate::time::Instant;

/// Completed timing samples, with saturating totals. Zero-duration samples are
/// valid on the tick-granular clock; clock failures never alter UI behavior.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MetricTime {
    pub samples: u64,
    pub total_ns: u64,
    pub max_ns: u64,
    pub clock_failures: u64,
}
impl MetricTime {
    pub(crate) fn finish(&mut self, enabled: bool, start: Option<Instant>) {
        if !enabled {
            return;
        }
        if let Some(elapsed) = start.and_then(|start| start.elapsed().ok()) {
            let ns = elapsed.as_nanos().min(u64::MAX as u128) as u64;
            self.samples = self.samples.saturating_add(1);
            self.total_ns = self.total_ns.saturating_add(ns);
            self.max_ns = self.max_ns.max(ns);
        } else {
            self.clock_failures = self.clock_failures.saturating_add(1);
        }
    }
}
pub(crate) fn start(enabled: bool) -> Option<Instant> {
    if enabled {
        Instant::now().ok()
    } else {
        None
    }
}

/// Window transport counters. Input queue depth counts packets already received
/// by this process; it does not include compositor/kernel IPC queues. Timings
/// for transport stages count completed stages; draw_total also includes errors
/// and bounded recovery. FrameDone is a compositor notification, not scanout.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WindowMetrics {
    pub input_queued: usize,
    pub protocol_queued: usize,
    pub input_received: u64,
    pub input_delivered: u64,
    pub input_queue_high_water: usize,
    pub protocol_queue_high_water: usize,
    pub commits: u64,
    pub completed_frames: u64,
    pub draw_total: MetricTime,
    pub import_attach: MetricTime,
    pub commit_result: MetricTime,
    pub frame_done: MetricTime,
    pub retire: MetricTime,
}

/// Component render and shared-buffer-copy elapsed durations are separate from
/// nested window transaction timings. They can include scheduler preemption;
/// they are not exclusive CPU times. No samples are printed automatically.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ComponentMetrics {
    pub redraw_calls: u64,
    pub render_calls: u64,
    pub render: MetricTime,
    pub copy: MetricTime,
    pub redraw_total: MetricTime,
    pub window: WindowMetrics,
}
