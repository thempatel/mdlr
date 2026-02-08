use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::DefId;

/// Extract all call targets from a function/method body.
///
/// Uses typeck results to resolve calls to their fully-qualified DefId paths.
/// This is the primary advantage over tree-sitter: trait method calls are resolved
/// to their concrete implementations.
pub fn extract_calls(tcx: TyCtxt<'_>, fn_def_id: DefId, body: &hir::Body<'_>) -> Vec<String> {
    let mut calls = Vec::new();
    let typeck = match fn_def_id.as_local() {
        Some(local_id) => tcx.typeck(local_id),
        None => return calls,
    };

    visit_expr_for_calls(tcx, typeck, body.value, &mut calls);
    calls
}

fn visit_expr_for_calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    typeck: &rustc_middle::ty::TypeckResults<'tcx>,
    expr: &hir::Expr<'_>,
    calls: &mut Vec<String>,
) {
    match &expr.kind {
        // Direct function calls: foo(), Struct::method(), etc.
        hir::ExprKind::Call(func, args) => {
            if let Some(def_id) = resolve_call_expr(typeck, func) {
                let path = tcx.def_path_str(def_id);
                if !calls.contains(&path) {
                    calls.push(path);
                }
            }
            // Visit the function expression and arguments recursively
            visit_expr_for_calls(tcx, typeck, func, calls);
            for arg in args.iter() {
                visit_expr_for_calls(tcx, typeck, arg, calls);
            }
        }
        // Method calls: x.method(args)
        hir::ExprKind::MethodCall(_segment, receiver, args, _span) => {
            if let Some(def_id) = typeck.type_dependent_def_id(expr.hir_id) {
                let path = tcx.def_path_str(def_id);
                if !calls.contains(&path) {
                    calls.push(path);
                }
            }
            visit_expr_for_calls(tcx, typeck, receiver, calls);
            for arg in args.iter() {
                visit_expr_for_calls(tcx, typeck, arg, calls);
            }
        }
        // Block expressions
        hir::ExprKind::Block(block, _) => {
            for stmt in block.stmts {
                visit_stmt_for_calls(tcx, typeck, stmt, calls);
            }
            if let Some(expr) = block.expr {
                visit_expr_for_calls(tcx, typeck, expr, calls);
            }
        }
        // If expressions
        hir::ExprKind::If(cond, then_branch, else_branch) => {
            visit_expr_for_calls(tcx, typeck, cond, calls);
            visit_expr_for_calls(tcx, typeck, then_branch, calls);
            if let Some(else_br) = else_branch {
                visit_expr_for_calls(tcx, typeck, else_br, calls);
            }
        }
        // Match expressions
        hir::ExprKind::Match(scrutinee, arms, _) => {
            visit_expr_for_calls(tcx, typeck, scrutinee, calls);
            for arm in arms.iter() {
                if let Some(guard) = &arm.guard {
                    visit_expr_for_calls(tcx, typeck, guard, calls);
                }
                visit_expr_for_calls(tcx, typeck, arm.body, calls);
            }
        }
        // Loop
        hir::ExprKind::Loop(block, _, _, _) => {
            for stmt in block.stmts {
                visit_stmt_for_calls(tcx, typeck, stmt, calls);
            }
            if let Some(expr) = block.expr {
                visit_expr_for_calls(tcx, typeck, expr, calls);
            }
        }
        // Binary ops (don't contain calls but recurse)
        hir::ExprKind::Binary(_, lhs, rhs) => {
            visit_expr_for_calls(tcx, typeck, lhs, calls);
            visit_expr_for_calls(tcx, typeck, rhs, calls);
        }
        // Unary ops
        hir::ExprKind::Unary(_, operand) => {
            visit_expr_for_calls(tcx, typeck, operand, calls);
        }
        // Assignment
        hir::ExprKind::Assign(lhs, rhs, _) => {
            visit_expr_for_calls(tcx, typeck, lhs, calls);
            visit_expr_for_calls(tcx, typeck, rhs, calls);
        }
        hir::ExprKind::AssignOp(_, lhs, rhs) => {
            visit_expr_for_calls(tcx, typeck, lhs, calls);
            visit_expr_for_calls(tcx, typeck, rhs, calls);
        }
        // Field access
        hir::ExprKind::Field(base, _) => {
            visit_expr_for_calls(tcx, typeck, base, calls);
        }
        // Index
        hir::ExprKind::Index(base, idx, _) => {
            visit_expr_for_calls(tcx, typeck, base, calls);
            visit_expr_for_calls(tcx, typeck, idx, calls);
        }
        // Address-of / borrow
        hir::ExprKind::AddrOf(_, _, operand) => {
            visit_expr_for_calls(tcx, typeck, operand, calls);
        }
        // Break / return with value
        hir::ExprKind::Break(_, Some(expr)) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        hir::ExprKind::Ret(Some(expr)) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        // Struct literal
        hir::ExprKind::Struct(_, fields, base) => {
            for field in fields.iter() {
                visit_expr_for_calls(tcx, typeck, field.expr, calls);
            }
            if let hir::StructTailExpr::Base(base) = base {
                visit_expr_for_calls(tcx, typeck, base, calls);
            }
        }
        // Tuple
        hir::ExprKind::Tup(exprs) => {
            for e in exprs.iter() {
                visit_expr_for_calls(tcx, typeck, e, calls);
            }
        }
        // Array
        hir::ExprKind::Array(exprs) => {
            for e in exprs.iter() {
                visit_expr_for_calls(tcx, typeck, e, calls);
            }
        }
        // Repeat
        hir::ExprKind::Repeat(expr, _) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        // Cast
        hir::ExprKind::Cast(expr, _) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        // Type
        hir::ExprKind::Type(expr, _) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        // Let expression (if-let guard)
        hir::ExprKind::Let(let_expr) => {
            visit_expr_for_calls(tcx, typeck, let_expr.init, calls);
        }
        // DropTemps (compiler-inserted)
        hir::ExprKind::DropTemps(expr) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        // Yield
        hir::ExprKind::Yield(expr, _) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        // Closure — visit the body
        hir::ExprKind::Closure(closure) => {
            let body = tcx.hir_body(closure.body);
            visit_expr_for_calls(tcx, typeck, body.value, calls);
        }
        // Leaf expressions that don't contain sub-expressions
        hir::ExprKind::Path(_)
        | hir::ExprKind::Lit(_)
        | hir::ExprKind::Err(_)
        | hir::ExprKind::Continue(_)
        | hir::ExprKind::Break(_, None)
        | hir::ExprKind::Ret(None)
        | hir::ExprKind::ConstBlock(_)
        | hir::ExprKind::Become(_)
        | hir::ExprKind::InlineAsm(_)
        | hir::ExprKind::OffsetOf(_, _)
        | hir::ExprKind::UnsafeBinderCast(_, _, _) => {}

        // Catch-all for any future variants
        _ => {}
    }
}

fn visit_stmt_for_calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    typeck: &rustc_middle::ty::TypeckResults<'tcx>,
    stmt: &hir::Stmt<'_>,
    calls: &mut Vec<String>,
) {
    match &stmt.kind {
        hir::StmtKind::Let(local) => {
            if let Some(init) = local.init {
                visit_expr_for_calls(tcx, typeck, init, calls);
            }
        }
        hir::StmtKind::Expr(expr) | hir::StmtKind::Semi(expr) => {
            visit_expr_for_calls(tcx, typeck, expr, calls);
        }
        hir::StmtKind::Item(_) => {}
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
