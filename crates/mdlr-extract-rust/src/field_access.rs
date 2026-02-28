use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;

use crate::walk::ExprVisitor;

/// Extract field reads and writes from a function/method body.
///
/// Detects `self.field` patterns:
/// - Read: `self.field` in any position except assignment LHS
/// - Write: `self.field` in assignment LHS (`=`, `+=`, etc.)
/// - `self.method()` is NOT a field read (it's a method call)
/// - `self.field.method()` — `field` IS a read
pub fn extract_field_access<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &hir::Body<'tcx>,
) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();

    let mut visitor =
        FieldAccessVisitor { tcx, reads: &mut reads, writes: &mut writes };
    visitor.walk_expr(body.value, FieldContext::Read);
    (reads, writes)
}

/// Context for determining if a field access is a read or write.
#[derive(Clone, Copy, PartialEq, Default)]
enum FieldContext {
    #[default]
    Read,
    WriteLhs,
}

struct FieldAccessVisitor<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    reads: &'a mut Vec<String>,
    writes: &'a mut Vec<String>,
}

impl<'a, 'tcx> ExprVisitor<'tcx> for FieldAccessVisitor<'a, 'tcx> {
    type Ctx = FieldContext;

    fn tcx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_field(
        &mut self,
        base: &hir::Expr<'tcx>,
        ident: rustc_span::Ident,
        ctx: FieldContext,
    ) {
        if is_self_expr(base) {
            let field_name = ident.as_str().to_string();
            match ctx {
                FieldContext::WriteLhs => {
                    if !self.writes.contains(&field_name) {
                        self.writes.push(field_name);
                    }
                }
                FieldContext::Read => {
                    if !self.reads.contains(&field_name) {
                        self.reads.push(field_name);
                    }
                }
            }
        } else {
            // For chained access like self.field.subfield, the inner field IS a read
            self.walk_expr(base, FieldContext::Read);
        }
    }

    fn visit_assign(
        &mut self,
        lhs: &hir::Expr<'tcx>,
        rhs: &hir::Expr<'tcx>,
        _ctx: FieldContext,
    ) {
        self.walk_expr(lhs, FieldContext::WriteLhs);
        self.walk_expr(rhs, FieldContext::Read);
    }

    fn visit_assign_op(
        &mut self,
        _op: hir::AssignOp,
        lhs: &hir::Expr<'tcx>,
        rhs: &hir::Expr<'tcx>,
        _ctx: FieldContext,
    ) {
        self.walk_expr(lhs, FieldContext::WriteLhs);
        self.walk_expr(rhs, FieldContext::Read);
    }

    fn visit_addr_of(&mut self, operand: &hir::Expr<'tcx>, ctx: FieldContext) {
        self.walk_expr(operand, ctx);
    }

    fn visit_index(
        &mut self,
        base: &hir::Expr<'tcx>,
        idx: &hir::Expr<'tcx>,
        ctx: FieldContext,
    ) {
        self.walk_expr(base, ctx);
        self.walk_expr(idx, FieldContext::Read);
    }

    fn visit_method_call(
        &mut self,
        _segment: &'tcx hir::PathSegment<'tcx>,
        receiver: &hir::Expr<'tcx>,
        args: &'tcx [hir::Expr<'tcx>],
        _span: rustc_span::Span,
        _hir_id: hir::HirId,
        _ctx: FieldContext,
    ) {
        // Don't treat the direct receiver as a write context
        self.walk_expr(receiver, FieldContext::Read);
        for arg in args.iter() {
            self.walk_expr(arg, FieldContext::Read);
        }
    }
}

/// Check if an expression refers to `self`.
fn is_self_expr(expr: &hir::Expr<'_>) -> bool {
    match &expr.kind {
        hir::ExprKind::Path(hir::QPath::Resolved(_, path)) => {
            if let Some(segment) = path.segments.last() {
                segment.ident.as_str() == "self"
            } else {
                false
            }
        }
        // Handle deref: *self
        hir::ExprKind::Unary(hir::UnOp::Deref, inner) => is_self_expr(inner),
        _ => false,
    }
}
