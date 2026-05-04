use ruff_python_ast::{self as ast, Expr, Stmt};

/// Compute cognitive complexity for a function body.
pub fn compute_cognitive_body(body: &[Stmt]) -> usize {
    let mut score = 0;
    let nesting = 0;
    for stmt in body {
        score += compute_cognitive_stmt(stmt, nesting);
    }
    score
}

fn compute_cognitive_stmt(stmt: &Stmt, nesting: usize) -> usize {
    match stmt {
        Stmt::If(node) => {
            let mut score = 1 + nesting; // +1 inherent + nesting penalty

            // Visit test at current nesting
            score += compute_cognitive_expr(&node.test, nesting);

            // Visit body at increased nesting
            for s in &node.body {
                score += compute_cognitive_stmt(s, nesting + 1);
            }

            // Handle elif/else
            for clause in &node.elif_else_clauses {
                if clause.test.is_some() {
                    // elif: +1 inherent, no extra nesting penalty (it's a continuation)
                    score += 1;
                    if let Some(test) = &clause.test {
                        score += compute_cognitive_expr(test, nesting);
                    }
                } else {
                    // else: +1 inherent, no nesting penalty
                    score += 1;
                }
                for s in &clause.body {
                    score += compute_cognitive_stmt(s, nesting + 1);
                }
            }
            score
        }
        Stmt::For(node) => {
            let mut score = 1 + nesting;
            for s in &node.body {
                score += compute_cognitive_stmt(s, nesting + 1);
            }
            for s in &node.orelse {
                score += compute_cognitive_stmt(s, nesting + 1);
            }
            score
        }
        Stmt::While(node) => {
            let mut score = 1 + nesting;
            score += compute_cognitive_expr(&node.test, nesting);
            for s in &node.body {
                score += compute_cognitive_stmt(s, nesting + 1);
            }
            for s in &node.orelse {
                score += compute_cognitive_stmt(s, nesting + 1);
            }
            score
        }
        Stmt::Try(node) => {
            let mut score = 0;
            for s in &node.body {
                score += compute_cognitive_stmt(s, nesting);
            }
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = handler;
                // Each except: +1 inherent + nesting
                score += 1 + nesting;
                for s in &h.body {
                    score += compute_cognitive_stmt(s, nesting + 1);
                }
            }
            for s in &node.orelse {
                score += compute_cognitive_stmt(s, nesting);
            }
            for s in &node.finalbody {
                score += compute_cognitive_stmt(s, nesting);
            }
            score
        }
        Stmt::Match(node) => {
            let mut score = 1 + nesting;
            for case in &node.cases {
                for s in &case.body {
                    score += compute_cognitive_stmt(s, nesting + 1);
                }
            }
            score
        }
        Stmt::With(node) => {
            let mut score = 0;
            // with block increases nesting
            for s in &node.body {
                score += compute_cognitive_stmt(s, nesting + 1);
            }
            score
        }
        Stmt::Expr(node) => compute_cognitive_expr(&node.value, nesting),
        Stmt::Return(node) => node
            .value
            .as_ref()
            .map_or(0, |v| compute_cognitive_expr(v, nesting)),
        Stmt::Assign(node) => compute_cognitive_expr(&node.value, nesting),
        Stmt::AugAssign(node) => compute_cognitive_expr(&node.value, nesting),
        Stmt::AnnAssign(node) => node
            .value
            .as_ref()
            .map_or(0, |v| compute_cognitive_expr(v, nesting)),
        // Don't recurse into nested function/class defs
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => 0,
        _ => 0,
    }
}

fn compute_cognitive_expr(expr: &Expr, nesting: usize) -> usize {
    match expr {
        Expr::If(node) => {
            // Ternary: +1 inherent + nesting penalty
            let mut score = 1 + nesting;
            score += compute_cognitive_expr(&node.test, nesting);
            score += compute_cognitive_expr(&node.body, nesting + 1);
            score += compute_cognitive_expr(&node.orelse, nesting + 1);
            score
        }
        Expr::BoolOp(node) => {
            // +1 for each logical operator (no nesting penalty)
            let op_count = node.values.len().saturating_sub(1);
            let mut score = op_count;
            for val in &node.values {
                score += compute_cognitive_expr(val, nesting);
            }
            score
        }
        Expr::Call(node) => {
            let mut score = compute_cognitive_expr(&node.func, nesting);
            for arg in node.arguments.args.iter() {
                score += compute_cognitive_expr(arg, nesting);
            }
            score
        }
        Expr::Lambda(node) => {
            // Lambda increases nesting for its body
            compute_cognitive_expr(&node.body, nesting + 1)
        }
        _ => 0,
    }
}
