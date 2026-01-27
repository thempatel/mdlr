//! Name resolution for Rust code.
//!
//! Resolves identifiers and paths to their definitions within a workspace.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::Parser;

use super::cargo::CargoWorkspace;
use super::modules::{ModuleGraph, ModulePath};
use super::uses::{extract_use_statement, resolve_path_prefix, UseKind, UseStatement};

/// Resolution context for a workspace.
///
/// Contains all the information needed to resolve names across a Cargo workspace.
#[derive(Debug)]
pub struct ResolutionContext {
    /// The Cargo workspace.
    pub workspace: CargoWorkspace,
    /// Module graphs for each crate, keyed by crate name.
    pub module_graphs: HashMap<String, ModuleGraph>,
    /// Mapping from crate name to its sanitized Rust identifier.
    /// E.g., "my-crate" -> "my_crate"
    crate_name_mapping: HashMap<String, String>,
}

/// The result of a successful resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    /// The crate containing the definition.
    pub crate_name: String,
    /// The full module path within the crate.
    pub module_path: ModulePath,
    /// The item name (last segment).
    pub item_name: String,
}

impl ResolvedPath {
    /// Get the full path as a string with `::` separators.
    pub fn full_path(&self) -> String {
        let mut path = self.module_path.clone();
        if !self.item_name.is_empty() {
            path.push(self.item_name.clone());
        }

        // Replace "crate" with actual crate name, or prepend crate name if not present
        if path.first().is_some_and(|s| s == "crate") {
            path[0] = self.crate_name.clone();
        } else if !self.crate_name.is_empty() {
            // For external crates, prepend the crate name
            path.insert(0, self.crate_name.clone());
        }

        path.join("::")
    }

    /// Get the path relative to the crate root.
    pub fn crate_relative_path(&self) -> String {
        let mut path = self.module_path.clone();
        if !self.item_name.is_empty() {
            path.push(self.item_name.clone());
        }
        path.join("::")
    }
}

impl ResolutionContext {
    /// Build a resolution context for a workspace.
    pub fn build(workspace: CargoWorkspace) -> Self {
        let mut module_graphs = HashMap::new();
        let mut crate_name_mapping = HashMap::new();

        for crate_info in &workspace.members {
            // Build module graph for each entry point
            let entry_points = crate_info.entry_points();

            for entry_point in entry_points {
                if let Ok(graph) = ModuleGraph::build(&crate_info.name, entry_point) {
                    // Store with both original and sanitized names
                    module_graphs.insert(crate_info.name.clone(), graph);

                    // Map sanitized name (hyphens -> underscores)
                    let sanitized = crate_info.name.replace('-', "_");
                    if sanitized != crate_info.name {
                        crate_name_mapping.insert(sanitized.clone(), crate_info.name.clone());
                    }

                    // Only process one entry point per crate for now
                    break;
                }
            }
        }

        ResolutionContext {
            workspace,
            module_graphs,
            crate_name_mapping,
        }
    }

    /// Resolve a name from a given context.
    ///
    /// # Arguments
    /// * `name` - The name to resolve (e.g., "HashMap", "crate::foo::Bar", "foo::bar")
    /// * `from_crate` - The crate the resolution is happening from
    /// * `from_module` - The module path within that crate
    ///
    /// Returns the resolved path, or None if unresolved.
    pub fn resolve(
        &self,
        name: &str,
        from_crate: &str,
        from_module: &[String],
    ) -> Option<ResolvedPath> {
        // Parse the name into segments
        let segments: Vec<&str> = name.split("::").collect();

        if segments.is_empty() {
            return None;
        }

        // Get the module graph for the source crate
        let from_graph = self.module_graphs.get(from_crate)?;

        // Strategy 1: Check if it's a crate/self/super relative path
        if segments[0] == "crate" || segments[0] == "self" || segments[0] == "super" {
            return self.resolve_relative_path(&segments, from_crate, from_module, from_graph);
        }

        // Strategy 2: Check local scope (items in current module)
        if segments.len() == 1 {
            if let Some(resolved) = self.check_local_scope(segments[0], from_crate, from_module, from_graph) {
                return Some(resolved);
            }
        }

        // Strategy 3: Check imports (use statements)
        if let Some(resolved) = self.check_imports(&segments, from_crate, from_module, from_graph) {
            return Some(resolved);
        }

        // Strategy 4: Check prelude items
        if segments.len() == 1 {
            if let Some(resolved) = self.check_prelude(segments[0]) {
                return Some(resolved);
            }
        }

        // Strategy 5: Check if it's an external crate name (workspace member or path dep)
        if let Some(resolved) = self.check_cross_crate(&segments) {
            return Some(resolved);
        }

        // Strategy 6: Check child modules
        if let Some(resolved) = self.check_child_modules(&segments, from_crate, from_module, from_graph) {
            return Some(resolved);
        }

        None
    }

    /// Resolve a crate/self/super relative path.
    fn resolve_relative_path(
        &self,
        segments: &[&str],
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        let owned_segments: Vec<String> = segments.iter().map(|s| s.to_string()).collect();

        let resolved_path = resolve_path_prefix(&owned_segments, from_module)?;

        // The last segment is the item name, rest is the module path
        if resolved_path.len() < 2 {
            return None;
        }

        let (module_path, item_name) = resolved_path.split_at(resolved_path.len() - 1);
        let item_name = &item_name[0];

        // Verify the item exists
        if from_graph.find_item(&module_path.to_vec(), item_name).is_some() {
            return Some(ResolvedPath {
                crate_name: from_crate.to_string(),
                module_path: module_path.to_vec(),
                item_name: item_name.clone(),
            });
        }

        // Maybe it's a module, not an item
        let full_path: Vec<String> = resolved_path;
        if from_graph.get_module(&full_path).is_some() {
            return Some(ResolvedPath {
                crate_name: from_crate.to_string(),
                module_path: full_path[..full_path.len() - 1].to_vec(),
                item_name: full_path.last()?.clone(),
            });
        }

        None
    }

    /// Check if a name exists in the current module's local scope.
    fn check_local_scope(
        &self,
        name: &str,
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        if from_graph.find_item(&from_module.to_vec(), name).is_some() {
            return Some(ResolvedPath {
                crate_name: from_crate.to_string(),
                module_path: from_module.to_vec(),
                item_name: name.to_string(),
            });
        }
        None
    }

    /// Check imports (use statements) in the current module.
    fn check_imports(
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
                    return self.resolve_use_statement(use_stmt, from_crate, from_module, from_graph);
                } else {
                    // The import is a type/module, and we're accessing something inside it
                    // e.g., Parser::new where Parser is imported from tree_sitter
                    let mut full_path = use_stmt.segments.clone();
                    full_path.extend(segments[1..].iter().map(|s| s.to_string()));

                    // For external crates, construct the path directly without verification
                    if use_stmt.is_external() && !use_stmt.segments.is_empty() {
                        let crate_name = &use_stmt.segments[0];
                        // Module path is everything except the crate name and the final item
                        let module_path: Vec<String> = full_path[1..full_path.len() - 1]
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
            UseKind::Single | UseKind::SelfImport => {
                let segments = &use_stmt.segments;

                // Handle super/self relative imports
                if !segments.is_empty() && (segments[0] == "super" || segments[0] == "self") {
                    // Resolve the path prefix (super/self) first
                    if let Some(resolved_path) = resolve_path_prefix(segments, from_module) {
                        if resolved_path.len() >= 2 {
                            let module_path = &resolved_path[..resolved_path.len() - 1];
                            let item_name = &resolved_path[resolved_path.len() - 1];

                            // Check for direct item or re-export
                            if let Some((resolved_module, _)) =
                                from_graph.find_item_or_reexport(&module_path.to_vec(), item_name)
                            {
                                return Some(ResolvedPath {
                                    crate_name: from_crate.to_string(),
                                    module_path: resolved_module,
                                    item_name: item_name.clone(),
                                });
                            }
                        }
                    }
                }

                // Handle crate-relative imports
                if use_stmt.is_crate_relative() {
                    if segments.len() < 2 {
                        return None;
                    }

                    let module_path = &segments[..segments.len() - 1];
                    let item_name = &segments[segments.len() - 1];

                    // Check for direct item or re-export
                    if let Some((resolved_module, _)) =
                        from_graph.find_item_or_reexport(&module_path.to_vec(), item_name)
                    {
                        return Some(ResolvedPath {
                            crate_name: from_crate.to_string(),
                            module_path: resolved_module,
                            item_name: item_name.clone(),
                        });
                    }
                }

                // Handle external crate imports
                if use_stmt.is_external() && !segments.is_empty() {
                    let crate_name = &segments[0];

                    // Check if it's a local crate (workspace member or path dep)
                    let actual_crate_name = self
                        .crate_name_mapping
                        .get(crate_name)
                        .cloned()
                        .unwrap_or_else(|| crate_name.clone());

                    if let Some(crate_graph) = self.module_graphs.get(&actual_crate_name) {
                        // Local crate - verify item exists
                        if segments.len() >= 2 {
                            let mut module_path = vec!["crate".to_string()];
                            module_path.extend(segments[1..segments.len() - 1].iter().cloned());

                            let item_name = &segments[segments.len() - 1];

                            if crate_graph.find_item(&module_path, item_name).is_some() {
                                return Some(ResolvedPath {
                                    crate_name: actual_crate_name,
                                    module_path,
                                    item_name: item_name.clone(),
                                });
                            }
                        }
                    } else {
                        // External crate (not in workspace) - return canonical path without verification
                        // This allows consistent resolution across files even without the crate's module graph
                        if segments.len() >= 2 {
                            let item_name = &segments[segments.len() - 1];
                            // Use the crate name as-is (e.g., "tree_sitter", "anyhow")
                            // Module path is everything between crate name and item name
                            let module_path: Vec<String> = segments[1..segments.len() - 1]
                                .iter()
                                .cloned()
                                .collect();

                            return Some(ResolvedPath {
                                crate_name: crate_name.clone(),
                                module_path,
                                item_name: item_name.clone(),
                            });
                        } else if segments.len() == 1 {
                            // Just the crate name itself (rare but possible)
                            return Some(ResolvedPath {
                                crate_name: crate_name.clone(),
                                module_path: vec![],
                                item_name: String::new(),
                            });
                        }
                    }
                }

                None
            }
            UseKind::Glob => {
                // Glob imports are harder - we'd need to know what we're looking for
                // For now, return None and let the caller handle unresolved
                None
            }
        }
    }

    /// Check if a name is in the prelude.
    fn check_prelude(&self, name: &str) -> Option<ResolvedPath> {
        // Common prelude items - we mark these as from "std" but unresolved
        // since we don't have std's module graph
        let prelude_items = [
            "Option", "Some", "None", "Result", "Ok", "Err", "Vec", "String", "Box", "Clone",
            "Copy", "Default", "Drop", "Eq", "PartialEq", "Ord", "PartialOrd", "Hash", "Debug",
            "Display", "Iterator", "IntoIterator", "From", "Into", "TryFrom", "TryInto",
            "AsRef", "AsMut", "Deref", "DerefMut", "Send", "Sync", "Sized", "Unpin", "Fn",
            "FnMut", "FnOnce", "ToString", "ToOwned",
        ];

        if prelude_items.contains(&name) {
            // Return a synthetic path indicating it's from std prelude
            return Some(ResolvedPath {
                crate_name: "std".to_string(),
                module_path: vec!["prelude".to_string()],
                item_name: name.to_string(),
            });
        }

        None
    }

    /// Check if the path refers to another crate (workspace member or external).
    fn check_cross_crate(&self, segments: &[&str]) -> Option<ResolvedPath> {
        if segments.is_empty() {
            return None;
        }

        let first = segments[0];

        // Check if this is a crate name (with hyphen to underscore mapping)
        let actual_crate_name = self
            .crate_name_mapping
            .get(first)
            .cloned()
            .unwrap_or_else(|| first.to_string());

        if let Some(crate_graph) = self.module_graphs.get(&actual_crate_name) {
            // Workspace member crate - verify item exists
            if segments.len() == 1 {
                // Just the crate name, refers to the crate root
                return Some(ResolvedPath {
                    crate_name: actual_crate_name,
                    module_path: vec!["crate".to_string()],
                    item_name: "".to_string(), // No specific item
                });
            }

            // Build the path within the crate
            let mut module_path = vec!["crate".to_string()];

            // Try progressively shorter module paths to find an item
            for i in (1..segments.len()).rev() {
                module_path = vec!["crate".to_string()];
                module_path.extend(segments[1..i].iter().map(|s| s.to_string()));

                let item_name = segments[i];

                if crate_graph.find_item(&module_path, item_name).is_some() {
                    return Some(ResolvedPath {
                        crate_name: actual_crate_name,
                        module_path,
                        item_name: item_name.to_string(),
                    });
                }
            }
        } else if segments.len() >= 2 {
            // External crate (not in workspace) - assume valid and construct path
            // This handles cases like tree_sitter::Parser, anyhow::Result, etc.
            // We can't verify the item exists, but we can construct a canonical path
            let crate_name = first.to_string();
            let module_path: Vec<String> = segments[1..segments.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect();
            let item_name = segments[segments.len() - 1].to_string();

            return Some(ResolvedPath {
                crate_name,
                module_path,
                item_name,
            });
        }

        None
    }

    /// Check if path refers to a child module.
    fn check_child_modules(
        &self,
        segments: &[&str],
        from_crate: &str,
        from_module: &[String],
        from_graph: &ModuleGraph,
    ) -> Option<ResolvedPath> {
        if segments.is_empty() {
            return None;
        }

        let module = from_graph.get_module(&from_module.to_vec())?;
        let first = segments[0];

        // Check if first segment is a child module
        if module.has_child(first) {
            let mut child_path = from_module.to_vec();
            child_path.push(first.to_string());

            if segments.len() == 1 {
                // Just the module name
                return Some(ResolvedPath {
                    crate_name: from_crate.to_string(),
                    module_path: from_module.to_vec(),
                    item_name: first.to_string(),
                });
            }

            // Recurse into the child module
            let remaining: Vec<&str> = segments[1..].to_vec();
            let remaining_str = remaining.join("::");
            return self.resolve(&remaining_str, from_crate, &child_path);
        }

        None
    }

    /// Get the module graph for a crate.
    pub fn get_crate_graph(&self, crate_name: &str) -> Option<&ModuleGraph> {
        self.module_graphs.get(crate_name)
    }

    /// List all crates in the resolution context.
    pub fn crate_names(&self) -> Vec<&str> {
        self.module_graphs.keys().map(|s| s.as_str()).collect()
    }

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
            let resolved = self.resolve(call_name, &crate_name, &module_path)?;
            return Some(resolved.full_path());
        }

        // File is not in module graph (e.g., main.rs binary)
        // Parse use statements from the source file and resolve using those
        self.resolve_call_for_standalone_file(call_name, file_path, &crate_name)
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
        // Parse the file's use statements
        let source = std::fs::read_to_string(file_path).ok()?;
        let uses = parse_file_uses(&source)?;

        // Parse the call name into segments
        let segments: Vec<&str> = call_name.split("::").collect();

        if segments.is_empty() {
            return None;
        }

        // Strategy 1: Check if this is a direct external crate path (e.g., tree_sitter::Parser)
        // Only applies when the first segment looks like a crate name (lowercase, not a type)
        if segments.len() >= 2 {
            let first_segment = segments[0];

            // Check if the first segment is an external crate
            let actual_crate = self
                .crate_name_mapping
                .get(first_segment)
                .map(|s| s.as_str())
                .unwrap_or(first_segment);

            if self.module_graphs.contains_key(actual_crate) {
                // It's a workspace crate - resolve through its module graph
                let result = self.check_cross_crate(&segments);
                if let Some(resolved) = result {
                    return Some(resolved.full_path());
                }
            } else if !["crate", "self", "super"].contains(&first_segment) {
                // Check if this looks like a crate name (snake_case, starts with lowercase)
                // vs a type name (PascalCase, starts with uppercase)
                // Type names like "SourceWalker" should not be treated as crate paths
                let looks_like_crate = first_segment
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_lowercase());

                if looks_like_crate {
                    // External crate (not in workspace) - construct canonical path
                    let module_path: Vec<String> = segments[1..segments.len() - 1]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                    let item_name = segments[segments.len() - 1].to_string();

                    return Some(ResolvedPath {
                        crate_name: first_segment.to_string(),
                        module_path,
                        item_name,
                    }.full_path());
                }
            }
        }

        // Strategy 2: Check if it's imported via use statements
        let first = segments[0];
        for use_stmt in &uses {
            let imported_name = match use_stmt.imported_name() {
                Some(name) => name,
                None => continue,
            };
            if imported_name != first {
                continue;
            }

            // Found a matching import
            if segments.len() == 1 {
                // Simple import - resolve the use statement
                return self.resolve_use_for_standalone(use_stmt, crate_name);
            } else {
                // The import is a type/module, and we're accessing something inside it
                // e.g., Parser::new where Parser is imported from tree_sitter
                // or SourceWalker::new where SourceWalker is imported from mdlr::walk

                // For external crates (including workspace crates accessed via their name)
                if use_stmt.is_external() && !use_stmt.segments.is_empty() {
                    let ext_crate_name = &use_stmt.segments[0];

                    // Check if it's a workspace crate
                    let actual_crate = self
                        .crate_name_mapping
                        .get(ext_crate_name)
                        .cloned()
                        .unwrap_or_else(|| ext_crate_name.clone());

                    if let Some(crate_graph) = self.module_graphs.get(&actual_crate) {
                        // Workspace crate - the import points to a type
                        // Build the module path: crate::module (where the type is defined)
                        let mut mod_path = vec!["crate".to_string()];
                        mod_path.extend(use_stmt.segments[1..use_stmt.segments.len() - 1].iter().cloned());

                        let type_name = use_stmt.segments.last()?;
                        let method_name = &segments[1];
                        let impl_name = format!("impl {}", type_name);

                        // First check if the impl is in the direct module path
                        if crate_graph.find_item(&mod_path, &impl_name).is_some() {
                            return Some(ResolvedPath {
                                crate_name: actual_crate,
                                module_path: mod_path,
                                item_name: format!("impl {}::{}", type_name, method_name),
                            }.full_path());
                        }

                        // If not found directly, check if the type is re-exported
                        // Follow the re-export to find the actual module where impl lives
                        if let Some((resolved_module, _)) =
                            crate_graph.find_item_or_reexport(&mod_path, type_name)
                        {
                            // Check if the impl is in the resolved module
                            if crate_graph.find_item(&resolved_module, &impl_name).is_some() {
                                return Some(ResolvedPath {
                                    crate_name: actual_crate,
                                    module_path: resolved_module,
                                    item_name: format!("impl {}::{}", type_name, method_name),
                                }.full_path());
                            }
                        }

                        // Fall back to treating it as a direct item (might be a nested module)
                        let mut full_path = use_stmt.segments.clone();
                        full_path.extend(segments[1..].iter().map(|s| s.to_string()));

                        let mut full_mod_path = vec!["crate".to_string()];
                        full_mod_path.extend(full_path[1..full_path.len() - 1].iter().cloned());

                        let item = &full_path[full_path.len() - 1];

                        if crate_graph.find_item(&full_mod_path, item).is_some() {
                            return Some(ResolvedPath {
                                crate_name: actual_crate,
                                module_path: full_mod_path,
                                item_name: item.clone(),
                            }.full_path());
                        }
                    } else {
                        // External crate (not in workspace) - construct path without verification
                        // For Type::method, construct type::method path
                        let mut full_path = use_stmt.segments.clone();
                        full_path.extend(segments[1..].iter().map(|s| s.to_string()));

                        let module_path: Vec<String> = full_path[1..full_path.len() - 1]
                            .iter()
                            .cloned()
                            .collect();
                        let item_name = full_path.last()?.clone();

                        return Some(ResolvedPath {
                            crate_name: ext_crate_name.clone(),
                            module_path,
                            item_name,
                        }.full_path());
                    }
                }

                // For crate-relative paths (use crate::module::Type)
                if use_stmt.is_crate_relative() && use_stmt.segments.len() >= 2 {
                    // Build the module path where the type is defined
                    let mod_path: Vec<String> = use_stmt.segments[..use_stmt.segments.len() - 1].to_vec();
                    let type_name = use_stmt.segments.last()?;
                    let method_name = &segments[1];

                    if let Some(graph) = self.module_graphs.get(crate_name) {
                        // Look for "impl Type" in the module
                        let impl_name = format!("impl {}", type_name);
                        if graph.find_item(&mod_path, &impl_name).is_some() {
                            return Some(ResolvedPath {
                                crate_name: crate_name.to_string(),
                                module_path: mod_path,
                                item_name: format!("impl {}::{}", type_name, method_name),
                            }.full_path());
                        }
                    }

                    // Fall back to direct path resolution
                    let mut full_path = use_stmt.segments.clone();
                    full_path.extend(segments[1..].iter().map(|s| s.to_string()));
                    let path_str = full_path.join("::");
                    return self.resolve(&path_str, crate_name, &["crate".to_string()].to_vec())
                        .map(|r| r.full_path());
                }
            }
        }

        // Strategy 3: Check prelude
        if segments.len() == 1 {
            if let Some(resolved) = self.check_prelude(first) {
                return Some(resolved.full_path());
            }
        }

        // Strategy 4: Check if it's a direct cross-crate path
        if let Some(resolved) = self.check_cross_crate(&segments) {
            return Some(resolved.full_path());
        }

        None
    }

    /// Resolve a use statement for a standalone file.
    fn resolve_use_for_standalone(
        &self,
        use_stmt: &UseStatement,
        from_crate: &str,
    ) -> Option<String> {
        let segments = &use_stmt.segments;

        // Handle crate-relative imports (use crate::foo::Bar)
        if use_stmt.is_crate_relative() && segments.len() >= 2 {
            let module_path = &segments[..segments.len() - 1];
            let item_name = &segments[segments.len() - 1];

            if let Some(graph) = self.module_graphs.get(from_crate) {
                if let Some((resolved_module, _)) =
                    graph.find_item_or_reexport(&module_path.to_vec(), item_name)
                {
                    return Some(ResolvedPath {
                        crate_name: from_crate.to_string(),
                        module_path: resolved_module,
                        item_name: item_name.clone(),
                    }.full_path());
                }
            }
        }

        // Handle external crate imports (use mdlr::foo::Bar)
        if use_stmt.is_external() && !segments.is_empty() {
            let ext_crate_name = &segments[0];

            // Check if it's a workspace crate
            let actual_crate = self
                .crate_name_mapping
                .get(ext_crate_name)
                .cloned()
                .unwrap_or_else(|| ext_crate_name.clone());

            if let Some(crate_graph) = self.module_graphs.get(&actual_crate) {
                // Workspace crate - verify item exists
                if segments.len() >= 2 {
                    let mut module_path = vec!["crate".to_string()];
                    module_path.extend(segments[1..segments.len() - 1].iter().cloned());

                    let item_name = &segments[segments.len() - 1];

                    // Try direct item first
                    if crate_graph.find_item(&module_path, item_name).is_some() {
                        return Some(ResolvedPath {
                            crate_name: actual_crate,
                            module_path,
                            item_name: item_name.clone(),
                        }.full_path());
                    }

                    // Try re-exports
                    if let Some((resolved_module, _)) =
                        crate_graph.find_item_or_reexport(&module_path, item_name)
                    {
                        return Some(ResolvedPath {
                            crate_name: actual_crate,
                            module_path: resolved_module,
                            item_name: item_name.clone(),
                        }.full_path());
                    }
                }
            } else {
                // External crate (not in workspace) - return canonical path
                if segments.len() >= 2 {
                    let item_name = &segments[segments.len() - 1];
                    let module_path: Vec<String> = segments[1..segments.len() - 1]
                        .iter()
                        .cloned()
                        .collect();

                    return Some(ResolvedPath {
                        crate_name: ext_crate_name.clone(),
                        module_path,
                        item_name: item_name.clone(),
                    }.full_path());
                }
            }
        }

        None
    }

    /// Map a file path to its crate and module path.
    pub fn file_to_module(&self, file_path: &Path) -> Option<(String, ModulePath)> {
        // Canonicalize the input file path for comparison
        let canonical_file = file_path.canonicalize().ok()?;

        // Find the crate this file belongs to
        for crate_info in &self.workspace.members {
            // Canonicalize the crate root for comparison
            let canonical_root = crate_info.root.canonicalize().ok();
            let root_match = canonical_root
                .as_ref()
                .is_some_and(|r| canonical_file.starts_with(r));

            if root_match {
                // Found the crate, now find the module
                if let Some(graph) = self.module_graphs.get(&crate_info.name) {
                    for (module_path, module_node) in &graph.modules {
                        // Canonicalize the module file path for comparison
                        if let Ok(canonical_module) = module_node.file.canonicalize() {
                            if canonical_module == canonical_file {
                                return Some((crate_info.name.clone(), module_path.clone()));
                            }
                        }
                    }
                }

                // If not found in graph, it's probably the crate root or a binary
                return Some((crate_info.name.clone(), vec!["crate".to_string()]));
            }
        }

        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel_path: &str, content: &str) {
        let full_path = dir.join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }

    fn setup_workspace() -> (TempDir, ResolutionContext) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create a workspace with two crates
        write_file(
            root,
            "Cargo.toml",
            r#"
[workspace]
members = ["crate-a", "crate-b"]
"#,
        );

        // crate-a
        write_file(
            root,
            "crate-a/Cargo.toml",
            r#"
[package]
name = "crate-a"
version = "0.1.0"
"#,
        );
        write_file(
            root,
            "crate-a/src/lib.rs",
            r#"
mod foo;

pub struct RootStruct;

pub fn root_fn() {}
"#,
        );
        write_file(
            root,
            "crate-a/src/foo.rs",
            r#"
pub struct FooStruct;

pub fn foo_fn() {}
"#,
        );

        // crate-b depends on crate-a
        write_file(
            root,
            "crate-b/Cargo.toml",
            r#"
[package]
name = "crate-b"
version = "0.1.0"

[dependencies]
crate-a = { path = "../crate-a" }
"#,
        );
        write_file(
            root,
            "crate-b/src/lib.rs",
            r#"
use crate_a::RootStruct;
use crate_a::foo::FooStruct;

pub fn use_a() {
    let _ = RootStruct;
    let _ = FooStruct;
}
"#,
        );

        let workspace = CargoWorkspace::discover(root).unwrap();
        let ctx = ResolutionContext::build(workspace);

        (tmp, ctx)
    }

    #[test]
    fn test_resolve_local_item() {
        let (_tmp, ctx) = setup_workspace();

        let resolved = ctx.resolve(
            "RootStruct",
            "crate-a",
            &vec!["crate".to_string()],
        );

        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.crate_name, "crate-a");
        assert_eq!(resolved.item_name, "RootStruct");
    }

    #[test]
    fn test_resolve_crate_relative() {
        let (_tmp, ctx) = setup_workspace();

        let resolved = ctx.resolve(
            "crate::foo::FooStruct",
            "crate-a",
            &vec!["crate".to_string()],
        );

        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.crate_name, "crate-a");
        assert_eq!(resolved.module_path, vec!["crate", "foo"]);
        assert_eq!(resolved.item_name, "FooStruct");
    }

    #[test]
    fn test_resolve_child_module() {
        let (_tmp, ctx) = setup_workspace();

        let resolved = ctx.resolve(
            "foo::foo_fn",
            "crate-a",
            &vec!["crate".to_string()],
        );

        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.item_name, "foo_fn");
    }

    #[test]
    fn test_resolve_prelude() {
        let (_tmp, ctx) = setup_workspace();

        let resolved = ctx.resolve(
            "Option",
            "crate-a",
            &vec!["crate".to_string()],
        );

        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.crate_name, "std");
        assert_eq!(resolved.item_name, "Option");
    }

    #[test]
    fn test_resolve_cross_crate() {
        let (_tmp, ctx) = setup_workspace();

        // Note: crate-a is referred to as crate_a in Rust code
        let resolved = ctx.resolve(
            "crate_a::RootStruct",
            "crate-b",
            &vec!["crate".to_string()],
        );

        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.crate_name, "crate-a");
        assert_eq!(resolved.item_name, "RootStruct");
    }

    #[test]
    fn test_full_path() {
        let resolved = ResolvedPath {
            crate_name: "my_crate".to_string(),
            module_path: vec!["crate".to_string(), "foo".to_string()],
            item_name: "Bar".to_string(),
        };

        assert_eq!(resolved.full_path(), "my_crate::foo::Bar");
    }

    #[test]
    fn test_file_to_module() {
        let (_tmp, ctx) = setup_workspace();

        // The file paths include the temp directory, so we need to construct them properly
        let crate_a = ctx.workspace.find_crate("crate-a").unwrap();
        let lib_path = crate_a.lib_path.as_ref().unwrap();

        let (crate_name, module_path) = ctx.file_to_module(lib_path).unwrap();
        assert_eq!(crate_name, "crate-a");
        assert_eq!(module_path, vec!["crate"]);
    }
}
