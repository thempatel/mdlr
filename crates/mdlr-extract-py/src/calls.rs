use ruff_python_ast::{self as ast, Expr, Stmt};

/// Extract call targets from a function body.
pub fn extract_calls_body(body: &[Stmt]) -> Vec<String> {
    let mut calls = Vec::new();
    for stmt in body {
        extract_calls_stmt(stmt, &mut calls);
    }
    calls
}

fn extract_calls_stmt(stmt: &Stmt, calls: &mut Vec<String>) {
    match stmt {
        Stmt::Expr(node) => extract_calls_expr(&node.value, calls),
        Stmt::Return(node) => {
            if let Some(val) = &node.value {
                extract_calls_expr(val, calls);
            }
        }
        Stmt::Assign(node) => {
            extract_calls_expr(&node.value, calls);
        }
        Stmt::AugAssign(node) => {
            extract_calls_expr(&node.value, calls);
        }
        Stmt::AnnAssign(node) => {
            if let Some(val) = &node.value {
                extract_calls_expr(val, calls);
            }
        }
        Stmt::If(node) => {
            extract_calls_expr(&node.test, calls);
            extract_calls_body_stmts(&node.body, calls);
            for clause in &node.elif_else_clauses {
                if let Some(test) = &clause.test {
                    extract_calls_expr(test, calls);
                }
                extract_calls_body_stmts(&clause.body, calls);
            }
        }
        Stmt::For(node) => {
            extract_calls_expr(&node.iter, calls);
            extract_calls_body_stmts(&node.body, calls);
            extract_calls_body_stmts(&node.orelse, calls);
        }
        Stmt::While(node) => {
            extract_calls_expr(&node.test, calls);
            extract_calls_body_stmts(&node.body, calls);
            extract_calls_body_stmts(&node.orelse, calls);
        }
        Stmt::Try(node) => {
            extract_calls_body_stmts(&node.body, calls);
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = handler;
                extract_calls_body_stmts(&h.body, calls);
            }
            extract_calls_body_stmts(&node.orelse, calls);
            extract_calls_body_stmts(&node.finalbody, calls);
        }
        Stmt::Match(node) => {
            extract_calls_expr(&node.subject, calls);
            for case in &node.cases {
                extract_calls_body_stmts(&case.body, calls);
            }
        }
        Stmt::With(node) => {
            for item in &node.items {
                extract_calls_expr(&item.context_expr, calls);
            }
            extract_calls_body_stmts(&node.body, calls);
        }
        Stmt::Raise(node) => {
            if let Some(exc) = &node.exc {
                extract_calls_expr(exc, calls);
            }
        }
        Stmt::Assert(node) => {
            extract_calls_expr(&node.test, calls);
            if let Some(msg) = &node.msg {
                extract_calls_expr(msg, calls);
            }
        }
        // Don't recurse into nested function/class defs — their calls are their own.
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
        _ => {}
    }
}

fn extract_calls_body_stmts(body: &[Stmt], calls: &mut Vec<String>) {
    for stmt in body {
        extract_calls_stmt(stmt, calls);
    }
}

fn extract_calls_expr(expr: &Expr, calls: &mut Vec<String>) {
    match expr {
        Expr::Call(node) => {
            if let Some(name) = callee_to_string(&node.func) {
                record(calls, name);
            }
            // Recurse into arguments
            for arg in node.arguments.args.iter() {
                extract_calls_expr(arg, calls);
            }
            // Recurse into callee for chained calls
            extract_calls_expr(&node.func, calls);
        }
        Expr::BoolOp(node) => {
            for val in &node.values {
                extract_calls_expr(val, calls);
            }
        }
        Expr::BinOp(node) => {
            extract_calls_expr(&node.left, calls);
            extract_calls_expr(&node.right, calls);
        }
        Expr::UnaryOp(node) => {
            extract_calls_expr(&node.operand, calls);
        }
        Expr::If(node) => {
            extract_calls_expr(&node.test, calls);
            extract_calls_expr(&node.body, calls);
            extract_calls_expr(&node.orelse, calls);
        }
        Expr::Compare(node) => {
            extract_calls_expr(&node.left, calls);
            for comp in node.comparators.iter() {
                extract_calls_expr(comp, calls);
            }
        }
        Expr::Attribute(node) => {
            extract_calls_expr(&node.value, calls);
        }
        Expr::Subscript(node) => {
            extract_calls_expr(&node.value, calls);
            extract_calls_expr(&node.slice, calls);
        }
        Expr::Starred(node) => {
            extract_calls_expr(&node.value, calls);
        }
        Expr::Await(node) => {
            extract_calls_expr(&node.value, calls);
        }
        Expr::Yield(node) => {
            if let Some(val) = &node.value {
                extract_calls_expr(val, calls);
            }
        }
        Expr::YieldFrom(node) => {
            extract_calls_expr(&node.value, calls);
        }
        Expr::List(node) => {
            for elt in &node.elts {
                extract_calls_expr(elt, calls);
            }
        }
        Expr::Set(node) => {
            for elt in &node.elts {
                extract_calls_expr(elt, calls);
            }
        }
        Expr::Tuple(node) => {
            for elt in &node.elts {
                extract_calls_expr(elt, calls);
            }
        }
        Expr::Dict(node) => {
            for item in &node.items {
                if let Some(key) = &item.key {
                    extract_calls_expr(key, calls);
                }
                extract_calls_expr(&item.value, calls);
            }
        }
        // Don't recurse into lambdas — they're their own scope
        Expr::Lambda(_) => {}
        _ => {}
    }
}

/// Build a call target string from a callee expression.
fn callee_to_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attr) => {
            let obj = expr_to_string(&attr.value)?;
            let prop = attr.attr.as_str();
            // Map self.method() → Self.method
            if obj == "self" {
                Some(format!("Self.{prop}"))
            } else {
                Some(format!("{obj}.{prop}"))
            }
        }
        _ => None,
    }
}

/// Convert an expression to a simple string for object parts.
fn expr_to_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attr) => {
            let obj = expr_to_string(&attr.value)?;
            let prop = attr.attr.as_str();
            Some(format!("{obj}.{prop}"))
        }
        _ => None,
    }
}

fn record(calls: &mut Vec<String>, name: String) {
    if !calls.contains(&name) {
        calls.push(name);
    }
}
