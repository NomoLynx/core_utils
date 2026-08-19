//! A minimal e-graph with equality saturation, inspired by the `egg` library.
//!
//! Core pieces (mirroring `egg`'s architecture):
//! * Union-find -> tracks which e-classes are equivalent.
//! * Hashcons -> deduplicates e-nodes (maps canonical e-node -> e-class).
//! * Deferred rebuild-> restores congruence in batches (egg's key optimization).
//! * Pattern matcher -> e-matching for rewrite-rule left-hand sides.
//! * Extractor -> picks the lowest-cost term from each e-class.
//!
//! The engine is generic over any [`Language`]; the concrete arithmetic
//! language lives in the `arith` module.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;
use std::hash::Hasher;

use super::hooks::Hook;
use super::lang::{Id, Language};
use super::rewrite::*;
use super::scheduler::{BackoffScheduler, RewriteScheduler};

pub use super::config::RunConfig;

/// A deterministic hash of a node, used to break equal-cost ties in extraction
/// so results are reproducible regardless of `HashMap` iteration order.
fn stable_hash<L: Language>(node: &L) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node.hash(&mut hasher);
    hasher.finish()
}

/// A single equivalence class: the set of e-nodes proven equal, plus the
/// analysis data derived for the class.
pub struct EClass<L: Language, N: Analysis<L>> {
    nodes: Vec<L>,
    /// Parent e-nodes that reference this class (op node, its owning class id).
    /// Used to propagate congruence when classes merge.
    parents: Vec<(L, Id)>,
    /// The e-class analysis data (e.g. `Option<i64>` for constant propagation).
    data: N::Data,
}

impl<L: Language, N: Analysis<L>> Default for EClass<L, N>
where
    N::Data: Default,
{
    fn default() -> Self {
        EClass {
            nodes: Vec::new(),
            parents: Vec::new(),
            data: N::Data::default(),
        }
    }
}

pub trait Analysis<L: Language>: Sized {
    /// The per-class lattice value. Must be comparable (to detect changes) and
    /// have a `Default` representing "no information yet".
    type Data: Clone + PartialEq + Default;

    /// Compute the data for `node` given the current graph (its children
    /// already have data). Called when a new e-node is added.
    fn make(egraph: &EGraph<L, Self>, node: &L) -> Self::Data;

    /// Merge `b` into `a` in place when two classes unite. Return `true` if
    /// `a` changed, so the engine can re-run `modify`.
    fn merge(&mut self, a: &mut Self::Data, b: Self::Data) -> bool;

    /// Optional hook invoked after a class's data changes. Use it to add nodes
    /// or unions (e.g. materialize a folded constant). Default: no-op.
    fn modify(_egraph: &mut EGraph<L, Self>, _id: Id) {}
}

/// The trivial no-op analysis. Used as the default so the e-graph can run with
/// no analysis at all.
impl<L: Language> Analysis<L> for () {
    type Data = ();

    fn make(_egraph: &EGraph<L, Self>, _node: &L) {}
    fn merge(&mut self, _a: &mut (), _b: ()) -> bool {
        false
    }
}

/// Why two terms were made equal — the label on a proof-forest edge.
#[derive(Clone)]
pub enum Justification {
    /// A rewrite rule fired (or an internal reason like `"analysis"`).
    Rule(&'static str),
    /// Two e-nodes became structurally identical because their children were
    /// merged. The pairs are the (owner-child, canonical-child) ids that must
    /// be explained recursively to justify this step.
    Congruence(Vec<(Id, Id)>),
}

/// The e-graph, generic over a term language `L` and an analysis `N`.
pub struct EGraph<L: Language, N: Analysis<L> = ()> {
    /// The analysis instance (holds any mutable analysis state).
    analysis: N,
    /// Union-find parent pointers, indexed by `Id`.
    union_find: Vec<Id>,
    /// Canonical e-node -> e-class id. The hashcons / memo table.
    memo: HashMap<L, Id>,
    /// e-class id -> class data.
    classes: HashMap<Id, EClass<L, N>>,
    /// Classes whose congruence may be stale; processed during `rebuild`.
    pending: Vec<Id>,
    /// Classes whose analysis data changed; `modify` runs on them in `rebuild`.
    analysis_pending: Vec<Id>,
    /// Proof forest for explanations: undirected justification edges between the
    /// exact ids passed to each `merge`. Two terms are equal iff connected here.
    /// Each edge is labelled with *why* the merge happened (see [`Justification`]).
    /// Unlike the union-find, these edges are never compressed away, so the full
    /// history survives.
    explain_edges: HashMap<Id, Vec<(Id, Justification)>>,
    /// The justification applied to the next `merge`. Set by `run` to the firing
    /// rule, and by `rebuild` to a congruence proof; defaults to a generic rule.
    current_reason: Justification,
    /// A frozen textual form of each id's term, captured when the id is created
    /// (before any merge collapses classes). Explanations print these so each
    /// proof line shows a *distinct* intermediate term rather than the whole
    /// merged class's cheapest extraction.
    term_snapshot: HashMap<Id, String>,
    /// Side-effect observers, fired on every `add` and `merge` (including the
    /// cascading congruence merges inside `rebuild`).
    hooks: Vec<Box<dyn Hook<L>>>,
}

impl<L: Language, N: Analysis<L> + Default> Default for EGraph<L, N> {
    fn default() -> Self {
        EGraph {
            analysis: N::default(),
            union_find: Vec::new(),
            memo: HashMap::new(),
            classes: HashMap::new(),
            pending: Vec::new(),
            analysis_pending: Vec::new(),
            explain_edges: HashMap::new(),
            current_reason: Justification::Rule("congruence"),
            term_snapshot: HashMap::new(),
            hooks: Vec::new(),
        }
    }
}

impl<L: Language, N: Analysis<L> + Default> EGraph<L, N> {
    pub fn new() -> Self {
        EGraph::default()
    }
}

impl<L: Language, N: Analysis<L>> EGraph<L, N> {
    /// Canonical class id for `id`, without path compression (safe on `&self`).
    /// Exposed so external tools (e.g. visualization) can group nodes by class.
    pub fn canonical_id(&self, id: Id) -> Id {
        self.find_immut(id)
    }

    /// Read-only view of the e-nodes in `id`'s class (borrow, no clone).
    pub fn nodes(&self, id: Id) -> &[L] {
        &self.classes[&self.find_immut(id)].nodes
    }

    /// Canonical class id of an already-interned e-node, or `None` if that
    /// exact node was never added. Read-only: canonicalizes `node`'s children
    /// and probes the hashcons table without inserting anything.
    ///
    /// This is the inverse of [`add`](Self::add) without the side effect, and
    /// the key to recovering the LHS-matched node from inside a dynamic RHS
    /// applier: rebuild the matched operator from the [`Subst`] bindings, look
    /// up its class here, then read the stored node via [`nodes`](Self::nodes).
    pub fn lookup(&self, node: &L) -> Option<Id> {
        let mut node = node.clone();
        for child in node.children_mut() {
            *child = self.find_immut(*child);
        }
        self.memo.get(&node).map(|&id| self.find_immut(id))
    }

    /// Find the e-node in `id`'s class that matches `template` (same operator
    /// and arity, via [`Language::matches`]). Returns the first such stored
    /// node, or `None`.
    pub fn match_node(&self, id: Id, template: &L) -> Option<&L> {
        let id = self.find_immut(id);
        self.classes
            .get(&id)?
            .nodes
            .iter()
            .find(|n| n.matches(template))
    }

    /// Compare the classes of `a` and `b` with a caller-supplied function.
    ///
    /// If `a` and `b` are already the *same* canonical class, returns
    /// [`Ordering::Equal`](std::cmp::Ordering::Equal) without invoking `cmp`.
    /// Otherwise `cmp` is called with each class's e-nodes and analysis data,
    /// so you can order/compare classes by any custom criterion (node count,
    /// cheapest operator, analysis lattice value, etc.).
    ///
    /// Note: this is a *structural/heuristic* comparison, not e-graph equality.
    /// Two classes being ordered `Equal` here does not mean their terms are
    /// proven equal — for that, compare [`canonical_id`](Self::canonical_id).
    pub fn compare_classes<F>(&self, a: Id, b: Id, cmp: F) -> std::cmp::Ordering
    where
        F: FnOnce(ClassView<'_, L, N>, ClassView<'_, L, N>) -> std::cmp::Ordering,
    {
        let (ca, cb) = (self.find_immut(a), self.find_immut(b));
        if ca == cb {
            return std::cmp::Ordering::Equal;
        }
        let va = ClassView {
            nodes: &self.classes[&ca].nodes,
            data: &self.classes[&ca].data,
        };
        let vb = ClassView {
            nodes: &self.classes[&cb].nodes,
            data: &self.classes[&cb].data,
        };
        cmp(va, vb)
    }

    /// Structurally compare two **specific e-nodes** with a custom, per-node
    /// predicate, recursing through their children.
    ///
    /// `pred` decides whether two individual e-nodes "match" at a single level
    /// (your own notion — e.g. same operator ignoring a payload, or matching
    /// modulo some rename). Two nodes are considered a structural match when:
    /// 1. `pred(a, b)` holds, **and**
    /// 2. they have the same number of children, **and**
    /// 3. every child position matches: for the child classes `a_i` and
    /// `b_i`, *some* e-node in `a_i`'s class structurally matches *some*
    /// e-node in `b_i`'s class (existential, since a class holds many
    /// equivalent nodes).
    ///
    /// This is deliberately **not** e-graph semantic equality: the graph's own
    /// equality is "are these the same e-class"; here you define matching by
    /// `pred` at every level and the recursion is driven by structure, so two
    /// nodes can match even if they live in different classes (and vice versa).
    ///
    /// A visited-set guards against the cyclic classes an e-graph may contain,
    /// so the recursion always terminates.
    pub fn nodes_match_deep<F>(&self, a: &L, b: &L, pred: &F) -> bool
    where
        F: Fn(&L, &L) -> bool,
    {
        let mut visited = std::collections::HashSet::new();
        self.nodes_match_deep_inner(a, b, pred, &mut visited)
    }

    fn nodes_match_deep_inner<F>(
        &self,
        a: &L,
        b: &L,
        pred: &F,
        visited: &mut std::collections::HashSet<(Id, Id)>,
    ) -> bool
    where
        F: Fn(&L, &L) -> bool,
    {
        // Level check: the caller's custom node predicate must accept this pair.
        if !pred(a, b) {
            return false;
        }
        let (ac, bc) = (a.children(), b.children());
        if ac.len() != bc.len() {
            return false;
        }
        // Each child position: some node of a's child class must structurally
        // match some node of b's child class.
        for (&ai, &bi) in ac.iter().zip(bc.iter()) {
            let (ai, bi) = (self.find_immut(ai), self.find_immut(bi));
            // Cycle guard: if we're already comparing this class pair upstream,
            // treat it as matching to break the cycle.
            if !visited.insert((ai, bi)) {
                continue;
            }
            let a_nodes = self.classes.get(&ai).map(|c| &c.nodes[..]).unwrap_or(&[]);
            let b_nodes = self.classes.get(&bi).map(|c| &c.nodes[..]).unwrap_or(&[]);
            let matched = a_nodes.iter().any(|an| {
                b_nodes
                    .iter()
                    .any(|bn| self.nodes_match_deep_inner(an, bn, pred, visited))
            });
            visited.remove(&(ai, bi));
            if !matched {
                return false;
            }
        }
        true
    }

    /// Snapshot of the e-graph for visualization: each canonical e-class id
    /// paired with a clone of its e-nodes. Iteration order is unspecified.
    pub fn classes_snapshot(&self) -> Vec<(Id, Vec<L>)> {
        let mut out: Vec<(Id, Vec<L>)> = self
            .classes
            .iter()
            .filter(|(id, _)| self.find_immut(**id) == **id)
            .map(|(id, class)| (*id, class.nodes.clone()))
            .collect();
        out.sort_by_key(|(id, _)| id.0);
        out
    }

    /// Read-only access to a class's analysis data.
    pub fn data(&self, id: Id) -> &N::Data {
        &self.classes[&self.find_immut(id)].data
    }

    /// Register a side-effect [`Hook`] to observe future `add`/`merge` events.
    /// Multiple hooks may be registered; each is notified of every event.
    pub fn add_hook(&mut self, hook: Box<dyn Hook<L>>) {
        self.hooks.push(hook);
    }

    /// Fire `on_add` on every registered hook. The hook vec is temporarily
    /// moved out so hooks can run without borrowing the rest of `self`.
    fn fire_on_add(&mut self, id: Id, node: &L) {
        let mut hooks = std::mem::take(&mut self.hooks);
        for h in &mut hooks {
            h.on_add(id, node);
        }
        self.hooks = hooks;
    }

    /// Fire `on_merge` on every registered hook.
    fn fire_on_merge(&mut self, survivor: Id, absorbed: Id) {
        let mut hooks = std::mem::take(&mut self.hooks);
        for h in &mut hooks {
            h.on_merge(survivor, absorbed);
        }
        self.hooks = hooks;
    }

    // ---- Union-find ------------------------------------------------------

    /// Canonical representative of `id`, with path halving.
    pub fn find(&mut self, id: Id) -> Id {
        let mut current = id;
        while self.union_find[current.0 as usize] != current {
            let parent = self.union_find[current.0 as usize];
            // Path halving: point to grandparent.
            let grand = self.union_find[parent.0 as usize];
            self.union_find[current.0 as usize] = grand;
            current = grand;
        }
        current
    }

    /// Read-only find (no compression) for use in `&self` contexts.
    fn find_immut(&self, id: Id) -> Id {
        let mut current = id;
        while self.union_find[current.0 as usize] != current {
            current = self.union_find[current.0 as usize];
        }
        current
    }

    fn make_set(&mut self) -> Id {
        let id = Id(self.union_find.len() as u32);
        self.union_find.push(id);
        self.classes.insert(id, EClass::default());
        id
    }

    // ---- Canonicalization ------------------------------------------------

    /// Rewrite a node's children to their canonical class ids.
    fn canonicalize(&mut self, node: &mut L) {
        for child in node.children_mut() {
            *child = self.find(*child);
        }
    }

    // ---- Core operations: add / merge ------------------------------------

    /// Add an e-node, returning the e-class it belongs to. Deduplicates via the
    /// hashcons table so structurally identical nodes share a class.
    pub fn add(&mut self, mut node: L) -> Id {
        self.canonicalize(&mut node);
        if let Some(&existing) = self.memo.get(&node) {
            return self.find(existing);
        }

        // Compute analysis data before inserting (children already have data).
        let data = N::make(self, &node);

        let id = self.make_set();
        // Register this node as a parent of each of its children.
        let children: Vec<Id> = node.children().to_vec();
        for child in children {
            let child = self.find(child);
            self.classes
                .get_mut(&child)
                .unwrap()
                .parents
                .push((node.clone(), id));
        }
        self.memo.insert(node.clone(), id);
        let class = self.classes.get_mut(&id).unwrap();
        class.nodes.push(node);
        class.data = data;
        // Freeze this id's term for explanations, built from children's frozen
        // terms (which already exist, since children were added earlier).
        let node_ref = &self.classes[&id].nodes[0];
        let child_terms: Vec<String> = node_ref
            .children()
            .iter()
            .map(|c| {
                let cc = self.find_immut(*c);
                self.term_snapshot
                    .get(&cc)
                    .cloned()
                    .unwrap_or_else(|| "?".to_string())
            })
            .collect();
        let snap = self.classes[&id].nodes[0].display(&child_terms);
        self.term_snapshot.insert(id, snap);
        // Let the analysis react to the new class's data during rebuild.
        self.analysis_pending.push(id);
        // Notify observers of the newly created e-node.
        let node_clone = self.classes[&id].nodes[0].clone();
        self.fire_on_add(id, &node_clone);
        id
    }

    /// Classic union-find naming alias for [`merge`](Self::merge).
    #[allow(dead_code)]
    pub fn union(&mut self, a: Id, b: Id) -> Id {
        self.merge(a, b)
    }

    /// Merge the classes of `a` and `b`; returns the surviving canonical id.
    /// Actual congruence repair is deferred to `rebuild`.
    pub fn merge(&mut self, a: Id, b: Id) -> Id {
        // Raw ids (before canonicalization) become an edge in the proof forest.
        let (raw_a, raw_b) = (a, b);
        let mut a = self.find(a);
        let mut b = self.find(b);
        if a == b {
            return a;
        }
        // Record why these two terms were made equal, for later explanation.
        let reason = self.current_reason.clone();
        self.explain_edges
            .entry(raw_a)
            .or_default()
            .push((raw_b, reason.clone()));
        self.explain_edges
            .entry(raw_b)
            .or_default()
            .push((raw_a, reason));

        // Merge smaller class into larger (union by size) to keep trees flat.
        let a_len = self.classes[&a].nodes.len() + self.classes[&a].parents.len();
        let b_len = self.classes[&b].nodes.len() + self.classes[&b].parents.len();
        if a_len < b_len {
            std::mem::swap(&mut a, &mut b);
        }

        // b is absorbed into a.
        self.union_find[b.0 as usize] = a;
        let b_class = self.classes.remove(&b).unwrap();
        let b_data = b_class.data;
        let a_class = self.classes.get_mut(&a).unwrap();
        a_class.nodes.extend(b_class.nodes);
        a_class.parents.extend(b_class.parents);

        // Combine analysis data; if it changed, schedule `modify` for `a`.
        let mut a_data = std::mem::take(&mut a_class.data);
        let changed = self.analysis.merge(&mut a_data, b_data);
        self.classes.get_mut(&a).unwrap().data = a_data;
        if changed {
            self.analysis_pending.push(a);
        }

        // a's parents may now be congruent with others -> revisit in rebuild.
        self.pending.push(a);
        // Notify observers: class `b` was absorbed into surviving class `a`.
        self.fire_on_merge(a, b);
        a
    }

    // ---- Rebuild: restore congruence + hashcons invariants ---------------

    /// Restore congruence: after merges, re-canonicalize parent e-nodes and
    /// merge any that became structurally identical. Runs to a fixpoint.
    /// This batched approach is `egg`'s central performance idea.
    pub fn rebuild(&mut self) {
        loop {
            // ---- Congruence closure: process pending merges to a fixpoint.
            while let Some(class) = self.pending.pop() {
                let class = self.find(class);
                let parents = std::mem::take(&mut self.classes.get_mut(&class).unwrap().parents);

                // Re-intern each parent node under its canonical form. If two
                // parents canonicalize to the same node, their classes merge.
                let mut new_parents: Vec<(L, Id)> = Vec::with_capacity(parents.len());
                for (mut node, owner) in parents {
                    // Keep the pre-canonicalization form so we can pair its
                    // children against the canonical children for the proof.
                    let orig = node.clone();
                    self.canonicalize(&mut node);
                    if let Some(&existing) = self.memo.get(&node) {
                        let existing = self.find(existing);
                        let owner_c = self.find(owner);
                        if existing != owner_c {
                            // This merge is by congruence: the two nodes match
                            // because their children became equal. Record which
                            // child pairs must be explained to justify it.
                            let pairs: Vec<(Id, Id)> = orig
                                .children()
                                .iter()
                                .zip(node.children().iter())
                                .map(|(&o, &c)| (o, c))
                                .filter(|(o, c)| o != c)
                                .collect();
                            self.current_reason = Justification::Congruence(pairs);
                            self.merge(existing, owner_c);
                            self.current_reason = Justification::Rule("congruence");
                        }
                    }
                    let owner_c = self.find(owner);
                    self.memo.insert(node.clone(), owner_c);
                    new_parents.push((node, owner_c));
                }

                let class = self.find(class);
                self.classes
                    .get_mut(&class)
                    .unwrap()
                    .parents
                    .extend(new_parents);
            }

            // Congruence is now stable. Rebuild the hashcons table so stale
            // keys are dropped and duplicate e-nodes per class collapse.
            self.collect_memo();

            // ---- Analysis: propagate data upward, then run `modify`.
            //
            // When a class's data changes, its parents may now be computable
            // (or improvable), so we recompute their data and cascade upward.
            // `modify` can add nodes/unions (e.g. materialize a constant),
            // repopulating `pending`, so the outer loop reruns until stable.
            if self.analysis_pending.is_empty() {
                break;
            }
            let changed = self.propagate_analysis();
            let mut seen = std::collections::HashSet::new();
            for id in changed {
                let id = self.find(id);
                if seen.insert(id) {
                    N::modify(self, id);
                }
            }
        }
    }

    /// Recompute analysis data bottom-up from the changed classes, cascading
    /// to parents until a fixpoint. Returns every class whose data ended up
    /// changed (so `modify` can run on them).
    fn propagate_analysis(&mut self) -> Vec<Id> {
        let mut worklist: Vec<Id> = std::mem::take(&mut self.analysis_pending);
        let mut changed_classes: Vec<Id> = Vec::new();

        while let Some(id) = worklist.pop() {
            let id = self.find(id);
            changed_classes.push(id);

            // Snapshot the parents referencing this class, then recompute each
            // parent's data and merge it into the parent's owner class.
            let parents: Vec<(L, Id)> = self.classes[&id].parents.clone();
            for (node, owner) in parents {
                let owner = self.find(owner);
                let new_data = N::make(self, &node);
                let mut owner_data =
                    std::mem::take(&mut self.classes.get_mut(&owner).unwrap().data);
                let did_change = self.analysis.merge(&mut owner_data, new_data);
                self.classes.get_mut(&owner).unwrap().data = owner_data;
                if did_change {
                    worklist.push(owner);
                }
            }
        }

        changed_classes
    }

    /// Reconstruct the `memo` table and drop duplicate e-nodes per class.
    ///
    /// After a batch of merges, the old `memo` may hold keys whose children
    /// point to now-absorbed classes. Rather than track and prune each stale
    /// key, we clear the table and re-intern every live node in canonical form.
    fn collect_memo(&mut self) {
        self.memo.clear();

        let ids: Vec<Id> = self.classes.keys().copied().collect();
        for id in ids {
            let id = self.find(id);
            let mut nodes = std::mem::take(&mut self.classes.get_mut(&id).unwrap().nodes);

            // Canonicalize every node, then dedup: congruent nodes in the same
            // class become identical and collapse to one entry.
            for node in &mut nodes {
                self.canonicalize(node);
            }
            let mut seen: HashMap<L, ()> = HashMap::with_capacity(nodes.len());
            nodes.retain(|node| seen.insert(node.clone(), ()).is_none());

            for node in &nodes {
                self.memo.insert(node.clone(), id);
            }
            self.classes.get_mut(&id).unwrap().nodes = nodes;
        }
    }

    /// Debug-only structural self-check. Verifies the invariants that must hold
    /// after [`rebuild`](Self::rebuild); panics with a descriptive message on
    /// the first violation. Cheap enough to call in tests and after saturation.
    ///

    /// Checks:
    /// 1. Union-find consistency — every class key is its own root; every
    /// non-root id is absent from `classes`.
    /// 2. Node canonicality — every stored node's children are canonical roots.
    /// 3. Hashcons agreement — `memo` maps each canonical node to its class.
    /// 4. Congruence closure — no two distinct classes share a canonical node.
    /// 5. Parent-list correctness — a node references a class iff that class
    /// lists the node as a parent.
    pub fn check_invariants(&self) {
        // 1. Union-find: class keys are roots; non-roots aren't class keys.
        for id_raw in 0..self.union_find.len() {
            let id = Id(id_raw as u32);
            let root = self.find_immut(id);
            if self.union_find[id_raw] == id {
                assert!(
                    self.classes.contains_key(&id),
                    "invariant 1: root {id:?} has no class"
                );
            } else {
                assert!(
                    !self.classes.contains_key(&id),
                    "invariant 1: non-root {id:?} still has a class"
                );
            }
            assert!(
                self.classes.contains_key(&root),
                "invariant 1: root {root:?} of {id:?} missing from classes"
            );
        }

        // 2 & 3. Every node is canonical and interned to its own class.
        for (&id, class) in &self.classes {
            assert_eq!(self.find_immut(id), id, "invariant 1: {id:?} not canonical");
            for node in &class.nodes {
                for &child in node.children() {
                    assert_eq!(
                        self.find_immut(child),
                        child,
                        "invariant 2: node in {id:?} has non-canonical child {child:?}"
                    );
                }
                match self.memo.get(node) {
                    Some(&owner) => assert_eq!(
                        self.find_immut(owner),
                        id,
                        "invariant 3: memo maps a node of {id:?} to a different class"
                    ),
                    None => panic!("invariant 3: node of {id:?} missing from memo"),
                }
            }
        }

        // 4. Congruence closure: no canonical node appears in two classes.
        let mut seen: HashMap<&L, Id> = HashMap::new();
        for (&id, class) in &self.classes {
            for node in &class.nodes {
                if let Some(&other) = seen.get(node) {
                    assert_eq!(
                        other, id,
                        "invariant 4: identical node in classes {other:?} and {id:?} \
(congruence not closed)"
                    );
                }
                seen.insert(node, id);
            }
        }

        // 5. Parent lists match actual child references, in both directions.
        // Forward: every (node, owner) parent entry is real and canonical.
        for (&id, class) in &self.classes {
            for (pnode, owner) in &class.parents {
                let owner = self.find_immut(*owner);
                assert!(
                    self.classes.contains_key(&owner),
                    "invariant 5: parent of {id:?} owned by missing class {owner:?}"
                );
                assert!(
                    pnode.children().iter().any(|&c| self.find_immut(c) == id),
                    "invariant 5: parent entry in {id:?} does not reference it as a child"
                );
            }
        }
        // Backward: every child reference is registered in that child's parents.
        for (&id, class) in &self.classes {
            for node in &class.nodes {
                for &child in node.children() {
                    let child = self.find_immut(child);
                    let listed = self.classes[&child]
                        .parents
                        .iter()
                        .any(|(pnode, powner)| self.find_immut(*powner) == id && pnode == node);
                    assert!(
                        listed,
                        "invariant 5: {child:?} missing parent entry for a node of {id:?}"
                    );
                }
            }
        }
    }

    // ---- Extraction ------------------------------------------------------

    /// Extract the lowest-cost term for `root` under a pluggable `cost_fn`.
    ///
    /// The cost model is decoupled from the language: pass any
    /// [`CostFunction`] (e.g. [`AstSize`], [`AstDepth`], [`LanguageCost`], or
    /// your own) to extract with different objectives without touching the
    /// language definition.
    pub fn extract_best<CF>(&self, root: Id, cost_fn: &mut CF) -> Expr<L>
    where
        CF: CostFunction<L>,
    {
        let mut best_cost: HashMap<Id, CF::Cost> = HashMap::new();
        let mut best_node: HashMap<Id, L> = HashMap::new();

        // Iterate to a fixpoint: a node is costed only once allits children
        // have a known cost; better (smaller) costs propagate upward.
        let mut changed = true;
        while changed {
            changed = false;
            for (&id, class) in &self.classes {
                for node in &class.nodes {
                    // Skip until every child already has a best cost.
                    let ready = node
                        .children()
                        .iter()
                        .all(|&c| best_cost.contains_key(&self.find_immut(c)));
                    if !ready {
                        continue;
                    }

                    let cost = cost_fn.cost(node, |c| best_cost[&self.find_immut(c)].clone());

                    // Update on a strictly lower cost, or — for deterministic
                    // results — on an equal cost when this node has a smaller
                    // stable hash than the current pick.
                    let improved = match best_cost.get(&id) {
                        None => true,
                        Some(current) => match cost.partial_cmp(current) {
                            Some(std::cmp::Ordering::Less) => true,
                            Some(std::cmp::Ordering::Equal) => {
                                stable_hash(node) < stable_hash(&best_node[&id])
                            }
                            _ => false,
                        },
                    };
                    if improved {
                        best_cost.insert(id, cost);
                        best_node.insert(id, node.clone());
                        changed = true;
                    }
                }
            }
        }

        self.build_expr(self.find_immut(root), &best_node)
    }

    fn build_expr(&self, id: Id, best: &HashMap<Id, L>) -> Expr<L> {
        let mut on_path = std::collections::HashSet::new();
        self.build_expr_guarded(id, best, &mut on_path)
            .expect("root e-class has no finite (acyclic) representative")
    }

    /// Build a finite expression tree for `id`, guarding against cycles.
    ///
    /// The cost fixpoint in `extract_best` normally picks a class's cheapest
    /// node, which is acyclic. But an e-class can contain a node that refers
    /// back to itself (e.g. after unioning `x` with `x + 0`). If the chosen
    /// representative were cyclic, naive recursion would loop forever. We track
    /// the ids currently on the recursion path and refuse to re-enter one,
    /// returning `None` so the caller can fall back to another node.
    fn build_expr_guarded(
        &self,
        id: Id,
        best: &HashMap<Id, L>,
        on_path: &mut std::collections::HashSet<Id>,
    ) -> Option<Expr<L>> {
        let id = self.find_immut(id);
        if !on_path.insert(id) {
            // `id` is already an ancestor -> following it would cycle.
            return None;
        }

        // Try the fixpoint's pick first, then any other node in the class, so a
        // cyclic representative can be bypassed by an acyclic alternative.
        let class = self.classes.get(&id);
        let preferred = best.get(&id);
        let candidates = preferred
            .into_iter()
            .chain(class.into_iter().flat_map(|c| c.nodes.iter()));

        let mut result = None;
        for node in candidates {
            let mut kids = Vec::with_capacity(node.children().len());
            let mut ok = true;
            for &c in node.children() {
                match self.build_expr_guarded(c, best, on_path) {
                    Some(k) => kids.push(k),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                result = Some(Expr {
                    node: node.clone(),
                    children: kids,
                });
                break;
            }
        }

        on_path.remove(&id);
        result
    }
}

/// A concrete extracted expression tree (for display).
pub struct Expr<L: Language> {
    node: L,
    children: Vec<Expr<L>>,
}

impl<L: Language> fmt::Display for Expr<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kids: Vec<String> = self.children.iter().map(|c| c.to_string()).collect();
        write!(f, "{}", self.node.display(&kids))
    }
}

// ============================ Cost functions ==============================

// Cost models live in the `cost` module; re-exported here for convenience.
pub use super::cost::{AstDepth, AstSize, CostFunction, LanguageCost};

impl<L: Language, N: Analysis<L>> EGraph<L, N> {
    /// Find all substitutions under which `pattern` matches e-class `id`.
    fn match_pattern(&self, pattern: &Pattern<L>, id: Id, subst: &Subst) -> Vec<Subst> {
        let id = self.find_immut(id);
        match pattern {
            Pattern::Var(name) => {
                // If already bound, it must match the same class.
                match subst.get(name) {
                    Some(&bound) if self.find_immut(bound) != id => vec![],
                    _ => {
                        let mut s = subst.clone();
                        s.insert(name.clone(), id);
                        vec![s]
                    }
                }
            }
            Pattern::Op { node, children } => {
                let mut results = Vec::new();
                for enode in &self.classes[&id].nodes {
                    if enode.matches(node) {
                        // Match each child pattern against the corresponding
                        // child class, threading substitutions.
                        let mut substs = vec![subst.clone()];
                        for (cp, &cid) in children.iter().zip(enode.children()) {
                            let mut next = Vec::new();
                            for s in &substs {
                                next.extend(self.match_pattern(cp, cid, s));
                            }
                            substs = next;
                            if substs.is_empty() {
                                break;
                            }
                        }
                        results.extend(substs);
                    }
                }
                results
            }
        }
    }

    /// Instantiate `pattern` under `subst`, adding needed nodes; returns its class.
    fn instantiate(&mut self, pattern: &Pattern<L>, subst: &Subst) -> Id {
        match pattern {
            Pattern::Var(name) => self.find(subst[name]),
            Pattern::Op { node, children } => {
                let ids: Vec<Id> = children
                    .iter()
                    .map(|c| self.instantiate(c, subst))
                    .collect();
                let mut node = node.clone();
                for (slot, id) in node.children_mut().iter_mut().zip(ids) {
                    *slot = id;
                }
                self.add(node)
            }
        }
    }

    /// Run equality saturation with the default [`BackoffScheduler`], which
    /// temporarily bans rules that match explosively so no single rule can
    /// dominate the search. See [`EGraph::run_with_scheduler`] for details.
    pub fn run(&mut self, rules: &[Rewrite<L, N>], config: RunConfig) {
        let mut scheduler = BackoffScheduler::default();
        self.run_with_scheduler(rules, config, &mut scheduler);
    }

    /// Run equality saturation until a stop condition in `config` is hit:
    /// saturation (no enabled rule produced a merge), the iteration limit, or
    /// the node-count cap. The `scheduler` decides, each iteration, which rules
    /// are allowed to fire and observes how many matches each produced.
    ///
    /// Note: saturation is only declared when the last iteration made no
    /// progress *and* no rule is currently banned — a banned rule may still
    /// have work to do once it is un-banned.
    pub fn run_with_scheduler<S: RewriteScheduler<L, N>>(
        &mut self,
        rules: &[Rewrite<L, N>],
        config: RunConfig,
        scheduler: &mut S,
    ) {
        for iter in 0..config.max_iters {
            // ---- Node-count cap: stop before doing more work if too large.
            if let Some(limit) = config.node_limit {
                if self.total_nodes() >= limit {
                    eprintln!("Stopped at node limit ({limit}) after {iter} iteration(s).");
                    return;
                }
            }

            // ---- Match phase (read-only): collect every applicable rewrite,
            // but only for rules the scheduler currently enables.
            let ids: Vec<Id> = self.classes.keys().copied().collect();
            let mut matches: Vec<(&Rewrite<L, N>, Id, Subst)> = Vec::new();
            let mut any_banned = false;
            for (idx, rule) in rules.iter().enumerate() {
                if !scheduler.is_enabled(iter, idx, rule.name) {
                    any_banned = true;
                    continue;
                }
                let mut rule_matches = 0usize;
                for &id in &ids {
                    // Static pattern or dynamic matcher: both yield substitutions
                    // for the candidate class `id`.
                    let substs = match &rule.lhs {
                        Lhs::Pattern(p) => self.match_pattern(p, id, &Subst::new()),
                        Lhs::Dynamic(matcher) => matcher(self, id),
                    };
                    for mut subst in substs {
                        // Thread the matched LHS class through to the RHS/guard
                        // under a reserved key, so an applier can recover the
                        // exact matched node via `match_node`/`nodes`.
                        subst.insert(super::rewrite::MATCHED_LHS.to_string(), self.find_immut(id));

                        // Gate on the rule's condition, if any.
                        if let Some(cond) = &rule.condition {
                            if !cond(self, &subst) {
                                continue;
                            }
                        }
                        rule_matches += 1;
                        matches.push((rule, id, subst));
                    }
                }
                // Report the match count so the scheduler can throttle
                // explosive rules (possibly banning them next iteration).
                scheduler.on_matches(iter, idx, rule.name, rule_matches);
            }

            // ---- Apply phase (write): produce RHS and merge with LHS.
            let mut did_merge = false;
            for (rule, root, subst) in matches {
                // The class to merge the RHS into: for a static pattern LHS it is
                // the pattern re-instantiated from the match; for a dynamic
                // matcher it is the candidate class the matcher ran on.
                let lhs_id = match &rule.lhs {
                    Lhs::Pattern(p) => self.instantiate(p, &subst),
                    Lhs::Dynamic(_) => self.find(root),
                };
                // The RHS is either a static template or a computed class.
                let rhs_id = match &rule.rhs {
                    Rhs::Pattern(p) => Some(self.instantiate(p, &subst)),
                    Rhs::Dynamic(applier) => applier(self, &subst),
                    Rhs::DynamicNode(applier) => {
                        // Recover the matched LHS e-node so the applier doesn't
                        // have to rebuild an operator template. For a static
                        // pattern LHS, match on its root operator; for a dynamic
                        // matcher LHS, use a representative node of the class.
                        let matched = match &rule.lhs {
                            Lhs::Pattern(Pattern::Op { node, .. }) => {
                                self.match_node(root, node).cloned()
                            }
                            _ => self.nodes(root).first().cloned(),
                        };
                        match matched {
                            Some(node) => applier(self, &subst, &node),
                            None => None,
                        }
                    }
                };
                if let Some(rhs_id) = rhs_id {
                    if self.find(lhs_id) != self.find(rhs_id) {
                        // Justify the upcoming merge with this rule's name so
                        // `explain` can report it. Reset afterwards so any
                        // internal (congruence) merges are labelled correctly.
                        self.current_reason = Justification::Rule(rule.name);
                        self.merge(lhs_id, rhs_id);
                        self.current_reason = Justification::Rule("congruence");
                        did_merge = true;
                    }
                }
            }

            // ---- Rebuild phase: restore congruence in one batch.
            self.rebuild();

            // Only saturate when nothing merged AND no rule is sitting banned;
            // a banned rule might stillproduce merges once it is released.
            if !did_merge && !any_banned {
                eprintln!("Saturated after {} iteration(s).", iter + 1);
                return;
            }
        }
        eprintln!("Stopped at iteration limit ({}).", config.max_iters);
    }

    /// Total number of e-nodes across all e-classes.
    pub fn total_nodes(&self) -> usize {
        self.classes.values().map(|c| c.nodes.len()).sum()
    }

    /// Look up an id whose frozen snapshot term equals `term`. Useful for
    /// pinpointing a specific intermediate term (e.g. to explain against it)
    /// after saturation has collapsed classes.
    pub fn find_by_term(&self, term: &str) -> Option<Id> {
        self.term_snapshot
            .iter()
            .find(|(_, t)| t.as_str() == term)
            .map(|(id, _)| *id)
    }

    // ---- Explanations ----------------------------------------------------

    /// Find a justification path proving that the terms at `a` and `b` are
    /// equal, if one exists. Returns the chain of ids visited together with the
    /// [`Justification`] for each step (the first step's reason is an empty
    /// rule).
    ///
    /// This is a breadth-first search over the proof forest (`explain_edges`),
    /// which records the exact ids joined by every `merge`. Because those edges
    /// are never compressed away (unlike the union-find), the original history
    /// is recoverable. BFS yields a shortest proof.
    pub fn explain(&self, a: Id, b: Id) -> Option<Vec<(Id, Justification)>> {
        if a == b {
            return Some(vec![(a, Justification::Rule(""))]);
        }
        // Standard BFS, remembering each node's predecessor and the reason.
        let mut prev: HashMap<Id, (Id, Justification)> = HashMap::new();
        let mut visited: std::collections::HashSet<Id> = std::collections::HashSet::new();
        let mut queue: VecDeque<Id> = VecDeque::new();
        visited.insert(a);
        queue.push_back(a);
        while let Some(cur) = queue.pop_front() {
            if let Some(edges) = self.explain_edges.get(&cur) {
                for (next, reason) in edges {
                    let next = *next;
                    if visited.insert(next) {
                        prev.insert(next, (cur, reason.clone()));
                        if next == b {
                            // Reconstruct the path from b back to a.
                            let mut chain = vec![(b, reason.clone())];
                            let mut node = cur;
                            while node != a {
                                let (p, r) = prev[&node].clone();
                                chain.push((node, r));
                                node = p;
                            }
                            chain.push((a, Justification::Rule("")));
                            chain.reverse();
                            return Some(chain);
                        }
                        queue.push_back(next);
                    }
                }
            }
        }
        None
    }

    /// Produce a human-readable, rule-by-rule proof that the terms at `a` and
    /// `b` are equal. Each line shows the frozen term and the rule that rewrote
    /// the previous line into it. Congruence steps (two terms equal because
    /// their children became equal) are expanded recursively, with the sub-proof
    /// for each differing child pair indented underneath. Returns `None` if the
    /// two are not known to be equal.
    pub fn explain_equivalence(&self, a: Id, b: Id) -> Option<String> {
        let mut out = String::new();
        self.write_explanation(a, b, 0, &mut out)?;
        Some(out)
    }

    /// Recursive helper for [`explain_equivalence`]. `depth` bounds recursion so
    /// pathological cyclic proofs cannot loop forever.
    fn write_explanation(&self, a: Id, b: Id, depth: usize, out: &mut String) -> Option<()> {
        let path = self.explain(a, b)?;
        let pad = " ".repeat(depth);
        let term_of = |id: &Id| {
            self.term_snapshot
                .get(id)
                .cloned()
                .unwrap_or_else(|| "?".to_string())
        };
        for (i, (id, reason)) in path.iter().enumerate() {
            if i == 0 {
                out.push_str(&format!("{pad}{}\n", term_of(id)));
                continue;
            }
            match reason {
                Justification::Rule(name) => {
                    out.push_str(&format!("{pad} = {} [by {name}]\n", term_of(id)));
                }
                Justification::Congruence(pairs) => {
                    out.push_str(&format!("{pad} = {} [by congruence]\n", term_of(id)));
                    // Justify the congruence by proving each differing child
                    // pair, unless we have recursed too deep.
                    if depth < 4 {
                        for (co, cc) in pairs {
                            out.push_str(&format!(
                                "{pad} where {} = {}:\n",
                                term_of(co),
                                term_of(cc)
                            ));
                            self.write_explanation(*co, *cc, depth + 3, out);
                        }
                    }
                }
            }
        }
        Some(())
    }

    /// Independently verify that the terms at `a` and `b` really are equal, by
    /// *replaying* a proof rather than trusting the union-find. Returns `Ok(())`
    /// if a sound proof exists, or `Err(reason)` describing the first failure.
    ///
    /// This is the checker counterpart to [`explain`]: it re-derives equality
    /// from the primitive inference rules, so a bug in the explanation search
    /// (or in the forest) would surface as a rejected proof.
    ///
    /// Each step must be one of:
    /// - **reflexivity** — identical ids;
    /// - **transitivity** — consecutive steps whose endpoints line up;
    /// - **rule** — a recorded justification edge exists between the two ids;
    /// - **congruence** — the two nodes share operator/arity and every differing
    /// child pair is itself provably equal (checked recursively).
    pub fn check_proof(&self, a: Id, b: Id) -> Result<(), String> {
        self.check_proof_inner(a, b, 0)
    }

    fn check_proof_inner(&self, a: Id, b: Id, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("proof exceeded maximum recursion depth".into());
        }
        if a == b {
            return Ok(()); // reflexivity
        }
        let path = self
            .explain(a, b)
            .ok_or_else(|| format!("no proof connecting {a:?} and {b:?}"))?;

        // Transitivity: verify the chain starts at `a`, ends at `b`, and each
        // consecutive pair is a valid single step.
        if path.first().map(|(id, _)| *id) != Some(a) {
            return Err(format!("proof does not start at {a:?}"));
        }
        if path.last().map(|(id, _)| *id) != Some(b) {
            return Err(format!("proof does not end at {b:?}"));
        }

        for window in path.windows(2) {
            let (from, _) = window[0];
            let (to, reason) = &window[1];
            let to = *to;
            match reason {
                Justification::Rule(name) => {
                    // A rule step must correspond to a real recorded edge.
                    let has_edge = self
                        .explain_edges
                        .get(&from)
                        .map(|es| {
                            es.iter().any(|(n, r)| {
                                *n == to && matches!(r, Justification::Rule(m) if m == name)
                            })
                        })
                        .unwrap_or(false);
                    if !has_edge {
                        return Err(format!(
                            "step {from:?} -> {to:?} claims rule '{name}' but no such edge exists"
                        ));
                    }
                }
                Justification::Congruence(pairs) => {
                    // Endpoints must be applications of the same operator with
                    // equal arity, matching the recorded differing child pairs.
                    let fnode = self.canonical_node(from);
                    let tnode = self.canonical_node(to);
                    match (fnode, tnode) {
                        (Some(fnode), Some(tnode)) => {
                            if !fnode.matches(&tnode) {
                                return Err(format!(
                                    "congruence step {from:?} -> {to:?} joins different operators"
                                ));
                            }
                            if fnode.children().len() != tnode.children().len() {
                                return Err(format!(
                                    "congruence step {from:?} -> {to:?} has mismatched arity"
                                ));
                            }
                        }
                        _ => {
                            return Err(format!("congruence step {from:?} -> {to:?} lacks concrete nodes to compare"));
                        }
                    }
                    // Each differing child pair must itself be provable.
                    for (co, cc) in pairs {
                        self.check_proof_inner(*co, *cc, depth + 1)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A representative canonical node for `id`, if the class has any nodes.
    /// Used by the proof checker to compare operators/arities.
    fn canonical_node(&self, id: Id) -> Option<L> {
        let id = self.find_immut(id);
        self.classes.get(&id).and_then(|c| c.nodes.first().cloned())
    }
}

/// A borrowed, read-only view of a single e-class, passed to the comparison
/// callback of [`EGraph::compare_classes`]. Exposes the class's e-nodes and its
/// analysis data without granting mutable access to the graph.
pub struct ClassView<'a, L: Language, N: Analysis<L>> {
    /// All e-nodes proven equal in this class.
    pub nodes: &'a [L],
    /// The class's analysis data.
    pub data: &'a N::Data,
}
