use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;

/// Shared HIR expression walker with hook methods for customization.
///
/// Each visitor struct implements this trait, overriding only the hooks
/// it cares about. The default hook implementations perform standard
/// recursion using `Ctx::default()` (not the incoming `ctx`), so
/// context values like `FieldContext::WriteLhs` never leak into
/// unrelated arms.
pub trait ExprVisitor<'tcx> {
    /// Per-expression context threaded through the walk.
    /// Use `()` when no context is needed; `FieldContext` for read/write tracking.
    type Ctx: Copy + Default;

    fn tcx(&self) -> TyCtxt<'tcx>;

    // --- Hook methods (override these) ---

    fn visit_if(
        &mut self,
        cond: &hir::Expr<'tcx>,
        then_branch: &hir::Expr<'tcx>,
        else_branch: Option<&hir::Expr<'tcx>>,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(cond, d);
        self.walk_expr(then_branch, d);
        if let Some(else_br) = else_branch {
            self.walk_expr(else_br, d);
        }
    }

    fn visit_match(
        &mut self,
        scrutinee: &hir::Expr<'tcx>,
        arms: &'tcx [hir::Arm<'tcx>],
        _source: hir::MatchSource,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(scrutinee, d);
        for arm in arms.iter() {
            if let Some(guard) = &arm.guard {
                self.walk_expr(guard, d);
            }
            self.walk_expr(arm.body, d);
        }
    }

    fn visit_loop(&mut self, block: &hir::Block<'tcx>, _ctx: Self::Ctx) {
        self.walk_block(block);
    }

    fn visit_block_expr(&mut self, block: &hir::Block<'tcx>, _ctx: Self::Ctx) {
        self.walk_block(block);
    }

    fn visit_closure(
        &mut self,
        closure: &'tcx hir::Closure<'tcx>,
        _ctx: Self::Ctx,
    ) {
        let body = self.tcx().hir_body(closure.body);
        self.walk_expr(body.value, Self::Ctx::default());
    }

    fn visit_binary(
        &mut self,
        _op: hir::BinOp,
        lhs: &hir::Expr<'tcx>,
        rhs: &hir::Expr<'tcx>,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(lhs, d);
        self.walk_expr(rhs, d);
    }

    fn visit_call(
        &mut self,
        func: &hir::Expr<'tcx>,
        args: &'tcx [hir::Expr<'tcx>],
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(func, d);
        for arg in args.iter() {
            self.walk_expr(arg, d);
        }
    }

    fn visit_method_call(
        &mut self,
        _segment: &'tcx hir::PathSegment<'tcx>,
        receiver: &hir::Expr<'tcx>,
        args: &'tcx [hir::Expr<'tcx>],
        _span: rustc_span::Span,
        _hir_id: hir::HirId,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(receiver, d);
        for arg in args.iter() {
            self.walk_expr(arg, d);
        }
    }

    fn visit_field(
        &mut self,
        base: &hir::Expr<'tcx>,
        _ident: rustc_span::Ident,
        _ctx: Self::Ctx,
    ) {
        self.walk_expr(base, Self::Ctx::default());
    }

    fn visit_assign(
        &mut self,
        lhs: &hir::Expr<'tcx>,
        rhs: &hir::Expr<'tcx>,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(lhs, d);
        self.walk_expr(rhs, d);
    }

    fn visit_assign_op(
        &mut self,
        _op: hir::AssignOp,
        lhs: &hir::Expr<'tcx>,
        rhs: &hir::Expr<'tcx>,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(lhs, d);
        self.walk_expr(rhs, d);
    }

    fn visit_addr_of(&mut self, operand: &hir::Expr<'tcx>, _ctx: Self::Ctx) {
        self.walk_expr(operand, Self::Ctx::default());
    }

    fn visit_index(
        &mut self,
        base: &hir::Expr<'tcx>,
        idx: &hir::Expr<'tcx>,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(base, d);
        self.walk_expr(idx, d);
    }

    // --- Walk methods (shared boilerplate) ---

    fn walk_expr(&mut self, expr: &hir::Expr<'tcx>, ctx: Self::Ctx) {
        match &expr.kind {
            hir::ExprKind::If(cond, then_branch, else_branch) => {
                self.visit_if(cond, then_branch, else_branch.as_deref(), ctx);
            }
            hir::ExprKind::Match(scrutinee, arms, source) => {
                self.visit_match(scrutinee, arms, *source, ctx);
            }
            hir::ExprKind::Loop(block, _, _, _) => {
                self.visit_loop(block, ctx);
            }
            hir::ExprKind::Block(block, _) => {
                self.visit_block_expr(block, ctx);
            }
            hir::ExprKind::Closure(closure) => {
                self.visit_closure(closure, ctx);
            }
            hir::ExprKind::Binary(op, lhs, rhs) => {
                self.visit_binary(*op, lhs, rhs, ctx);
            }
            hir::ExprKind::Call(func, args) => {
                self.visit_call(func, args, ctx);
            }
            hir::ExprKind::MethodCall(segment, receiver, args, span) => {
                self.visit_method_call(
                    segment,
                    receiver,
                    args,
                    *span,
                    expr.hir_id,
                    ctx,
                );
            }
            hir::ExprKind::Field(base, ident) => {
                self.visit_field(base, *ident, ctx);
            }
            hir::ExprKind::Assign(lhs, rhs, _) => {
                self.visit_assign(lhs, rhs, ctx);
            }
            hir::ExprKind::AssignOp(op, lhs, rhs) => {
                self.visit_assign_op(*op, lhs, rhs, ctx);
            }
            hir::ExprKind::AddrOf(_, _, operand) => {
                self.visit_addr_of(operand, ctx);
            }
            hir::ExprKind::Index(base, idx, _) => {
                self.visit_index(base, idx, ctx);
            }

            // --- Boilerplate recursion (no hooks needed) ---
            hir::ExprKind::Unary(_, operand) => {
                self.walk_expr(operand, Self::Ctx::default());
            }
            hir::ExprKind::Ret(Some(expr)) => {
                self.walk_expr(expr, Self::Ctx::default());
            }
            hir::ExprKind::Break(_, Some(expr)) => {
                self.walk_expr(expr, Self::Ctx::default());
            }
            hir::ExprKind::Struct(_, fields, base) => {
                let d = Self::Ctx::default();
                for field in fields.iter() {
                    self.walk_expr(field.expr, d);
                }
                if let hir::StructTailExpr::Base(base) = base {
                    self.walk_expr(base, d);
                }
            }
            hir::ExprKind::Tup(exprs) | hir::ExprKind::Array(exprs) => {
                let d = Self::Ctx::default();
                for e in exprs.iter() {
                    self.walk_expr(e, d);
                }
            }
            hir::ExprKind::Repeat(expr, _) => {
                self.walk_expr(expr, Self::Ctx::default());
            }
            hir::ExprKind::Cast(expr, _) | hir::ExprKind::Type(expr, _) => {
                self.walk_expr(expr, Self::Ctx::default());
            }
            hir::ExprKind::Let(let_expr) => {
                self.walk_expr(let_expr.init, Self::Ctx::default());
            }
            hir::ExprKind::DropTemps(expr) => {
                self.walk_expr(expr, Self::Ctx::default());
            }
            hir::ExprKind::Yield(expr, _) => {
                self.walk_expr(expr, Self::Ctx::default());
            }

            // Leaf expressions — no sub-expressions to recurse into
            _ => {}
        }
    }

    fn walk_block(&mut self, block: &hir::Block<'tcx>) {
        for stmt in block.stmts {
            self.walk_stmt(stmt);
        }
        if let Some(expr) = block.expr {
            self.walk_expr(expr, Self::Ctx::default());
        }
    }

    fn walk_stmt(&mut self, stmt: &hir::Stmt<'tcx>) {
        match &stmt.kind {
            hir::StmtKind::Let(local) => {
                if let Some(init) = local.init {
                    self.walk_expr(init, Self::Ctx::default());
                }
            }
            hir::StmtKind::Expr(expr) | hir::StmtKind::Semi(expr) => {
                self.walk_expr(expr, Self::Ctx::default());
            }
            hir::StmtKind::Item(_) => {}
        }
    }
}
