use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;

use crate::walk::ExprVisitor;

/// Count branch points in a function body for cyclomatic complexity.
///
/// Counting rules:
/// - `ExprKind::If` → +1 (covers `if` and `if let`)
/// - `ExprKind::Match` with `MatchSource::Normal` → arms - 1
///   (skip ForLoopDesugar, TryDesugar, etc. to avoid double-counting)
/// - `ExprKind::Loop` → +1 (covers `loop`, `while`, `for` — all desugar to Loop in HIR)
/// - `ExprKind::Binary(And|Or)` → +1 (short-circuit operators)
pub fn count_branches<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &hir::Body<'tcx>,
) -> usize {
    let mut visitor = BranchVisitor { tcx, count: 0 };
    visitor.walk_expr(body.value, ());
    visitor.count
}

struct BranchVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    count: usize,
}

impl<'tcx> ExprVisitor<'tcx> for BranchVisitor<'tcx> {
    type Ctx = ();

    fn tcx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_if(
        &mut self,
        cond: &hir::Expr<'tcx>,
        then_branch: &hir::Expr<'tcx>,
        else_branch: Option<&hir::Expr<'tcx>>,
        _ctx: (),
    ) {
        self.count += 1;
        self.walk_expr(cond, ());
        self.walk_expr(then_branch, ());
        if let Some(else_br) = else_branch {
            self.walk_expr(else_br, ());
        }
    }

    fn visit_match(
        &mut self,
        scrutinee: &hir::Expr<'tcx>,
        arms: &'tcx [hir::Arm<'tcx>],
        source: hir::MatchSource,
        _ctx: (),
    ) {
        if source == hir::MatchSource::Normal && arms.len() > 1 {
            self.count += arms.len() - 1;
        }
        self.walk_expr(scrutinee, ());
        for arm in arms.iter() {
            if let Some(guard) = &arm.guard {
                self.walk_expr(guard, ());
            }
            self.walk_expr(arm.body, ());
        }
    }

    fn visit_loop(&mut self, block: &hir::Block<'tcx>, _ctx: ()) {
        self.count += 1;
        self.walk_block(block);
    }

    fn visit_binary(
        &mut self,
        op: hir::BinOp,
        lhs: &hir::Expr<'tcx>,
        rhs: &hir::Expr<'tcx>,
        _ctx: (),
    ) {
        match op.node {
            hir::BinOpKind::And | hir::BinOpKind::Or => {
                self.count += 1;
            }
            _ => {}
        }
        self.walk_expr(lhs, ());
        self.walk_expr(rhs, ());
    }
}
