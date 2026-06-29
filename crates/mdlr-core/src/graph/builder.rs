use crate::graph::{Edge, EdgeKind, Graph, Unit};
use std::collections::{HashMap, HashSet};

/// Build a dependency graph from a collection of units.
///
/// This function resolves call references between units and creates edges
/// representing the call relationships. It uses both exact matching and
/// heuristic name resolution to map call expressions to their target units.
pub fn build(units: Vec<Unit>) -> Graph {
    build_with_progress(units, |_| {})
}

pub fn build_with_progress(
    units: Vec<Unit>,
    on_progress: impl Fn(usize),
) -> Graph {
    let mut graph = Graph::new();

    let unit_ids: HashSet<_> = units.iter().map(|u| u.id.clone()).collect();
    let name_to_ids = build_name_index(&units);

    for (i, unit) in units.iter().enumerate() {
        resolve_unit_calls(&mut graph, unit, &unit_ids, &name_to_ids);
        on_progress(i);
    }

    for unit in units {
        graph.add_unit(unit);
    }

    graph
}

/// Build an index from various name forms to full unit IDs.
///
/// This enables resolving calls by:
/// 1. Exact match: full ID -> full ID
/// 2. Short name: "function_name" -> full ID (may have conflicts)
/// 3. With type: "Foo::method" -> full ID
/// 4. Module path: "module::function" -> full ID
fn build_name_index(units: &[Unit]) -> HashMap<String, Vec<String>> {
    let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();

    for unit in units {
        // Add the full ID (for exact matches)
        name_to_ids.entry(unit.id.clone()).or_default().push(unit.id.clone());

        // Extract the local part (after the first "::")
        let Some(idx) = unit.id.find("::") else {
            continue;
        };
        let local = &unit.id[idx + 2..];

        // Add local name
        name_to_ids
            .entry(local.to_string())
            .or_default()
            .push(unit.id.clone());

        // Index methods by Type::method form
        index_methods(&mut name_to_ids, local, &unit.id);

        // Index short name (last segment)
        index_short_name(&mut name_to_ids, local, &unit.id);
    }

    name_to_ids
}

/// Index methods by method name and Type::method form.
fn index_methods(
    name_to_ids: &mut HashMap<String, Vec<String>>,
    local: &str,
    full_id: &str,
) {
    // Handle Type::method style IDs
    if let Some(last_idx) = local.rfind("::") {
        let method_name = &local[last_idx + 2..];
        name_to_ids
            .entry(method_name.to_string())
            .or_default()
            .push(full_id.to_string());
    }
}

/// Index by the short name (last segment) for crate-based IDs.
fn index_short_name(
    name_to_ids: &mut HashMap<String, Vec<String>>,
    local: &str,
    full_id: &str,
) {
    let Some(last_idx) = local.rfind("::") else {
        return;
    };
    let short_name = &local[last_idx + 2..];
    if !short_name.is_empty() {
        name_to_ids
            .entry(short_name.to_string())
            .or_default()
            .push(full_id.to_string());
    }
}

/// Resolve all calls for a unit and add edges to the graph.
fn resolve_unit_calls(
    graph: &mut Graph,
    unit: &Unit,
    unit_ids: &HashSet<String>,
    name_to_ids: &HashMap<String, Vec<String>>,
) {
    let caller_file = unit.file.to_string_lossy();
    let caller_crate = crate_of(&unit.id);

    for call in &unit.calls {
        // First check if the call is already a fully resolved path that matches a unit ID
        if unit_ids.contains(call) {
            if call != &unit.id {
                graph.add_edge(Edge {
                    from: unit.id.clone(),
                    to: call.clone(),
                    kind: EdgeKind::Calls,
                });
            }
            continue;
        }

        // Fall back to heuristic resolution
        let resolved = resolve_call(
            call,
            &caller_file,
            caller_crate,
            unit_ids,
            name_to_ids,
        );

        if let Some(target_id) = resolved {
            // Don't create self-loops
            if target_id != unit.id {
                graph.add_edge(Edge {
                    from: unit.id.clone(),
                    to: target_id,
                    kind: EdgeKind::Calls,
                });
            }
        }
    }
}

/// Resolve a call expression to a fully qualified unit ID using heuristics.
fn resolve_call(
    call: &str,
    caller_file: &str,
    caller_crate: &str,
    unit_ids: &HashSet<String>,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    // 1. Try exact match first (for fully qualified calls)
    if unit_ids.contains(call) {
        return Some(call.to_string());
    }

    // 2. Try prefixing with caller's file path (same-file calls)
    let same_file_id = format!("{}::{}", caller_file, call);
    if unit_ids.contains(&same_file_id) {
        return Some(same_file_id);
    }

    // 3. Look up in name index
    if let Some(resolved) =
        resolve_from_name_index(call, caller_file, caller_crate, name_to_ids)
    {
        return Some(resolved);
    }

    // 4. Handle method calls like "self.field" or "obj.method"
    if let Some(resolved) =
        resolve_method_call(call, caller_file, caller_crate, name_to_ids)
    {
        return Some(resolved);
    }

    // 5. Handle path-style calls like "module::function" or "Type::method"
    resolve_path_call(call, caller_crate, name_to_ids)
}

/// Resolve from name index, preferring same-file then same-crate candidates.
fn resolve_from_name_index(
    call: &str,
    caller_file: &str,
    caller_crate: &str,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let candidates = name_to_ids.get(call)?;
    pick_best_candidate(candidates, caller_file, caller_crate)
}

/// Resolve method calls like "obj.method" by extracting the method name.
fn resolve_method_call(
    call: &str,
    caller_file: &str,
    caller_crate: &str,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if !call.contains('.') {
        return None;
    }

    let method = call.rsplit('.').next()?;
    let candidates = name_to_ids.get(method)?;
    pick_best_candidate(candidates, caller_file, caller_crate)
}

/// Resolve path-style calls by stripping the first component, restricted to the
/// caller's own crate.
///
/// Stripping the call's crate prefix can otherwise match a same-named unit in a
/// sibling crate: `mdlr_extract_ts::visitor::make_span` (a method whose real id
/// is `…::UnitExtractor::make_span`, so it misses exact match) strips to
/// `visitor::make_span`, which uniquely matches *mdlr_extract_rust*'s free
/// function — inflating that unit's `fan_in` with foreign callers. Requiring the
/// candidate to share the caller's crate prevents the cross-crate leak.
fn resolve_path_call(
    call: &str,
    caller_crate: &str,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let idx = call.find("::")?;
    let without_prefix = &call[idx + 2..];
    let candidates = name_to_ids.get(without_prefix)?;

    let mut same_crate =
        candidates.iter().filter(|c| crate_of(c) == caller_crate);
    let first = same_crate.next()?;
    // Ambiguous even within the crate — don't guess.
    if same_crate.next().is_some() {
        return None;
    }
    Some(first.clone())
}

/// The crate segment of a unit id — everything before the first `::`.
fn crate_of(id: &str) -> &str {
    id.split("::").next().unwrap_or(id)
}

/// Pick the best candidate from a list, preferring same-file, then same-crate
/// matches. Without the same-crate step, a short name shared across the sibling
/// extractor crates (e.g. `make_span`, `count_params`) resolves to an arbitrary
/// crate's unit, inflating that unit's `fan_in` with foreign callers.
fn pick_best_candidate(
    candidates: &[String],
    caller_file: &str,
    caller_crate: &str,
) -> Option<String> {
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }

    // Prefer a candidate in the caller's own file (only bites when ids are
    // file-path based; harmless otherwise).
    for candidate in candidates {
        if candidate.starts_with(caller_file) {
            return Some(candidate.clone());
        }
    }

    // Then prefer a candidate in the caller's own crate.
    for candidate in candidates {
        if crate_of(candidate) == caller_crate {
            return Some(candidate.clone());
        }
    }

    // If still ambiguous, take the first one (arbitrary but deterministic)
    Some(candidates[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Span, UnitKind};
    use std::path::PathBuf;

    fn make_unit(id: &str, calls: Vec<&str>) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
            file: PathBuf::from("test.rs"),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
            },
            reads: vec![],
            writes: vec![],
            calls: calls.into_iter().map(String::from).collect(),
            tags: vec![],
            params: 0,
            branches: 0,
            max_scope_lines: 0,
            parent: None,
            cognitive_complexity: 0,
            partial: false,
        }
    }

    #[test]
    fn test_build_creates_edges_for_calls() {
        let units = vec![
            make_unit("test::foo", vec!["bar"]),
            make_unit("test::bar", vec![]),
        ];
        let graph = build(units);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, "test::foo");
        assert_eq!(graph.edges[0].to, "test::bar");
    }

    #[test]
    fn test_build_no_self_loops() {
        let units = vec![make_unit("test::foo", vec!["foo"])];
        let graph = build(units);
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn test_path_call_does_not_leak_across_crates() {
        // `crate_b` calls its own `visitor::make_span` (a method, so the exact
        // id misses); stripping the crate prefix yields `visitor::make_span`,
        // which only matches crate_a's free function. That cross-crate match
        // must NOT create an edge (it would inflate crate_a's fan_in).
        let units = vec![
            make_unit("crate_b::caller", vec!["crate_b::visitor::make_span"]),
            make_unit("crate_a::visitor::make_span", vec![]),
        ];
        let graph = build(units);
        assert!(
            !graph.edges.iter().any(|e| e.to == "crate_a::visitor::make_span"),
            "cross-crate path resolution should not create an edge"
        );
    }

    #[test]
    fn test_short_name_resolves_to_callers_own_crate() {
        // `make_span` exists in two sibling crates; a bare-name call must
        // resolve to the caller's own crate, not an arbitrary one (otherwise
        // the other crate's make_span gets foreign fan_in).
        let units = vec![
            make_unit("crate_a::calls::extract", vec!["make_span"]),
            make_unit("crate_a::visitor::make_span", vec![]),
            make_unit("crate_b::visitor::make_span", vec![]),
        ];
        let graph = build(units);
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from == "crate_a::calls::extract")
            .expect("edge from caller");
        assert_eq!(edge.to, "crate_a::visitor::make_span");
    }

    #[test]
    fn test_build_resolves_full_id() {
        let units = vec![
            make_unit("crate::module::caller", vec!["crate::module::callee"]),
            make_unit("crate::module::callee", vec![]),
        ];
        let graph = build(units);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to, "crate::module::callee");
    }

    #[test]
    fn test_build_resolves_short_name() {
        let units = vec![
            make_unit("crate::module::caller", vec!["helper"]),
            make_unit("crate::utils::helper", vec![]),
        ];
        let graph = build(units);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to, "crate::utils::helper");
    }
}
