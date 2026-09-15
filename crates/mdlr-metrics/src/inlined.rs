//! `inlined_size`: what a unit would measure if every helper only it calls
//! were folded back into it.
//!
//! `function_size` rewards splitting a unit up, whether or not the split made
//! the code simpler. Ten 30-line helpers called from one 20-line parent read
//! as ten healthy units, so moving lines out of a large unit lowers every row
//! it touches without removing a single line from the codebase. `inlined_size`
//! is immune to that move: lines pushed into an exclusive helper stay on the
//! parent's total.
//!
//! Absorbing a callee needs all of:
//! - a `Calls` edge from the parent,
//! - a span outside the parent's, so a nested unit — whose lines the parent's
//!   own size already counts — is not counted twice,
//! - exactly one *distinct* referencing unit, of any edge kind, so nothing
//!   else can reach it,
//! - the same file as the parent, so a helper that was moved out of the module
//!   is left alone,
//! - function or method kind.
//!
//! Distinctness matters: the graph builder walks `Unit::calls` without
//! deduplicating, so a parent that calls the same helper on two lines yields
//! two edges. Counting raw edges would read that as two callers and leave the
//! helper unabsorbed — the opposite of what the metric is for.
//!
//! Each absorbed unit therefore has exactly one parent, which makes the absorb
//! relation a functional graph: a forest plus cycles, since a set of units that
//! only call each other are each other's exclusive caller. `cluster_size`
//! carries the cycle guard that keeps the recursion terminating.

use mdlr_core::{EdgeKind, Graph, UnitKind};
use std::collections::{HashMap, HashSet};

use crate::complexity::DistributionMetrics;

/// Default threshold for exclusive helpers a unit must absorb before its
/// `inlined_size` is reported
pub const DEFAULT_INLINED_MIN_EXCLUSIVE_FANOUT: usize = 3;

#[derive(Debug, Clone)]
pub struct InlinedMetrics {
    /// Own lines plus every exclusive helper absorbed, transitively.
    pub inlined_size: DistributionMetrics,
    /// Helpers nothing else calls, hanging directly off each unit.
    ///
    /// The gate signal, not a reported metric. A wide tree is one unit spread
    /// out; a narrow one is a chain of steps that happen to call each other,
    /// which is ordinary code. Without this, a pipeline of single-caller
    /// stages reads as one enormous unit.
    pub exclusive_fanout: HashMap<String, usize>,
}

/// Direct helpers nothing else calls, keyed by parent id.
fn exclusive_helpers(graph: &Graph) -> HashMap<&str, Vec<&str>> {
    let mut referrers: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        referrers
            .entry(edge.to.as_str())
            .or_default()
            .insert(edge.from.as_str());
    }

    let unit_by_id: HashMap<&str, _> =
        graph.units.iter().map(|u| (u.id.as_str(), u)).collect();

    let mut helpers: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind != EdgeKind::Calls {
            continue;
        }
        let (Some(parent), Some(callee)) = (
            unit_by_id.get(edge.from.as_str()),
            unit_by_id.get(edge.to.as_str()),
        ) else {
            continue;
        };
        let exclusive =
            referrers.get(edge.to.as_str()).is_some_and(|r| r.len() == 1);
        let local = callee.file == parent.file;
        let callable =
            matches!(callee.kind, UnitKind::Function | UnitKind::Method);
        // A nested unit sits inside the parent's span, so the parent's own
        // size already counts its lines. Absorbing it would count them twice.
        let nested = local
            && callee.span.start_line > parent.span.start_line
            && callee.span.end_line <= parent.span.end_line;
        if exclusive && local && callable && !nested {
            helpers
                .entry(edge.from.as_str())
                .or_default()
                .insert(edge.to.as_str());
        }
    }

    helpers
        .into_iter()
        .map(|(parent, set)| {
            let mut ids: Vec<&str> = set.into_iter().collect();
            ids.sort_unstable();
            (parent, ids)
        })
        .collect()
}

/// Cluster size for `id`, memoised into `memo`. `seen` holds the ids on the
/// current walk, so a unit reached twice returns 0 rather than recursing
/// forever. That covers a unit that calls itself and a cycle of units that
/// call each other.
///
/// Returns the size and whether such a cut happened. A cut total counts only
/// the part of the cycle reachable from this root, so caching it would hand a
/// root-dependent value to a later root and make the output depend on the
/// order `compute` visits units in.
fn cluster_size<'a>(
    id: &'a str,
    own: &HashMap<&'a str, usize>,
    helpers: &HashMap<&'a str, Vec<&'a str>>,
    memo: &mut HashMap<&'a str, usize>,
    seen: &mut HashSet<&'a str>,
) -> (usize, bool) {
    if let Some(cached) = memo.get(id) {
        return (*cached, false);
    }
    if !seen.insert(id) {
        return (0, true);
    }
    let mut total = own.get(id).copied().unwrap_or(0);
    let mut truncated = false;
    for helper in helpers.get(id).into_iter().flatten() {
        let (size, cut) = cluster_size(helper, own, helpers, memo, seen);
        total += size;
        truncated |= cut;
    }
    seen.remove(id);
    if !truncated {
        memo.insert(id, total);
    }
    (total, truncated)
}

#[tracing::instrument(name = "compute_inlined", skip_all)]
pub fn compute(graph: &Graph) -> InlinedMetrics {
    let own: HashMap<&str, usize> = graph
        .units
        .iter()
        .filter(|u| matches!(u.kind, UnitKind::Function | UnitKind::Method))
        .map(|u| {
            let size = u.span.end_line.saturating_sub(u.span.start_line) + 1;
            (u.id.as_str(), size)
        })
        .collect();

    let helpers = exclusive_helpers(graph);
    let mut memo = HashMap::new();
    let mut sizes = HashMap::new();
    for id in own.keys() {
        let mut seen = HashSet::new();
        let (size, _) = cluster_size(id, &own, &helpers, &mut memo, &mut seen);
        sizes.insert((*id).to_string(), size);
    }

    let exclusive_fanout = own
        .keys()
        .map(|id| {
            let width = helpers.get(id).map_or(0, |h| h.len());
            ((*id).to_string(), width)
        })
        .collect();

    InlinedMetrics {
        inlined_size: DistributionMetrics::from_counts(sizes),
        exclusive_fanout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdlr_core::{Edge, Span, Unit};

    fn unit(id: &str, file: &str, lines: usize) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
            file: file.into(),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: lines,
                end_col: 0,
            },
            reads: vec![],
            writes: vec![],
            calls: vec![],
            tags: vec![],
            params: 0,
            branches: 0,
            max_scope_lines: 0,
            parent: None,
            cognitive_complexity: 0,
            partial: false,
        }
    }

    fn spanned(id: &str, file: &str, start: usize, end: usize) -> Unit {
        let mut u = unit(id, file, end);
        u.span.start_line = start;
        u
    }

    fn calls(from: &str, to: &str) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind: EdgeKind::Calls,
        }
    }

    fn size_of(m: &InlinedMetrics, id: &str) -> usize {
        m.inlined_size
            .distribution
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, v)| *v)
            .expect("unit in distribution")
    }

    /// A unit spread across ten exclusive helpers keeps its original weight,
    /// and the gate signal sees the full width.
    #[test]
    fn absorbs_exclusive_helpers() {
        let mut graph = Graph::new();
        graph.add_unit(unit("parent", "a.rs", 20));
        for i in 0..10 {
            let id = format!("helper{i}");
            graph.add_unit(unit(&id, "a.rs", 28));
            graph.add_edge(calls("parent", &id));
        }

        let m = compute(&graph);
        assert_eq!(size_of(&m, "parent"), 20 + 10 * 28);
        assert_eq!(size_of(&m, "helper0"), 28);
        assert_eq!(m.exclusive_fanout["parent"], 10);
    }

    /// A chain of single-caller steps absorbs the same way, which is why the
    /// value alone cannot be reported: only the width tells the two apart.
    #[test]
    fn chain_absorbs_but_stays_narrow() {
        let mut graph = Graph::new();
        for i in 0..5 {
            graph.add_unit(unit(&format!("step{i}"), "a.rs", 26));
        }
        for i in 0..4 {
            graph.add_edge(calls(
                &format!("step{i}"),
                &format!("step{}", i + 1),
            ));
        }

        let m = compute(&graph);
        assert_eq!(size_of(&m, "step0"), 5 * 26);
        assert_eq!(m.exclusive_fanout["step0"], 1);
    }

    #[test]
    fn shared_helper_is_not_absorbed() {
        let mut graph = Graph::new();
        graph.add_unit(unit("a", "a.rs", 10));
        graph.add_unit(unit("b", "a.rs", 10));
        graph.add_unit(unit("shared", "a.rs", 40));
        graph.add_edge(calls("a", "shared"));
        graph.add_edge(calls("b", "shared"));

        let m = compute(&graph);
        assert_eq!(size_of(&m, "a"), 10);
        assert_eq!(m.exclusive_fanout["a"], 0);
    }

    #[test]
    fn helper_in_another_file_is_not_absorbed() {
        let mut graph = Graph::new();
        graph.add_unit(unit("parent", "a.rs", 10));
        graph.add_unit(unit("helper", "b.rs", 40));
        graph.add_edge(calls("parent", "helper"));

        let m = compute(&graph);
        assert_eq!(size_of(&m, "parent"), 10);
        assert_eq!(m.exclusive_fanout["parent"], 0);
    }

    /// A second incoming edge of any kind disqualifies the helper, so a unit
    /// that is called once and read once stays on its own.
    #[test]
    fn non_call_edge_disqualifies_helper() {
        let mut graph = Graph::new();
        graph.add_unit(unit("parent", "a.rs", 10));
        graph.add_unit(unit("helper", "a.rs", 40));
        graph.add_edge(calls("parent", "helper"));
        graph.add_edge(Edge {
            from: "other".to_string(),
            to: "helper".to_string(),
            kind: EdgeKind::Reads,
        });

        let m = compute(&graph);
        assert_eq!(size_of(&m, "parent"), 10);
    }

    /// The graph builder emits one edge per call site, so calling the same
    /// helper twice must not read as two callers, nor as two helpers.
    #[test]
    fn repeated_call_sites_collapse() {
        let mut graph = Graph::new();
        graph.add_unit(unit("parent", "a.rs", 10));
        graph.add_unit(unit("helper", "a.rs", 40));
        graph.add_edge(calls("parent", "helper"));
        graph.add_edge(calls("parent", "helper"));

        let m = compute(&graph);
        assert_eq!(size_of(&m, "parent"), 50);
        assert_eq!(m.exclusive_fanout["parent"], 1);
    }

    #[test]
    fn self_call_terminates() {
        let mut graph = Graph::new();
        graph.add_unit(unit("recurse", "a.rs", 12));
        graph.add_edge(calls("recurse", "recurse"));

        let m = compute(&graph);
        assert_eq!(size_of(&m, "recurse"), 12);
    }

    /// Extractors that emit nested functions as their own units put them
    /// inside the parent's span, so the parent's own size already counts
    /// their lines. Absorbing them would report a unit larger than its file.
    #[test]
    fn nested_helper_is_not_absorbed() {
        let mut graph = Graph::new();
        graph.add_unit(spanned("outer", "a.ts", 1, 52));
        for i in 0..4 {
            let id = format!("outer::inner{i}");
            let start = 2 + i * 11;
            graph.add_unit(spanned(&id, "a.ts", start, start + 10));
            graph.add_edge(calls("outer", &id));
        }

        let m = compute(&graph);
        assert_eq!(size_of(&m, "outer"), 52);
        assert_eq!(m.exclusive_fanout["outer"], 0);
    }

    /// Two units that only call each other are each other's exclusive caller,
    /// so the absorb relation has a cycle. The truncated total that the cycle
    /// guard produces must not be memoised: cached, it would leak into
    /// whichever root the `HashMap` iteration order happens to visit later.
    #[test]
    fn mutual_recursion_is_order_independent() {
        let mut graph = Graph::new();
        graph.add_unit(spanned("alpha", "m.ts", 1, 4));
        graph.add_unit(spanned("beta", "m.ts", 6, 11));
        graph.add_edge(calls("alpha", "beta"));
        graph.add_edge(calls("beta", "alpha"));

        for _ in 0..16 {
            let m = compute(&graph);
            assert_eq!(size_of(&m, "alpha"), 4 + 6);
            assert_eq!(size_of(&m, "beta"), 6 + 4);
        }
    }
}
