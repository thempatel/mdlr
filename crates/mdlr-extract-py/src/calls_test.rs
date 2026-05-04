#[cfg(test)]
mod tests {
    use crate::calls::extract_calls_body;

    fn parse_fn_body(source: &str) -> Vec<ruff_python_ast::Stmt> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let stmts = parsed.into_suite();
        match &stmts[0] {
            ruff_python_ast::Stmt::FunctionDef(f) => f.body.clone(),
            _ => panic!("expected function def"),
        }
    }

    #[test]
    fn test_simple_call() {
        let body = parse_fn_body("def f():\n    foo()\n");
        assert_eq!(extract_calls_body(&body), vec!["foo"]);
    }

    #[test]
    fn test_method_call() {
        let body = parse_fn_body("def f():\n    obj.method()\n");
        assert_eq!(extract_calls_body(&body), vec!["obj.method"]);
    }

    #[test]
    fn test_self_method_call() {
        let body = parse_fn_body("def f(self):\n    self.bar()\n");
        assert_eq!(extract_calls_body(&body), vec!["Self.bar"]);
    }

    #[test]
    fn test_chained_call() {
        let body = parse_fn_body("def f():\n    a.b.c()\n");
        assert_eq!(extract_calls_body(&body), vec!["a.b.c"]);
    }

    #[test]
    fn test_no_calls() {
        let body = parse_fn_body("def f():\n    x = 1\n    return x\n");
        assert!(extract_calls_body(&body).is_empty());
    }

    #[test]
    fn test_multiple_calls() {
        let body =
            parse_fn_body("def f():\n    foo()\n    bar()\n    baz()\n");
        let calls = extract_calls_body(&body);
        assert_eq!(calls, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn test_call_in_if() {
        let body = parse_fn_body(
            "\
def f():
    if check():
        do_something()
",
        );
        let calls = extract_calls_body(&body);
        assert!(calls.contains(&"check".to_string()));
        assert!(calls.contains(&"do_something".to_string()));
    }

    #[test]
    fn test_call_in_return() {
        let body = parse_fn_body("def f():\n    return compute(x)\n");
        assert_eq!(extract_calls_body(&body), vec!["compute"]);
    }

    #[test]
    fn test_call_in_assignment() {
        let body = parse_fn_body("def f():\n    x = make_thing()\n");
        assert_eq!(extract_calls_body(&body), vec!["make_thing"]);
    }

    #[test]
    fn test_call_deduplication() {
        let body =
            parse_fn_body("def f():\n    foo()\n    foo()\n    foo()\n");
        assert_eq!(extract_calls_body(&body), vec!["foo"]);
    }

    #[test]
    fn test_nested_function_calls_not_included() {
        let body = parse_fn_body(
            "\
def f():
    foo()
    def inner():
        bar()
",
        );
        assert_eq!(extract_calls_body(&body), vec!["foo"]);
    }

    #[test]
    fn test_call_in_try_except() {
        let body = parse_fn_body(
            "\
def f():
    try:
        risky()
    except Exception:
        handle()
",
        );
        let calls = extract_calls_body(&body);
        assert!(calls.contains(&"risky".to_string()));
        assert!(calls.contains(&"handle".to_string()));
    }

    #[test]
    fn test_call_in_for_loop() {
        let body = parse_fn_body(
            "\
def f():
    for x in get_items():
        process(x)
",
        );
        let calls = extract_calls_body(&body);
        assert!(calls.contains(&"get_items".to_string()));
        assert!(calls.contains(&"process".to_string()));
    }

    #[test]
    fn test_call_in_with() {
        let body = parse_fn_body(
            "\
def f():
    with open('file') as f:
        f.read()
",
        );
        let calls = extract_calls_body(&body);
        assert!(calls.contains(&"open".to_string()));
        assert!(calls.contains(&"f.read".to_string()));
    }
}
