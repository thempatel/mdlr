use crate::graph::{Edge, EdgeKind, Graph, Unit};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Trait for providing call resolution context.
/// This is implemented by language-specific extractors.
pub trait CallResolver {
    /// Resolve a call expression to a fully qualified path.
    fn resolve_call(&self, call: &str, caller_file: &Path) -> Option<String>;

    /// Get the crate name and module path for a file.
    fn file_to_module(&self, file: &Path) -> Option<(String, Vec<String>)>;
}

/// Build a dependency graph from a collection of units.
///
/// This function resolves call references between units and creates edges
/// representing the call relationships. It uses both heuristic matching
/// and semantic resolution (when a `CallResolver` is provided) to
/// map call expressions to their target units.
pub fn build(units: Vec<Unit>, resolver: Option<&dyn CallResolver>) -> Graph {
    let mut graph = Graph::new();

    let unit_ids: HashSet<_> = units.iter().map(|u| u.id.clone()).collect();
    let name_to_ids = build_name_index(&units);

    // Resolve calls and create edges
    for unit in &units {
        resolve_unit_calls(
            &mut graph,
            unit,
            &unit_ids,
            &name_to_ids,
            resolver,
        );
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
    resolver: Option<&dyn CallResolver>,
) {
    let caller_file = unit.file.to_string_lossy();

    for call in &unit.calls {
        // First check if the call is already a fully resolved crate path that matches a unit ID
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

        // Try resolution context first (if available), then fall back to heuristic resolution
        let resolved = resolver
            .and_then(|ctx| {
                resolve_call_with_context(call, &unit.file, ctx, unit_ids)
            })
            .or_else(|| {
                resolve_call(call, &caller_file, unit_ids, name_to_ids)
            });

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
        resolve_from_name_index(call, caller_file, name_to_ids)
    {
        return Some(resolved);
    }

    // 4. Handle method calls like "self.field" or "obj.method"
    if let Some(resolved) = resolve_method_call(call, caller_file, name_to_ids)
    {
        return Some(resolved);
    }

    // 5. Handle path-style calls like "module::function" or "Type::method"
    resolve_path_call(call, name_to_ids)
}

/// Resolve from name index, preferring same-file candidates.
fn resolve_from_name_index(
    call: &str,
    caller_file: &str,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let candidates = name_to_ids.get(call)?;
    pick_best_candidate(candidates, caller_file)
}

/// Resolve method calls like "obj.method" by extracting the method name.
fn resolve_method_call(
    call: &str,
    caller_file: &str,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if !call.contains('.') {
        return None;
    }

    let method = call.rsplit('.').next()?;
    let candidates = name_to_ids.get(method)?;
    pick_best_candidate(candidates, caller_file)
}

/// Resolve path-style calls by stripping the first component.
fn resolve_path_call(
    call: &str,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let idx = call.find("::")?;
    let without_prefix = &call[idx + 2..];
    let candidates = name_to_ids.get(without_prefix)?;

    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }
    None
}

/// Pick the best candidate from a list, preferring same-file matches.
fn pick_best_candidate(
    candidates: &[String],
    caller_file: &str,
) -> Option<String> {
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }

    // Multiple matches - prefer same file
    for candidate in candidates {
        if candidate.starts_with(caller_file) {
            return Some(candidate.clone());
        }
    }

    // If still ambiguous, take the first one (arbitrary but deterministic)
    Some(candidates[0].clone())
}

/// Resolve a call using the semantic resolution context.
///
/// This uses Cargo workspace information, module graphs, and use statements
/// to provide more accurate resolution than heuristic matching.
fn resolve_call_with_context(
    call: &str,
    caller_file: &Path,
    ctx: &dyn CallResolver,
    unit_ids: &HashSet<String>,
) -> Option<String> {
    // First, try to resolve using the semantic context
    let resolved_path = ctx.resolve_call(call, caller_file)?;

    // Now try to map the resolved path back to a unit ID
    // The resolved path looks like "crate_name::module::item"
    // but our unit IDs look like "src/file.rs::item"

    // Strategy 1: Check if resolved path matches any unit ID directly
    if unit_ids.contains(&resolved_path) {
        return Some(resolved_path);
    }

    // Strategy 2: Try to find the unit by matching the item name
    // Extract the item name from the resolved path
    let item_name = resolved_path.rsplit("::").next()?;

    // Look for units that end with this item name
    for unit_id in unit_ids {
        // Check if the unit ID ends with "::item_name"
        if unit_id.ends_with(&format!("::{}", item_name)) {
            // If there's a file path in the resolved path, try to match it
            // For now, just return the first match (could be improved with better heuristics)
            return Some(unit_id.clone());
        }
    }

    // Strategy 3: For cross-crate resolution, find the matching crate's files
    // The resolution context knows which crate each file belongs to
    if let Some((resolved_crate, _)) = ctx.file_to_module(caller_file) {
        // If the resolved path starts with a different crate, find that crate's units
        let path_parts: Vec<&str> = resolved_path.split("::").collect();
        if !path_parts.is_empty() {
            let target_crate = path_parts[0];
            if target_crate != resolved_crate
                && target_crate != "crate"
                && target_crate != "std"
            {
                // Look for units in that crate
                for unit_id in unit_ids {
                    // Extract crate info from unit ID's file path
                    // This is a heuristic - check if the file path contains the crate name
                    if unit_id.contains(&format!(
                        "{}/",
                        target_crate.replace('_', "-")
                    )) || unit_id.contains(&format!("{}/", target_crate))
                    {
                        if unit_id.ends_with(&format!("::{}", item_name)) {
                            return Some(unit_id.clone());
                        }
                    }
                }
            }
        }
    }

    None
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
            parent: None,
        }
    }

    #[test]
    fn test_build_creates_edges_for_calls() {
        let units = vec![
            make_unit("test::foo", vec!["bar"]),
            make_unit("test::bar", vec![]),
        ];
        let graph = build(units, None);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, "test::foo");
        assert_eq!(graph.edges[0].to, "test::bar");
    }

    #[test]
    fn test_build_no_self_loops() {
        let units = vec![make_unit("test::foo", vec!["foo"])];
        let graph = build(units, None);
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn test_build_resolves_full_id() {
        let units = vec![
            make_unit("crate::module::caller", vec!["crate::module::callee"]),
            make_unit("crate::module::callee", vec![]),
        ];
        let graph = build(units, None);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to, "crate::module::callee");
    }

    #[test]
    fn test_build_resolves_short_name() {
        let units = vec![
            make_unit("crate::module::caller", vec!["helper"]),
            make_unit("crate::utils::helper", vec![]),
        ];
        let graph = build(units, None);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to, "crate::utils::helper");
    }
}
