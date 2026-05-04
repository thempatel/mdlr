use ruff_python_ast::{self as ast, Expr, Stmt};

/// Count branch points in a function body.
pub fn count_branches_body(body: &[Stmt]) -> usize {
    let mut count = 0;
    for stmt in body {
        count += count_branches_stmt(stmt);
    }
    count
}

fn count_branches_stmt(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::If(node) => {
            let mut count = 1; // the `if` itself
            // Each `elif` is an additional branch
            for clause in &node.elif_else_clauses {
                if clause.test.is_some() {
                    count += 1; // elif
                }
            }
            // Recurse into bodies
            count += count_branches_body(&node.body);
            for clause in &node.elif_else_clauses {
                count += count_branches_body(&clause.body);
            }
            // Count branches in test expression
            count += count_branches_expr(&node.test);
            count
        }
        Stmt::For(node) => {
            1 + count_branches_body(&node.body)
                + count_branches_body(&node.orelse)
        }
        Stmt::While(node) => {
            1 + count_branches_expr(&node.test)
                + count_branches_body(&node.body)
                + count_branches_body(&node.orelse)
        }
        Stmt::Try(node) => {
            let handler_count = node.handlers.len();
            let mut count = if handler_count > 0 { handler_count } else { 0 };
            count += count_branches_body(&node.body);
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = handler;
                count += count_branches_body(&h.body);
            }
            count += count_branches_body(&node.orelse);
            count += count_branches_body(&node.finalbody);
            count
        }
        Stmt::Match(node) => {
            let case_count = node.cases.len();
            let mut count = if case_count > 1 { case_count - 1 } else { 0 };
            for case in &node.cases {
                count += count_branches_body(&case.body);
            }
            count
        }
        Stmt::With(node) => count_branches_body(&node.body),
        // Expression statement — check for ternary, and/or in the expr
        Stmt::Expr(node) => count_branches_expr(&node.value),
        Stmt::Return(node) => {
            node.value.as_ref().map_or(0, |v| count_branches_expr(v))
        }
        Stmt::Assign(node) => count_branches_expr(&node.value),
        Stmt::AugAssign(node) => count_branches_expr(&node.value),
        Stmt::AnnAssign(node) => {
            node.value.as_ref().map_or(0, |v| count_branches_expr(v))
        }
        // Don't recurse into nested function/class defs
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => 0,
        _ => 0,
    }
}

fn count_branches_expr(expr: &Expr) -> usize {
    match expr {
        Expr::If(_) => {
            // Ternary: x if cond else y
            1 + count_branches_children_expr(expr)
        }
        Expr::BoolOp(node) => {
            // Each `and`/`or` connector is a branch point
            // values has N items, so N-1 operators
            let op_count = node.values.len().saturating_sub(1);
            op_count + count_branches_children_expr(expr)
        }
        _ => count_branches_children_expr(expr),
    }
}

/// Recurse into child expressions without double-counting the parent.
fn count_branches_children_expr(expr: &Expr) -> usize {
    let mut count = 0;
    match expr {
        Expr::BoolOp(node) => {
            for val in &node.values {
                count += count_branches_expr(val);
            }
        }
        Expr::If(node) => {
            count += count_branches_expr(&node.test);
            count += count_branches_expr(&node.body);
            count += count_branches_expr(&node.orelse);
        }
        Expr::Call(node) => {
            count += count_branches_expr(&node.func);
            for arg in node.arguments.args.iter() {
                count += count_branches_expr(arg);
            }
        }
        Expr::BinOp(node) => {
            count += count_branches_expr(&node.left);
            count += count_branches_expr(&node.right);
        }
        Expr::UnaryOp(node) => {
            count += count_branches_expr(&node.operand);
        }
        Expr::Lambda(node) => {
            count += count_branches_expr(&node.body);
        }
        Expr::Dict(node) => {
            for item in &node.items {
                if let Some(key) = &item.key {
                    count += count_branches_expr(key);
                }
                count += count_branches_expr(&item.value);
            }
        }
        Expr::Set(node) => {
            for elt in &node.elts {
                count += count_branches_expr(elt);
            }
        }
        Expr::ListComp(node) => {
            count += count_branches_expr(&node.elt);
        }
        Expr::SetComp(node) => {
            count += count_branches_expr(&node.elt);
        }
        Expr::Generator(node) => {
            count += count_branches_expr(&node.elt);
        }
        Expr::Await(node) => {
            count += count_branches_expr(&node.value);
        }
        Expr::Yield(node) => {
            if let Some(val) = &node.value {
                count += count_branches_expr(val);
            }
        }
        Expr::YieldFrom(node) => {
            count += count_branches_expr(&node.value);
        }
        Expr::Compare(node) => {
            count += count_branches_expr(&node.left);
            for comp in node.comparators.iter() {
                count += count_branches_expr(comp);
            }
        }
        Expr::Attribute(node) => {
            count += count_branches_expr(&node.value);
        }
        Expr::Subscript(node) => {
            count += count_branches_expr(&node.value);
            count += count_branches_expr(&node.slice);
        }
        Expr::Starred(node) => {
            count += count_branches_expr(&node.value);
        }
        Expr::List(node) => {
            for elt in &node.elts {
                count += count_branches_expr(elt);
            }
        }
        Expr::Tuple(node) => {
            for elt in &node.elts {
                count += count_branches_expr(elt);
            }
        }
        _ => {}
    }
    count
}
