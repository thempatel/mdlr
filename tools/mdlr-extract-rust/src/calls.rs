use rustc_driver;
use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::DefId;

use crate::visitor::qualified_def_path_str;
use crate::walk::ExprVisitor;

/// Extract all call targets from a function/method body.
///
/// Uses typeck results to resolve calls to their fully-qualified DefId paths.
/// This is the primary advantage over tree-sitter: trait method calls are resolved
/// to their concrete implementations.
///
/// Returns `(calls, partial)` where `partial` is true if typeck failed and
/// call resolution could not be performed.
pub fn extract_calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    fn_def_id: DefId,
    body: &hir::Body<'tcx>,
) -> (Vec<String>, bool) {
    let mut calls = Vec::new();
    let typeck = match fn_def_id.as_local() {
        Some(local_id) => {
            // typeck may fail for functions with type errors — catch and degrade
            // gracefully by returning an empty call list marked as partial.
            match rustc_driver::catch_fatal_errors(|| tcx.typeck(local_id)) {
                Ok(results) => results,
                Err(_) => return (calls, true),
            }
        }
        None => return (calls, false),
    };

    let mut visitor = CallVisitor { tcx, typeck, calls: &mut calls };
    visitor.walk_expr(body.value, ());
    (calls, false)
}

struct CallVisitor<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck: &'tcx rustc_middle::ty::TypeckResults<'tcx>,
    calls: &'a mut Vec<String>,
}

impl<'a, 'tcx> ExprVisitor<'tcx> for CallVisitor<'a, 'tcx> {
    type Ctx = ();

    fn tcx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_call(
        &mut self,
        func: &hir::Expr<'tcx>,
        args: &'tcx [hir::Expr<'tcx>],
        _ctx: (),
    ) {
        if let Some(def_id) = resolve_call_expr(self.typeck, func) {
            let path = qualified_def_path_str(self.tcx, def_id);
            if !self.calls.contains(&path) {
                self.calls.push(path);
            }
        }
        self.walk_expr(func, ());
        for arg in args.iter() {
            self.walk_expr(arg, ());
        }
    }

    fn visit_method_call(
        &mut self,
        _segment: &'tcx hir::PathSegment<'tcx>,
        receiver: &hir::Expr<'tcx>,
        args: &'tcx [hir::Expr<'tcx>],
        _span: rustc_span::Span,
        hir_id: hir::HirId,
        _ctx: (),
    ) {
        if let Some(def_id) = self.typeck.type_dependent_def_id(hir_id) {
            let path = qualified_def_path_str(self.tcx, def_id);
            if !self.calls.contains(&path) {
                self.calls.push(path);
            }
        }
        self.walk_expr(receiver, ());
        for arg in args.iter() {
            self.walk_expr(arg, ());
        }
    }
}

/// Resolve a Call expression's function operand to a DefId.
fn resolve_call_expr(
    typeck: &rustc_middle::ty::TypeckResults<'_>,
    func_expr: &hir::Expr<'_>,
) -> Option<DefId> {
    match &func_expr.kind {
        hir::ExprKind::Path(qpath) => {
            let res = typeck.qpath_res(qpath, func_expr.hir_id);
            match res {
                hir::def::Res::Def(_, def_id) => Some(def_id),
                _ => None,
            }
        }
        _ => None,
    }
}
