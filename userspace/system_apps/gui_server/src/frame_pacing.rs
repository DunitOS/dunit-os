//! Bounded input service and absolute frame deadlines, independent of transport.
pub const INPUT_BATCH_LIMIT: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameDeadline {
    pub ns: u64,
}
impl FrameDeadline {
    pub fn new(start_ns: u64, interval_ms: u32) -> Self {
        Self {
            ns: start_ns.saturating_add(interval_ms as u64 * 1_000_000),
        }
    }
    pub fn due(self, now_ns: u64) -> bool {
        now_ns >= self.ns
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerSample<T> {
    Queued(T),
    Stationary,
}
pub struct InputBatch {
    consumed: usize,
    finished: bool,
}
impl InputBatch {
    pub fn new() -> Self {
        Self {
            consumed: 0,
            finished: false,
        }
    }
    /// Never consumes beyond the service budget. Empty queues get exactly one
    /// stationary pass so geometry changes under an idle pointer remain visible.
    pub fn next<T, E>(
        &mut self,
        read: impl FnOnce() -> Result<Option<T>, E>,
    ) -> Result<Option<PointerSample<T>>, E> {
        if self.finished || self.consumed == INPUT_BATCH_LIMIT {
            return Ok(None);
        }
        match read()? {
            Some(event) => {
                self.consumed += 1;
                Ok(Some(PointerSample::Queued(event)))
            }
            None => {
                self.finished = true;
                Ok((self.consumed == 0).then_some(PointerSample::Stationary))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[test]
    fn deadline_charges_work_instead_of_adding_another_period() {
        let d = FrameDeadline::new(100_000_000, 16);
        assert!(!d.due(115_000_000));
        assert!(d.due(116_000_000));
        assert!(d.due(140_000_000));
        let next = FrameDeadline::new(140_000_000, 16);
        assert_eq!(next.ns, 156_000_000);
    }
    #[test]
    fn deadline_saturates_without_wrapping() {
        assert_eq!(FrameDeadline::new(u64::MAX - 3, 16).ns, u64::MAX);
    }
    #[test]
    fn batch_preserves_each_edge_wheel_direction_and_loss_record() {
        let expected = [
            (1, 0, false),
            (0, 0, false),
            (0, 1, false),
            (0, -1, false),
            (1, 0, true),
            (0, 0, false),
        ];
        let mut q = VecDeque::from(expected);
        let mut b = InputBatch::new();
        let mut got = Vec::new();
        while let Some(sample) = b.next(|| Ok::<_, ()>(q.pop_front())).unwrap() {
            match sample {
                PointerSample::Queued(e) => got.push(e),
                PointerSample::Stationary => panic!("no synthetic pass after queued records"),
            }
        }
        assert_eq!(got, expected);
    }
    #[test]
    fn cap_defers_next_record_without_consuming_it() {
        let mut q: VecDeque<_> = (0..INPUT_BATCH_LIMIT + 1).collect();
        let mut b = InputBatch::new();
        let mut got = 0;
        while b.next(|| Ok::<_, ()>(q.pop_front())).unwrap().is_some() {
            got += 1;
        }
        assert_eq!(got, INPUT_BATCH_LIMIT);
        assert_eq!(q.pop_front(), Some(INPUT_BATCH_LIMIT));
    }
    #[test]
    fn idle_gets_one_stationary_pass_and_errors_are_visible() {
        let mut b = InputBatch::new();
        assert_eq!(
            b.next(|| Ok::<Option<()>, ()>(None)),
            Ok(Some(PointerSample::Stationary))
        );
        assert_eq!(
            b.next(|| panic!("must not poll again")),
            Ok::<Option<PointerSample<()>>, ()>(None)
        );
        let mut b = InputBatch::new();
        assert_eq!(b.next(|| Err::<Option<()>, _>(-5)), Err(-5));
    }
}
