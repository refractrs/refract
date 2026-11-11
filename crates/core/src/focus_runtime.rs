//! Host-facing focus protocol. Registration is committed with the mounted tree;
//! host observations and callbacks share a graph batch, with no synthetic blur
//! between descendants of a common ancestor.
use super::*;
use crate::focus::{FocusBinding, Group};

impl App {
    pub(super) fn prepare_focus(
        &self,
        tx: &Preparation,
    ) -> Result<BTreeMap<ElementId, FocusBinding>, ViewError> {
        let mut bindings = self.focus.bindings.clone();
        for id in &tx.removed {
            bindings.remove(id);
        }
        for (&id, mount) in &tx.changed {
            match mount.handler.as_ref().and_then(|h| h.focus.clone()) {
                Some(binding) if mount.element.is_some() => {
                    bindings.insert(id, binding);
                }
                _ => {
                    bindings.remove(&id);
                }
            }
        }
        let entries: Vec<_> = bindings.values().collect();
        for (i, binding) in entries.iter().enumerate() {
            if entries[..i].iter().any(|other| binding.same(other)) {
                return Err(ViewError::new(
                    "ambiguous focus attachment: a binding value identifies more than one element",
                ));
            }
        }
        Ok(bindings)
    }

    pub(super) fn commit_focus(&mut self, bindings: BTreeMap<ElementId, FocusBinding>) {
        self.focus.topology_changed = true;
        for binding in bindings.values() {
            if !self.focus.groups.contains_key(&binding.group) {
                let observer = self.runtime.observer();
                let (_, deps) = self.runtime.capture(|rt| binding.source.read(rt));
                self.runtime.accept(&observer, deps);
                self.focus.groups.insert(
                    binding.group,
                    Group {
                        source: binding.source.clone(),
                        acknowledged: binding.source.cleared(),
                        observer,
                    },
                );
            }
        }
        self.focus.bindings = bindings;
    }

    /// Read a committed application focus request. `Some(None)` requests blur;
    /// `None` means there is no request. Call after committing pending views and
    /// confirm the resulting host focus with `confirm_focus`, even on rejection.
    /// Conflicting requests are reconciled before an error is returned.
    pub fn take_focus_request(&mut self) -> Result<Option<Option<ElementId>>, ViewError> {
        let mut targets = Vec::new();
        let mut release = false;
        let mut changed = false;
        for (&group_id, group) in &self.focus.groups {
            if !self.focus.topology_changed && !group.observer.is_dirty(&self.runtime) {
                continue;
            }
            let desired = group.source.read(&self.runtime);
            if group.source.equal(&*desired, &*group.acknowledged) {
                continue;
            }
            changed = true;
            let target = self
                .focus
                .bindings
                .iter()
                .find(|(_, b)| b.group == group_id && group.source.equal(&*desired, &*b.value))
                .map(|(&id, _)| id);
            if let Some(id) = target.filter(|id| {
                self.mounts
                    .get(id)
                    .and_then(|m| m.element.as_ref())
                    .is_some_and(|e| !e.disabled)
            }) {
                targets.push(id);
            } else if group.source.equal(&*desired, &*group.source.cleared()) {
                release |= self
                    .focus
                    .current
                    .and_then(|id| self.focus.bindings.get(&id))
                    .is_some_and(|b| b.group == group_id);
            }
        }
        if targets.len() > 1 {
            self.confirm_focus(self.focus.current);
            return Err(ViewError::new(
                "conflicting focus requests: use a shared optional target to select one element",
            ));
        }
        Ok(targets.first().copied().map(Some).or_else(|| {
            if release {
                Some(None)
            } else if changed {
                Some(self.focus.current)
            } else {
                None
            }
        }))
    }

    /// Publish a user/host transition. Unchanged focus does not consume pending
    /// application requests. Adapters report the final target, not blur/focus pairs.
    pub fn observe_focus(&mut self, target: Option<ElementId>) {
        if target != self.focus.current {
            self.confirm_focus(target);
        }
    }

    /// Acknowledge host focus after applying requests and committed tree changes.
    pub fn confirm_focus(&mut self, target: Option<ElementId>) {
        let target = target.filter(|id| {
            self.mounts
                .get(id)
                .is_some_and(|m| m.owner.get() && m.element.as_ref().is_some_and(|e| !e.disabled))
        });
        let mut path = Vec::new();
        let mut cursor = target;
        while let Some(id) = cursor {
            let Some(mount) = self.mounts.get(&id) else {
                break;
            };
            if mount.element.is_some() {
                path.push(id);
            }
            cursor = mount.parent;
        }
        let previous = self.focus.current;
        let mut events = Vec::new();
        if previous != target {
            if let Some(id) = previous {
                events.push((id, Event::Focus(false)));
            }
            if let Some(id) = target {
                events.push((id, Event::Focus(true)));
            }
        }
        for &id in &self.focus.path {
            if !path.contains(&id) {
                events.push((id, Event::FocusWithin(false)));
            }
        }
        for &id in path.iter().rev() {
            if !self.focus.path.contains(&id) {
                events.push((id, Event::FocusWithin(true)));
            }
        }
        let callbacks: Vec<_> = events
            .into_iter()
            .filter_map(|(id, event)| {
                let mount = self.mounts.get(&id)?;
                Some((id, event, mount.handler.clone()?, mount.environment.clone()))
            })
            .collect();
        self.focus.current = target;
        self.focus.path = path;
        self.focus.topology_changed = false;
        let selected = target.and_then(|id| self.focus.bindings.get(&id)).cloned();
        self.runtime.batch(|rt| {
            for (&id, group) in &mut self.focus.groups {
                let value = selected
                    .as_ref()
                    .filter(|b| b.group == id)
                    .map(|b| b.value.clone())
                    .unwrap_or_else(|| group.source.cleared());
                group.source.write(rt, &*value);
                // Acknowledge before callbacks: requests made by a focus callback
                // must survive and execute in the next host transaction.
                group.acknowledge(rt);
            }
            for (id, event, handler, environment) in callbacks {
                handler(
                    &mut ActionContext {
                        runtime: rt,
                        environment: &environment,
                        target: id,
                        current_target: id,
                        event: &event,
                        response: &mut EventResponse::default(),
                    },
                    &event,
                );
            }
        });
        self.focus
            .groups
            .retain(|id, _| self.focus.bindings.values().any(|b| b.group == *id));
    }

    pub(super) fn clear_focus_bindings(&mut self) {
        self.runtime.batch(|rt| {
            for group in self.focus.groups.values() {
                group.source.write(rt, &*group.source.cleared());
            }
        });
        self.focus = Default::default();
    }
}
