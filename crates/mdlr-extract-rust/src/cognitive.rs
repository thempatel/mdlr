use ra_ap_syntax::ast::{self, HasLoopBody};

use crate::walk::CstVisitor;

/// Compute cognitive complexity (SonarSource formulation) for a function body.
///
/// Unlike cyclomatic complexity, cognitive complexity penalizes nesting depth:
/// each control structure adds `1 + current_nesting_depth` to the score.
pub fn compute_cognitive_complexity(body: &ast::BlockExpr) -> usize {
    let mut visitor = CognitiveVisitor { score: 0 };
    if let Some(stmt_list) = body.stmt_list() {
        visitor.walk_stmt_list(&stmt_list, 0);
    }
    visitor.score
}

struct CognitiveVisitor {
    score: usize,
}

impl CognitiveVisitor {
    fn walk_stmt_with_ctx(&mut self, stmt: &ast::Stmt, nesting: usize) {
        match stmt {
            ast::Stmt::LetStmt(local) => {
                if let Some(init) = local.initializer() {
                    self.walk_expr(&init, nesting);
                }
            }
            ast::Stmt::ExprStmt(es) => {
                if let Some(expr) = es.expr() {
                    self.walk_expr(&expr, nesting);
                }
            }
            _ => {}
        }
    }
}

impl CstVisitor for CognitiveVisitor {
    /// Nesting depth as context.
    type Ctx = usize;

    fn visit_if(&mut self, expr: &ast::IfExpr, nesting: usize) {
        // +1 inherent + nesting penalty
        self.score += 1 + nesting;
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, nesting);
        }
        if let Some(then_branch) = expr.then_branch() {
            // Walk then-branch contents at increased nesting
            if let Some(stmt_list) = then_branch.stmt_list() {
                for stmt in stmt_list.statements() {
                    self.walk_stmt_with_ctx(&stmt, nesting + 1);
                }
                if let Some(tail) = stmt_list.tail_expr() {
                    self.walk_expr(&tail, nesting + 1);
                }
            }
        }
        if let Some(else_branch) = expr.else_branch() {
            match else_branch {
                ast::ElseBranch::IfExpr(elif) => {
                    // `else if` — walk at same nesting (the if itself adds +1)
                    self.walk_expr(&ast::Expr::from(elif), nesting);
                }
                ast::ElseBranch::Block(block) => {
                    // Plain `else`: +1 inherent, no nesting penalty
                    self.score += 1;
                    if let Some(stmt_list) = block.stmt_list() {
                        for stmt in stmt_list.statements() {
                            self.walk_stmt_with_ctx(&stmt, nesting + 1);
                        }
                        if let Some(tail) = stmt_list.tail_expr() {
                            self.walk_expr(&tail, nesting + 1);
                        }
                    }
                }
            }
        }
    }

    fn visit_match(&mut self, expr: &ast::MatchExpr, nesting: usize) {
        // +1 inherent + nesting penalty
        self.score += 1 + nesting;
        if let Some(scrutinee) = expr.expr() {
            self.walk_expr(&scrutinee, nesting);
        }
        let arm_nesting = nesting + 1;
        if let Some(arm_list) = expr.match_arm_list() {
            for arm in arm_list.arms() {
                if let Some(guard) = arm.guard() {
                    if let Some(guard_expr) = guard.condition() {
                        self.walk_expr(&guard_expr, arm_nesting);
                    }
                }
                if let Some(body) = arm.expr() {
                    self.walk_expr(&body, arm_nesting);
                }
            }
        }
    }

    fn visit_for(&mut self, expr: &ast::ForExpr, nesting: usize) {
        self.score += 1 + nesting;
        if let Some(iterable) = expr.iterable() {
            self.walk_expr(&iterable, nesting);
        }
        if let Some(body) = expr.loop_body() {
            if let Some(stmt_list) = body.stmt_list() {
                for stmt in stmt_list.statements() {
                    self.walk_stmt_with_ctx(&stmt, nesting + 1);
                }
                if let Some(tail) = stmt_list.tail_expr() {
                    self.walk_expr(&tail, nesting + 1);
                }
            }
        }
    }

    fn visit_while(&mut self, expr: &ast::WhileExpr, nesting: usize) {
        self.score += 1 + nesting;
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, nesting);
        }
        if let Some(body) = expr.loop_body() {
            if let Some(stmt_list) = body.stmt_list() {
                for stmt in stmt_list.statements() {
                    self.walk_stmt_with_ctx(&stmt, nesting + 1);
                }
                if let Some(tail) = stmt_list.tail_expr() {
                    self.walk_expr(&tail, nesting + 1);
                }
            }
        }
    }

    fn visit_loop(&mut self, expr: &ast::LoopExpr, nesting: usize) {
        self.score += 1 + nesting;
        if let Some(body) = expr.loop_body() {
            if let Some(stmt_list) = body.stmt_list() {
                for stmt in stmt_list.statements() {
                    self.walk_stmt_with_ctx(&stmt, nesting + 1);
                }
                if let Some(tail) = stmt_list.tail_expr() {
                    self.walk_expr(&tail, nesting + 1);
                }
            }
        }
    }

    fn visit_bin_expr(&mut self, expr: &ast::BinExpr, nesting: usize) {
        // +1 for && or || (no nesting penalty for boolean operators)
        if let Some(op) = expr.op_kind() {
            if matches!(op, ast::BinaryOp::LogicOp(_)) {
                self.score += 1;
            }
        }
        if let Some(lhs) = expr.lhs() {
            self.walk_expr(&lhs, nesting);
        }
        if let Some(rhs) = expr.rhs() {
            self.walk_expr(&rhs, nesting);
        }
    }

    fn visit_closure(&mut self, closure: &ast::ClosureExpr, nesting: usize) {
        // Closures increase nesting but don't add to the score
        if let Some(body) = closure.body() {
            self.walk_expr(&body, nesting + 1);
        }
    }

    fn visit_block_expr(&mut self, block: &ast::BlockExpr, nesting: usize) {
        // Pass nesting through to block contents
        if let Some(stmt_list) = block.stmt_list() {
            for stmt in stmt_list.statements() {
                self.walk_stmt_with_ctx(&stmt, nesting);
            }
            if let Some(tail) = stmt_list.tail_expr() {
                self.walk_expr(&tail, nesting);
            }
        }
    }
}
