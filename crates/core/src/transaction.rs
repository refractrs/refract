//! Animation intent is scoped to one committed update, independently of state.
use crate::{Animation, AnyView, Description, Memo, Runtime, View};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transaction {
    pub animation: Option<Animation>,
    /// Prevent value-scoped animation modifiers from enabling animations.
    pub disables_animations: bool,
}

pub fn with_animation<R>(
    rt: &mut Runtime,
    animation: Animation,
    action: impl FnOnce(&mut Runtime) -> R,
) -> R {
    with_transaction(
        rt,
        Transaction {
            animation: Some(animation),
            disables_animations: false,
        },
        action,
    )
}

/// Scope synchronous writes. An asynchronous completion needs its own transaction.
pub fn with_transaction<R>(
    rt: &mut Runtime,
    transaction: Transaction,
    action: impl FnOnce(&mut Runtime) -> R,
) -> R {
    rt.with_mutation_context(transaction, action)
}

#[doc(hidden)]
#[derive(Clone)]
pub struct AnimationScope {
    pub(crate) value: Option<Memo>,
    pub(crate) modify: Rc<dyn Fn(&mut Transaction)>,
}

/// A composition boundary that changes transaction intent without a host box.
pub struct TransactionView {
    pub(crate) child: AnyView,
    pub(crate) scope: AnimationScope,
}

impl View for TransactionView {
    fn describe(&self) -> Option<Description> {
        Some(Description::Transaction(
            self.scope.clone(),
            self.child.clone(),
        ))
    }
}

#[derive(Clone, Default)]
pub(crate) struct AnimationContext {
    pub epoch: u64,
    pub transaction: Transaction,
    pub overrides: Vec<Rc<dyn Fn(&mut Transaction)>>,
}

impl AnimationContext {
    pub fn resolved(&self) -> Transaction {
        let mut result = self.transaction;
        for modify in &self.overrides {
            modify(&mut result);
        }
        if result.disables_animations {
            result.animation = None;
        }
        result
    }
}
