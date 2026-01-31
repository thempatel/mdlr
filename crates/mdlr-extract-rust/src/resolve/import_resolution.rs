//! Import resolution for use statements.
//!
//! This module handles resolving names through use statements and imports.

use super::modules::ModuleGraph;
use super::resolve::{ResolutionContext, ResolvedPath};
use super::uses::{UseKind, UseStatement, resolve_path_prefix};

/// Import resolution methods for ResolutionContext.
impl ResolutionContext {
    /// Check imports (use statements) in the current module.
    pub(super) fn check_imports(
        &self,
        segments: &[&str],
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        let module = from_graph.get_module(&from_module.to_vec())?;
        let first = segments[0];

        // Find a use statement that imports this name
        for use_stmt in &module.uses {
            let imported_name = use_stmt.imported_name()?;

            if imported_name == first {
                // Found a matching import
                if segments.len() == 1 {
                    // Simple case: the import directly names the item
                    return self.resolve_use_statement(
                        use_stmt,
                        from_crate,
                        from_module,
                        from_graph,
                    );
                } else {
                    // The import is a type/module, and we're accessing something inside it
                    // e.g., Parser::new where Parser is imported from tree_sitter
                    let mut full_path = use_stmt.segments.clone();
                    full_path
                        .extend(segments[1..].iter().map(|s| s.to_string()));

                    // For external crates, construct the path directly without verification
                    if use_stmt.is_external() && !use_stmt.segments.is_empty()
                    {
                        let crate_name = &use_stmt.segments[0];
                        // Module path is everything except the crate name and the final item
                        let module_path: Vec<String> = full_path
                            [1..full_path.len() - 1]
                            .iter()
                            .cloned()
                            .collect();
                        let item_name = full_path.last()?.clone();

                        return Some(ResolvedPath {
                            crate_name: crate_name.clone(),
                            module_path,
                            item_name,
                        });
                    }

                    // Try to resolve the extended path for local crates
                    let path_str = full_path.join("::");
                    return self.resolve(&path_str, from_crate, from_module);
                }
            }
        }

        None
    }

    /// Resolve a use statement to its definition.
    fn resolve_use_statement(
        &self,
        use_stmt: &UseStatement,
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        match use_stmt.kind {
            UseKind::Single | UseKind::SelfImport => self
                .resolve_single_import(
                    use_stmt,
                    from_crate,
                    from_module,
                    from_graph,
                ),
            UseKind::Glob => {
                // Glob imports are harder - we'd need to know what we're looking for
                // For now, return None and let the caller handle unresolved
                None
            }
        }
    }

    /// Resolve a single import (non-glob use statement).
    fn resolve_single_import(
        &self,
        use_stmt: &UseStatement,
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        let segments = &use_stmt.segments;

        // Handle super/self relative imports
        if !segments.is_empty()
            && (segments[0] == "super" || segments[0] == "self")
        {
            if let Some(result) = self.resolve_relative_import(
                segments,
                from_crate,
                from_module,
                from_graph,
            ) {
                return Some(result);
            }
        }

        // Handle crate-relative imports
        if use_stmt.is_crate_relative() {
            if let Some(result) = self.resolve_crate_relative_import(
                segments, from_crate, from_graph,
            ) {
                return Some(result);
            }
        }

        // Handle external crate imports
        if use_stmt.is_external() && !segments.is_empty() {
            if let Some(result) = self.resolve_external_import(segments) {
                return Some(result);
            }
        }

        None
    }

    /// Resolve a super/self relative import.
    fn resolve_relative_import(
        &self,
        segments: &[String],
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        let resolved_path = resolve_path_prefix(segments, from_module)?;
        if resolved_path.len() < 2 {
            return None;
        }

        let module_path = &resolved_path[..resolved_path.len() - 1];
        let item_name = &resolved_path[resolved_path.len() - 1];

        let (resolved_module, _) = from_graph
            .find_item_or_reexport(&module_path.to_vec(), item_name)?;
        Some(ResolvedPath {
            crate_name: from_crate.to_string(),
            module_path: resolved_module,
            item_name: item_name.clone(),
        })
    }

    /// Resolve a crate-relative import (crate::...).
    fn resolve_crate_relative_import(
        &self,
        segments: &[String],
        from_crate: &str,
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        if segments.len() < 2 {
            return None;
        }

        let module_path = &segments[..segments.len() - 1];
        let item_name = &segments[segments.len() - 1];

        let (resolved_module, _) = from_graph
            .find_item_or_reexport(&module_path.to_vec(), item_name)?;
        Some(ResolvedPath {
            crate_name: from_crate.to_string(),
            module_path: resolved_module,
            item_name: item_name.clone(),
        })
    }

    /// Resolve an external crate import.
    pub(super) fn resolve_external_import(
        &self,
        segments: &[String],
    ) -> Option<ResolvedPath> {
        let crate_name = &segments[0];

        // Check if it's a local crate (workspace member or path dep)
        let actual_crate_name = self
            .crate_name_mapping
            .get(crate_name)
            .cloned()
            .unwrap_or_else(|| crate_name.clone());

        if let Some(crate_graph) = self.module_graphs.get(&actual_crate_name) {
            self.resolve_local_crate_import(
                segments,
                &actual_crate_name,
                crate_graph,
            )
        } else {
            resolve_unknown_external_import(segments, crate_name)
        }
    }

    /// Resolve an import from a local crate (workspace member or path dep).
    fn resolve_local_crate_import(
        &self,
        segments: &[String],
        actual_crate_name: &str,
        crate_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        if segments.len() < 2 {
            return None;
        }

        let mut module_path = vec!["crate".to_string()];
        module_path.extend(segments[1..segments.len() - 1].iter().cloned());

        let item_name = &segments[segments.len() - 1];

        crate_graph.find_item(&module_path, item_name)?;
        Some(ResolvedPath {
            crate_name: actual_crate_name.to_string(),
            module_path,
            item_name: item_name.clone(),
        })
    }
}

/// Resolve an import from an external crate not in the workspace.
pub(super) fn resolve_unknown_external_import(
    segments: &[String],
    crate_name: &str,
) -> Option<ResolvedPath> {
    if segments.len() >= 2 {
        let item_name = &segments[segments.len() - 1];
        let module_path: Vec<String> =
            segments[1..segments.len() - 1].iter().cloned().collect();

        Some(ResolvedPath {
            crate_name: crate_name.to_string(),
            module_path,
            item_name: item_name.clone(),
        })
    } else if segments.len() == 1 {
        // Just the crate name itself (rare but possible)
        Some(ResolvedPath {
            crate_name: crate_name.to_string(),
            module_path: vec![],
            item_name: String::new(),
        })
    } else {
        None
    }
}
