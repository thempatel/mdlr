use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;

/// Count branch points in a function body for cyclomatic complexity.
///
/// Counting rules:
/// - `ExprKind::If` → +1 (covers `if` and `if let`)
/// - `ExprKind::Match` with `MatchSource::Normal` → arms - 1
///   (skip ForLoopDesugar, TryDesugar, etc. to avoid double-counting)
/// - `ExprKind::Loop` → +1 (covers `loop`, `while`, `for` — all desugar to Loop in HIR)
/// - `ExprKind::Binary(And|Or)` → +1 (short-circuit operators)
pub fn count_branches(tcx: TyCtxt<'_>, body: &hir::Body<'_>) -> usize {
    let mut count = 0;
    visit_expr_for_branches(tcx, body.value, &mut count);
    count
}

fn visit_expr_for_branches(tcx: TyCtxt<'_>, expr: &hir::Expr<'_>, count: &mut usize) {
    match &expr.kind {
        // if / if-let → +1
        hir::ExprKind::If(cond, then_branch, else_branch) => {
            *count += 1;
            visit_expr_for_branches(tcx, cond, count);
            visit_expr_for_branches(tcx, then_branch, count);
            if let Some(else_br) = else_branch {
                visit_expr_for_branches(tcx, else_br, count);
            }
        }

        // match — only count user-written match expressions (Normal source)
        hir::ExprKind::Match(scrutinee, arms, source) => {
            if *source == hir::MatchSource::Normal {
                // arms - 1 for cyclomatic complexity (N branches adds N-1 decision points)
                if arms.len() > 1 {
                    *count += arms.len() - 1;
                }
            }
            visit_expr_for_branches(tcx, scrutinee, count);
            for arm in arms.iter() {
                if let Some(guard) = &arm.guard {
                    visit_expr_for_branches(tcx, guard, count);
                }
                visit_expr_for_branches(tcx, arm.body, count);
            }
        }

        // loop/while/for all desugar to Loop in HIR → +1
        hir::ExprKind::Loop(block, _, _, _) => {
            *count += 1;
            for stmt in block.stmts {
                visit_stmt_for_branches(tcx, stmt, count);
            }
            if let Some(expr) = block.expr {
                visit_expr_for_branches(tcx, expr, count);
            }
        }

        // && and || short-circuit operators → +1
        hir::ExprKind::Binary(op, lhs, rhs) => {
            match op.node {
                hir::BinOpKind::And | hir::BinOpKind::Or => {
                    *count += 1;
                }
                _ => {}
            }
            visit_expr_for_branches(tcx, lhs, count);
            visit_expr_for_branches(tcx, rhs, count);
        }

        // Recurse into all other expression kinds

        hir::ExprKind::Block(block, _) => {
            for stmt in block.stmts {
                visit_stmt_for_branches(tcx, stmt, count);
            }
            if let Some(expr) = block.expr {
                visit_expr_for_branches(tcx, expr, count);
            }
        }

        hir::ExprKind::Call(func, args) => {
            visit_expr_for_branches(tcx, func, count);
            for arg in args.iter() {
                visit_expr_for_branches(tcx, arg, count);
            }
        }

        hir::ExprKind::MethodCall(_, receiver, args, _) => {
            visit_expr_for_branches(tcx, receiver, count);
            for arg in args.iter() {
                visit_expr_for_branches(tcx, arg, count);
            }
        }

        hir::ExprKind::Assign(lhs, rhs, _) => {
            visit_expr_for_branches(tcx, lhs, count);
            visit_expr_for_branches(tcx, rhs, count);
        }

        hir::ExprKind::AssignOp(_, lhs, rhs) => {
            visit_expr_for_branches(tcx, lhs, count);
            visit_expr_for_branches(tcx, rhs, count);
        }

        hir::ExprKind::Field(base, _) => {
            visit_expr_for_branches(tcx, base, count);
        }

        hir::ExprKind::Index(base, idx, _) => {
            visit_expr_for_branches(tcx, base, count);
            visit_expr_for_branches(tcx, idx, count);
        }

        hir::ExprKind::Unary(_, operand) => {
            visit_expr_for_branches(tcx, operand, count);
        }

        hir::ExprKind::AddrOf(_, _, operand) => {
            visit_expr_for_branches(tcx, operand, count);
        }

        hir::ExprKind::Ret(Some(expr)) => {
            visit_expr_for_branches(tcx, expr, count);
        }

        hir::ExprKind::Break(_, Some(expr)) => {
            visit_expr_for_branches(tcx, expr, count);
        }

        hir::ExprKind::Struct(_, fields, base) => {
            for field in fields.iter() {
                visit_expr_for_branches(tcx, field.expr, count);
            }
            if let hir::StructTailExpr::Base(base) = base {
                visit_expr_for_branches(tcx, base, count);
            }
        }

        hir::ExprKind::Tup(exprs) | hir::ExprKind::Array(exprs) => {
            for e in exprs.iter() {
                visit_expr_for_branches(tcx, e, count);
            }
        }

        hir::ExprKind::Repeat(expr, _) => {
            visit_expr_for_branches(tcx, expr, count);
        }

        hir::ExprKind::Cast(expr, _) | hir::ExprKind::Type(expr, _) => {
            visit_expr_for_branches(tcx, expr, count);
        }

        hir::ExprKind::Let(let_expr) => {
            visit_expr_for_branches(tcx, let_expr.init, count);
        }

        hir::ExprKind::DropTemps(expr) => {
            visit_expr_for_branches(tcx, expr, count);
        }

        hir::ExprKind::Yield(expr, _) => {
            visit_expr_for_branches(tcx, expr, count);
        }

        hir::ExprKind::Closure(closure) => {
            let body = tcx.hir_body(closure.body);
            visit_expr_for_branches(tcx, body.value, count);
        }

        // Leaf expressions — no sub-expressions to recurse into
        _ => {}
    }
}

fn visit_stmt_for_branches(tcx: TyCtxt<'_>, stmt: &hir::Stmt<'_>, count: &mut usize) {
    match &stmt.kind {
        hir::StmtKind::Let(local) => {
            if let Some(init) = local.init {
                visit_expr_for_branches(tcx, init, count);
            }
        }
        hir::StmtKind::Expr(expr) | hir::StmtKind::Semi(expr) => {
            visit_expr_for_branches(tcx, expr, count);
        }
        hir::StmtKind::Item(_) => {}
    }
}
