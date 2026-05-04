use ra_ap_syntax::ast::{self, HasLoopBody};

use crate::walk::CstVisitor;

/// Count branch points in a function body for cyclomatic complexity.
///
/// Counting rules:
/// - `if` / `if let` → +1
/// - `match` → arms - 1  (only if >1 arm)
/// - `for` / `while` / `loop` → +1
/// - `&&` / `||` → +1 (short-circuit operators)
pub fn count_branches(body: &ast::BlockExpr) -> usize {
    let mut visitor = BranchVisitor { count: 0 };
    visitor.walk_block(body, ());
    visitor.count
}

struct BranchVisitor {
    count: usize,
}

impl CstVisitor for BranchVisitor {
    type Ctx = ();

    fn visit_if(&mut self, expr: &ast::IfExpr, _ctx: ()) {
        self.count += 1;
        let d = ();
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, d);
        }
        if let Some(then_branch) = expr.then_branch() {
            self.walk_block(&then_branch, d);
        }
        if let Some(else_branch) = expr.else_branch() {
            match else_branch {
                ast::ElseBranch::Block(block) => self.walk_block(&block, d),
                ast::ElseBranch::IfExpr(elif) => {
                    self.walk_expr(&ast::Expr::from(elif), d)
                }
            }
        }
    }

    fn visit_match(&mut self, expr: &ast::MatchExpr, _ctx: ()) {
        if let Some(arm_list) = expr.match_arm_list() {
            let arm_count = arm_list.arms().count();
            if arm_count > 1 {
                self.count += arm_count - 1;
            }
        }
        let d = ();
        if let Some(scrutinee) = expr.expr() {
            self.walk_expr(&scrutinee, d);
        }
        if let Some(arm_list) = expr.match_arm_list() {
            for arm in arm_list.arms() {
                if let Some(guard) = arm.guard() {
                    if let Some(guard_expr) = guard.condition() {
                        self.walk_expr(&guard_expr, d);
                    }
                }
                if let Some(body) = arm.expr() {
                    self.walk_expr(&body, d);
                }
            }
        }
    }

    fn visit_for(&mut self, expr: &ast::ForExpr, _ctx: ()) {
        self.count += 1;
        let d = ();
        if let Some(iterable) = expr.iterable() {
            self.walk_expr(&iterable, d);
        }
        if let Some(body) = expr.loop_body() {
            self.walk_block(&body, d);
        }
    }

    fn visit_while(&mut self, expr: &ast::WhileExpr, _ctx: ()) {
        self.count += 1;
        let d = ();
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, d);
        }
        if let Some(body) = expr.loop_body() {
            self.walk_block(&body, d);
        }
    }

    fn visit_loop(&mut self, expr: &ast::LoopExpr, _ctx: ()) {
        self.count += 1;
        if let Some(body) = expr.loop_body() {
            self.walk_block(&body, ());
        }
    }

    fn visit_bin_expr(&mut self, expr: &ast::BinExpr, _ctx: ()) {
        if let Some(op) = expr.op_kind() {
            if matches!(op, ast::BinaryOp::LogicOp(_)) {
                self.count += 1;
            }
        }
        let d = ();
        if let Some(lhs) = expr.lhs() {
            self.walk_expr(&lhs, d);
        }
        if let Some(rhs) = expr.rhs() {
            self.walk_expr(&rhs, d);
        }
    }
}
