use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;

use crate::walk::ExprVisitor;

/// Find the largest single scope block within a function body.
///
/// Measures the line count of each scope-creating expression:
/// - `if` then/else bodies
/// - `match` arm bodies
/// - `loop`/`while`/`for` bodies
/// - Block expressions (`{}`)
/// - Closures
///
/// The function's own top-level block is excluded (that's `function_size`).
/// Returns 0 for functions with no nested scope blocks.
pub fn max_scope_lines<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &hir::Body<'tcx>,
) -> usize {
    let mut visitor = ScopeVisitor { tcx, max: 0 };
    // Walk into the top-level block's contents without measuring
    // the block itself (which would duplicate function_size).
    if let hir::ExprKind::Block(block, _) = &body.value.kind {
        visitor.walk_block(block);
    } else {
        // Expression-bodied function (e.g. closure) — walk directly
        visitor.walk_expr(body.value, ());
    }
    visitor.max
}

struct ScopeVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    max: usize,
}

/// Compute the line count of a span, returning 0 for macro-expanded or dummy spans.
fn span_lines(tcx: TyCtxt<'_>, span: rustc_span::Span) -> usize {
    if span.from_expansion() || span.is_dummy() {
        return 0;
    }
    let sm = tcx.sess.source_map();
    let lo = sm.lookup_char_pos(span.lo());
    let hi = sm.lookup_char_pos(span.hi());
    hi.line.saturating_sub(lo.line) + 1
}

/// Record a scope span, updating max if it's larger.
fn record_scope(tcx: TyCtxt<'_>, span: rustc_span::Span, max: &mut usize) {
    let lines = span_lines(tcx, span);
    if lines > *max {
        *max = lines;
    }
}

impl<'tcx> ExprVisitor<'tcx> for ScopeVisitor<'tcx> {
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
        record_scope(self.tcx, then_branch.span, &mut self.max);
        self.walk_expr(cond, ());
        self.walk_expr(then_branch, ());
        if let Some(else_br) = else_branch {
            record_scope(self.tcx, else_br.span, &mut self.max);
            self.walk_expr(else_br, ());
        }
    }

    fn visit_match(
        &mut self,
        scrutinee: &hir::Expr<'tcx>,
        arms: &'tcx [hir::Arm<'tcx>],
        _source: hir::MatchSource,
        _ctx: (),
    ) {
        self.walk_expr(scrutinee, ());
        for arm in arms.iter() {
            record_scope(self.tcx, arm.body.span, &mut self.max);
            if let Some(guard) = &arm.guard {
                self.walk_expr(guard, ());
            }
            self.walk_expr(arm.body, ());
        }
    }

    fn visit_loop(&mut self, block: &hir::Block<'tcx>, _ctx: ()) {
        record_scope(self.tcx, block.span, &mut self.max);
        self.walk_block(block);
    }

    fn visit_block_expr(&mut self, block: &hir::Block<'tcx>, _ctx: ()) {
        record_scope(self.tcx, block.span, &mut self.max);
        self.walk_block(block);
    }

    fn visit_closure(&mut self, closure: &'tcx hir::Closure<'tcx>, _ctx: ()) {
        let body = self.tcx.hir_body(closure.body);
        record_scope(self.tcx, body.value.span, &mut self.max);
        self.walk_expr(body.value, ());
    }
}
