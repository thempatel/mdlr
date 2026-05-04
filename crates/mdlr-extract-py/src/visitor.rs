use mdlr_core::{Span, Unit, UnitKind};
use ruff_python_ast::{self as ast, Stmt};
use ruff_text_size::Ranged;
use std::path::PathBuf;

use crate::branches;
use crate::calls;
use crate::cognitive;
use crate::field_access;
use crate::scopes;

/// Extract all units from a parsed Python module.
pub fn extract_units(
    body: &[Stmt],
    source: &str,
    rel_path: &str,
) -> Vec<Unit> {
    let line_index = LineIndex::new(source);
    let module_path = file_to_module_path(rel_path);
    let mut extractor = UnitExtractor {
        module_path,
        rel_path: rel_path.to_string(),
        line_index: &line_index,
        source,
        units: Vec::new(),
        scope_stack: Vec::new(),
        class_depth: 0,
    };
    extractor.visit_body(body);
    extractor.units
}

/// Simple line index: maps byte offset → (line, col), both 1-indexed.
pub(crate) struct LineIndex {
    /// Byte offsets where each line starts (line_starts[0] = 0).
    line_starts: Vec<usize>,
}

impl LineIndex {
    #[cfg(test)]
    pub(crate) fn new_from_source(source: &str) -> Self {
        Self::new(source)
    }

    fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex { line_starts }
    }

    /// Convert byte offset to (line, col), both 1-indexed.
    pub(crate) fn offset_to_line_col(&self, offset: usize) -> (usize, usize) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(after) => after - 1,
        };
        let col = offset - self.line_starts[line];
        (line + 1, col)
    }
}

/// Convert a relative file path to a Python module path.
/// `src/mymod/core.py` → `src.mymod.core`
fn file_to_module_path(rel_path: &str) -> String {
    let p = rel_path
        .strip_suffix(".pyi")
        .or_else(|| rel_path.strip_suffix(".py"))
        .unwrap_or(rel_path);
    // Replace slashes with dots
    let module = p.replace('/', ".");
    // Handle __init__ → parent module name
    module.strip_suffix(".__init__").unwrap_or(&module).to_string()
}

struct UnitExtractor<'a> {
    module_path: String,
    rel_path: String,
    line_index: &'a LineIndex,
    source: &'a str,
    units: Vec<Unit>,
    /// Stack of enclosing named scopes for building compound IDs.
    scope_stack: Vec<String>,
    /// Tracks how deep we are inside class definitions.
    class_depth: usize,
}

impl<'a> UnitExtractor<'a> {
    /// Build an ID like `src.mymod.core::Foo.bar`.
    fn make_id(&self, name: &str) -> String {
        if self.scope_stack.is_empty() {
            format!("{}::{}", self.module_path, name)
        } else {
            format!(
                "{}::{}.{}",
                self.module_path,
                self.scope_stack.join("."),
                name
            )
        }
    }

    /// Build a parent ID from the current scope stack.
    fn parent_id(&self) -> Option<String> {
        if self.scope_stack.is_empty() {
            return None;
        }
        Some(format!("{}::{}", self.module_path, self.scope_stack.join(".")))
    }

    fn make_span(&self, range: ruff_text_size::TextRange) -> Span {
        let (start_line, start_col) =
            self.line_index.offset_to_line_col(range.start().into());
        let (end_line, end_col) =
            self.line_index.offset_to_line_col(range.end().into());
        Span { start_line, start_col, end_line, end_col }
    }

    /// Count function parameters, excluding `self` and `cls`.
    fn count_params(params: &ast::Parameters) -> usize {
        let all_params = params
            .posonlyargs
            .iter()
            .chain(params.args.iter())
            .chain(params.kwonlyargs.iter());

        let count = all_params
            .filter(|p| {
                let name = p.parameter.name.as_str();
                name != "self" && name != "cls"
            })
            .count();

        // Count *args and **kwargs
        let extra =
            params.vararg.is_some() as usize + params.kwarg.is_some() as usize;
        count + extra
    }

    /// Check if a method's first parameter is `self` (instance method).
    fn is_instance_method(params: &ast::Parameters) -> bool {
        params
            .posonlyargs
            .first()
            .or(params.args.first())
            .is_some_and(|p| p.parameter.name.as_str() == "self")
    }

    /// Visit a function definition and produce a Unit.
    fn visit_function_def(&mut self, func: &ast::StmtFunctionDef) {
        let name = func.name.as_str();
        let in_class =
            self.scope_stack.last().is_some_and(|_| self.is_class_scope());
        let kind =
            if in_class { UnitKind::Method } else { UnitKind::Function };

        let params = Self::count_params(&func.parameters);
        let parent = self.parent_id();
        let span = self.make_span(func.range());

        let branch_count = branches::count_branches_body(&func.body);
        let cognitive_complexity =
            cognitive::compute_cognitive_body(&func.body);
        let max_scope = scopes::max_scope_lines_body(
            &func.body,
            self.source,
            self.line_index,
        );
        let call_targets = calls::extract_calls_body(&func.body);
        let (reads, writes) =
            if in_class && Self::is_instance_method(&func.parameters) {
                field_access::extract_field_access_body(&func.body)
            } else {
                (vec![], vec![])
            };

        let unit = Unit {
            id: self.make_id(name),
            kind,
            file: PathBuf::from(&self.rel_path),
            span,
            reads,
            writes,
            calls: call_targets,
            tags: vec![],
            params,
            branches: branch_count,
            max_scope_lines: max_scope,
            parent,
            cognitive_complexity,
            partial: false,
        };
        self.units.push(unit);

        // Recurse into nested functions
        self.scope_stack.push(name.to_string());
        self.visit_body(&func.body);
        self.scope_stack.pop();
    }

    /// Visit a class definition and produce a Struct Unit + child methods.
    fn visit_class_def(&mut self, class: &ast::StmtClassDef) {
        let name = class.name.as_str();

        let unit = Unit {
            id: self.make_id(name),
            kind: UnitKind::Struct,
            file: PathBuf::from(&self.rel_path),
            span: self.make_span(class.range()),
            reads: vec![],
            writes: vec![],
            calls: vec![],
            tags: vec![],
            params: 0,
            branches: 0,
            max_scope_lines: 0,
            parent: self.parent_id(),
            cognitive_complexity: 0,
            partial: false,
        };
        self.units.push(unit);

        // Visit class body with class name on scope stack
        self.scope_stack.push(name.to_string());
        self.class_depth += 1;
        self.visit_body(&class.body);
        self.class_depth -= 1;
        self.scope_stack.pop();
    }

    /// Check if we're currently inside a class scope.
    fn is_class_scope(&self) -> bool {
        self.class_depth > 0
    }

    /// Visit a list of statements, extracting only function and class definitions.
    fn visit_body(&mut self, body: &[Stmt]) {
        for stmt in body {
            match stmt {
                Stmt::FunctionDef(func) => self.visit_function_def(func),
                Stmt::ClassDef(class) => self.visit_class_def(class),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_to_module_path() {
        assert_eq!(file_to_module_path("src/mymod/core.py"), "src.mymod.core");
        assert_eq!(file_to_module_path("src/mymod/__init__.py"), "src.mymod");
        assert_eq!(file_to_module_path("main.py"), "main");
        assert_eq!(file_to_module_path("types.pyi"), "types");
    }

    #[test]
    fn test_extract_simple_function() {
        let source = "def foo(x, y):\n    return x + y\n";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let units = extract_units(parsed.suite(), source, "test.py");
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, "test::foo");
        assert_eq!(units[0].kind, UnitKind::Function);
        assert_eq!(units[0].params, 2);
    }

    #[test]
    fn test_extract_class_with_methods() {
        let source = "\
class Foo:
    def bar(self, x):
        return self.val + x

    def baz(self):
        pass
";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let units = extract_units(parsed.suite(), source, "mymod/core.py");
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].id, "mymod.core::Foo");
        assert_eq!(units[0].kind, UnitKind::Struct);
        assert_eq!(units[1].id, "mymod.core::Foo.bar");
        assert_eq!(units[1].kind, UnitKind::Method);
        assert_eq!(units[1].params, 1); // self excluded
        assert_eq!(units[1].parent.as_deref(), Some("mymod.core::Foo"));
        assert_eq!(units[2].id, "mymod.core::Foo.baz");
    }

    #[test]
    fn test_nested_function() {
        let source = "\
def outer():
    def inner():
        pass
";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let units = extract_units(parsed.suite(), source, "test.py");
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].id, "test::outer");
        assert_eq!(units[0].kind, UnitKind::Function);
        assert_eq!(units[1].id, "test::outer.inner");
        assert_eq!(units[1].kind, UnitKind::Function);
        assert_eq!(units[1].parent.as_deref(), Some("test::outer"));
    }
}
