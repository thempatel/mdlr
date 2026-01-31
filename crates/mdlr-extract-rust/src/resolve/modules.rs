//! Module graph construction for Rust crates.
//!
//! Builds a tree structure representing Rust's module hierarchy by following
//! `mod` declarations and Rust's file conventions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use super::item_extraction::{
    extract_impl_item, extract_item_def, extract_use_declaration,
    extract_visibility, item_kind_from_node, node_span, node_text,
};
use super::uses::{UseStatement, Visibility};

/// A module path as a sequence of segments.
/// Example: `["crate", "foo", "bar"]` represents `crate::foo::bar`.
pub type ModulePath = Vec<String>;

/// A complete module graph for a single crate.
#[derive(Debug, Clone)]
pub struct ModuleGraph {
    /// The crate name (used as the root module name).
    pub crate_name: String,
    /// Path to the crate root file (lib.rs or main.rs).
    pub root_file: PathBuf,
    /// All modules indexed by their full path.
    pub modules: HashMap<ModulePath, ModuleNode>,
}

/// A single module in the module graph.
#[derive(Debug, Clone)]
pub struct ModuleNode {
    /// The file this module is defined in.
    pub file: PathBuf,
    /// Items defined in this module.
    pub items: Vec<ItemDef>,
    /// Names of child modules (populated from `mod` declarations).
    pub children: Vec<String>,
    /// Use statements in this module.
    pub uses: Vec<UseStatement>,
    /// Re-exports (pub use statements).
    pub reexports: Vec<UseStatement>,
}

/// A definition of an item within a module.
#[derive(Debug, Clone)]
pub struct ItemDef {
    /// The item's name.
    pub name: String,
    /// The kind of item.
    pub kind: ItemKind,
    /// Visibility of the item.
    pub visibility: Visibility,
    /// Location in the source file.
    pub span: ItemSpan,
}

/// The kind of a defined item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Static,
    TypeAlias,
    Macro,
    Module,
}

/// Source location of an item.
#[derive(Debug, Clone, Copy)]
pub struct ItemSpan {
    pub start_line: usize,
    pub end_line: usize,
}

impl ModuleGraph {
    /// Build a module graph starting from a crate root file.
    pub fn build(crate_name: &str, root_file: &Path) -> Result<Self> {
        let mut graph = ModuleGraph {
            crate_name: crate_name.to_string(),
            root_file: root_file.to_path_buf(),
            modules: HashMap::new(),
        };

        // Start with the crate root module
        let root_path = vec!["crate".to_string()];
        graph.build_module(&root_path, root_file)?;

        Ok(graph)
    }

    /// Recursively build a module and its children.
    fn build_module(
        &mut self,
        module_path: &ModulePath,
        file_path: &Path,
    ) -> Result<()> {
        // Skip if already processed
        if self.modules.contains_key(module_path) {
            return Ok(());
        }

        // Parse the file
        let source =
            std::fs::read_to_string(file_path).with_context(|| {
                format!("Failed to read {}", file_path.display())
            })?;

        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;

        let tree =
            parser.parse(&source, None).context("Failed to parse source")?;

        // Extract module information
        let mut node = ModuleNode {
            file: file_path.to_path_buf(),
            items: Vec::new(),
            children: Vec::new(),
            uses: Vec::new(),
            reexports: Vec::new(),
        };

        self.extract_module_info(
            tree.root_node(),
            &source,
            file_path,
            &mut node,
        )?;

        // Collect child module names before inserting
        let children: Vec<String> = node.children.clone();

        // Insert the module
        self.modules.insert(module_path.clone(), node);

        // Process child modules
        for child_name in children {
            let child_path: ModulePath = module_path
                .iter()
                .cloned()
                .chain(std::iter::once(child_name.clone()))
                .collect();

            // Resolve the child module's file path
            if let Some(child_file) =
                resolve_module_file(file_path, &child_name)
            {
                self.build_module(&child_path, &child_file)?;
            }
        }

        Ok(())
    }

    /// Extract items, children, and use statements from a module.
    fn extract_module_info(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        module: &mut ModuleNode,
    ) -> Result<()> {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if let Some(kind) = item_kind_from_node(child.kind()) {
                if let Some(item) = extract_item_def(child, source, kind) {
                    module.items.push(item);
                }
            } else {
                match child.kind() {
                    "impl_item" => extract_impl_item(child, source, module),
                    "mod_item" => self
                        .extract_mod_item(child, source, file_path, module)?,
                    "use_declaration" => {
                        extract_use_declaration(child, source, module)
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    /// Extract a mod_item node into the module.
    fn extract_mod_item(
        &self,
        child: Node,
        source: &str,
        file_path: &Path,
        module: &mut ModuleNode,
    ) -> Result<()> {
        let Some(name_node) = child.child_by_field_name("name") else {
            return Ok(());
        };
        let mod_name = node_text(name_node, source);

        // Check if it has a body (inline module)
        if let Some(body) = child.child_by_field_name("body") {
            // Inline module - extract recursively
            self.extract_module_info(body, source, file_path, module)?;
        } else {
            // External module - add to children to be resolved later
            module.children.push(mod_name.clone());
        }

        // Also add as an item
        module.items.push(ItemDef {
            name: mod_name,
            kind: ItemKind::Module,
            visibility: extract_visibility(child, source),
            span: node_span(child),
        });

        Ok(())
    }

    /// Find a module by its path.
    pub fn get_module(&self, path: &ModulePath) -> Option<&ModuleNode> {
        self.modules.get(path)
    }

    /// Find an item by name within a module.
    pub fn find_item(
        &self,
        module_path: &ModulePath,
        name: &str,
    ) -> Option<&ItemDef> {
        self.modules
            .get(module_path)?
            .items
            .iter()
            .find(|item| item.name == name)
    }

    /// Find an item by name, checking both direct items and re-exports.
    ///
    /// Returns the resolved module path and item definition if found.
    /// For re-exports, follows the re-export to find the actual definition.
    pub fn find_item_or_reexport(
        &self,
        module_path: &ModulePath,
        name: &str,
    ) -> Option<(ModulePath, &ItemDef)> {
        let module = self.modules.get(module_path)?;

        // First check direct items
        if let Some(item) = module.items.iter().find(|item| item.name == name)
        {
            return Some((module_path.clone(), item));
        }

        // Then check re-exports
        for reexport in &module.reexports {
            if let Some(imported_name) = reexport.imported_name() {
                if imported_name == name {
                    // Follow the re-export to find the actual item
                    // reexport.segments is like ["types", "Unit"]
                    // We need to resolve this relative to the current module
                    let mut target_module = module_path.clone();
                    for segment in &reexport.segments
                        [..reexport.segments.len().saturating_sub(1)]
                    {
                        target_module.push(segment.clone());
                    }

                    if let Some(item) = self.find_item(&target_module, name) {
                        return Some((target_module, item));
                    }
                }
            }
        }

        None
    }

    /// Get all items visible from a module (including re-exports).
    pub fn visible_items<'a>(
        &'a self,
        module_path: &'a ModulePath,
    ) -> Vec<(&'a ModulePath, &'a ItemDef)> {
        let mut items = Vec::new();

        if let Some(module) = self.modules.get(module_path) {
            for item in &module.items {
                items.push((module_path, item));
            }
        }

        items
    }

    /// Get the full path to an item given a module path and item name.
    pub fn item_path(
        &self,
        module_path: &ModulePath,
        name: &str,
    ) -> Option<ModulePath> {
        self.find_item(module_path, name)?;
        let mut path = module_path.clone();
        path.push(name.to_string());
        Some(path)
    }
}

impl ModuleNode {
    /// Check if this module contains an item with the given name.
    pub fn has_item(&self, name: &str) -> bool {
        self.items.iter().any(|item| item.name == name)
    }

    /// Get an item by name.
    pub fn get_item(&self, name: &str) -> Option<&ItemDef> {
        self.items.iter().find(|item| item.name == name)
    }

    /// Check if this module has a child module with the given name.
    pub fn has_child(&self, name: &str) -> bool {
        self.children.contains(&name.to_string())
    }
}

/// Resolve a module file path following Rust's conventions.
///
/// Given a parent file and a module name, returns the path to the module's file.
/// Looks for:
/// 1. `<name>.rs` in the same directory
/// 2. `<name>/mod.rs` in a subdirectory
fn resolve_module_file(parent_file: &Path, mod_name: &str) -> Option<PathBuf> {
    let parent_dir = parent_file.parent()?;
    let parent_stem = parent_file.file_stem()?.to_str()?;

    // Determine the base directory for looking up the module
    let base_dir = if parent_stem == "mod"
        || parent_stem == "lib"
        || parent_stem == "main"
    {
        parent_dir.to_path_buf()
    } else {
        // For a file like foo.rs, look in foo/ directory
        parent_dir.join(parent_stem)
    };

    // Try <name>.rs first
    let file_path = base_dir.join(format!("{}.rs", mod_name));
    if file_path.exists() {
        return Some(file_path);
    }

    // Try <name>/mod.rs
    let mod_path = base_dir.join(mod_name).join("mod.rs");
    if mod_path.exists() {
        return Some(mod_path);
    }

    None
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

    #[test]
    fn test_simple_module_graph() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(
            root,
            "src/lib.rs",
            r#"
pub fn public_fn() {}
fn private_fn() {}
pub struct PublicStruct;
struct PrivateStruct;
"#,
        );

        let lib_path = root.join("src/lib.rs");
        let graph = ModuleGraph::build("test_crate", &lib_path).unwrap();

        assert_eq!(graph.crate_name, "test_crate");

        let root_module =
            graph.get_module(&vec!["crate".to_string()]).unwrap();
        assert_eq!(root_module.items.len(), 4);

        assert!(root_module.has_item("public_fn"));
        assert!(root_module.has_item("private_fn"));

        let public_fn = root_module.get_item("public_fn").unwrap();
        assert_eq!(public_fn.visibility, Visibility::Public);

        let private_fn = root_module.get_item("private_fn").unwrap();
        assert_eq!(private_fn.visibility, Visibility::Private);
    }

    #[test]
    fn test_module_with_children() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(
            root,
            "src/lib.rs",
            r#"
mod foo;
mod bar;

pub fn root_fn() {}
"#,
        );

        write_file(
            root,
            "src/foo.rs",
            r#"
pub fn foo_fn() {}
"#,
        );

        write_file(
            root,
            "src/bar/mod.rs",
            r#"
pub fn bar_fn() {}
"#,
        );

        let lib_path = root.join("src/lib.rs");
        let graph = ModuleGraph::build("test_crate", &lib_path).unwrap();

        // Check root module has children
        let root_module =
            graph.get_module(&vec!["crate".to_string()]).unwrap();
        assert!(root_module.children.contains(&"foo".to_string()));
        assert!(root_module.children.contains(&"bar".to_string()));

        // Check foo module
        let foo_path = vec!["crate".to_string(), "foo".to_string()];
        let foo_module = graph.get_module(&foo_path).unwrap();
        assert!(foo_module.has_item("foo_fn"));

        // Check bar module
        let bar_path = vec!["crate".to_string(), "bar".to_string()];
        let bar_module = graph.get_module(&bar_path).unwrap();
        assert!(bar_module.has_item("bar_fn"));
    }

    #[test]
    fn test_nested_modules() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(
            root,
            "src/lib.rs",
            r#"
mod outer;
"#,
        );

        write_file(
            root,
            "src/outer.rs",
            r#"
mod inner;
pub fn outer_fn() {}
"#,
        );

        write_file(
            root,
            "src/outer/inner.rs",
            r#"
pub fn inner_fn() {}
"#,
        );

        let lib_path = root.join("src/lib.rs");
        let graph = ModuleGraph::build("test_crate", &lib_path).unwrap();

        // Check the nested module
        let inner_path = vec![
            "crate".to_string(),
            "outer".to_string(),
            "inner".to_string(),
        ];
        let inner_module = graph.get_module(&inner_path).unwrap();
        assert!(inner_module.has_item("inner_fn"));
    }

    #[test]
    fn test_visibility_parsing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(
            root,
            "src/lib.rs",
            r#"
pub fn public_fn() {}
pub(crate) fn crate_fn() {}
pub(super) fn super_fn() {}
fn private_fn() {}
"#,
        );

        let lib_path = root.join("src/lib.rs");
        let graph = ModuleGraph::build("test_crate", &lib_path).unwrap();

        let root_module =
            graph.get_module(&vec!["crate".to_string()]).unwrap();

        assert_eq!(
            root_module.get_item("public_fn").unwrap().visibility,
            Visibility::Public
        );
        assert_eq!(
            root_module.get_item("crate_fn").unwrap().visibility,
            Visibility::PubCrate
        );
        assert_eq!(
            root_module.get_item("super_fn").unwrap().visibility,
            Visibility::PubSuper
        );
        assert_eq!(
            root_module.get_item("private_fn").unwrap().visibility,
            Visibility::Private
        );
    }

    #[test]
    fn test_resolve_module_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create files
        write_file(root, "src/lib.rs", "");
        write_file(root, "src/foo.rs", "");
        write_file(root, "src/bar/mod.rs", "");

        let lib_path = root.join("src/lib.rs");

        // From lib.rs, foo should resolve to foo.rs
        let foo_path = resolve_module_file(&lib_path, "foo").unwrap();
        assert!(foo_path.ends_with("foo.rs"));

        // From lib.rs, bar should resolve to bar/mod.rs
        let bar_path = resolve_module_file(&lib_path, "bar").unwrap();
        assert!(bar_path.ends_with("bar/mod.rs"));
    }
}
