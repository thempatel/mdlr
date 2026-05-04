#[cfg(test)]
mod tests {
    use crate::cognitive::compute_cognitive_body;

    fn parse_fn_body(source: &str) -> Vec<ruff_python_ast::Stmt> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let stmts = parsed.into_suite();
        match &stmts[0] {
            ruff_python_ast::Stmt::FunctionDef(f) => f.body.clone(),
            _ => panic!("expected function def"),
        }
    }

    #[test]
    fn test_no_complexity() {
        let body = parse_fn_body("def f():\n    return 1\n");
        assert_eq!(compute_cognitive_body(&body), 0);
    }

    #[test]
    fn test_single_if() {
        // if at nesting 0: +1
        let body = parse_fn_body("def f():\n    if x:\n        pass\n");
        assert_eq!(compute_cognitive_body(&body), 1);
    }

    #[test]
    fn test_if_else() {
        // if: +1, else: +1
        let body = parse_fn_body(
            "\
def f():
    if x:
        pass
    else:
        pass
",
        );
        assert_eq!(compute_cognitive_body(&body), 2);
    }

    #[test]
    fn test_if_elif_else() {
        // if: +1, elif: +1, else: +1
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
        assert_eq!(compute_cognitive_body(&body), 3);
    }

    #[test]
    fn test_nested_if() {
        // outer if: +1 (nesting=0), inner if: +1+1 (nesting=1)
        let body = parse_fn_body(
            "\
def f():
    if x:
        if y:
            pass
",
        );
        assert_eq!(compute_cognitive_body(&body), 3);
    }

    #[test]
    fn test_for_loop() {
        // for at nesting 0: +1
        let body = parse_fn_body("def f():\n    for x in xs:\n        pass\n");
        assert_eq!(compute_cognitive_body(&body), 1);
    }

    #[test]
    fn test_for_with_nested_if() {
        // for: +1 (nesting=0), if: +1+1 (nesting=1)
        let body = parse_fn_body(
            "\
def f():
    for x in xs:
        if x > 0:
            pass
",
        );
        assert_eq!(compute_cognitive_body(&body), 3);
    }

    #[test]
    fn test_while_loop() {
        let body = parse_fn_body("def f():\n    while x:\n        pass\n");
        assert_eq!(compute_cognitive_body(&body), 1);
    }

    #[test]
    fn test_try_except() {
        // each except: +1 + nesting
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
        assert_eq!(compute_cognitive_body(&body), 2); // 2 * (+1+0)
    }

    #[test]
    fn test_logical_operators() {
        // and: +1, or: +1 (no nesting penalty)
        let body = parse_fn_body("def f():\n    x = a and b or c\n");
        assert_eq!(compute_cognitive_body(&body), 2);
    }

    #[test]
    fn test_ternary() {
        // ternary at nesting 0: +1
        let body = parse_fn_body("def f():\n    x = 1 if cond else 2\n");
        assert_eq!(compute_cognitive_body(&body), 1);
    }

    #[test]
    fn test_deeply_nested() {
        // for: +1(0), if: +1+1(1), if: +1+2(2)
        let body = parse_fn_body(
            "\
def f():
    for x in xs:
        if x > 0:
            if x < 10:
                pass
",
        );
        assert_eq!(compute_cognitive_body(&body), 6);
    }

    #[test]
    fn test_nested_function_not_counted() {
        // Only the outer if counts
        let body = parse_fn_body(
            "\
def f():
    if x:
        pass
    def inner():
        if y:
            if z:
                pass
",
        );
        assert_eq!(compute_cognitive_body(&body), 1);
    }

    #[test]
    fn test_match() {
        // match at nesting 0: +1
        let body = parse_fn_body(
            "\
def f():
    match x:
        case 1:
            pass
        case 2:
            pass
",
        );
        assert_eq!(compute_cognitive_body(&body), 1);
    }
}
