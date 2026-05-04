#[cfg(test)]
mod tests {
    use crate::scopes::max_scope_lines_body;
    use crate::visitor::LineIndex;

    fn parse_fn_body(source: &str) -> Vec<ruff_python_ast::Stmt> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let stmts = parsed.into_suite();
        match &stmts[0] {
            ruff_python_ast::Stmt::FunctionDef(f) => f.body.clone(),
            _ => panic!("expected function def"),
        }
    }

    fn compute_max_scope(source: &str) -> usize {
        let body = parse_fn_body(source);
        let line_index = LineIndex::new_from_source(source);
        max_scope_lines_body(&body, source, &line_index)
    }

    #[test]
    fn test_no_scopes() {
        let score = compute_max_scope("def f():\n    x = 1\n    return x\n");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_single_if() {
        let score = compute_max_scope(
            "\
def f():
    if x:
        a = 1
        b = 2
",
        );
        assert!(score >= 3); // the if block spans multiple lines
    }

    #[test]
    fn test_nested_scopes_largest_wins() {
        let score = compute_max_scope(
            "\
def f():
    if x:
        pass
    for i in range(10):
        a = 1
        b = 2
        c = 3
        d = 4
",
        );
        // The for loop body is larger than the if body
        assert!(score >= 5);
    }

    #[test]
    fn test_nested_function_not_counted() {
        let score = compute_max_scope(
            "\
def f():
    x = 1
    def inner():
        if a:
            if b:
                if c:
                    pass
",
        );
        // Nested function is excluded
        assert_eq!(score, 0);
    }
}
