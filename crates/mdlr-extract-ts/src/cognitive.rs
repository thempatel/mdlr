use swc_ecma_ast::*;
use swc_ecma_visit::{Visit, VisitWith};

/// Compute cognitive complexity for a block statement.
pub fn compute_cognitive_block(block: &BlockStmt) -> usize {
    let mut visitor = CognitiveVisitor { score: 0, nesting: 0 };
    block.visit_with(&mut visitor);
    visitor.score
}

/// Compute cognitive complexity for a single expression (arrow body).
pub fn compute_cognitive_expr(expr: &Expr) -> usize {
    let mut visitor = CognitiveVisitor { score: 0, nesting: 0 };
    expr.visit_with(&mut visitor);
    visitor.score
}

struct CognitiveVisitor {
    score: usize,
    nesting: usize,
}

impl CognitiveVisitor {
    /// Visit children at increased nesting, then restore.
    fn with_nesting<F: FnOnce(&mut Self)>(&mut self, f: F) {
        self.nesting += 1;
        f(self);
        self.nesting -= 1;
    }
}

impl Visit for CognitiveVisitor {
    fn visit_if_stmt(&mut self, n: &IfStmt) {
        // +1 inherent + nesting penalty
        self.score += 1 + self.nesting;

        // Visit test at current nesting
        n.test.visit_with(self);

        // Visit consequent at increased nesting
        self.with_nesting(|this| {
            n.cons.visit_with(this);
        });

        // Handle else
        if let Some(alt) = &n.alt {
            // `else if` — walk at same nesting (the if inside will add its own +1)
            if is_if_stmt(alt) {
                alt.visit_with(self);
            } else {
                // Plain `else`: +1 inherent, no nesting penalty
                self.score += 1;
                self.with_nesting(|this| {
                    alt.visit_with(this);
                });
            }
        }
    }

    fn visit_switch_stmt(&mut self, n: &SwitchStmt) {
        self.score += 1 + self.nesting;
        n.discriminant.visit_with(self);
        self.with_nesting(|this| {
            for case in &n.cases {
                case.visit_with(this);
            }
        });
    }

    fn visit_for_stmt(&mut self, n: &ForStmt) {
        self.score += 1 + self.nesting;
        n.init.visit_with(self);
        n.test.visit_with(self);
        n.update.visit_with(self);
        self.with_nesting(|this| {
            n.body.visit_with(this);
        });
    }

    fn visit_for_in_stmt(&mut self, n: &ForInStmt) {
        self.score += 1 + self.nesting;
        n.right.visit_with(self);
        self.with_nesting(|this| {
            n.body.visit_with(this);
        });
    }

    fn visit_for_of_stmt(&mut self, n: &ForOfStmt) {
        self.score += 1 + self.nesting;
        n.right.visit_with(self);
        self.with_nesting(|this| {
            n.body.visit_with(this);
        });
    }

    fn visit_while_stmt(&mut self, n: &WhileStmt) {
        self.score += 1 + self.nesting;
        n.test.visit_with(self);
        self.with_nesting(|this| {
            n.body.visit_with(this);
        });
    }

    fn visit_do_while_stmt(&mut self, n: &DoWhileStmt) {
        self.score += 1 + self.nesting;
        self.with_nesting(|this| {
            n.body.visit_with(this);
        });
        n.test.visit_with(self);
    }

    fn visit_cond_expr(&mut self, n: &CondExpr) {
        // Ternary: +1 inherent + nesting penalty
        self.score += 1 + self.nesting;
        n.test.visit_with(self);
        self.with_nesting(|this| {
            n.cons.visit_with(this);
            n.alt.visit_with(this);
        });
    }

    fn visit_bin_expr(&mut self, n: &BinExpr) {
        // +1 for logical operators (no nesting penalty)
        match n.op {
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                self.score += 1;
            }
            _ => {}
        }
        n.visit_children_with(self);
    }

    fn visit_catch_clause(&mut self, n: &CatchClause) {
        self.score += 1 + self.nesting;
        self.with_nesting(|this| {
            n.body.visit_with(this);
        });
    }

    // Do NOT descend into nested function/arrow expressions — they are
    // separate units and their complexity is counted independently.
    fn visit_arrow_expr(&mut self, _n: &ArrowExpr) {}
    fn visit_fn_expr(&mut self, _n: &FnExpr) {}
    fn visit_fn_decl(&mut self, _n: &FnDecl) {}
}

/// Check if a statement is an `if` (for else-if chain detection).
fn is_if_stmt(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::If(_))
}
