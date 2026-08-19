//! Side-effect hooks for observing structural changes to the e-graph.
//!
//! A [`Hook`] lets external code react to the two events that mutate e-graph
//! structure — a new e-node being added and two e-classes merging — without
//! that logic being baked into the core engine. Typical uses: keeping an
//! operator index in sync, relocating external `Id`-keyed metadata when a class
//! is absorbed, or gathering metrics.
//!
//! Hooks observe events for their *side effects*; they cannot alter the merge
//! decision. Crucially, the engine fires them for **every** merge, including the
//! cascading congruence merges performed internally during `rebuild`, so an
//! external index never drifts out of sync.

use super::lang::Id;
use super::lang::Language;

/// Observer of e-graph structural events. All methods default to no-ops, so an
/// implementor need only override the events it cares about.
pub trait Hook<L: Language> {
    /// Called after a brand-new e-node `node` is interned as class`id`.
    fn on_add(&mut self, _id: Id, _node: &L) {}

    /// Called after two classes merge: `absorbed` no longer exists and all of
    /// its nodes/parents now live under `survivor`. External structures keyed
    /// by `absorbed` should be moved onto `survivor`.
    fn on_merge(&mut self, _survivor: Id, _absorbed: Id) {}
}

/// A hook that simply counts how many adds and merges have occurred. Useful for
/// metrics and for demonstrating the hook mechanism.
#[derive(Default)]
pub struct CountingHook {
    pub adds: usize,
    pub merges: usize,
}

impl<L: Language> Hook<L> for CountingHook {
    fn on_add(&mut self, _id: Id, _node: &L) {
        self.adds += 1;
    }
    fn on_merge(&mut self, _survivor: Id, _absorbed: Id) {
        self.merges += 1;
    }
}
