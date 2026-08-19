//! Render an [`EGraph`] as a Mermaid `flowchart`, using one `subgraph` per
//! e-class. E-nodes become nodes inside their class subgraph; a node's child
//! edges point at the child e-class subgraphs (labelled by operand index when
//! there is more than one child).
//!
//! The output is a `mermaid` code block body (no fences) suitable for pasting
//! into Markdown or a Mermaid live editor.

use std::collections::HashMap;
use std::fmt::Write;

use super::egraph::{Analysis, EGraph};
use super::lang::{Id, Language};

/// Emit Mermaid `flowchart` source for `egraph`.
///
/// Every e-class is a `subgraph`; every e-node is a node whose label is the
/// operator applied to its child e-class references. Edges go from each e-node
/// to the subgraph of each child e-class.
pub fn to_mermaid<L: Language, N: Analysis<L>>(egraph: &EGraph<L, N>) -> String {
    let snapshot = egraph.classes_snapshot();

    // Stable, compact index per canonical class id, for readable names.
    let mut class_index: HashMap<Id, usize> = HashMap::new();
    for (idx, (id, _)) in snapshot.iter().enumerate() {
        class_index.insert(*id, idx);
    }

    // Label used to refer to a child e-class inside a node's operator display.
    let class_ref = |child: Id| -> String {
        let c = egraph.canonical_id(child);
        format!("c{}", class_index.get(&c).copied().unwrap_or(usize::MAX))
    };

    let mut out = String::new();
    let _ = writeln!(out, "flowchart TD");

    // Emit each e-class as a subgraph containing its e-nodes.
    for (id, nodes) in &snapshot {
        let ci = class_index[id];
        let _ = writeln!(out, " subgraph c{ci}[\"e-class {ci}\"]");
        for (ni, node) in nodes.iter().enumerate() {
            let child_labels: Vec<String> = node.children().iter().map(|&c| class_ref(c)).collect();
            let label = escape(&node.display(&child_labels));
            let _ = writeln!(out, " n{ci}_{ni}[\"{label}\"]");
        }
        let _ = writeln!(out, " end");
    }

    // Emit child edges from each e-node to the child e-class subgraphs.
    for (id, nodes) in &snapshot {
        let ci = class_index[id];
        for (ni, node) in nodes.iter().enumerate() {
            let children = node.children();
            for (arg, &child) in children.iter().enumerate() {
                let target = egraph.canonical_id(child);
                let ti = class_index.get(&target).copied().unwrap_or(usize::MAX);
                if children.len() > 1 {
                    let _ = writeln!(out, " n{ci}_{ni} -->|{arg}| c{ti}");
                } else {
                    let _ = writeln!(out, " n{ci}_{ni} --> c{ti}");
                }
            }
        }
    }

    out
}

/// Escape characters that break Mermaid node labels inside `["..."]`.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ===================== Equivalent-forms view =============================

/// Emit a Mermaid `flowchart` that, for the e-class of `root`, lists every
/// distinct term the class represents as sibling child nodes of a single
/// "class" node — so you can read off "this form == all these other forms".
///
/// Enumeration is bounded: `max_depth` caps recursion into child classes (to
/// stay finite under cycles) and `max_terms` caps how many forms are shown.
pub fn to_mermaid_equivalences<L: Language, N: Analysis<L>>(
    egraph: &EGraph<L, N>,
    root: Id,
    max_depth: usize,
    max_terms: usize,
) -> String {
    // Build a lookup: canonical class id -> its e-nodes.
    let nodes_by_class: HashMap<Id, Vec<L>> = egraph.classes_snapshot().into_iter().collect();
    let root = egraph.canonical_id(root);

    let mut forms = enumerate_terms(egraph, &nodes_by_class, root, max_depth, max_terms);
    forms.sort();
    forms.dedup();

    let ci = root.0;
    let mut out = String::new();
    let _ = writeln!(out, "flowchart TD");
    let _ = writeln!(out, " class{ci}([\"e-class {ci} (equivalent forms)\"])");
    for (i, form) in forms.iter().enumerate() {
        let _ = writeln!(out, " f{ci}_{i}[\"{}\"]", escape(form));
        let _ = writeln!(out, " class{ci} --- f{ci}_{i}");
    }
    out
}

/// Recursively enumerate the distinct rendered terms represented by `class`,
/// up to `max_depth` levels deep and `max_terms` results.
fn enumerate_terms<L: Language, N: Analysis<L>>(
    egraph: &EGraph<L, N>,
    nodes_by_class: &HashMap<Id, Vec<L>>,
    class: Id,
    max_depth: usize,
    max_terms: usize,
) -> Vec<String> {
    let class = egraph.canonical_id(class);
    let mut results: Vec<String> = Vec::new();
    let Some(nodes) = nodes_by_class.get(&class) else {
        return results;
    };

    for node in nodes {
        let children = node.children();
        if children.is_empty() {
            push_unique(&mut results, node.display(&[]), max_terms);
            if results.len() >= max_terms {
                break;
            }
            continue;
        }
        if max_depth == 0 {
            // Can't expand children further: use the class id as a placeholder.
            let placeholder: Vec<String> = children
                .iter()
                .map(|&c| format!("c{}", egraph.canonical_id(c).0))
                .collect();
            push_unique(&mut results, node.display(&placeholder), max_terms);
            if results.len() >= max_terms {
                break;
            }
            continue;
        }

        // Enumerate each child's forms, then take the cartesian product.
        let child_forms: Vec<Vec<String>> = children
            .iter()
            .map(|&c| enumerate_terms(egraph, nodes_by_class, c, max_depth - 1, max_terms))
            .collect();
        if child_forms.iter().any(|f| f.is_empty()) {
            continue;
        }
        for combo in cartesian(&child_forms) {
            push_unique(&mut results, node.display(&combo), max_terms);
            if results.len() >= max_terms {
                break;
            }
        }
        if results.len() >= max_terms {
            break;
        }
    }
    results
}

/// Push `value` into `out` if absent and capacity remains.
fn push_unique(out: &mut Vec<String>, value: String, max_terms: usize) {
    if out.len() < max_terms && !out.contains(&value) {
        out.push(value);
    }
}

/// Cartesian product of a list of string-choice lists.
fn cartesian(lists: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut acc: Vec<Vec<String>> = vec![Vec::new()];
    for list in lists {
        let mut next = Vec::new();
        for prefix in &acc {
            for item in list {
                let mut combo = prefix.clone();
                combo.push(item.clone());
                next.push(combo);
            }
        }
        acc = next;
    }
    acc
}
