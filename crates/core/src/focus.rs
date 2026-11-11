//! Focus attachments are core metadata, never host style or graph-owned native objects.
use crate::{Binding, ElementId, Observer, Runtime};
use refract_graph::BindingId;
use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

pub(crate) trait Source {
    fn read(&self, rt: &Runtime) -> Rc<dyn Any>;
    fn equal(&self, a: &dyn Any, b: &dyn Any) -> bool;
    fn cleared(&self) -> Rc<dyn Any>;
    fn write(&self, rt: &mut Runtime, value: &dyn Any);
}
struct TypedSource<T> {
    binding: Binding<T>,
    cleared: T,
}
impl<T: Clone + PartialEq + 'static> Source for TypedSource<T> {
    fn read(&self, rt: &Runtime) -> Rc<dyn Any> {
        Rc::new(self.binding.get(rt))
    }
    fn equal(&self, a: &dyn Any, b: &dyn Any) -> bool {
        a.downcast_ref::<T>() == b.downcast_ref::<T>()
    }
    fn cleared(&self) -> Rc<dyn Any> {
        Rc::new(self.cleared.clone())
    }
    fn write(&self, rt: &mut Runtime, value: &dyn Any) {
        self.binding
            .set_if_changed(rt, value.downcast_ref::<T>().unwrap().clone());
    }
}

#[derive(Clone)]
pub(crate) struct FocusBinding {
    pub group: BindingId,
    pub source: Rc<dyn Source>,
    pub value: Rc<dyn Any>,
}
impl FocusBinding {
    pub fn boolean(binding: Binding<bool>) -> Self {
        Self::new(binding, true, false)
    }
    pub fn value<T: Clone + PartialEq + 'static>(binding: Binding<Option<T>>, value: T) -> Self {
        Self::new(binding, Some(value), None)
    }
    fn new<T: Clone + PartialEq + 'static>(binding: Binding<T>, value: T, cleared: T) -> Self {
        Self {
            group: binding.identity(),
            source: Rc::new(TypedSource { binding, cleared }),
            value: Rc::new(value),
        }
    }
    pub fn same(&self, other: &Self) -> bool {
        self.group == other.group && self.source.equal(&*self.value, &*other.value)
    }
}

pub(crate) struct Group {
    pub source: Rc<dyn Source>,
    pub acknowledged: Rc<dyn Any>,
    pub observer: Observer,
}
#[derive(Default)]
pub(crate) struct Focus {
    pub bindings: BTreeMap<ElementId, FocusBinding>,
    pub groups: HashMap<BindingId, Group>,
    pub current: Option<ElementId>,
    pub path: Vec<ElementId>,
    pub topology_changed: bool,
}
impl Group {
    pub fn acknowledge(&mut self, rt: &mut Runtime) {
        let (value, deps) = rt.capture(|rt| self.source.read(rt));
        self.acknowledged = value;
        rt.accept(&self.observer, deps);
    }
}
