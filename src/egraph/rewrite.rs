//! Rewrite rules: patterns, substitutions, and the rule type.
//!
//! This module is the *declarative* half of the engine — it defines what a
//! rewrite rule looks like — while the *operational* half (matching,
//! instantiation, and the saturation loop) lives on [`EGraph`] in the
//! `egraph` module, since it needs privileged access to the graph internals.
//!
//! A rule has three moving parts, each of which can be **static** (a fixed
//! [`Pattern`]) or **dynamic** (a Rust closure):
//!
//! | part | static form | dynamic form |
//! |------|-------------|--------------|
//! | LHS | [`Pattern`] matched by the built-in e-matcher | [`Matcher`] — you compute the [`Subst`]s |
//! | RHS | [`Pattern`] template, instantiated from the match | [`Applier`] — you build and return the result class |
//! | guard| always fires | [`Condition`] — a predicate over the match |

use std::collections::HashMap;

use super::egraph::{Analysis, EGraph};
use super::lang::{Id, Language};

/// A pattern over language `L`.
///
/// * `Var` matches any e-class and binds it by name.
/// * `Op` matches nodes whose operator equals `node` (via [`Language::matches`])
/// and recursively matches `children`. The child ids inside `node` are
/// placeholders; only its operator and arity matter.
#[derive(Clone)]
pub enum Pattern<L: Language> {
    Var(String),
    Op { node: L, children: Vec<Pattern<L>> },
}

impl<L: Language> Pattern<L> {
    /// Build an operator pattern from a template `node` and its sub-patterns.
    pub fn op(node: L, children: Vec<Pattern<L>>) -> Self {
        Pattern::Op { node, children }
    }

    /// Build a variable pattern.
    pub fn var(name: &str) -> Self {
        Pattern::Var(name.to_string())
    }
}

/// Bindings from pattern variable name -> matched e-class.
pub type Subst = HashMap<String, Id>;

/// Reserved [`Subst`] key under which the engine records the **matched LHS
/// e-class** before invoking a dynamic RHS [`Applier`] or a [`Condition`].
///
/// Read it to recover the node the LHS matched, from inside the RHS:
/// ```ignore
/// let root = subst[MATCHED_LHS]; // the class the LHS matched
/// let node = eg.match_node(root, &template)?; // the actual matched e-node
/// ```
/// The name is chosen not to collide with ordinary pattern variables.
pub const MATCHED_LHS: &str = "$lhs";

/// A predicate that gates whether a [`Rewrite`] fires for a given match. It
/// receives the e-graph (so it can read e-class [`Analysis`] data) and the
/// substitution (variable bindings), and returns `true` to allow the rewrite.
pub type Condition<L, N> = Box<dyn Fn(&EGraph<L, N>, &Subst) -> bool>;

/// A dynamic right-hand side: given the e-graph and the match bindings, build
/// (and add) a term, returning its e-class to merge with the matched class —
/// or `None` to decline. This is `egg`'s `Applier` concept: the RHS is
/// *computed* from the match rather than a fixed template.
pub type Applier<L, N> = Box<dyn Fn(&mut EGraph<L, N>, &Subst) -> Option<Id>>;

/// Like [`Applier`], but the engine also hands you the **matched LHS e-node**
/// (`&L`) so you don't have to rebuild an operator template and look it up
/// yourself. For a static [`Pattern`] LHS this is the e-node whose operator the
/// pattern matched; for a dynamic [`Matcher`] LHS it is a representative node of
/// the matched class.
pub type NodeApplier<L, N> = Box<dyn Fn(&mut EGraph<L, N>, &Subst, &L) -> Option<Id>>;

/// A dynamic left-hand side: given the e-graph and a candidate e-class, produce
/// any number of substitutions (each a full set of placeholder bindings). This
/// is the matching-side mirror of [`Applier`]: instead of a static [`Pattern`],
/// *you* decide what matches and what to bind. Return an empty vec to match
/// nothing for the given class.
///
/// Use it when the *set of matches* can't be expressed structurally — e.g.
/// matching modulo commutativity, or selecting classes by their [`Analysis`]
/// data.
pub type Matcher<L, N> = Box<dyn Fn(&EGraph<L, N>, Id) -> Vec<Subst>>;

/// The left-hand side of a rewrite: either a static pattern (matched by the
/// built-in e-matcher) or a dynamic [`Matcher`] function.
pub enum Lhs<L: Language, N: Analysis<L>> {
    /// A structural pattern.
    Pattern(Pattern<L>),
    /// A function that computes substitutions from a candidate e-class.
    Dynamic(Matcher<L, N>),
}

/// The right-hand side of a rewrite: either a static pattern template or a
/// dynamic applier function.
pub enum Rhs<L: Language, N: Analysis<L>> {
    /// A fixed template; holes are filled from the substitution.
    Pattern(Pattern<L>),

    /// A function that computes the resulting e-class from the match.
    Dynamic(Applier<L, N>),

    /// Like [`Dynamic`](Rhs::Dynamic), but also receives the matched LHS e-node.
    DynamicNode(NodeApplier<L, N>),
}

/// A rewrite rule `lhs => rhs`, optionally guarded by a [`Condition`]. The
/// `lhs` may be a static [`Pattern`] or a dynamic [`Matcher`]; the `rhs` may be
/// a static [`Pattern`] or a dynamic [`Applier`].
pub struct Rewrite<L: Language, N: Analysis<L> = ()> {
    #[allow(dead_code)]
    pub name: &'static str,
    pub lhs: Lhs<L, N>,
    pub rhs: Rhs<L, N>,
    /// If present, the rule fires only when this predicate returns `true`.
    pub condition: Option<Condition<L, N>>,
}

impl<L: Language, N: Analysis<L>> Rewrite<L, N> {
    /// An unconditional rewrite `lhs => rhs` with a static pattern RHS.
    pub fn new(name: &'static str, lhs: Pattern<L>, rhs: Pattern<L>) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Pattern(lhs),
            rhs: Rhs::Pattern(rhs),
            condition: None,
        }
    }

    /// A conditional rewrite: fires only when `condition` holds for the match.
    pub fn conditional(
        name: &'static str,
        lhs: Pattern<L>,
        rhs: Pattern<L>,
        condition: impl Fn(&EGraph<L, N>, &Subst) -> bool + 'static,
    ) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Pattern(lhs),
            rhs: Rhs::Pattern(rhs),
            condition: Some(Box::new(condition)),
        }
    }

    /// A dynamic rewrite: the RHS is *computed* by `applier` from the match.
    /// The applier adds nodes to the graph and returns the e-class to merge
    /// with the matched class, or `None` to decline for this match.
    pub fn dynamic(
        name: &'static str,
        lhs: Pattern<L>,
        applier: impl Fn(&mut EGraph<L, N>, &Subst) -> Option<Id> + 'static,
    ) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Pattern(lhs),
            rhs: Rhs::Dynamic(Box::new(applier)),
            condition: None,
        }
    }

    /// A rewrite whose *left-hand side* is a dynamic [`Matcher`]: the matcher
    /// computes the substitutions itself (e.g. matching modulo commutativity,
    /// or selecting classes by analysis data) instead of a static pattern. Each
    /// substitution it returns is applied against the static template `rhs`, and
    /// the resulting class is merged with the candidate class the matcher ran on.
    pub fn dynamic_match(
        name: &'static str,
        matcher: impl Fn(&EGraph<L, N>, Id) -> Vec<Subst> + 'static,
        rhs: Pattern<L>,
    ) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Dynamic(Box::new(matcher)),
            rhs: Rhs::Pattern(rhs),
            condition: None,
        }
    }

    /// Like [`dynamic_match`](Self::dynamic_match), but the RHS is also dynamic:
    /// a full custom rule where both the matches and the result are computed.
    pub fn dynamic_match_apply(
        name: &'static str,
        matcher: impl Fn(&EGraph<L, N>, Id) -> Vec<Subst> + 'static,
        applier: impl Fn(&mut EGraph<L, N>, &Subst) -> Option<Id> + 'static,
    ) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Dynamic(Box::new(matcher)),
            rhs: Rhs::Dynamic(Box::new(applier)),
            condition: None,
        }
    }

    /// A dynamic rewrite like [`dynamic`](Self::dynamic), but the engine also
    /// passes the **matched LHS e-node** to `applier` as a third argument, so
    /// you can read its operands directly without rebuilding an operator
    /// template and calling [`EGraph::match_node`] yourself.
    pub fn dynamic_with_node(
        name: &'static str,
        lhs: Pattern<L>,
        applier: impl Fn(&mut EGraph<L, N>, &Subst, &L) -> Option<Id> + 'static,
    ) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Pattern(lhs),
            rhs: Rhs::DynamicNode(Box::new(applier)),
            condition: None,
        }
    }

    /// Like [`dynamic_match_apply`](Self::dynamic_match_apply) — a dynamic
    /// [`Matcher`] LHS with a dynamic RHS — but the engine also passes a
    /// representative **matched e-node** of the candidate class to `applier`,
    /// so you don't have to fetch it via [`EGraph::nodes`] yourself.
    pub fn dynamic_match_apply_with_node(
            name: &'static str,
            matcher: impl Fn(&EGraph<L, N>, Id) -> Vec<Subst> + 'static,
            applier: impl Fn(&mut EGraph<L, N>, &Subst, &L) -> Option<Id> + 'static,
            ) -> Self {
        Rewrite {
            name,
            lhs: Lhs::Dynamic(Box::new(matcher)),
            rhs: Rhs::DynamicNode(Box::new(applier)),
            condition: None,
        }
    }
}

/// Look up the e-class bound to pattern variable `name` in a `subst`.
/// Convenience for writing rewrite conditions and matchers.
pub fn binding(subst: &Subst, name: &str) -> Option<Id> {
    subst.get(name).copied()
}
