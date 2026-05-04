#[cfg(test)]
mod tests {
    use crate::branches::count_branches_body;

    fn parse_body(source: &str) -> Vec<ruff_python_ast::Stmt> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        parsed.into_suite()
    }

    /// Helper: parse a function and return its body statements.
    fn parse_fn_body(source: &str) -> Vec<ruff_python_ast::Stmt> {
        let stmts = parse_body(source);
        match &stmts[0] {
            ruff_python_ast::Stmt::FunctionDef(f) => f.body.clone(),
            _ => panic!("expected function def"),
        }
    }

    #[test]
    fn test_no_branches() {
        let body = parse_fn_body("def f():\n    x = 1\n    return x\n");
        assert_eq!(count_branches_body(&body), 0);
    }

    #[test]
    fn test_single_if() {
        let body = parse_fn_body("def f():\n    if x:\n        pass\n");
        assert_eq!(count_branches_body(&body), 1);
    }

    #[test]
    fn test_if_elif_else() {
        let body = parse_fn_body(
            "\
def f():
    if x:
        pass
    elif y:
        pass
    else:
        pass
",
        );
        assert_eq!(count_branches_body(&body), 2); // if + elif
    }

    #[test]
    fn test_for_loop() {
        let body = parse_fn_body("def f():\n    for x in xs:\n        pass\n");
        assert_eq!(count_branches_body(&body), 1);
    }

    #[test]
    fn test_while_loop() {
        let body = parse_fn_body("def f():\n    while x:\n        pass\n");
        assert_eq!(count_branches_body(&body), 1);
    }

    #[test]
    fn test_try_except() {
        let body = parse_fn_body(
            "\
def f():
    try:
        pass
    except ValueError:
        pass
    except TypeError:
        pass
",
        );
        assert_eq!(count_branches_body(&body), 2); // 2 except handlers
    }

    #[test]
    fn test_match_case() {
        let body = parse_fn_body(
            "\
def f():
    match x:
        case 1:
            pass
        case 2:
            pass
        case 3:
            pass
",
        );
        assert_eq!(count_branches_body(&body), 2); // 3 cases - 1
    }

    #[test]
    fn test_ternary() {
        let body = parse_fn_body("def f():\n    x = 1 if cond else 2\n");
        assert_eq!(count_branches_body(&body), 1);
    }

    #[test]
    fn test_and_or() {
        let body = parse_fn_body("def f():\n    x = a and b or c\n");
        assert_eq!(count_branches_body(&body), 2); // and + or
    }

    #[test]
    fn test_nested_if_in_for() {
        let body = parse_fn_body(
            "\
def f():
    for x in xs:
        if x > 0:
            pass
",
        );
        assert_eq!(count_branches_body(&body), 2); // for + if
    }

    #[test]
    fn test_while_with_and_in_condition() {
        let body =
            parse_fn_body("def f():\n    while a and b:\n        pass\n");
        assert_eq!(count_branches_body(&body), 2); // while + and
    }

    #[test]
    fn test_nested_function_not_counted() {
        let body = parse_fn_body(
            "\
def f():
    if x:
        pass
    def inner():
        if y:
            pass
",
        );
        // Only the outer if counts; inner def is skipped
        assert_eq!(count_branches_body(&body), 1);
    }
}
