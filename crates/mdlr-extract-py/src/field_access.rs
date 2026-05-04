use ruff_python_ast::{self as ast, Expr, Stmt};

/// Extract `self.field` reads and writes from a function body.
/// Only call this for instance methods (first param is `self`).
pub fn extract_field_access_body(body: &[Stmt]) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for stmt in body {
        extract_field_access_stmt(stmt, &mut reads, &mut writes);
    }
    (reads, writes)
}

fn extract_field_access_stmt(
    stmt: &Stmt,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
) {
    match stmt {
        // self.field = value → write
        Stmt::Assign(node) => {
            for target in &node.targets {
                if let Some(field) = self_field_name(target) {
                    record_write(writes, field);
                } else {
                    extract_field_access_expr(target, reads, writes);
                }
            }
            extract_field_access_expr(&node.value, reads, writes);
        }
        // self.field += value → write + read
        Stmt::AugAssign(node) => {
            if let Some(field) = self_field_name(&node.target) {
                record_write(writes, field.clone());
                record_read(reads, field);
            } else {
                extract_field_access_expr(&node.target, reads, writes);
            }
            extract_field_access_expr(&node.value, reads, writes);
        }
        // self.field: type = value → write
        Stmt::AnnAssign(node) => {
            if let Some(field) = self_field_name(&node.target) {
                if node.value.is_some() {
                    record_write(writes, field);
                }
            }
            if let Some(val) = &node.value {
                extract_field_access_expr(val, reads, writes);
            }
        }
        Stmt::Expr(node) => {
            extract_field_access_expr(&node.value, reads, writes);
        }
        Stmt::Return(node) => {
            if let Some(val) = &node.value {
                extract_field_access_expr(val, reads, writes);
            }
        }
        Stmt::If(node) => {
            extract_field_access_expr(&node.test, reads, writes);
            extract_field_access_body_stmts(&node.body, reads, writes);
            for clause in &node.elif_else_clauses {
                if let Some(test) = &clause.test {
                    extract_field_access_expr(test, reads, writes);
                }
                extract_field_access_body_stmts(&clause.body, reads, writes);
            }
        }
        Stmt::For(node) => {
            extract_field_access_expr(&node.iter, reads, writes);
            extract_field_access_body_stmts(&node.body, reads, writes);
            extract_field_access_body_stmts(&node.orelse, reads, writes);
        }
        Stmt::While(node) => {
            extract_field_access_expr(&node.test, reads, writes);
            extract_field_access_body_stmts(&node.body, reads, writes);
            extract_field_access_body_stmts(&node.orelse, reads, writes);
        }
        Stmt::Try(node) => {
            extract_field_access_body_stmts(&node.body, reads, writes);
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = handler;
                extract_field_access_body_stmts(&h.body, reads, writes);
            }
            extract_field_access_body_stmts(&node.orelse, reads, writes);
            extract_field_access_body_stmts(&node.finalbody, reads, writes);
        }
        Stmt::Match(node) => {
            extract_field_access_expr(&node.subject, reads, writes);
            for case in &node.cases {
                extract_field_access_body_stmts(&case.body, reads, writes);
            }
        }
        Stmt::With(node) => {
            for item in &node.items {
                extract_field_access_expr(&item.context_expr, reads, writes);
            }
            extract_field_access_body_stmts(&node.body, reads, writes);
        }
        Stmt::Raise(node) => {
            if let Some(exc) = &node.exc {
                extract_field_access_expr(exc, reads, writes);
            }
        }
        Stmt::Assert(node) => {
            extract_field_access_expr(&node.test, reads, writes);
        }
        // Don't recurse into nested function/class defs
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
        _ => {}
    }
}

fn extract_field_access_body_stmts(
    body: &[Stmt],
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
) {
    for stmt in body {
        extract_field_access_stmt(stmt, reads, writes);
    }
}

fn extract_field_access_expr(
    expr: &Expr,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
) {
    match expr {
        // self.field in expression context → read (unless it's a method call)
        Expr::Attribute(node) => {
            if is_self_expr(&node.value) {
                record_read(reads, node.attr.to_string());
            } else {
                extract_field_access_expr(&node.value, reads, writes);
            }
        }
        // self.method() — the `method` is NOT a field read, it's a call
        Expr::Call(node) => {
            if let Expr::Attribute(attr) = node.func.as_ref() {
                if is_self_expr(&attr.value) {
                    // Skip — self.method() is a call, not a field read
                    // But recurse into arguments
                    for arg in node.arguments.args.iter() {
                        extract_field_access_expr(arg, reads, writes);
                    }
                    return;
                }
            }
            extract_field_access_expr(&node.func, reads, writes);
            for arg in node.arguments.args.iter() {
                extract_field_access_expr(arg, reads, writes);
            }
        }
        Expr::BoolOp(node) => {
            for val in &node.values {
                extract_field_access_expr(val, reads, writes);
            }
        }
        Expr::BinOp(node) => {
            extract_field_access_expr(&node.left, reads, writes);
            extract_field_access_expr(&node.right, reads, writes);
        }
        Expr::UnaryOp(node) => {
            extract_field_access_expr(&node.operand, reads, writes);
        }
        Expr::If(node) => {
            extract_field_access_expr(&node.test, reads, writes);
            extract_field_access_expr(&node.body, reads, writes);
            extract_field_access_expr(&node.orelse, reads, writes);
        }
        Expr::Compare(node) => {
            extract_field_access_expr(&node.left, reads, writes);
            for comp in node.comparators.iter() {
                extract_field_access_expr(comp, reads, writes);
            }
        }
        Expr::Subscript(node) => {
            extract_field_access_expr(&node.value, reads, writes);
            extract_field_access_expr(&node.slice, reads, writes);
        }
        Expr::List(node) => {
            for elt in &node.elts {
                extract_field_access_expr(elt, reads, writes);
            }
        }
        Expr::Tuple(node) => {
            for elt in &node.elts {
                extract_field_access_expr(elt, reads, writes);
            }
        }
        Expr::Dict(node) => {
            for item in &node.items {
                if let Some(key) = &item.key {
                    extract_field_access_expr(key, reads, writes);
                }
                extract_field_access_expr(&item.value, reads, writes);
            }
        }
        Expr::Starred(node) => {
            extract_field_access_expr(&node.value, reads, writes);
        }
        Expr::Await(node) => {
            extract_field_access_expr(&node.value, reads, writes);
        }
        // Don't recurse into lambdas
        Expr::Lambda(_) => {}
        _ => {}
    }
}

/// Check if expr is `self`.
fn is_self_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(name) if name.id.as_str() == "self")
}

/// Check if an expression is `self.field` and return the field name.
fn self_field_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Attribute(node) => {
            if is_self_expr(&node.value) {
                Some(node.attr.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn record_read(reads: &mut Vec<String>, name: String) {
    if !reads.contains(&name) {
        reads.push(name);
    }
}

fn record_write(writes: &mut Vec<String>, name: String) {
    if !writes.contains(&name) {
        writes.push(name);
    }
}
