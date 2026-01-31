//! Call resolution for code extraction.
//!
//! This module handles resolving function calls and method calls during
//! extraction, particularly for files not in the module graph (e.g., binary
//! entry points like main.rs).

use std::path::Path;

use tree_sitter::Parser;

use super::modules::ModuleGraph;
use super::resolve::{ResolutionContext, ResolvedPath};
use super::uses::{UseStatement, extract_use_statement};

/// Check if a name looks like a crate name (starts with lowercase) vs a type name (starts with uppercase).
fn is_crate_like_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_lowercase())
}

/// Parse use statements from a Rust source file.
///
/// This is used for resolving names in files that aren't part of the module graph
/// (e.g., binary entry points like main.rs).
fn parse_file_uses(source: &str) -> Option<Vec<UseStatement>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into()).ok()?;

    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let mut uses = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "use_declaration" {
            uses.extend(extract_use_statement(child, source));
        }
    }

    Some(uses)
}

/// Call resolution methods for ResolutionContext.
///
/// These methods handle resolving function and method calls during extraction.
impl ResolutionContext {
    /// Resolve a call name to a full path, given file context.
    ///
    /// This is the main entry point for call resolution during extraction.
    pub fn resolve_call(
        &self,
        call_name: &str,
        file_path: &Path,
    ) -> Option<String> {
        // Find which crate this file belongs to
        let (crate_name, module_path) = self.file_to_module(file_path)?;

        // Check if this specific file is in the module graph
        // (not just if the module path exists - the same module path could map
        // to different files, e.g., lib.rs and main.rs both map to ["crate"])
        let canonical_file = file_path.canonicalize().ok();
        let in_module_graph = self
            .module_graphs
            .get(&crate_name)
            .and_then(|graph| graph.get_module(&module_path))
            .is_some_and(|module| {
                canonical_file
                    .as_ref()
                    .zip(module.file.canonicalize().ok())
                    .is_some_and(|(f1, f2)| f1 == &f2)
            });

        if in_module_graph {
            // Use normal resolution for files in the module graph
            let resolved =
                self.resolve(call_name, &crate_name, &module_path)?;
            return Some(resolved.full_path());
        }

        // File is not in module graph (e.g., main.rs binary)
        // Parse use statements from the source file and resolve using those
        self.resolve_call_for_standalone_file(
            call_name,
            file_path,
            &crate_name,
        )
    }

    /// Resolve a call for a file not in the module graph (e.g., binary entry points).
    ///
    /// This parses use statements directly from the source file and uses them
    /// for resolution, combined with the workspace's type information.
    fn resolve_call_for_standalone_file(
        &self,
        call_name: &str,
        file_path: &Path,
        crate_name: &str,
    ) -> Option<String> {
        let source = std::fs::read_to_string(file_path).ok()?;
        let uses = parse_file_uses(&source)?;
        let segments: Vec<&str> = call_name.split("::").collect();

        if segments.is_empty() {
            return None;
        }

        // Strategy 1: Direct external crate path (e.g., tree_sitter::Parser)
        if let Some(resolved) = self.try_resolve_direct_crate_path(&segments) {
            return Some(resolved);
        }

        // Strategy 2: Import-based resolution
        if let Some(resolved) =
            self.try_resolve_from_imports(&segments, &uses, crate_name)
        {
            return Some(resolved);
        }

        // Strategy 3: Prelude
        if segments.len() == 1 {
            if let Some(resolved) = self.check_prelude(segments[0]) {
                return Some(resolved.full_path());
            }
        }

        // Strategy 4: Cross-crate fallback
        self.check_cross_crate(&segments).map(|r| r.full_path())
    }

    /// Try to resolve a direct crate-qualified path like `tree_sitter::Parser`.
    fn try_resolve_direct_crate_path(
        &self,
        segments: &[&str],
    ) -> Option<String> {
        if segments.len() < 2 {
            return None;
        }

        let first_segment = segments[0];

        // Skip relative path keywords
        if ["crate", "self", "super"].contains(&first_segment) {
            return None;
        }

        let actual_crate = self
            .crate_name_mapping
            .get(first_segment)
            .map(|s| s.as_str())
            .unwrap_or(first_segment);

        if self.module_graphs.contains_key(actual_crate) {
            // Workspace crate - resolve through its module graph
            return self.check_cross_crate(segments).map(|r| r.full_path());
        }

        // Check if this looks like a crate name (starts with lowercase)
        // vs a type name (starts with uppercase)
        if !is_crate_like_name(first_segment) {
            return None;
        }

        // External crate (not in workspace) - construct canonical path
        let module_path: Vec<String> = segments[1..segments.len() - 1]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let item_name = segments[segments.len() - 1].to_string();

        Some(
            ResolvedPath {
                crate_name: first_segment.to_string(),
                module_path,
                item_name,
            }
            .full_path(),
        )
    }

    /// Try to resolve using import statements.
    fn try_resolve_from_imports(
        &self,
        segments: &[&str],
        uses: &[UseStatement],
        crate_name: &str,
    ) -> Option<String> {
        let first = segments[0];

        for use_stmt in uses {
            let imported_name = use_stmt.imported_name()?;
            if imported_name != first {
                continue;
            }

            // Found a matching import
            if segments.len() == 1 {
                return self.resolve_use_for_standalone(use_stmt, crate_name);
            }

            // The import is a type/module, and we're accessing something inside it
            if use_stmt.is_external() && !use_stmt.segments.is_empty() {
                if let Some(resolved) =
                    self.resolve_method_via_external_import(use_stmt, segments)
                {
                    return Some(resolved);
                }
            }

            // For crate-relative paths (use crate::module::Type)
            if use_stmt.is_crate_relative() && use_stmt.segments.len() >= 2 {
                if let Some(resolved) = self.resolve_method_via_crate_import(
                    use_stmt, segments, crate_name,
                ) {
                    return Some(resolved);
                }
            }
        }

        None
    }

    /// Resolve a method call through an external crate import (e.g., `use mdlr::walk::SourceWalker`).
    fn resolve_method_via_external_import(
        &self,
        use_stmt: &UseStatement,
        segments: &[&str],
    ) -> Option<String> {
        let ext_crate_name = &use_stmt.segments[0];
        let actual_crate = self
            .crate_name_mapping
            .get(ext_crate_name)
            .cloned()
            .unwrap_or_else(|| ext_crate_name.clone());

        if let Some(crate_graph) = self.module_graphs.get(&actual_crate) {
            return self.resolve_method_on_workspace_type(
                use_stmt,
                segments,
                crate_graph,
                &actual_crate,
            );
        }

        // External crate (not in workspace) - construct path without verification
        self.resolve_method_on_external_type(use_stmt, segments)
    }

    /// Resolve a method on a type from a workspace crate.
    fn resolve_method_on_workspace_type(
        &self,
        use_stmt: &UseStatement,
        segments: &[&str],
        crate_graph: &ModuleGraph,
        actual_crate: &str,
    ) -> Option<String> {
        // Build the module path: crate::module (where the type is defined)
        let mut mod_path = vec!["crate".to_string()];
        mod_path.extend(
            use_stmt.segments[1..use_stmt.segments.len() - 1].iter().cloned(),
        );

        let type_name = use_stmt.segments.last()?;
        let method_name = &segments[1];
        let impl_name = format!("impl {}", type_name);

        // First check if the impl is in the direct module path
        if crate_graph.find_item(&mod_path, &impl_name).is_some() {
            return Some(
                ResolvedPath {
                    crate_name: actual_crate.to_string(),
                    module_path: mod_path,
                    item_name: format!("impl {}::{}", type_name, method_name),
                }
                .full_path(),
            );
        }

        // If not found directly, check if the type is re-exported
        if let Some((resolved_module, _)) =
            crate_graph.find_item_or_reexport(&mod_path, type_name)
        {
            if crate_graph.find_item(&resolved_module, &impl_name).is_some() {
                return Some(
                    ResolvedPath {
                        crate_name: actual_crate.to_string(),
                        module_path: resolved_module,
                        item_name: format!(
                            "impl {}::{}",
                            type_name, method_name
                        ),
                    }
                    .full_path(),
                );
            }
        }

        // Fall back to treating it as a direct item (might be a nested module)
        let mut full_path = use_stmt.segments.clone();
        full_path.extend(segments[1..].iter().map(|s| s.to_string()));

        let mut full_mod_path = vec!["crate".to_string()];
        full_mod_path
            .extend(full_path[1..full_path.len() - 1].iter().cloned());

        let item = &full_path[full_path.len() - 1];

        if crate_graph.find_item(&full_mod_path, item).is_some() {
            return Some(
                ResolvedPath {
                    crate_name: actual_crate.to_string(),
                    module_path: full_mod_path,
                    item_name: item.clone(),
                }
                .full_path(),
            );
        }

        None
    }

    /// Resolve a method on a type from an external (non-workspace) crate.
    fn resolve_method_on_external_type(
        &self,
        use_stmt: &UseStatement,
        segments: &[&str],
    ) -> Option<String> {
        let ext_crate_name = &use_stmt.segments[0];
        let mut full_path = use_stmt.segments.clone();
        full_path.extend(segments[1..].iter().map(|s| s.to_string()));

        let module_path: Vec<String> =
            full_path[1..full_path.len() - 1].iter().cloned().collect();
        let item_name = full_path.last()?.clone();

        Some(
            ResolvedPath {
                crate_name: ext_crate_name.clone(),
                module_path,
                item_name,
            }
            .full_path(),
        )
    }

    /// Resolve a method via a crate-relative import (e.g., `use crate::module::Type`).
    fn resolve_method_via_crate_import(
        &self,
        use_stmt: &UseStatement,
        segments: &[&str],
        crate_name: &str,
    ) -> Option<String> {
        let mod_path: Vec<String> =
            use_stmt.segments[..use_stmt.segments.len() - 1].to_vec();
        let type_name = use_stmt.segments.last()?;
        let method_name = &segments[1];

        if let Some(graph) = self.module_graphs.get(crate_name) {
            let impl_name = format!("impl {}", type_name);
            if graph.find_item(&mod_path, &impl_name).is_some() {
                return Some(
                    ResolvedPath {
                        crate_name: crate_name.to_string(),
                        module_path: mod_path,
                        item_name: format!(
                            "impl {}::{}",
                            type_name, method_name
                        ),
                    }
                    .full_path(),
                );
            }
        }

        // Fall back to direct path resolution
        let mut full_path = use_stmt.segments.clone();
        full_path.extend(segments[1..].iter().map(|s| s.to_string()));
        let path_str = full_path.join("::");
        self.resolve(&path_str, crate_name, &["crate".to_string()].to_vec())
            .map(|r| r.full_path())
    }

    /// Resolve a use statement for a standalone file.
    fn resolve_use_for_standalone(
        &self,
        use_stmt: &UseStatement,
        from_crate: &str,
    ) -> Option<String> {
        // Handle crate-relative imports (use crate::foo::Bar)
        if use_stmt.is_crate_relative() {
            return self.resolve_crate_relative_use(use_stmt, from_crate);
        }

        // Handle external crate imports (use mdlr::foo::Bar)
        if use_stmt.is_external() {
            return self.resolve_external_use(use_stmt);
        }

        None
    }

    /// Resolve a crate-relative use statement (use crate::foo::Bar).
    fn resolve_crate_relative_use(
        &self,
        use_stmt: &UseStatement,
        from_crate: &str,
    ) -> Option<String> {
        let segments = &use_stmt.segments;
        if segments.len() < 2 {
            return None;
        }

        let module_path = &segments[..segments.len() - 1];
        let item_name = &segments[segments.len() - 1];

        let graph = self.module_graphs.get(from_crate)?;
        let (resolved_module, _) =
            graph.find_item_or_reexport(&module_path.to_vec(), item_name)?;

        Some(
            ResolvedPath {
                crate_name: from_crate.to_string(),
                module_path: resolved_module,
                item_name: item_name.clone(),
            }
            .full_path(),
        )
    }

    /// Resolve an external use statement (use mdlr::foo::Bar).
    fn resolve_external_use(&self, use_stmt: &UseStatement) -> Option<String> {
        let segments = &use_stmt.segments;
        if segments.is_empty() || segments.len() < 2 {
            return None;
        }

        let ext_crate_name = &segments[0];
        let actual_crate = self
            .crate_name_mapping
            .get(ext_crate_name)
            .cloned()
            .unwrap_or_else(|| ext_crate_name.clone());

        if let Some(crate_graph) = self.module_graphs.get(&actual_crate) {
            return self.resolve_workspace_crate_use(
                segments,
                &actual_crate,
                crate_graph,
            );
        }

        // External crate (not in workspace) - return canonical path
        self.resolve_non_workspace_crate_use(segments, ext_crate_name)
    }

    /// Resolve a use from a workspace crate.
    fn resolve_workspace_crate_use(
        &self,
        segments: &[String],
        actual_crate: &str,
        crate_graph: &ModuleGraph,
    ) -> Option<String> {
        let mut module_path = vec!["crate".to_string()];
        module_path.extend(segments[1..segments.len() - 1].iter().cloned());

        let item_name = &segments[segments.len() - 1];

        // Try direct item first
        if crate_graph.find_item(&module_path, item_name).is_some() {
            return Some(
                ResolvedPath {
                    crate_name: actual_crate.to_string(),
                    module_path,
                    item_name: item_name.clone(),
                }
                .full_path(),
            );
        }

        // Try re-exports
        let (resolved_module, _) =
            crate_graph.find_item_or_reexport(&module_path, item_name)?;
        Some(
            ResolvedPath {
                crate_name: actual_crate.to_string(),
                module_path: resolved_module,
                item_name: item_name.clone(),
            }
            .full_path(),
        )
    }

    /// Resolve a use from an external (non-workspace) crate.
    fn resolve_non_workspace_crate_use(
        &self,
        segments: &[String],
        ext_crate_name: &str,
    ) -> Option<String> {
        let item_name = segments.last()?.clone();
        let module_path: Vec<String> =
            segments[1..segments.len() - 1].iter().cloned().collect();

        Some(
            ResolvedPath {
                crate_name: ext_crate_name.to_string(),
                module_path,
                item_name,
            }
            .full_path(),
        )
    }
}
