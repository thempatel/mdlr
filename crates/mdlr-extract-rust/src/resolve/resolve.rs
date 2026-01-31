//! Name resolution for Rust code.
//!
//! Resolves identifiers and paths to their definitions within a workspace.

use std::collections::HashMap;
use std::path::Path;

use mdlr_core::CallResolver;

use super::cargo::CargoWorkspace;
use super::modules::{ModuleGraph, ModulePath};
use super::uses::resolve_path_prefix;

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
    pub(super) crate_name_mapping: HashMap<String, String>,
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
                if let Ok(graph) =
                    ModuleGraph::build(&crate_info.name, entry_point)
                {
                    // Store with both original and sanitized names
                    module_graphs.insert(crate_info.name.clone(), graph);

                    // Map sanitized name (hyphens -> underscores)
                    let sanitized = crate_info.name.replace('-', "_");
                    if sanitized != crate_info.name {
                        crate_name_mapping.insert(
                            sanitized.clone(),
                            crate_info.name.clone(),
                        );
                    }
                    // TODO: Need to process all and cache resolution across all
                    // Only process one entry point per crate for now
                    break;
                }
            }
        }

        ResolutionContext { workspace, module_graphs, crate_name_mapping }
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
        if segments[0] == "crate"
            || segments[0] == "self"
            || segments[0] == "super"
        {
            return self.resolve_relative_path(
                &segments,
                from_crate,
                from_module,
                from_graph,
            );
        }

        // Strategy 2: Check local scope (items in current module)
        if segments.len() == 1 {
            if let Some(resolved) = self.check_local_scope(
                segments[0],
                from_crate,
                from_module,
                from_graph,
            ) {
                return Some(resolved);
            }
        }

        // Strategy 3: Check imports (use statements)
        if let Some(resolved) =
            self.check_imports(&segments, from_crate, from_module, from_graph)
        {
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
        if let Some(resolved) = self.check_child_modules(
            &segments,
            from_crate,
            from_module,
            from_graph,
        ) {
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
        let owned_segments: Vec<String> =
            segments.iter().map(|s| s.to_string()).collect();

        let resolved_path = resolve_path_prefix(&owned_segments, from_module)?;

        // The last segment is the item name, rest is the module path
        if resolved_path.len() < 2 {
            return None;
        }

        let (module_path, item_name) =
            resolved_path.split_at(resolved_path.len() - 1);
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

    /// Check if a name is in the prelude.
    pub(super) fn check_prelude(&self, name: &str) -> Option<ResolvedPath> {
        // Common prelude items - we mark these as from "std" but unresolved
        // since we don't have std's module graph
        let prelude_items = [
            "Option",
            "Some",
            "None",
            "Result",
            "Ok",
            "Err",
            "Vec",
            "String",
            "Box",
            "Clone",
            "Copy",
            "Default",
            "Drop",
            "Eq",
            "PartialEq",
            "Ord",
            "PartialOrd",
            "Hash",
            "Debug",
            "Display",
            "Iterator",
            "IntoIterator",
            "From",
            "Into",
            "TryFrom",
            "TryInto",
            "AsRef",
            "AsMut",
            "Deref",
            "DerefMut",
            "Send",
            "Sync",
            "Sized",
            "Unpin",
            "Fn",
            "FnMut",
            "FnOnce",
            "ToString",
            "ToOwned",
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
    pub(super) fn check_cross_crate(
        &self,
        segments: &[&str],
    ) -> Option<ResolvedPath> {
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

            // Try progressively shorter module paths to find an item
            for i in (1..segments.len()).rev() {
                let mut module_path = vec!["crate".to_string()];
                module_path
                    .extend(segments[1..i].iter().map(|s| s.to_string()));

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

            return Some(ResolvedPath { crate_name, module_path, item_name });
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

    /// Map a file path to its crate and module path.
    pub fn file_to_module(
        &self,
        file_path: &Path,
    ) -> Option<(String, ModulePath)> {
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
                        if let Ok(canonical_module) =
                            module_node.file.canonicalize()
                        {
                            if canonical_module == canonical_file {
                                return Some((
                                    crate_info.name.clone(),
                                    module_path.clone(),
                                ));
                            }
                        }
                    }
                }

                // If not found in graph, it's probably the crate root or a binary
                return Some((
                    crate_info.name.clone(),
                    vec!["crate".to_string()],
                ));
            }
        }

        None
    }
}

/// Implement CallResolver trait for ResolutionContext.
///
/// This allows the graph builder to use ResolutionContext for call resolution.
impl CallResolver for ResolutionContext {
    fn resolve_call(&self, call: &str, caller_file: &Path) -> Option<String> {
        // Get the crate and module for the caller file
        let (from_crate, from_module) = self.file_to_module(caller_file)?;

        // Try to resolve the call
        let resolved = self.resolve(call, &from_crate, &from_module)?;

        // Return the full path
        Some(resolved.full_path())
    }

    fn file_to_module(&self, file: &Path) -> Option<(String, Vec<String>)> {
        ResolutionContext::file_to_module(self, file)
    }
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

        let resolved =
            ctx.resolve("RootStruct", "crate-a", &vec!["crate".to_string()]);

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

        let resolved =
            ctx.resolve("foo::foo_fn", "crate-a", &vec!["crate".to_string()]);

        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.item_name, "foo_fn");
    }

    #[test]
    fn test_resolve_prelude() {
        let (_tmp, ctx) = setup_workspace();

        let resolved =
            ctx.resolve("Option", "crate-a", &vec!["crate".to_string()]);

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
