use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;

/// Extract field reads and writes from a function/method body.
///
/// Detects `self.field` patterns:
/// - Read: `self.field` in any position except assignment LHS
/// - Write: `self.field` in assignment LHS (`=`, `+=`, etc.)
/// - `self.method()` is NOT a field read (it's a method call)
/// - `self.field.method()` — `field` IS a read
pub fn extract_field_access(tcx: TyCtxt<'_>, body: &hir::Body<'_>) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();

    visit_expr_for_fields(tcx, body.value, &mut reads, &mut writes, FieldContext::Read);

    (reads, writes)
}

/// Context for determining if a field access is a read or write.
#[derive(Clone, Copy, PartialEq)]
enum FieldContext {
    Read,
    WriteLhs,
}

fn visit_expr_for_fields<'tcx>(
    tcx: TyCtxt<'tcx>,
    expr: &hir::Expr<'_>,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
    ctx: FieldContext,
) {
    match &expr.kind {
        // self.field — check context for read vs write
        hir::ExprKind::Field(base, ident) => {
            if is_self_expr(base) {
                let field_name = ident.as_str().to_string();
                match ctx {
                    FieldContext::WriteLhs => {
                        if !writes.contains(&field_name) {
                            writes.push(field_name);
                        }
                    }
                    FieldContext::Read => {
                        if !reads.contains(&field_name) {
                            reads.push(field_name);
                        }
                    }
                }
            } else {
                // For chained access like self.field.subfield, the inner field IS a read
                visit_expr_for_fields(tcx, base, reads, writes, FieldContext::Read);
            }
        }

        // Method call: self.method() — the receiver is NOT a field read
        // But self.field.method() — field IS a read (handled by Field case above when
        // the receiver is Field(self, "field"))
        hir::ExprKind::MethodCall(_segment, receiver, args, _span) => {
            // Don't treat the direct receiver as a write context
            visit_expr_for_fields(tcx, receiver, reads, writes, FieldContext::Read);
            for arg in args.iter() {
                visit_expr_for_fields(tcx, arg, reads, writes, FieldContext::Read);
            }
        }

        // Assignment: LHS is write context
        hir::ExprKind::Assign(lhs, rhs, _) => {
            visit_expr_for_fields(tcx, lhs, reads, writes, FieldContext::WriteLhs);
            visit_expr_for_fields(tcx, rhs, reads, writes, FieldContext::Read);
        }
        hir::ExprKind::AssignOp(_, lhs, rhs) => {
            visit_expr_for_fields(tcx, lhs, reads, writes, FieldContext::WriteLhs);
            visit_expr_for_fields(tcx, rhs, reads, writes, FieldContext::Read);
        }

        // Function calls — visit all parts
        hir::ExprKind::Call(func, args) => {
            visit_expr_for_fields(tcx, func, reads, writes, FieldContext::Read);
            for arg in args.iter() {
                visit_expr_for_fields(tcx, arg, reads, writes, FieldContext::Read);
            }
        }

        // Block
        hir::ExprKind::Block(block, _) => {
            for stmt in block.stmts {
                visit_stmt_for_fields(tcx, stmt, reads, writes);
            }
            if let Some(expr) = block.expr {
                visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
            }
        }

        // If
        hir::ExprKind::If(cond, then_branch, else_branch) => {
            visit_expr_for_fields(tcx, cond, reads, writes, FieldContext::Read);
            visit_expr_for_fields(tcx, then_branch, reads, writes, FieldContext::Read);
            if let Some(else_br) = else_branch {
                visit_expr_for_fields(tcx, else_br, reads, writes, FieldContext::Read);
            }
        }

        // Match
        hir::ExprKind::Match(scrutinee, arms, _) => {
            visit_expr_for_fields(tcx, scrutinee, reads, writes, FieldContext::Read);
            for arm in arms.iter() {
                if let Some(guard) = &arm.guard {
                    visit_expr_for_fields(tcx, guard, reads, writes, FieldContext::Read);
                }
                visit_expr_for_fields(tcx, arm.body, reads, writes, FieldContext::Read);
            }
        }

        // Loop
        hir::ExprKind::Loop(block, _, _, _) => {
            for stmt in block.stmts {
                visit_stmt_for_fields(tcx, stmt, reads, writes);
            }
            if let Some(expr) = block.expr {
                visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
            }
        }

        // Binary
        hir::ExprKind::Binary(_, lhs, rhs) => {
            visit_expr_for_fields(tcx, lhs, reads, writes, FieldContext::Read);
            visit_expr_for_fields(tcx, rhs, reads, writes, FieldContext::Read);
        }

        // Unary
        hir::ExprKind::Unary(_, operand) => {
            visit_expr_for_fields(tcx, operand, reads, writes, FieldContext::Read);
        }

        // AddrOf / borrow
        hir::ExprKind::AddrOf(_, _, operand) => {
            visit_expr_for_fields(tcx, operand, reads, writes, ctx);
        }

        // Index
        hir::ExprKind::Index(base, idx, _) => {
            visit_expr_for_fields(tcx, base, reads, writes, ctx);
            visit_expr_for_fields(tcx, idx, reads, writes, FieldContext::Read);
        }

        // Return / break with value
        hir::ExprKind::Ret(Some(expr)) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }
        hir::ExprKind::Break(_, Some(expr)) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }

        // Struct literal
        hir::ExprKind::Struct(_, fields, base) => {
            for field in fields.iter() {
                visit_expr_for_fields(tcx, field.expr, reads, writes, FieldContext::Read);
            }
            if let hir::StructTailExpr::Base(base) = base {
                visit_expr_for_fields(tcx, base, reads, writes, FieldContext::Read);
            }
        }

        // Tuple
        hir::ExprKind::Tup(exprs) => {
            for e in exprs.iter() {
                visit_expr_for_fields(tcx, e, reads, writes, FieldContext::Read);
            }
        }

        // Array
        hir::ExprKind::Array(exprs) => {
            for e in exprs.iter() {
                visit_expr_for_fields(tcx, e, reads, writes, FieldContext::Read);
            }
        }

        // Repeat
        hir::ExprKind::Repeat(expr, _) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }

        // Cast / Type
        hir::ExprKind::Cast(expr, _) | hir::ExprKind::Type(expr, _) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }

        // Let expression
        hir::ExprKind::Let(let_expr) => {
            visit_expr_for_fields(tcx, let_expr.init, reads, writes, FieldContext::Read);
        }

        // DropTemps
        hir::ExprKind::DropTemps(expr) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }

        // Yield
        hir::ExprKind::Yield(expr, _) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }

        // Closure
        hir::ExprKind::Closure(closure) => {
            let body = tcx.hir_body(closure.body);
            visit_expr_for_fields(tcx, body.value, reads, writes, FieldContext::Read);
        }

        // Leaf / terminal expressions
        _ => {}
    }
}

fn visit_stmt_for_fields<'tcx>(
    tcx: TyCtxt<'tcx>,
    stmt: &hir::Stmt<'_>,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
) {
    match &stmt.kind {
        hir::StmtKind::Let(local) => {
            if let Some(init) = local.init {
                visit_expr_for_fields(tcx, init, reads, writes, FieldContext::Read);
            }
        }
        hir::StmtKind::Expr(expr) | hir::StmtKind::Semi(expr) => {
            visit_expr_for_fields(tcx, expr, reads, writes, FieldContext::Read);
        }
        hir::StmtKind::Item(_) => {}
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
