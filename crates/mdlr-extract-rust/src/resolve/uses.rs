//! Use statement extraction and normalization.
//!
//! Parses Rust `use` declarations and normalizes them into a flat list of imports.

use tree_sitter::Node;

/// A normalized use statement representing a single import.
#[derive(Debug, Clone)]
pub struct UseStatement {
    /// The path segments (e.g., `["std", "collections", "HashMap"]`).
    pub segments: Vec<String>,
    /// The kind of import.
    pub kind: UseKind,
    /// Optional alias (from `as` clause).
    pub alias: Option<String>,
    /// Visibility of this use statement.
    pub visibility: Visibility,
}

/// The kind of use import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseKind {
    /// A single import: `use foo::Bar;`
    Single,
    /// A glob import: `use foo::*;`
    Glob,
    /// Re-export of self: `use foo::bar::{self};`
    SelfImport,
}

/// Visibility of an item or use statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    /// Private (default, no modifier).
    Private,
    /// `pub`
    Public,
    /// `pub(crate)`
    PubCrate,
    /// `pub(super)`
    PubSuper,
    /// `pub(in path)`
    PubIn(String),
}

impl UseStatement {
    /// Get the imported name (either the alias or the last segment).
    pub fn imported_name(&self) -> Option<&str> {
        if let Some(ref alias) = self.alias {
            Some(alias)
        } else {
            self.segments.last().map(|s| s.as_str())
        }
    }

    /// Get the full path as a string with `::` separators.
    pub fn path_string(&self) -> String {
        self.segments.join("::")
    }

    /// Check if this import starts with a specific prefix.
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.segments.first().is_some_and(|s| s == prefix)
    }

    /// Check if this is a crate-relative import.
    pub fn is_crate_relative(&self) -> bool {
        self.starts_with("crate")
    }

    /// Check if this is a self-relative import.
    pub fn is_self_relative(&self) -> bool {
        self.starts_with("self")
    }

    /// Check if this is a super-relative import.
    pub fn is_super_relative(&self) -> bool {
        self.starts_with("super")
    }

    /// Check if this is an external crate import.
    pub fn is_external(&self) -> bool {
        !self.is_crate_relative()
            && !self.is_self_relative()
            && !self.is_super_relative()
    }
}

/// Extract use statements from a use_declaration node.
///
/// A single `use` declaration can contain multiple imports (via nested braces),
/// so this returns a Vec of normalized UseStatement.
pub fn extract_use_statement(node: Node, source: &str) -> Vec<UseStatement> {
    let mut uses = Vec::new();
    extract_use_tree(node, source, &[], &mut uses);
    uses
}

/// Recursively extract use statements from a use tree.
pub fn extract_use_tree(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    match node.kind() {
        "use_declaration" => {
            extract_use_declaration_children(node, source, prefix, uses)
        }
        "scoped_identifier" | "identifier" => {
            extract_simple_path(node, source, prefix, uses)
        }
        "use_as_clause" => extract_use_as_clause(node, source, prefix, uses),
        "use_wildcard" => extract_use_wildcard(node, source, prefix, uses),
        "scoped_use_list" => {
            extract_scoped_use_list(node, source, prefix, uses)
        }
        "use_list" => extract_use_list(node, source, prefix, uses),
        _ => {
            // Recurse into unknown nodes
            for child in node.children(&mut node.walk()) {
                extract_use_tree(child, source, prefix, uses);
            }
        }
    }
}

/// Extract children from a use_declaration node.
fn extract_use_declaration_children(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    for child in node.children(&mut node.walk()) {
        if child.kind() != "visibility_modifier" && child.kind() != "use" {
            extract_use_tree(child, source, prefix, uses);
        }
    }
}

/// Extract a simple path like `foo::bar` or just `foo`.
fn extract_simple_path(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    let path = parse_path(node, source);
    let full_path: Vec<String> = prefix.iter().cloned().chain(path).collect();

    uses.push(UseStatement {
        segments: full_path,
        kind: UseKind::Single,
        alias: None,
        visibility: Visibility::Private,
    });
}

/// Extract a use_as_clause like `foo::Bar as Baz`.
fn extract_use_as_clause(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    if let Some(path_node) = node.child_by_field_name("path") {
        let path = parse_path(path_node, source);
        let full_path: Vec<String> =
            prefix.iter().cloned().chain(path).collect();

        let alias =
            node.child_by_field_name("alias").map(|n| node_text(n, source));

        uses.push(UseStatement {
            segments: full_path,
            kind: UseKind::Single,
            alias,
            visibility: Visibility::Private,
        });
    }
}

/// Extract a use_wildcard like `foo::*`.
fn extract_use_wildcard(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    let path = extract_wildcard_path(node, source);
    let full_path: Vec<String> = prefix.iter().cloned().chain(path).collect();

    uses.push(UseStatement {
        segments: full_path,
        kind: UseKind::Glob,
        alias: None,
        visibility: Visibility::Private,
    });
}

/// Extract the path from a use_wildcard node.
fn extract_wildcard_path(node: Node, source: &str) -> Vec<String> {
    if let Some(path_node) = node.child_by_field_name("path") {
        return parse_path(path_node, source);
    }
    // No path field, look for scoped_identifier/identifier child
    for child in node.children(&mut node.walk()) {
        if child.kind() == "scoped_identifier" || child.kind() == "identifier"
        {
            return parse_path(child, source);
        }
    }
    vec![]
}

/// Extract a scoped_use_list like `foo::{bar, baz}`.
fn extract_scoped_use_list(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    if let Some(path_node) = node.child_by_field_name("path") {
        let path = parse_path(path_node, source);
        let new_prefix: Vec<String> =
            prefix.iter().cloned().chain(path).collect();

        if let Some(list) = node.child_by_field_name("list") {
            extract_use_tree(list, source, &new_prefix, uses);
        }
    }
}

/// Extract a use_list like `{bar, baz}` inside a scoped_use_list.
fn extract_use_list(
    node: Node,
    source: &str,
    prefix: &[String],
    uses: &mut Vec<UseStatement>,
) {
    for child in node.children(&mut node.walk()) {
        if is_use_list_item(child.kind()) {
            extract_use_tree(child, source, prefix, uses);
        } else if child.kind() == "self" {
            // `use foo::{self}` imports the module itself
            uses.push(UseStatement {
                segments: prefix.to_vec(),
                kind: UseKind::SelfImport,
                alias: None,
                visibility: Visibility::Private,
            });
        }
    }
}

/// Check if a node kind is a valid use_list item.
fn is_use_list_item(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "scoped_identifier"
            | "use_as_clause"
            | "scoped_use_list"
            | "use_wildcard"
    )
}

/// Parse a path from an identifier or scoped_identifier node.
fn parse_path(node: Node, source: &str) -> Vec<String> {
    match node.kind() {
        "identifier" | "crate" | "self" | "super" => {
            vec![node_text(node, source)]
        }
        "scoped_identifier" => {
            let mut segments = Vec::new();

            // Get the path prefix
            if let Some(path) = node.child_by_field_name("path") {
                segments.extend(parse_path(path, source));
            }

            // Get the name
            if let Some(name) = node.child_by_field_name("name") {
                segments.push(node_text(name, source));
            }

            segments
        }
        "metavariable" => {
            // Macro metavariables like $crate
            vec![node_text(node, source)]
        }
        _ => vec![],
    }
}

fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Resolve a path prefix to an absolute module path.
///
/// Given a current module path and a use path, returns the absolute path.
/// - `crate::foo` -> `["crate", "foo"]`
/// - `self::foo` from `["crate", "bar"]` -> `["crate", "bar", "foo"]`
/// - `super::foo` from `["crate", "bar", "baz"]` -> `["crate", "bar", "foo"]`
pub fn resolve_path_prefix(
    use_path: &[String],
    current_module: &[String],
) -> Option<Vec<String>> {
    if use_path.is_empty() {
        return None;
    }

    let first = &use_path[0];

    match first.as_str() {
        "crate" => {
            // Already absolute
            Some(use_path.to_vec())
        }
        "self" => {
            // Relative to current module
            let mut result = current_module.to_vec();
            result.extend(use_path[1..].iter().cloned());
            Some(result)
        }
        "super" => {
            // Go up one level
            if current_module.len() <= 1 {
                // Can't go above crate root
                return None;
            }

            let mut result: Vec<String> =
                current_module[..current_module.len() - 1].to_vec();

            // Handle multiple super:: prefixes
            let mut i = 1;
            while i < use_path.len() && use_path[i] == "super" {
                if result.len() <= 1 {
                    return None;
                }
                result.pop();
                i += 1;
            }

            result.extend(use_path[i..].iter().cloned());
            Some(result)
        }
        _ => {
            // External crate or prelude item
            // Return as-is, let the caller handle external resolution
            Some(use_path.to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_use(code: &str) -> Vec<UseStatement> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();

        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();

        let mut uses = Vec::new();
        for child in root.children(&mut root.walk()) {
            if child.kind() == "use_declaration" {
                uses.extend(extract_use_statement(child, code));
            }
        }
        uses
    }

    #[test]
    fn test_simple_use() {
        let uses = parse_use("use std::collections::HashMap;");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].segments, vec!["std", "collections", "HashMap"]);
        assert_eq!(uses[0].kind, UseKind::Single);
        assert_eq!(uses[0].alias, None);
    }

    #[test]
    fn test_use_with_alias() {
        let uses = parse_use("use std::collections::HashMap as Map;");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].segments, vec!["std", "collections", "HashMap"]);
        assert_eq!(uses[0].alias, Some("Map".to_string()));
    }

    #[test]
    fn test_use_glob() {
        let uses = parse_use("use std::collections::*;");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].segments, vec!["std", "collections"]);
        assert_eq!(uses[0].kind, UseKind::Glob);
    }

    #[test]
    fn test_use_nested() {
        let uses = parse_use("use std::collections::{HashMap, HashSet};");
        assert_eq!(uses.len(), 2);

        let paths: Vec<_> = uses.iter().map(|u| u.path_string()).collect();
        assert!(paths.contains(&"std::collections::HashMap".to_string()));
        assert!(paths.contains(&"std::collections::HashSet".to_string()));
    }

    #[test]
    fn test_use_deeply_nested() {
        let uses =
            parse_use("use std::{collections::{HashMap, HashSet}, io::Read};");
        assert_eq!(uses.len(), 3);

        let paths: Vec<_> = uses.iter().map(|u| u.path_string()).collect();
        assert!(paths.contains(&"std::collections::HashMap".to_string()));
        assert!(paths.contains(&"std::collections::HashSet".to_string()));
        assert!(paths.contains(&"std::io::Read".to_string()));
    }

    #[test]
    fn test_use_self() {
        let uses = parse_use("use std::collections::{self, HashMap};");
        assert_eq!(uses.len(), 2);

        let self_import = uses.iter().find(|u| u.kind == UseKind::SelfImport);
        assert!(self_import.is_some());
        assert_eq!(self_import.unwrap().segments, vec!["std", "collections"]);

        let hashmap = uses
            .iter()
            .find(|u| u.path_string() == "std::collections::HashMap");
        assert!(hashmap.is_some());
    }

    #[test]
    fn test_crate_relative() {
        let uses = parse_use("use crate::module::Thing;");
        assert_eq!(uses.len(), 1);
        assert!(uses[0].is_crate_relative());
        assert_eq!(uses[0].segments, vec!["crate", "module", "Thing"]);
    }

    #[test]
    fn test_self_relative() {
        let uses = parse_use("use self::submodule::Item;");
        assert_eq!(uses.len(), 1);
        assert!(uses[0].is_self_relative());
    }

    #[test]
    fn test_super_relative() {
        let uses = parse_use("use super::sibling::Item;");
        assert_eq!(uses.len(), 1);
        assert!(uses[0].is_super_relative());
    }

    #[test]
    fn test_resolve_crate_path() {
        let use_path =
            vec!["crate".to_string(), "foo".to_string(), "Bar".to_string()];
        let current = vec!["crate".to_string(), "baz".to_string()];

        let resolved = resolve_path_prefix(&use_path, &current).unwrap();
        assert_eq!(resolved, vec!["crate", "foo", "Bar"]);
    }

    #[test]
    fn test_resolve_self_path() {
        let use_path = vec!["self".to_string(), "submod".to_string()];
        let current = vec!["crate".to_string(), "foo".to_string()];

        let resolved = resolve_path_prefix(&use_path, &current).unwrap();
        assert_eq!(resolved, vec!["crate", "foo", "submod"]);
    }

    #[test]
    fn test_resolve_super_path() {
        let use_path = vec!["super".to_string(), "sibling".to_string()];
        let current =
            vec!["crate".to_string(), "foo".to_string(), "bar".to_string()];

        let resolved = resolve_path_prefix(&use_path, &current).unwrap();
        assert_eq!(resolved, vec!["crate", "foo", "sibling"]);
    }

    #[test]
    fn test_resolve_multiple_super() {
        let use_path = vec![
            "super".to_string(),
            "super".to_string(),
            "other".to_string(),
        ];
        let current = vec![
            "crate".to_string(),
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ];

        let resolved = resolve_path_prefix(&use_path, &current).unwrap();
        assert_eq!(resolved, vec!["crate", "a", "other"]);
    }

    #[test]
    fn test_imported_name() {
        let use1 = UseStatement {
            segments: vec!["std".to_string(), "HashMap".to_string()],
            kind: UseKind::Single,
            alias: None,
            visibility: Visibility::Private,
        };
        assert_eq!(use1.imported_name(), Some("HashMap"));

        let use2 = UseStatement {
            segments: vec!["std".to_string(), "HashMap".to_string()],
            kind: UseKind::Single,
            alias: Some("Map".to_string()),
            visibility: Visibility::Private,
        };
        assert_eq!(use2.imported_name(), Some("Map"));
    }
}
