//! View composition, type erasure, and group descriptions.

use crate::{
    element::ElementIdentity, environment::Provider, Element, Handler, Memo, ViewContext, ViewError,
};
use std::{any::TypeId, rc::Rc};

/// Implement `body` for a composition boundary with its own mounted state and reads.
pub trait View: 'static {
    fn body(&self, _cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        Err::<Empty, _>(ViewError::new("primitive has no body"))
    }

    #[doc(hidden)]
    fn describe(&self) -> Option<Description> {
        None
    }

    /// Host identity and memo information must agree with `describe`. Built-in views
    /// can provide these without cloning their element or child descriptions.
    #[doc(hidden)]
    fn metadata(&self) -> ViewMetadata {
        self.describe().map(|d| d.metadata()).unwrap_or_default()
    }

    #[doc(hidden)]
    fn view_key(&self) -> Option<String> {
        None
    }

    fn into_any(self) -> AnyView
    where
        Self: Sized,
    {
        AnyView::new(self)
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct ViewMetadata {
    element_identity: Option<ElementIdentity>,
    memo: Option<Memo>,
}

impl ViewMetadata {
    pub(crate) fn element(element: &Element) -> Self {
        Self {
            element_identity: Some(element.identity()),
            memo: None,
        }
    }

    pub(crate) fn memo(memo: Memo) -> Self {
        Self {
            element_identity: None,
            memo: Some(memo),
        }
    }
}

// Object-safe bridge for heterogeneous child lists. Public View implementations
// can keep their concrete return types until they enter the retained tree.
trait Erased {
    fn evaluate(&self, cx: &mut ViewContext<'_>) -> Result<Description, ViewError>;
    fn type_id(&self) -> TypeId;
    fn key(&self) -> Option<String>;
    fn element_identity(&self) -> Option<ElementIdentity>;
    fn memo(&self) -> Option<Memo>;
}

impl<V: View> Erased for V {
    fn evaluate(&self, cx: &mut ViewContext<'_>) -> Result<Description, ViewError> {
        if let Some(d) = self.describe() {
            Ok(d)
        } else {
            Ok(Description::Group(vec![self.body(cx)?.into_any()]))
        }
    }

    fn type_id(&self) -> TypeId {
        TypeId::of::<V>()
    }

    fn key(&self) -> Option<String> {
        self.view_key()
    }

    fn element_identity(&self) -> Option<ElementIdentity> {
        View::metadata(self).element_identity
    }

    fn memo(&self) -> Option<Memo> {
        View::metadata(self).memo
    }
}

/// A shared, type-erased view description; cloning does not evaluate its body.
#[derive(Clone)]
pub struct AnyView(Rc<dyn Erased>);

impl AnyView {
    pub fn new<V: View>(view: V) -> Self {
        Self(Rc::new(view))
    }

    pub(crate) fn evaluate(&self, cx: &mut ViewContext<'_>) -> Result<Description, ViewError> {
        self.0.evaluate(cx)
    }

    pub(crate) fn type_id(&self) -> TypeId {
        Erased::type_id(self.0.as_ref())
    }

    pub(crate) fn element_identity(&self) -> Option<ElementIdentity> {
        self.0.element_identity()
    }

    pub(crate) fn memo_props(&self) -> Option<Memo> {
        self.0.memo()
    }

    pub(crate) fn identity_key(&self) -> Option<String> {
        self.0.key()
    }
}

impl View for AnyView {
    fn body(&self, cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        Ok(Described(self.evaluate(cx)?))
    }

    fn into_any(self) -> Self {
        self
    }
}

// Groups, providers, and memo boundaries participate in mounted identity without
// requiring their own host element. Only Element produces a native node.
#[doc(hidden)]
#[derive(Clone)]
pub enum Description {
    Element(Box<Element>, Vec<AnyView>, Option<Handler>),
    Group(Vec<AnyView>),
    Provider(Provider, AnyView),
    Memo(Memo, AnyView),
    Transaction(crate::AnimationScope, AnyView),
}

impl Description {
    pub(crate) fn metadata(&self) -> ViewMetadata {
        match self {
            Self::Element(element, _, _) => ViewMetadata::element(element),
            Self::Memo(memo, _) => ViewMetadata::memo(memo.clone()),
            _ => ViewMetadata::default(),
        }
    }
}

pub(crate) struct Described(pub(crate) Description);

impl View for Described {
    fn describe(&self) -> Option<Description> {
        Some(self.0.clone())
    }

    fn metadata(&self) -> ViewMetadata {
        self.0.metadata()
    }
}

/// An empty composition boundary, also used to preserve optional child positions.
pub struct Empty;

impl View for Empty {
    fn describe(&self) -> Option<Description> {
        Some(Description::Group(vec![]))
    }

    fn metadata(&self) -> ViewMetadata {
        ViewMetadata::default()
    }
}

/// A closure-backed composition boundary created by [`view_fn`].
pub struct FunctionView<F>(F);

/// Create a composition boundary whose reads and mounted slots belong to this closure.
pub fn view_fn<F, V>(f: F) -> FunctionView<F>
where
    F: Fn(&mut ViewContext<'_>) -> Result<V, ViewError> + 'static,
    V: View,
{
    FunctionView(f)
}

impl<F, V> View for FunctionView<F>
where
    F: Fn(&mut ViewContext<'_>) -> Result<V, ViewError> + 'static,
    V: View,
{
    fn body(&self, cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        (self.0)(cx)
    }
}

/// Group children without introducing a host element.
pub fn fragment<V: View>(children: impl IntoIterator<Item = V>) -> impl View {
    Described(Description::Group(
        children.into_iter().map(View::into_any).collect(),
    ))
}
