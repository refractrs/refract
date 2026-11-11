//! Retained native elements. Patches update this tree, not the application's views.

mod input;
mod focus;
mod events;
mod menu;
mod paint;
mod scroll;
mod scroll_metrics;

use crate::{
    editor::TextEditor,
    image::{ImageHandle, ImageState, ImageStore},
    layout,
    renderer::{Point, Rect},
    text::{TextBlock, TextSystem, Typography},
    NativeError, Result,
};
use refract_core::{Element, ElementId, ElementKind, Patch, Update};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use taffy::prelude::{NodeId, TaffyTree};

#[derive(Default)]
struct MeasureContext {
    block: Option<TextBlock>,
    align: refract_core::Alignment,
    image_size: Option<(f32, f32)>,
}

type SharedText = Rc<RefCell<MeasureContext>>;

struct RetainedElement {
    element: Element,
    node: NodeId,
    parent: Option<ElementId>,
    children: Vec<ElementId>,
    typography: Typography,
    text: SharedText,
    rect: Rect,
    clip: Rect,
    scroll: Point,
    scroll_max: Point,
    scroll_tracker: refract_core::ScrollTracker,
    scroll_metrics: refract_core::ScrollMetrics,
    image: Option<ImageHandle>,
    editor: Option<TextEditor>,
}

/// Observable CPU work; presentation is counted separately by the event-loop host.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostStats {
    pub revision: u64,
    pub elements: usize,
    pub text_shapes: u64,
    pub images_decoded: u64,
    /// Calls to Taffy; its internal caches can skip unchanged nodes within a pass.
    pub layout_passes: u64,
    pub patches_applied: u64,
}

pub(crate) struct Host {
    entries: HashMap<ElementId, RetainedElement>,
    roots: Vec<ElementId>,
    tree: TaffyTree<SharedText>,
    root: NodeId,
    text: TextSystem,
    images: ImageStore,
    viewport: (f32, f32),
    needs_layout: bool,
    paint_order: Vec<ElementId>,
    pub focus: Option<ElementId>,
    keyboard_focus: bool,
    reveal_focus: bool,
    hover: Option<ElementId>,
    pressed: Option<ElementId>,
    scrolling: scroll::ScrollState,
    menu: Option<menu::EditMenu>,
    stats: HostStats,
    input: events::InputState,
}

impl Host {
    #[cfg(test)]
    pub fn new() -> Result<Self> {
        Self::with_waker(|| {})
    }

    pub fn with_waker(wake: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let mut tree = TaffyTree::new();
        tree.disable_rounding();
        let root = tree.new_leaf(taffy::Style {
            display: taffy::Display::Flex,
            flex_direction: taffy::FlexDirection::Column,
            align_items: taffy::AlignItems::CENTER,
            justify_content: taffy::JustifyContent::CENTER,
            ..Default::default()
        })?;
        Ok(Self {
            entries: HashMap::new(),
            roots: vec![],
            tree,
            root,
            text: TextSystem::default(),
            images: ImageStore::new(wake),
            viewport: (0.0, 0.0),
            needs_layout: true,
            paint_order: vec![],
            focus: None,
            keyboard_focus: false,
            reveal_focus: false,
            hover: None,
            pressed: None,
            scrolling: scroll::ScrollState::default(),
            menu: None,
            stats: HostStats::default(),
            input: events::InputState::default(),
        })
    }

    pub fn stats(&self) -> HostStats {
        HostStats {
            elements: self.entries.len(),
            text_shapes: self.text.shapes,
            images_decoded: self.images.decoded,
            ..self.stats
        }
    }

    /// Reject unsupported capabilities and malformed references before any mutation.
    /// Structural validation walks topology only when children/identities change.
    fn validate(&self, update: &Update) -> Result<()> {
        if update.base_revision != self.stats.revision || update.revision <= update.base_revision {
            return Err(NativeError::Host("stale host revision".into()));
        }
        let mut created = HashSet::new();
        let mut removed = HashSet::new();
        let mut children = HashMap::new();
        for patch in &update.patches {
            match patch {
                Patch::Create(id, element) => {
                    if self.entries.contains_key(id) || !created.insert(*id) {
                        return Err(NativeError::Host("duplicate element identity".into()));
                    }
                    validate_element(element)?;
                }
                Patch::Update(id, element) => {
                    let old = self
                        .entries
                        .get(id)
                        .ok_or_else(|| NativeError::Host("unknown update target".into()))?;
                    if old.element.kind != element.kind
                        || old.element.interaction != element.interaction
                    {
                        return Err(NativeError::Host("element kind changed".into()));
                    }
                    validate_element(element)?;
                }
                Patch::Remove(id) => {
                    if !self.entries.contains_key(id) || !removed.insert(*id) {
                        return Err(NativeError::Host("unknown or repeated removal".into()));
                    }
                }
                Patch::Children(parent, list) => {
                    children.insert(*parent, list.as_slice());
                }
            }
        }
        if created.is_empty() && removed.is_empty() && children.is_empty() {
            return Ok(());
        }
        let exists = |id: &ElementId| {
            !removed.contains(id) && (self.entries.contains_key(id) || created.contains(id))
        };
        for (parent, list) in &children {
            if parent.as_ref().is_some_and(|id| !exists(id)) || list.iter().any(|id| !exists(id)) {
                return Err(NativeError::Host("unknown child or parent".into()));
            }
        }
        let roots = children.get(&None).copied().unwrap_or(&self.roots);
        let mut visited = HashSet::new();
        let mut stack = roots.to_vec();
        while let Some(id) = stack.pop() {
            if !exists(&id) || !visited.insert(id) {
                return Err(NativeError::Host(
                    "cycle, duplicate, or removed child".into(),
                ));
            }
            let old = self
                .entries
                .get(&id)
                .map_or(&[][..], |entry| entry.children.as_slice());
            stack.extend_from_slice(children.get(&Some(id)).copied().unwrap_or(old));
        }
        let total = self.entries.len() + created.len() - removed.len();
        if visited.len() != total {
            return Err(NativeError::Host("unreachable host elements".into()));
        }
        Ok(())
    }

    pub fn apply(&mut self, update: &Update) -> Result<bool> {
        self.validate(update)?;
        // Capture the old order before a disabled or removed control loses its
        // position. Recovery only considers controls that already existed.
        let recovery = self.focus.filter(|focused| update.patches.iter().any(|patch| match patch {
            Patch::Remove(id) => id == focused,
            Patch::Update(id, element) => id == focused && (element.disabled
                || (self.entries[id].element.is_focusable() && !element.is_focusable())),
            _ => false,
        })).map(|focused| self.focus_recovery_order(focused));
        let mut refresh = HashSet::new();
        for patch in &update.patches {
            match patch {
                Patch::Create(id, element) => {
                    let image = element
                        .source
                        .as_ref()
                        .map(|source| self.images.acquire(source));
                    let text = Rc::new(RefCell::new(MeasureContext::default()));
                    let node = self
                        .tree
                        .new_leaf_with_context(layout::style(element), text.clone())?;
                    self.entries.insert(
                        *id,
                        RetainedElement {
                            element: element.clone(),
                            node,
                            parent: None,
                            children: vec![],
                            typography: Typography::default(),
                            text,
                            rect: Rect::default(),
                            clip: Rect::default(),
                            scroll: Point::default(),
                            scroll_max: Point::default(),
                            scroll_tracker: Default::default(),
                            scroll_metrics: Default::default(),
                            image,
                            editor: (element.kind == ElementKind::TextInput)
                                .then(|| TextEditor::new(element)),
                        },
                    );
                    refresh.insert(*id);
                    self.needs_layout = true;
                }
                Patch::Update(id, element) => {
                    let entry = self.entries.get_mut(id).unwrap();
                    let next_style = layout::style(element);
                    if self.tree.style(entry.node)? != &next_style {
                        self.tree.set_style(entry.node, next_style)?;
                        self.needs_layout = true;
                    }
                    if entry.element.source != element.source {
                        entry.image = element
                            .source
                            .as_ref()
                            .map(|source| self.images.acquire(source));
                        self.tree.mark_dirty(entry.node)?;
                        self.needs_layout = true;
                    }
                    entry.element = element.clone();
                    if element.disabled {
                        if let Some(editor) = &mut entry.editor {
                            editor.cancel_composition(&mut self.text);
                        }
                    }
                    refresh.insert(*id);
                }
                Patch::Children(parent, ids) => {
                    let nodes: Vec<_> = ids.iter().map(|id| self.entries[id].node).collect();
                    let parent_node = parent.map_or(self.root, |id| self.entries[&id].node);
                    self.tree.set_children(parent_node, &nodes)?;
                    if let Some(id) = parent {
                        self.entries.get_mut(id).unwrap().children = ids.clone();
                        refresh.insert(*id);
                    } else {
                        self.roots = ids.clone();
                        refresh.extend(ids);
                    }
                    for id in ids {
                        self.entries.get_mut(id).unwrap().parent = *parent;
                    }
                    self.needs_layout = true;
                }
                Patch::Remove(id) => {
                    let entry = self.entries.remove(id).unwrap();
                    self.tree.remove(entry.node)?;
                    if self.focus == Some(*id) {
                        self.focus = None;
                    }
                    if self.hover == Some(*id) {
                        self.hover = None;
                    }
                    if self.pressed == Some(*id) {
                        self.pressed = None;
                    }
                    self.needs_layout = true;
                }
            }
        }
        self.input.capture.retain(|_, id| self.entries.get(id).is_some_and(|e| !e.element.disabled));
        if self.focus.is_some_and(|id| self.entries.get(&id).is_none_or(|e| e.element.disabled)) {
            self.set_focus(None);
        }
        if let Some(candidates) = recovery {
            let target = candidates.into_iter().find(|id| {
                self.entries.get(id).is_some_and(|entry| entry.element.is_focusable())
            });
            self.set_focus(target);
            self.reveal_focus = target.is_some();
        }
        // Resolve inherited text only under affected roots. Eliminate redundant
        // descendants from the work set when an ancestor already needs refreshing.
        let roots: Vec<_> = refresh
            .iter()
            .copied()
            .filter(|id| {
                let mut cursor = self.entries.get(id).and_then(|e| e.parent);
                while let Some(parent) = cursor {
                    if refresh.contains(&parent) {
                        return false;
                    }
                    cursor = self.entries[&parent].parent;
                }
                self.entries.contains_key(id)
            })
            .collect();
        for id in roots {
            let inherited = self.entries[&id]
                .parent
                .map(|p| self.entries[&p].typography.clone())
                .unwrap_or_default();
            self.refresh_text(id, &inherited)?;
        }
        self.stats.revision = update.revision;
        self.stats.patches_applied += update.patches.len() as u64;
        Ok(!update.patches.is_empty())
    }

    /// Image completion changes host resources, not application state. Only
    /// current users of a completed resource need measurement/paint invalidation.
    pub fn poll_images(&mut self) -> Result<bool> {
        let changed = self.images.poll();
        if changed.is_empty() {
            return Ok(false);
        }
        let affected: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .image
                    .as_ref()
                    .filter(|image| changed.contains(&image.borrow().id))
                    .map(|_| *id)
            })
            .collect();
        for id in &affected {
            let inherited = self.entries[id]
                .parent
                .map(|p| self.entries[&p].typography.clone())
                .unwrap_or_default();
            self.refresh_text(*id, &inherited)?;
            self.tree.mark_dirty(self.entries[id].node)?;
        }
        self.needs_layout |= !affected.is_empty();
        Ok(!affected.is_empty())
    }

    fn refresh_text(&mut self, id: ElementId, inherited: &Typography) -> Result<()> {
        let entry = self.entries.get_mut(&id).unwrap();
        let typography = inherited.inherit(&entry.element.style);
        entry.typography = typography.clone();
        if let Some(editor) = &mut entry.editor {
            editor.sync(&entry.element, &typography, &mut self.text);
        }
        let value = match entry.element.kind {
            ElementKind::Text => Some(entry.element.text.clone()),
            ElementKind::Image => {
                let mut context = entry.text.borrow_mut();
                context.image_size = Some((160.0, 100.0));
                match entry.image.as_ref().map(|image| image.borrow()) {
                    Some(resource) => match &resource.state {
                        ImageState::Ready(image) => {
                            context.image_size =
                                Some((image.width() as f32, image.height() as f32));
                            context.block = None;
                            None
                        }
                        ImageState::Failed(_error) => Some(
                            entry
                                .element
                                .label
                                .clone()
                                .unwrap_or_else(|| "Image unavailable".into()),
                        ),
                        ImageState::Loading => {
                            context.block = None;
                            None
                        }
                    },
                    None => {
                        context.block = None;
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(value) = value {
            let mut context = entry.text.borrow_mut();
            let truncate = entry.element.style.truncate;
            let reshaped = context.block.as_ref().is_none_or(|block| {
                block.truncate != truncate || !block.matches(&value, &typography)
            });
            if reshaped {
                context.block = Some(self.text.shape_label(&value, &typography, truncate));
                self.tree.mark_dirty(entry.node)?;
                self.needs_layout = true;
            }
            context.align = typography.align;
        }
        let children = entry.children.clone();
        for child in children {
            self.refresh_text(child, &typography)?;
        }
        Ok(())
    }
}

fn validate_element(element: &Element) -> Result<()> {
    element.validate()?;
    Ok(())
}
