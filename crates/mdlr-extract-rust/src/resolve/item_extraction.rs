//! Item extraction helpers for module graph construction.
//!
//! This module contains helper functions for extracting item definitions
//! from tree-sitter nodes during module graph construction.

use tree_sitter::Node;

use super::modules::{ItemDef, ItemKind, ItemSpan, ModuleNode};
use super::uses::Visibility;

/// Map tree-sitter node kind to ItemKind for simple items.
pub fn item_kind_from_node(node_kind: &str) -> Option<ItemKind> {
    match node_kind {
        "function_item" => Some(ItemKind::Function),
        "struct_item" => Some(ItemKind::Struct),
        "enum_item" => Some(ItemKind::Enum),
        "trait_item" => Some(ItemKind::Trait),
        "const_item" => Some(ItemKind::Const),
        "static_item" => Some(ItemKind::Static),
        "type_item" => Some(ItemKind::TypeAlias),
        "macro_definition" => Some(ItemKind::Macro),
        _ => None,
    }
}

/// Extract an impl_item into the module.
pub fn extract_impl_item(child: Node, source: &str, module: &mut ModuleNode) {
    if let Some(type_node) = child.child_by_field_name("type") {
        let type_name = node_text(type_node, source);
        module.items.push(ItemDef {
            name: format!("impl {}", type_name),
            kind: ItemKind::Impl,
            visibility: Visibility::Private, // impls are always effectively pub
            span: node_span(child),
        });
    }
}

/// Extract a use_declaration into the module.
pub fn extract_use_declaration(
    child: Node,
    source: &str,
    module: &mut ModuleNode,
) {
    let visibility = extract_visibility(child, source);
    let uses = super::uses::extract_use_statement(child, source);

    for use_stmt in uses {
        if visibility == Visibility::Public {
            module.reexports.push(super::uses::UseStatement {
                visibility: visibility.clone(),
                ..use_stmt
            });
        } else {
            module.uses.push(use_stmt);
        }
    }
}

/// Extract an item definition from a tree-sitter node.
pub fn extract_item_def(
    node: Node,
    source: &str,
    kind: ItemKind,
) -> Option<ItemDef> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    let visibility = extract_visibility(node, source);

    Some(ItemDef { name, kind, visibility, span: node_span(node) })
}

/// Extract visibility from a node (looks for visibility_modifier child).
pub fn extract_visibility(node: Node, source: &str) -> Visibility {
    for child in node.children(&mut node.walk()) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(child, source);
            return match text.as_str() {
                "pub" => Visibility::Public,
                s if s.starts_with("pub(crate)") => Visibility::PubCrate,
                s if s.starts_with("pub(super)") => Visibility::PubSuper,
                s if s.starts_with("pub(in") => Visibility::PubIn(
                    s.trim_start_matches("pub(in ")
                        .trim_end_matches(')')
                        .to_string(),
                ),
                _ => Visibility::Public,
            };
        }
    }
    Visibility::Private
}

/// Get the text content of a tree-sitter node.
pub fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Get the source span of a tree-sitter node.
pub fn node_span(node: Node) -> ItemSpan {
    ItemSpan {
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
    }
}
