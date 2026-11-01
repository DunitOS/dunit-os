//! Deterministic progress snapshots and visibility-aware busy scheduling.
use super::{Renderer, View};
use crate::ProgressState;
use alloc::vec::Vec;
use dunit_render::ScrollArea;
use dunit_style::Style;
use dunit_ui::{Layout, NodeId, Rect, Tree};
use dunit_widgets::progress::ProgressPaint;
fn duration(style: &Style, name: &str) -> Option<u64> {
    style
        .get(name)?
        .as_duration()
        .filter(|ms| *ms > 0.)
        .map(|ms| (ms as u64).max(1))
}
fn visible(
    tree: &Tree,
    layout: &Layout,
    areas: &[Option<ScrollArea>],
    id: NodeId,
    track: Rect,
    width: u32,
    height: u32,
) -> bool {
    super::visible_rect(tree, layout, areas, id, track, width, height).is_some()
}
impl Renderer {
    /// No busy timer is needed for paused, disabled, reduced-motion or clipped
    /// indicators. Immediate rendering shows a stationary centered segment.
    pub fn busy_interval_ms(&self) -> Option<u64> {
        self.previous
            .as_ref()?
            .busy_interval_ms
            .filter(|_| self.motion_enabled)
    }
    pub(super) fn prepare_progress<M>(
        &self,
        views: &[&View<M>],
        tree: &Tree,
        layout: &Layout,
        styles: &[Style],
        areas: &[Option<ScrollArea>],
        now: Option<u64>,
        width: u32,
        height: u32,
    ) -> (Vec<Option<ProgressPaint>>, Option<u64>, usize) {
        let mut interval = None;
        let mut active = 0;
        let frames = views
            .iter()
            .enumerate()
            .map(|(index, v)| {
                let progress = v.progress.as_ref()?;
                Some(match progress.state {
                    ProgressState::Determinate { value, options } => {
                        ProgressPaint::determinate(options.fraction(value))
                    }
                    ProgressState::Busy { running } => {
                        let style = &styles[index];
                        let segment = style.number("busy-segment").unwrap_or(0.).clamp(0., 1.);
                        let cycle = duration(style, "busy-duration");
                        let cadence = duration(style, "busy-interval");
                        let rect = layout.rect(NodeId(index as u32));
                        let track = ProgressPaint::determinate(0.).geometry(style, rect).0;
                        let timed = running
                            && !v.interaction.disabled
                            && self.motion_enabled
                            && now.is_some()
                            && cycle.is_some()
                            && cadence.is_some()
                            && segment > 0.
                            && segment < 1.
                            && visible(
                                tree,
                                layout,
                                areas,
                                NodeId(index as u32),
                                track,
                                width,
                                height,
                            );
                        if timed {
                            active += 1;
                            let ms = cadence.unwrap();
                            interval = Some(interval.map_or(ms, |old: u64| old.min(ms)));
                        }
                        ProgressPaint::busy(
                            if timed { now } else { None },
                            cycle.unwrap_or(0),
                            segment,
                            running,
                        )
                    }
                })
            })
            .collect();
        (frames, interval, active)
    }
}
