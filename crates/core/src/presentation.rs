//! Retained property animation shared by all hosts. Frames never touch the graph.
use crate::{Animation, Color, Element, ElementId, Patch, Style, Update};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

#[derive(Clone)]
struct Track {
    from: f64,
    to: f64,
    velocity: f64,
    start: Duration,
    animation: Animation,
}
impl Track {
    fn sample(&self, now: Duration) -> crate::AnimationSample {
        self.animation.sample(
            self.from,
            self.to,
            self.velocity,
            now.saturating_sub(self.start),
        )
    }
}

/// Host-owned presentation state. Feed accepted semantic updates, then sample at
/// monotonic timestamps. Only active elements are visited on animation frames.
#[derive(Clone, Default)]
pub struct PresentationState {
    targets: HashMap<ElementId, Element>,
    tracks: HashMap<ElementId, BTreeMap<u16, Track>>,
    parents: HashMap<ElementId, Option<ElementId>>,
    exits: HashMap<ElementId, (crate::TransitionEffect, Track)>,
}

impl PresentationState {
    pub fn is_animating(&self) -> bool {
        !self.tracks.is_empty() || !self.exits.is_empty()
    }
    pub fn clear(&mut self) {
        self.targets.clear();
        self.tracks.clear();
        self.parents.clear();
        self.exits.clear();
    }

    /// Apply an already validated host update. Call in the same uninterrupted
    /// prepare/apply/commit sequence as the host. Speculative preparation never
    /// calls this method. Hosts may clone this state to stage fallible application.
    pub fn apply(&mut self, update: &Update, now: Duration) {
        let removed: HashSet<_> = update
            .patches
            .iter()
            .filter_map(|p| {
                if let Patch::Remove(id) = p {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        let candidates: HashSet<_> = removed
            .iter()
            .copied()
            .filter(|id| {
                self.targets.get(id).is_some_and(|e| e.transition.is_some())
                    && update
                        .animations
                        .get(id)
                        .is_some_and(|tx| tx.animation.is_some() && !tx.disables_animations)
            })
            .collect();
        for id in &candidates {
            let mut parent = self.parents.get(id).copied().flatten();
            let mut nested = false;
            while let Some(p) = parent {
                if candidates.contains(&p) {
                    nested = true;
                    break;
                }
                parent = self.parents.get(&p).copied().flatten();
            }
            if !nested {
                let effect = self.targets[id].transition.unwrap().removal;
                if effect != crate::TransitionEffect::default() {
                    self.exits.insert(
                        *id,
                        (
                            effect,
                            Track {
                                from: 0.0,
                                to: 1.0,
                                velocity: 0.0,
                                start: now,
                                animation: update.animations[id].animation.unwrap(),
                            },
                        ),
                    );
                }
            }
        }
        for patch in &update.patches {
            match patch {
                Patch::Create(id, element) => {
                    self.targets.insert(*id, element.clone());
                    self.tracks.remove(id);
                    if update.base_revision > 0 {
                        if let (Some(transition), Some(animation)) = (
                            element.transition,
                            update.animations.get(id).and_then(|tx| {
                                if tx.disables_animations {
                                    None
                                } else {
                                    tx.animation
                                }
                            }),
                        ) {
                            let mut initial = element.clone();
                            transition.insertion.apply(&mut initial.style);
                            self.targets.insert(*id, initial);
                            self.retarget(*id, element, Some(animation), now);
                        }
                    }
                }
                Patch::Update(id, element) => {
                    let animation = update.animations.get(id).and_then(|tx| {
                        if tx.disables_animations {
                            None
                        } else {
                            tx.animation
                        }
                    });
                    self.retarget(*id, element, animation, now);
                }
                Patch::Remove(id) => {
                    self.targets.remove(id);
                    self.tracks.remove(id);
                    self.parents.remove(id);
                }
                Patch::Children(parent, children) => {
                    for id in children {
                        self.parents.insert(*id, *parent);
                    }
                }
            }
        }
    }

    fn retarget(
        &mut self,
        id: ElementId,
        element: &Element,
        animation: Option<Animation>,
        now: Duration,
    ) {
        let old = self
            .targets
            .get(&id)
            .map(|e| values(&e.style))
            .unwrap_or_default();
        let next = values(&element.style);
        let tracks = self.tracks.entry(id).or_default();
        tracks.retain(|key, _| next.contains_key(key));
        for (&key, &to) in &next {
            let Some(&previous) = old.get(&key) else {
                tracks.remove(&key);
                continue;
            };
            if previous == to {
                continue;
            } // Unrelated text updates do not restart motion.
            let sample = tracks.get(&key).map(|track| track.sample(now));
            tracks.remove(&key);
            if let Some(animation) = animation {
                let from = sample.map_or(previous, |s| s.value);
                let velocity = sample.map_or(0.0, |s| s.velocity);
                if from != to || (animation.is_spring() && velocity != 0.0) {
                    tracks.insert(
                        key,
                        Track {
                            from,
                            to,
                            velocity,
                            start: now,
                            animation,
                        },
                    );
                }
            }
        }
        if tracks.is_empty() {
            self.tracks.remove(&id);
        }
        self.targets.insert(id, element.clone());
    }

    /// Current displayed description, including immediate semantic fields.
    pub fn element(&self, id: ElementId, now: Duration) -> Option<Element> {
        let mut result = self.targets.get(&id)?.clone();
        if let Some(tracks) = self.tracks.get(&id) {
            // Materialize default opacity when animating back to inherited absence.
            if tracks.contains_key(&0) {
                result.style.opacity = Some(result.style.opacity.unwrap_or(1.0));
            }
            visit(&mut result.style, |key, value| {
                if let Some(track) = tracks.get(&key) {
                    *value = track.sample(now).value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
                }
            });
        }
        Some(result)
    }

    /// Samples active elements, including their final exact target once. Settled
    /// tracks retire here, so idle hosts need no polling or perpetual frame loop.
    pub fn advance(&mut self, now: Duration) -> Vec<(ElementId, Element)> {
        self.exits
            .retain(|_, (_, track)| !track.sample(now).finished);
        let ids: Vec<_> = self.tracks.keys().copied().collect();
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            self.tracks
                .get_mut(&id)
                .unwrap()
                .retain(|_, track| !track.sample(now).finished);
            if self.tracks[&id].is_empty() {
                self.tracks.remove(&id);
            }
            if let Some(element) = self.element(id, now) {
                result.push((id, element));
            }
        }
        result
    }

    /// Outgoing visuals are sampled independently of logical element ownership.
    pub fn exit_effect(&self, id: ElementId, now: Duration) -> Option<crate::TransitionEffect> {
        let (effect, track) = self.exits.get(&id)?;
        Some(effect.at(track.sample(now).value as f32))
    }
    pub fn exit_ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.exits.keys().copied()
    }

    pub fn active_ids(&self) -> HashSet<ElementId> {
        self.tracks.keys().copied().collect()
    }
}

fn values(style: &Style) -> BTreeMap<u16, f64> {
    let mut values = BTreeMap::new();
    visit(&mut style.clone(), |key, value| {
        values.insert(key, f64::from(*value));
    });
    values
}

// Stable channel IDs. Absent optional dimensions/colors are not interpolated:
// intrinsic sizing, inheritance and incompatible structures change immediately.
fn visit(s: &mut Style, mut f: impl FnMut(u16, &mut f32)) {
    let mut opacity = s.opacity.unwrap_or(1.0);
    f(0, &mut opacity);
    if s.opacity.is_some() {
        s.opacity = Some(opacity.clamp(0.0, 1.0));
    }
    for (i, value) in [
        &mut s.transform.offset_x,
        &mut s.transform.offset_y,
        &mut s.transform.scale_x,
        &mut s.transform.scale_y,
        &mut s.transform.rotation,
    ]
    .into_iter()
    .enumerate()
    {
        f(1 + i as u16, value);
    }
    for (i, value) in [
        &mut s.radius.top_left,
        &mut s.radius.top_right,
        &mut s.radius.bottom_right,
        &mut s.radius.bottom_left,
        &mut s.padding.top,
        &mut s.padding.right,
        &mut s.padding.bottom,
        &mut s.padding.left,
        &mut s.gap,
    ]
    .into_iter()
    .enumerate()
    {
        f(10 + i as u16, value);
        *value = value.max(0.0);
    }
    for (i, value) in [
        &mut s.width,
        &mut s.height,
        &mut s.min_width,
        &mut s.max_width,
        &mut s.min_height,
        &mut s.max_height,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(value) = value {
            f(20 + i as u16, value);
            *value = value.max(0.0);
        }
    }
    // Independently retargeted springs can temporarily cross valid constraints.
    // Keep the displayed box valid without altering either track's momentum.
    if let (Some(min), Some(max)) = (&mut s.min_width, s.max_width) { *min = min.min(max); }
    if let (Some(min), Some(max)) = (&mut s.min_height, s.max_height) { *min = min.min(max); }
    if let Some(color) = &mut s.background {
        color_channels(30, color, &mut f);
    }
    if let Some(color) = &mut s.foreground {
        color_channels(34, color, &mut f);
    }
    if let Some(border) = &mut s.border {
        for (i, side) in [
            &mut border.top,
            &mut border.right,
            &mut border.bottom,
            &mut border.left,
        ]
        .into_iter()
        .enumerate()
        {
            let id = 40 + i as u16 * 5;
            f(id, &mut side.width);
            side.width = side.width.max(0.0);
            color_channels(id + 1, &mut side.color, &mut f);
        }
    }
    if let Some(shadow) = &mut s.shadow {
        f(60, &mut shadow.offset_x);
        f(61, &mut shadow.offset_y);
        f(62, &mut shadow.blur_radius);
        shadow.blur_radius = shadow.blur_radius.max(0.0);
        color_channels(63, &mut shadow.color, &mut f);
    }
}

// Match the existing gradient contract: interpolate premultiplied sRGB, avoiding
// dark fringes when alpha changes. Keep floating-point tracks until presentation.
fn color_channels(base: u16, color: &mut Color, f: &mut impl FnMut(u16, &mut f32)) {
    let alpha = f32::from(color.3) / 255.0;
    let mut channels = [
        f32::from(color.0) / 255.0 * alpha,
        f32::from(color.1) / 255.0 * alpha,
        f32::from(color.2) / 255.0 * alpha,
        alpha,
    ];
    for (i, v) in channels.iter_mut().enumerate() {
        f(base + i as u16, v);
    }
    let a = channels[3].clamp(0.0, 1.0);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    *color = if a > 0.000001 {
        Color(
            byte(channels[0] / a),
            byte(channels[1] / a),
            byte(channels[2] / a),
            byte(a),
        )
    } else {
        Color(0, 0, 0, 0)
    };
}
