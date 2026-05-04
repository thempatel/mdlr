#[cfg(test)]
mod tests {
    use crate::field_access::extract_field_access_body;

    fn parse_fn_body(source: &str) -> Vec<ruff_python_ast::Stmt> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let stmts = parsed.into_suite();
        // Get the method body from inside the class
        match &stmts[0] {
            ruff_python_ast::Stmt::ClassDef(cls) => match &cls.body[0] {
                ruff_python_ast::Stmt::FunctionDef(f) => f.body.clone(),
                _ => panic!("expected method def"),
            },
            ruff_python_ast::Stmt::FunctionDef(f) => f.body.clone(),
            _ => panic!("expected class or function def"),
        }
    }

    #[test]
    fn test_self_field_write() {
        let body = parse_fn_body(
            "\
class Foo:
    def __init__(self):
        self.x = 1
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert!(reads.is_empty());
        assert_eq!(writes, vec!["x"]);
    }

    #[test]
    fn test_self_field_read() {
        let body = parse_fn_body(
            "\
class Foo:
    def get(self):
        return self.x
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert_eq!(reads, vec!["x"]);
        assert!(writes.is_empty());
    }

    #[test]
    fn test_self_read_and_write() {
        let body = parse_fn_body(
            "\
class Foo:
    def update(self):
        self.y = self.x + 1
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert_eq!(reads, vec!["x"]);
        assert_eq!(writes, vec!["y"]);
    }

    #[test]
    fn test_self_method_not_a_read() {
        let body = parse_fn_body(
            "\
class Foo:
    def do_thing(self):
        self.helper()
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        // self.helper() is a call, not a field read
        assert!(reads.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn test_multiple_fields() {
        let body = parse_fn_body(
            "\
class Foo:
    def __init__(self, a, b):
        self.a = a
        self.b = b
        self.c = self.a + self.b
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert!(reads.contains(&"a".to_string()));
        assert!(reads.contains(&"b".to_string()));
        assert!(writes.contains(&"a".to_string()));
        assert!(writes.contains(&"b".to_string()));
        assert!(writes.contains(&"c".to_string()));
    }

    #[test]
    fn test_aug_assign_is_read_and_write() {
        let body = parse_fn_body(
            "\
class Foo:
    def inc(self):
        self.count += 1
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert_eq!(reads, vec!["count"]);
        assert_eq!(writes, vec!["count"]);
    }

    #[test]
    fn test_field_in_condition() {
        let body = parse_fn_body(
            "\
class Foo:
    def check(self):
        if self.enabled:
            return self.value
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert!(reads.contains(&"enabled".to_string()));
        assert!(reads.contains(&"value".to_string()));
        assert!(writes.is_empty());
    }

    #[test]
    fn test_no_self_access() {
        let body = parse_fn_body(
            "\
class Foo:
    def static_like(self):
        x = 1
        return x + 2
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert!(reads.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn test_deduplication() {
        let body = parse_fn_body(
            "\
class Foo:
    def f(self):
        x = self.val
        y = self.val
        self.val = x + y
",
        );
        let (reads, writes) = extract_field_access_body(&body);
        assert_eq!(reads, vec!["val"]);
        assert_eq!(writes, vec!["val"]);
    }
}
