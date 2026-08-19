//! Pluggable cost models for extraction, decoupled from the language.
//!
//! Modeled on `egg`'s `CostFunction`: given a node and a way to look up the
//! best cost of each child class, produce this node's total cost. Implement
//! [`CostFunction`] (or use a built-in) and pass it to
//! [`crate::egraph::EGraph::extract_best`].

use super::lang::{Id, Language};

/// A pluggable cost model for extraction, decoupled from the language.
pub trait CostFunction<L: Language> {
    /// The cost type. Any partially-ordered, cloneable value works, so costs
    /// can be integers, floats, tuples (for tie-breaking), etc.
    type Cost: PartialOrd + Clone;

    /// Cost of `node`, given a lookup for each child's already-computed cost.
    fn cost(&mut self, node: &L, child_cost: impl Fn(Id) -> Self::Cost) -> Self::Cost;
}

/// Counts AST nodes: every node costs 1 plus its children. Minimizes term size.
pub struct AstSize;

impl<L: Language> CostFunction<L> for AstSize {
    type Cost = usize;
    fn cost(&mut self, node: &L, child_cost: impl Fn(Id) -> usize) -> usize {
        node.children()
            .iter()
            .fold(1, |acc, &c| acc + child_cost(c))
    }
}

/// Minimizes tree depth: 1 + the maximum child depth.
pub struct AstDepth;

impl<L: Language> CostFunction<L> for AstDepth {
    type Cost = usize;
    fn cost(&mut self, node: &L, child_cost: impl Fn(Id) -> usize) -> usize {
        1 + node
            .children()
            .iter()
            .map(|&c| child_cost(c))
            .max()
            .unwrap_or(0)
    }
}

/// Uses each operator's own [`Language::cost`], summed over children. This is
/// the weighted model (e.g. shift cheaper than multiply).
pub struct LanguageCost;

impl<L: Language> CostFunction<L> for LanguageCost {
    type Cost = usize;
    fn cost(&mut self, node: &L, child_cost: impl Fn(Id) -> usize) -> usize {
        node.children()
            .iter()
            .fold(node.cost(), |acc, &c| acc + child_cost(c))
    }
}
