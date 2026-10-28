//! Stable-key paint transitions. This layer owns no clock or window resources.
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use dunit_style::{
    motion::{interpolate, Transition},
    Style, Value,
};
use dunit_ui::Tree;

pub(crate) fn declarations(value: &Value) -> Option<Vec<Transition>> {
    let parsed = Transition::parse_strict(value)?;
    parsed
        .iter()
        .all(|t| {
            matches!(
                t.property.as_str(),
                "color"
                    | "background"
                    | "border-color"
                    | "radius"
                    | "border-width"
                    | "backdrop-blur"
            )
        })
        .then_some(parsed)
}
#[derive(Clone)]
struct Track {
    from: Value,
    to: Value,
    start: u64,
    spec: Transition,
}
impl Track {
    fn sample(&self, now: u64) -> Value {
        if self.finished(now) {
            return self.to.clone();
        }
        let progress =
            (now.saturating_sub(self.start) as f64 / self.spec.duration_ms as f64) as f32;
        interpolate(&self.from, &self.to, progress, self.spec.easing)
    }
    fn finished(&self, now: u64) -> bool {
        now.saturating_sub(self.start) as f64 >= self.spec.duration_ms as f64
    }
}
#[derive(Default)]
pub(crate) struct Motion {
    tracks: BTreeMap<(String, String), Track>,
    time: Option<u64>,
}
impl Motion {
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.time = None;
    }
    pub fn active(&self) -> usize {
        self.tracks.len()
    }
    pub fn styles(
        &mut self,
        tree: &Tree,
        targets: &[Style],
        old: Option<(&Tree, &[Style], &[Style])>,
        now: Option<u64>,
        enabled: bool,
    ) -> Vec<Style> {
        let Some(now) = now.filter(|now| enabled && self.time.map_or(true, |old| *now >= old))
        else {
            self.clear();
            return targets.to_vec();
        };
        let mut live = BTreeMap::new();
        let mut shown = targets.to_vec();
        for node in tree.iter() {
            let Some(name) = node.name.as_deref() else {
                continue;
            };
            let Some((previous, old_targets, old_shown)) = old else {
                continue;
            };
            let Some(old_id) = previous.by_name(name) else {
                continue;
            };
            if previous.node(old_id).kind != node.kind {
                continue;
            }
            let index = node.id.0 as usize;
            let old_index = old_id.0 as usize;
            let Some(specs) = targets[index].get("transition").and_then(declarations) else {
                continue;
            };
            for spec in specs {
                if spec.duration_ms <= 0. {
                    continue;
                }
                let property = spec.property.as_str();
                let Some(to) = targets[index].get(property) else {
                    continue;
                };
                let Some(from) = old_shown[old_index].get(property) else {
                    continue;
                };
                if !matches!(
                    (from, to),
                    (Value::Color(_), Value::Color(_)) | (Value::Number(_), Value::Number(_))
                ) {
                    continue;
                }
                let key = (String::from(name), spec.property.clone());
                let existing = self.tracks.get(&key);
                let changed = old_targets[old_index].get(property) != Some(to);
                let track = if changed {
                    let from = existing.map_or_else(|| from.clone(), |track| track.sample(now));
                    if from == *to {
                        continue;
                    }
                    Track {
                        from,
                        to: to.clone(),
                        start: now,
                        spec,
                    }
                } else if let Some(track) =
                    existing.filter(|track| track.spec == spec && track.to == *to)
                {
                    track.clone()
                } else {
                    continue;
                };
                shown[index].set(track.spec.property.clone(), track.sample(now));
                if !track.finished(now) {
                    live.insert(key, track);
                }
            }
        }
        self.tracks = live;
        self.time = Some(now);
        shown
    }
}
