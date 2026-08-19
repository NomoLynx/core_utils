//! The language interface.
//!
//! `Id` identifies e-classes. `Language` is the trait your term language must
//! implement so the generic e-graph engine can operate on it — mirroring the
//! `Language` trait in the `egg` crate. Implement it for your own enum to plug
//! a completely different language into the same saturation engine.

use std::hash::Hash;

/// Identifier for an e-class. Always canonicalize with `EGraph::find` before use.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct Id(pub u32);

impl Id {
    /// A placeholder id, useful when building pattern template nodes whose
    /// child ids are ignored (only the operator/arity matters).
    pub const PLACEHOLDER: Id = Id(0);
}

macro_rules! impl_from_int_for_a {
    ($($t:ty),*) => {
        $(impl From<$t> for Id {
            fn from(v: $t) -> Self { Id(v as u32) }
        })*
    };
}

impl_from_int_for_a!(u8, u16, u32, i8, i16, i32, usize, isize);

/// A term language: an operator enum whose nodes carry child e-class `Id`s.
///
/// The engine is fully generic over this trait, so you can swap in your own
/// language by implementing it for your own node enum.
pub trait Language: Clone + PartialEq + Eq + Hash {
    /// Whether `self` and `other` are the *same operator* — same discriminant
    /// and same non-child payload (e.g. the same constant), ignoring the
    /// specific child ids. Used for e-matching and congruence.
    fn matches(&self, other: &Self) -> bool;

    /// This node's child e-class ids.
    fn children(&self) -> &[Id];

    /// Mutable access to this node's child e-class ids (for canonicalization
    /// and pattern instantiation).
    fn children_mut(&mut self) -> &mut [Id];

    /// Cost of this single operator (excluding children). Used by extraction.
    /// Override to make some operators cheaper (e.g. shift vs. multiply).
    fn cost(&self) -> usize {
        1
    }

    /// Render this node given its already-rendered children, for pretty output.
    fn display(&self, children: &[String]) -> String;
}
