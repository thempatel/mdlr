use anyhow::Result;
use mdlr_core::{Span, Unit, UnitKind};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser};

use crate::resolve::{CargoWorkspace, ResolutionContext};

/// Rust source code extractor.
///
/// Extracts units (functions, structs, methods) from Rust source files.
/// Builds a resolution context from the Cargo workspace for accurate
/// cross-crate call resolution.
pub struct RustExtractor {
    /// Resolution context built from Cargo workspace
    resolution_ctx: Option<ResolutionContext>,
}

impl RustExtractor {
    /// Create a new RustExtractor from a list of all project files.
    ///
    /// Filters out `Cargo.toml` files and builds the workspace and resolution
    /// context from them for accurate cross-crate call resolution.
    pub fn new(all_files: &[PathBuf]) -> Result<Self> {
        let cargo_files: Vec<PathBuf> = all_files
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n == "Cargo.toml"))
            .cloned()
            .collect();

        let resolution_ctx = if cargo_files.is_empty() {
            None
        } else {
            CargoWorkspace::from_cargo_files(&cargo_files)
                .ok()
                .map(ResolutionContext::build)
        };

        Ok(Self { resolution_ctx })
    }

    /// Create a new RustExtractor by discovering the workspace from a directory.
    ///
    /// Walks up from the given directory to find the workspace root.
    pub fn discover(start_dir: &Path) -> Result<Self> {
        let resolution_ctx = CargoWorkspace::discover(start_dir)
            .ok()
            .map(ResolutionContext::build);

        Ok(Self { resolution_ctx })
    }

    /// Extract units from all provided paths.
    ///
    /// Loads each file from disk and extracts its units.
    pub fn extract_all(&self, paths: &[PathBuf]) -> Result<Vec<Unit>> {
        let mut all_units = Vec::new();

        for path in paths {
            match self.extract_file(path) {
                Ok(units) => all_units.extend(units),
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to extract {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        Ok(all_units)
    }

    /// Extract units from a single file.
    fn extract_file(&self, path: &Path) -> Result<Vec<Unit>> {
        let source = std::fs::read_to_string(path)?;
        self.extract_source(&source, path)
    }

    /// Extract units from source code.
    fn extract_source(&self, source: &str, path: &Path) -> Result<Vec<Unit>> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse source"))?;

        // Get the crate name and module path for this file if resolution context is available
        let (crate_name, crate_module_path) = self
            .resolution_ctx
            .as_ref()
            .and_then(|ctx| ctx.file_to_module(path))
            .unzip();

        let mut units = Vec::new();
        let mut context = ExtractionContext {
            source,
            path,
            module_path: Vec::new(),
            current_struct: None,
            resolution_ctx: self.resolution_ctx.as_ref(),
            crate_name,
            crate_module_path,
        };

        extract_from_node(tree.root_node(), &mut context, &mut units);

        Ok(units)
    }

    /// Get the resolution context, if available.
    pub fn resolution_context(&self) -> Option<&ResolutionContext> {
        self.resolution_ctx.as_ref()
    }

    /// Create an extractor without resolution context (for testing).
    #[cfg(test)]
    fn new_without_context() -> Self {
        Self { resolution_ctx: None }
    }
}

struct ExtractionContext<'a> {
    source: &'a str,
    path: &'a Path,
    module_path: Vec<String>,
    /// Current struct ID we're inside an impl block for (for methods)
    current_struct: Option<String>,
    /// Resolution context for resolving calls to fully qualified names
    resolution_ctx: Option<&'a ResolutionContext>,
    /// The crate name this file belongs to (if resolution context is available)
    crate_name: Option<String>,
    /// The module path within the crate (if resolution context is available)
    crate_module_path: Option<Vec<String>>,
}

impl<'a> ExtractionContext<'a> {
    /// Generate a qualified name for a unit.
    ///
    /// When resolution context is available, uses crate-based naming:
    ///   "my_crate::module::Foo::method"
    ///
    /// Without resolution context, uses file-based naming:
    ///   "src/foo.rs::module::Foo::method"
    fn qualified_name(&self, name: &str) -> String {
        let mut parts = Vec::new();

        // Add module path if present (from inline mod declarations)
        if !self.module_path.is_empty() {
            parts.push(self.module_path.join("::"));
        }

        // Add parent struct if inside an impl block (for methods)
        if let Some(ref struct_name) = self.current_struct {
            // Extract just the struct name part
            // e.g., "my_crate::foo::Foo" -> "Foo"
            // or "src/foo.rs::Foo" -> "Foo"
            if let Some(idx) = struct_name.rfind("::") {
                let struct_local = &struct_name[idx + 2..];
                parts.push(struct_local.to_string());
            } else {
                parts.push(struct_name.clone());
            }
        }

        parts.push(name.to_string());

        let local_name = parts.join("::");

        // Use crate-based naming if resolution context is available
        if let (Some(crate_name), Some(crate_module)) =
            (&self.crate_name, &self.crate_module_path)
        {
            // Build the full crate path: crate_name::module::local_name
            // Skip "crate" from module path since we use the actual crate name
            let module_parts: Vec<&str> = crate_module
                .iter()
                .filter(|s| *s != "crate")
                .map(|s| s.as_str())
                .collect();

            if module_parts.is_empty() {
                format!("{}::{}", crate_name, local_name)
            } else {
                format!(
                    "{}::{}::{}",
                    crate_name,
                    module_parts.join("::"),
                    local_name
                )
            }
        } else {
            // Fall back to file-based naming
            format!("{}::{}", self.path.display(), local_name)
        }
    }

    /// Generate a qualified struct ID for the parent pointer.
    ///
    /// When resolution context is available, attempts to resolve the struct name
    /// to its canonical definition location. This ensures that methods defined
    /// in separate impl blocks (potentially in different files) all point to
    /// the same parent struct ID.
    fn qualified_struct_id(&self, struct_name: &str) -> String {
        // Try to resolve the struct name to its definition location
        if let (Some(ctx), Some(crate_name), Some(crate_module)) =
            (self.resolution_ctx, &self.crate_name, &self.crate_module_path)
        {
            if let Some(resolved) =
                ctx.resolve(struct_name, crate_name, crate_module)
            {
                // Build the canonical ID from the resolved path
                let mut parts = vec![resolved.crate_name];

                // Add module path, filtering out "crate"
                for segment in &resolved.module_path {
                    if segment != "crate" {
                        parts.push(segment.clone());
                    }
                }

                // Add the item name (struct name)
                if !resolved.item_name.is_empty() {
                    parts.push(resolved.item_name);
                }

                return parts.join("::");
            }
        }

        // Fall back to the original behavior if resolution fails
        let mut parts = Vec::new();

        // Add module path if present (from inline mod declarations)
        if !self.module_path.is_empty() {
            parts.push(self.module_path.join("::"));
        }

        parts.push(struct_name.to_string());

        let local_name = parts.join("::");

        // Use crate-based naming if resolution context is available
        if let (Some(crate_name), Some(crate_module)) =
            (&self.crate_name, &self.crate_module_path)
        {
            let module_parts: Vec<&str> = crate_module
                .iter()
                .filter(|s| *s != "crate")
                .map(|s| s.as_str())
                .collect();

            if module_parts.is_empty() {
                format!("{}::{}", crate_name, local_name)
            } else {
                format!(
                    "{}::{}::{}",
                    crate_name,
                    module_parts.join("::"),
                    local_name
                )
            }
        } else {
            format!("{}::{}", self.path.display(), local_name)
        }
    }

    /// Resolve a call expression to a fully qualified name.
    ///
    /// Returns the resolved crate path if resolution succeeds,
    /// otherwise returns the original call name.
    fn resolve_call(&self, call: &str) -> String {
        if let Some(ctx) = self.resolution_ctx {
            if let Some(resolved) = ctx.resolve_call(call, self.path) {
                return resolved;
            }
        }
        call.to_string()
    }
}

fn extract_from_node(
    node: Node,
    ctx: &mut ExtractionContext,
    units: &mut Vec<Unit>,
) {
    match node.kind() {
        "function_item" => {
            if let Some(unit) = extract_function(node, ctx) {
                units.push(unit);
            }
        }
        "struct_item" => {
            if let Some(unit) = extract_struct(node, ctx) {
                units.push(unit);
            }
        }
        "impl_item" => {
            // Extract methods inside impl blocks
            // Don't emit a unit for the impl block itself
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = node_text(type_node, ctx.source);
                let struct_id = ctx.qualified_struct_id(&type_name);

                let old_struct = ctx.current_struct.take();
                ctx.current_struct = Some(struct_id);

                for child in node.children(&mut node.walk()) {
                    extract_from_node(child, ctx, units);
                }

                ctx.current_struct = old_struct;
                return;
            }
        }
        "mod_item" => {
            if let Some(name) = get_node_name(node, ctx.source) {
                ctx.module_path.push(name);
                for child in node.children(&mut node.walk()) {
                    extract_from_node(child, ctx, units);
                }
                ctx.module_path.pop();
                return;
            }
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        extract_from_node(child, ctx, units);
    }
}

fn extract_function(node: Node, ctx: &ExtractionContext) -> Option<Unit> {
    let name = get_node_name(node, ctx.source)?;
    let raw_calls = extract_calls(node, ctx.source);
    let params = count_parameters(node);
    let branches = count_branches(node);
    let (reads, writes) = extract_field_access(node, ctx.source);

    // Resolve calls to fully qualified names
    let calls: Vec<String> =
        raw_calls.into_iter().map(|call| ctx.resolve_call(&call)).collect();

    // Determine if this is a method (inside an impl block) or a standalone function
    let (kind, parent) = if ctx.current_struct.is_some() {
        (UnitKind::Method, ctx.current_struct.clone())
    } else {
        (UnitKind::Function, None)
    };

    Some(Unit {
        id: ctx.qualified_name(&name),
        kind,
        file: ctx.path.to_path_buf(),
        span: node_span(node),
        reads,
        writes,
        calls,
        tags: Vec::new(),
        params,
        branches,
        parent,
    })
}

fn extract_struct(node: Node, ctx: &ExtractionContext) -> Option<Unit> {
    let name = get_node_name(node, ctx.source)?;

    Some(Unit {
        id: ctx.qualified_name(&name),
        kind: UnitKind::Struct,
        file: ctx.path.to_path_buf(),
        span: node_span(node),
        reads: Vec::new(),
        writes: Vec::new(),
        calls: Vec::new(),
        tags: Vec::new(),
        params: 0,
        branches: 0,
        parent: None,
    })
}

fn extract_calls(node: Node, source: &str) -> Vec<String> {
    let mut calls = Vec::new();
    collect_calls(node, source, &mut calls);
    calls.sort();
    calls.dedup();
    calls
}

fn collect_calls(node: Node, source: &str, calls: &mut Vec<String>) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            let call_name = extract_callable_name(func, source);
            if !call_name.is_empty() {
                calls.push(call_name);
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        collect_calls(child, source, calls);
    }
}

/// Extract just the function/method name from a call's function node.
/// Handles:
/// - Simple calls: `foo()` -> "foo"
/// - Path calls: `foo::bar()` -> "foo::bar"
/// - Method calls: `obj.method()` -> "obj.method"
/// - Chained calls: `foo().bar()` -> "bar" (the method being called)
fn extract_callable_name(node: Node, source: &str) -> String {
    match node.kind() {
        "identifier" | "scoped_identifier" => node_text(node, source),
        "field_expression" => {
            // obj.method - extract object and field
            let field = node
                .child_by_field_name("field")
                .map(|n| node_text(n, source))
                .unwrap_or_default();

            if let Some(value) = node.child_by_field_name("value") {
                // If the value is a call_expression, just return the field name
                // e.g., foo().bar() -> "bar"
                if value.kind() == "call_expression" {
                    return field;
                }
                // Otherwise build "value.field"
                let value_name = extract_callable_name(value, source);
                if value_name.is_empty() {
                    field
                } else {
                    format!("{}.{}", value_name, field)
                }
            } else {
                field
            }
        }
        _ => String::new(),
    }
}

/// Count the number of parameters in a function
fn count_parameters(node: Node) -> usize {
    let Some(params_node) = node.child_by_field_name("parameters") else {
        return 0;
    };

    let mut count = 0;
    for child in params_node.children(&mut params_node.walk()) {
        // Count parameter nodes (excluding self parameters for methods)
        if child.kind() == "parameter" {
            count += 1;
        } else if child.kind() == "self_parameter" {
            // Don't count self/&self/&mut self as a parameter
        }
    }
    count
}

/// Count branch points for cyclomatic complexity
/// Counts: if, else if, match arms, while, for, loop, && and ||
fn count_branches(node: Node) -> usize {
    let mut count = 0;
    count_branches_recursive(node, &mut count);
    count
}

fn count_branches_recursive(node: Node, count: &mut usize) {
    match node.kind() {
        "if_expression" => {
            // Count the if itself
            *count += 1;
        }
        "match_expression" => {
            // Count each match arm (minus 1 since one path is the default)
            let mut arm_count = 0;
            for child in node.children(&mut node.walk()) {
                if child.kind() == "match_block" {
                    for arm in child.children(&mut child.walk()) {
                        if arm.kind() == "match_arm" {
                            arm_count += 1;
                        }
                    }
                }
            }
            // Each arm beyond the first adds a branch
            if arm_count > 0 {
                *count += arm_count - 1;
            }
        }
        "while_expression" | "for_expression" | "loop_expression" => {
            *count += 1;
        }
        "binary_expression" => {
            // Check for && or || operators
            for child in node.children(&mut node.walk()) {
                if child.kind() == "&&" || child.kind() == "||" {
                    *count += 1;
                }
            }
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        count_branches_recursive(child, count);
    }
}

/// Extract field reads and writes from a function body
/// Returns (reads, writes) where each is a list of field names
fn extract_field_access(
    node: Node,
    source: &str,
) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    collect_field_access(node, source, &mut reads, &mut writes, false);
    reads.sort();
    reads.dedup();
    writes.sort();
    writes.dedup();
    (reads, writes)
}

/// Walk down the "function" part of a call expression to find self.field accesses.
/// Handles arbitrary chains like self.field.method1().method2().
fn process_call_function(
    node: Node,
    source: &str,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
    in_assignment_lhs: bool,
) {
    match node.kind() {
        "field_expression" => {
            // This is obj.method - check what obj is
            if let Some(value) = node.child_by_field_name("value") {
                process_call_value(
                    value,
                    source,
                    reads,
                    writes,
                    in_assignment_lhs,
                );
            }
        }
        "generic_function" => {
            // This is obj.method::<T>() with turbofish syntax
            // The function field contains the actual field_expression
            if let Some(func) = node.child_by_field_name("function") {
                process_call_function(
                    func,
                    source,
                    reads,
                    writes,
                    in_assignment_lhs,
                );
            }
        }
        "scoped_identifier" | "identifier" => {
            // Direct function call like foo() - no field access
        }
        _ => {}
    }
}

/// Process the value part of a field_expression in a call chain.
fn process_call_value(
    value: Node,
    source: &str,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
    in_assignment_lhs: bool,
) {
    match value.kind() {
        "field_expression" => {
            // obj is itself a field access (e.g., self.field)
            // Process it to capture the field read
            collect_field_access(
                value,
                source,
                reads,
                writes,
                in_assignment_lhs,
            );
        }
        "call_expression" => {
            // obj is a method call (e.g., self.method1())
            // Recurse to find any field access in the call chain
            if let Some(inner_func) = value.child_by_field_name("function") {
                process_call_function(
                    inner_func,
                    source,
                    reads,
                    writes,
                    in_assignment_lhs,
                );
            }
            // Also process arguments of the inner call
            if let Some(args) = value.child_by_field_name("arguments") {
                collect_field_access(
                    args,
                    source,
                    reads,
                    writes,
                    in_assignment_lhs,
                );
            }
        }
        _ => {
            // obj is something else (just "self", a variable, etc.)
            // No field access to capture
        }
    }
}

fn collect_field_access(
    node: Node,
    source: &str,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
    in_assignment_lhs: bool,
) {
    match node.kind() {
        "field_expression" => {
            // Check if this is self.field access
            if let Some(value) = node.child_by_field_name("value") {
                let value_text = node_text(value, source);
                if value_text == "self"
                    || value_text == "&self"
                    || value_text == "&mut self"
                {
                    if let Some(field) = node.child_by_field_name("field") {
                        let field_name = node_text(field, source);
                        if in_assignment_lhs {
                            writes.push(field_name);
                        } else {
                            reads.push(field_name);
                        }
                    }
                }
            }
        }
        "call_expression" => {
            // Handle method calls carefully to distinguish:
            // - self.method() -> NOT a field read (method call)
            // - self.field.method() -> field IS a read (field access chained with method)
            // - self.field.method1().method2() -> still a field read
            if let Some(func) = node.child_by_field_name("function") {
                // Walk down the call chain to find field accesses
                process_call_function(
                    func,
                    source,
                    reads,
                    writes,
                    in_assignment_lhs,
                );
            }
            // Always process arguments
            if let Some(args) = node.child_by_field_name("arguments") {
                collect_field_access(
                    args,
                    source,
                    reads,
                    writes,
                    in_assignment_lhs,
                );
            }
            return; // Don't recurse normally, we handled the relevant children
        }
        "assignment_expression" | "compound_assignment_expr" => {
            // Left side is a write, right side is a read
            if let Some(left) = node.child_by_field_name("left") {
                collect_field_access(left, source, reads, writes, true);
            }
            if let Some(right) = node.child_by_field_name("right") {
                collect_field_access(right, source, reads, writes, false);
            }
            return; // Don't recurse normally, we handled children
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        collect_field_access(child, source, reads, writes, in_assignment_lhs);
    }
}

fn get_node_name(node: Node, source: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    Some(node_text(name_node, source))
}

fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn node_span(node: Node) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span {
        start_line: start.row + 1,
        start_col: start.column,
        end_line: end.row + 1,
        end_col: end.column,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
fn hello() {
    println!("Hello, world!");
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        assert_eq!(units.len(), 2);
        assert_eq!(units[0].id, "test.rs::hello");
        assert_eq!(units[0].kind, UnitKind::Function);
        assert_eq!(units[1].id, "test.rs::add");
    }

    #[test]
    fn test_extract_struct() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
struct Point {
    x: i32,
    y: i32,
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, "test.rs::Point");
        assert_eq!(units[0].kind, UnitKind::Struct);
    }

    #[test]
    fn test_extract_impl_methods() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
struct Foo {
    x: i32,
}

impl Foo {
    fn new() -> Self {
        Self { x: 0 }
    }

    fn get_x(&self) -> i32 {
        self.x
    }

    fn set_x(&mut self, val: i32) {
        self.x = val;
    }
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        // Should have: struct Foo, new, get_x, set_x (no impl block unit)
        assert_eq!(units.len(), 4);

        let struct_unit =
            units.iter().find(|u| u.id == "test.rs::Foo").unwrap();
        assert_eq!(struct_unit.kind, UnitKind::Struct);

        let new_fn =
            units.iter().find(|u| u.id == "test.rs::Foo::new").unwrap();
        assert_eq!(new_fn.kind, UnitKind::Method);
        assert_eq!(new_fn.parent, Some("test.rs::Foo".to_string()));

        let get_x =
            units.iter().find(|u| u.id == "test.rs::Foo::get_x").unwrap();
        assert_eq!(get_x.kind, UnitKind::Method);
        assert_eq!(get_x.parent, Some("test.rs::Foo".to_string()));
    }

    #[test]
    fn test_extract_trait_impl_methods() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
struct Bar;

impl Display for Bar {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "Bar")
    }
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        // Should have: struct Bar, fmt method (no impl block unit)
        assert_eq!(units.len(), 2);

        let fmt_fn =
            units.iter().find(|u| u.id == "test.rs::Bar::fmt").unwrap();
        assert_eq!(fmt_fn.kind, UnitKind::Method);
        // For trait impls, parent is the type being implemented (Bar), not the trait
        assert_eq!(fmt_fn.parent, Some("test.rs::Bar".to_string()));
    }

    #[test]
    fn test_extract_field_access() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
impl Foo {
    fn reader(&self) -> i32 {
        self.x + self.y
    }

    fn writer(&mut self) {
        self.x = 10;
        self.y = self.z;
    }
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        let reader =
            units.iter().find(|u| u.id == "test.rs::Foo::reader").unwrap();
        assert!(reader.reads.contains(&"x".to_string()));
        assert!(reader.reads.contains(&"y".to_string()));
        assert!(reader.writes.is_empty());

        let writer =
            units.iter().find(|u| u.id == "test.rs::Foo::writer").unwrap();
        assert!(writer.writes.contains(&"x".to_string()));
        assert!(writer.writes.contains(&"y".to_string()));
        assert!(writer.reads.contains(&"z".to_string()));
    }

    #[test]
    fn test_extract_module() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
mod inner {
    fn nested() {}
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, "test.rs::inner::nested");
    }

    #[test]
    fn test_extract_params() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
fn no_params() {}
fn one_param(a: i32) {}
fn two_params(a: i32, b: String) {}
fn with_self(&self, x: i32) {}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        assert_eq!(units.len(), 4);
        assert_eq!(units[0].params, 0);
        assert_eq!(units[1].params, 1);
        assert_eq!(units[2].params, 2);
        assert_eq!(units[3].params, 1); // self doesn't count
    }

    #[test]
    fn test_extract_branches() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
fn simple() {
    let x = 1;
}

fn with_if(x: i32) {
    if x > 0 {
        println!("positive");
    }
}

fn with_match(x: Option<i32>) {
    match x {
        Some(v) => println!("{}", v),
        None => println!("none"),
    }
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        assert_eq!(units.len(), 3);
        assert_eq!(units[0].branches, 0, "simple should have 0 branches");
        assert_eq!(units[1].branches, 1, "with_if should have 1 branch");
        assert_eq!(
            units[2].branches, 1,
            "with_match with 2 arms should have 1 branch"
        );
    }

    #[test]
    fn test_method_calls_not_counted_as_field_reads() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
impl Foo {
    fn caller(&self) {
        // Method calls should NOT be counted as field reads
        self.do_something();
        self.other_method(self.field);
        let _ = self.chain().another();
    }

    fn do_something(&self) {}
    fn other_method(&self, x: i32) {}
    fn chain(&self) -> &Self { self }
    fn another(&self) {}
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        let caller =
            units.iter().find(|u| u.id == "test.rs::Foo::caller").unwrap();

        // Only self.field should be counted as a read, not the method names
        assert_eq!(caller.reads.len(), 1, "should only have 1 field read");
        assert!(
            caller.reads.contains(&"field".to_string()),
            "should contain 'field'"
        );
        assert!(
            !caller.reads.contains(&"do_something".to_string()),
            "method name should not be a read"
        );
        assert!(
            !caller.reads.contains(&"other_method".to_string()),
            "method name should not be a read"
        );
        assert!(
            !caller.reads.contains(&"chain".to_string()),
            "method name should not be a read"
        );
    }

    #[test]
    fn test_chained_field_method_call_is_counted() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
impl Foo {
    fn reader(&self) {
        // self.field.method() should count "field" as a read
        self.ctx.as_ref();
        self.data.clone();
        let _ = self.inner.get_value();
    }
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        let reader =
            units.iter().find(|u| u.id == "test.rs::Foo::reader").unwrap();

        // Fields accessed via chained method calls should be counted
        assert!(
            reader.reads.contains(&"ctx".to_string()),
            "should contain 'ctx' from self.ctx.as_ref()"
        );
        assert!(
            reader.reads.contains(&"data".to_string()),
            "should contain 'data' from self.data.clone()"
        );
        assert!(
            reader.reads.contains(&"inner".to_string()),
            "should contain 'inner' from self.inner.get_value()"
        );
        // But NOT the method names
        assert!(
            !reader.reads.contains(&"as_ref".to_string()),
            "method name should not be a read"
        );
        assert!(
            !reader.reads.contains(&"clone".to_string()),
            "method name should not be a read"
        );
        assert!(
            !reader.reads.contains(&"get_value".to_string()),
            "method name should not be a read"
        );
    }

    #[test]
    fn test_multi_chained_method_calls() {
        let extractor = RustExtractor::new_without_context();
        let source = r#"
impl Foo {
    fn reader(&self) {
        // Multiple chained method calls like self.members.iter().find(...)
        self.members.iter().find(|x| x.name == "test");
        self.items.iter().map(|x| x.clone()).collect::<Vec<_>>();
    }
}
"#;
        let units =
            extractor.extract_source(source, Path::new("test.rs")).unwrap();

        let reader =
            units.iter().find(|u| u.id == "test.rs::Foo::reader").unwrap();

        // Fields accessed through chained method calls should be counted
        assert!(
            reader.reads.contains(&"members".to_string()),
            "should contain 'members' from self.members.iter().find(...), got {:?}",
            reader.reads
        );
        assert!(
            reader.reads.contains(&"items".to_string()),
            "should contain 'items' from self.items.iter().map(...).collect(...), got {:?}",
            reader.reads
        );
        // But NOT the method names
        assert!(
            !reader.reads.contains(&"iter".to_string()),
            "method name should not be a read"
        );
        assert!(
            !reader.reads.contains(&"find".to_string()),
            "method name should not be a read"
        );
        assert!(
            !reader.reads.contains(&"map".to_string()),
            "method name should not be a read"
        );
        assert!(
            !reader.reads.contains(&"collect".to_string()),
            "method name should not be a read"
        );
    }
}
