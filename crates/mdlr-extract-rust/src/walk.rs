use ra_ap_syntax::ast::{self, HasArgList, HasLoopBody, RangeItem};

/// Shared CST expression walker with hook methods for customization.
///
/// Each visitor struct implements this trait, overriding only the hooks
/// it cares about. The default hook implementations perform standard
/// recursion using `Ctx::default()` (not the incoming `ctx`), so
/// context values like `FieldContext::WriteLhs` never leak into
/// unrelated arms.
pub trait CstVisitor {
    /// Per-expression context threaded through the walk.
    /// Use `()` when no context is needed; `FieldContext` for read/write tracking.
    type Ctx: Copy + Default;

    // --- Hook methods (override these) ---

    fn visit_if(&mut self, expr: &ast::IfExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
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

    fn visit_match(&mut self, expr: &ast::MatchExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
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

    fn visit_for(&mut self, expr: &ast::ForExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        if let Some(iterable) = expr.iterable() {
            self.walk_expr(&iterable, d);
        }
        if let Some(body) = expr.loop_body() {
            self.walk_block(&body, d);
        }
    }

    fn visit_while(&mut self, expr: &ast::WhileExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, d);
        }
        if let Some(body) = expr.loop_body() {
            self.walk_block(&body, d);
        }
    }

    fn visit_loop(&mut self, expr: &ast::LoopExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        if let Some(body) = expr.loop_body() {
            self.walk_block(&body, d);
        }
    }

    fn visit_block_expr(&mut self, block: &ast::BlockExpr, _ctx: Self::Ctx) {
        if let Some(stmt_list) = block.stmt_list() {
            self.walk_stmt_list(&stmt_list, Self::Ctx::default());
        }
    }

    fn visit_closure(&mut self, closure: &ast::ClosureExpr, _ctx: Self::Ctx) {
        if let Some(body) = closure.body() {
            self.walk_expr(&body, Self::Ctx::default());
        }
    }

    fn visit_bin_expr(&mut self, expr: &ast::BinExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        if let Some(lhs) = expr.lhs() {
            self.walk_expr(&lhs, d);
        }
        if let Some(rhs) = expr.rhs() {
            self.walk_expr(&rhs, d);
        }
    }

    fn visit_call(&mut self, expr: &ast::CallExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        if let Some(callee) = expr.expr() {
            self.walk_expr(&callee, d);
        }
        if let Some(arg_list) = expr.arg_list() {
            for arg in arg_list.args() {
                self.walk_expr(&arg, d);
            }
        }
    }

    fn visit_method_call(
        &mut self,
        expr: &ast::MethodCallExpr,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        if let Some(receiver) = expr.receiver() {
            self.walk_expr(&receiver, d);
        }
        if let Some(arg_list) = expr.arg_list() {
            for arg in arg_list.args() {
                self.walk_expr(&arg, d);
            }
        }
    }

    fn visit_field(&mut self, expr: &ast::FieldExpr, _ctx: Self::Ctx) {
        if let Some(base) = expr.expr() {
            self.walk_expr(&base, Self::Ctx::default());
        }
    }

    fn visit_assign(
        &mut self,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
        _ctx: Self::Ctx,
    ) {
        let d = Self::Ctx::default();
        self.walk_expr(lhs, d);
        self.walk_expr(rhs, d);
    }

    fn visit_index(&mut self, expr: &ast::IndexExpr, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        if let Some(base) = expr.base() {
            self.walk_expr(&base, d);
        }
        if let Some(idx) = expr.index() {
            self.walk_expr(&idx, d);
        }
    }

    // --- Walk methods (shared boilerplate) ---

    fn walk_expr(&mut self, expr: &ast::Expr, ctx: Self::Ctx) {
        match expr {
            ast::Expr::IfExpr(e) => self.visit_if(e, ctx),
            ast::Expr::MatchExpr(e) => self.visit_match(e, ctx),
            ast::Expr::ForExpr(e) => self.visit_for(e, ctx),
            ast::Expr::WhileExpr(e) => self.visit_while(e, ctx),
            ast::Expr::LoopExpr(e) => self.visit_loop(e, ctx),
            ast::Expr::BlockExpr(e) => self.visit_block_expr(e, ctx),
            ast::Expr::ClosureExpr(e) => self.visit_closure(e, ctx),
            ast::Expr::BinExpr(e) => {
                // Check if this is an assignment (=, +=, etc.)
                if let Some(op) = e.op_kind() {
                    if is_assign_op(op) {
                        if let (Some(lhs), Some(rhs)) = (e.lhs(), e.rhs()) {
                            self.visit_assign(&lhs, &rhs, ctx);
                            return;
                        }
                    }
                }
                self.visit_bin_expr(e, ctx);
            }
            ast::Expr::CallExpr(e) => self.visit_call(e, ctx),
            ast::Expr::MethodCallExpr(e) => self.visit_method_call(e, ctx),
            ast::Expr::FieldExpr(e) => self.visit_field(e, ctx),
            ast::Expr::IndexExpr(e) => self.visit_index(e, ctx),

            // --- Boilerplate recursion (no hooks needed) ---
            ast::Expr::PrefixExpr(e) => {
                if let Some(operand) = e.expr() {
                    self.walk_expr(&operand, Self::Ctx::default());
                }
            }
            ast::Expr::RefExpr(e) => {
                if let Some(operand) = e.expr() {
                    self.walk_expr(&operand, Self::Ctx::default());
                }
            }
            ast::Expr::ReturnExpr(e) => {
                if let Some(val) = e.expr() {
                    self.walk_expr(&val, Self::Ctx::default());
                }
            }
            ast::Expr::BreakExpr(e) => {
                if let Some(val) = e.expr() {
                    self.walk_expr(&val, Self::Ctx::default());
                }
            }
            ast::Expr::ParenExpr(e) => {
                if let Some(inner) = e.expr() {
                    self.walk_expr(&inner, Self::Ctx::default());
                }
            }
            ast::Expr::TupleExpr(e) => {
                let d = Self::Ctx::default();
                for field in e.fields() {
                    self.walk_expr(&field, d);
                }
            }
            ast::Expr::ArrayExpr(e) => {
                let d = Self::Ctx::default();
                for elem in e.exprs() {
                    self.walk_expr(&elem, d);
                }
            }
            ast::Expr::RecordExpr(e) => {
                let d = Self::Ctx::default();
                if let Some(field_list) = e.record_expr_field_list() {
                    for field in field_list.fields() {
                        if let Some(val) = field.expr() {
                            self.walk_expr(&val, d);
                        }
                    }
                    if let Some(spread) = field_list.spread() {
                        self.walk_expr(&spread, d);
                    }
                }
            }
            ast::Expr::CastExpr(e) => {
                if let Some(inner) = e.expr() {
                    self.walk_expr(&inner, Self::Ctx::default());
                }
            }
            ast::Expr::LetExpr(e) => {
                if let Some(init) = e.expr() {
                    self.walk_expr(&init, Self::Ctx::default());
                }
            }
            ast::Expr::AwaitExpr(e) => {
                if let Some(inner) = e.expr() {
                    self.walk_expr(&inner, Self::Ctx::default());
                }
            }
            ast::Expr::TryExpr(e) => {
                if let Some(inner) = e.expr() {
                    self.walk_expr(&inner, Self::Ctx::default());
                }
            }
            ast::Expr::YieldExpr(e) => {
                if let Some(inner) = e.expr() {
                    self.walk_expr(&inner, Self::Ctx::default());
                }
            }
            ast::Expr::RangeExpr(e) => {
                let d = Self::Ctx::default();
                if let Some(start) = e.start() {
                    self.walk_expr(&start, d);
                }
                if let Some(end) = e.end() {
                    self.walk_expr(&end, d);
                }
            }

            // Leaf expressions — no sub-expressions to recurse into
            _ => {}
        }
    }

    fn walk_block(&mut self, block: &ast::BlockExpr, ctx: Self::Ctx) {
        if let Some(stmt_list) = block.stmt_list() {
            self.walk_stmt_list(&stmt_list, ctx);
        }
    }

    fn walk_stmt_list(&mut self, stmt_list: &ast::StmtList, _ctx: Self::Ctx) {
        let d = Self::Ctx::default();
        for stmt in stmt_list.statements() {
            self.walk_stmt(&stmt, d);
        }
        if let Some(tail) = stmt_list.tail_expr() {
            self.walk_expr(&tail, d);
        }
    }

    fn walk_stmt(&mut self, stmt: &ast::Stmt, ctx: Self::Ctx) {
        match stmt {
            ast::Stmt::LetStmt(local) => {
                if let Some(init) = local.initializer() {
                    self.walk_expr(&init, ctx);
                }
            }
            ast::Stmt::ExprStmt(es) => {
                if let Some(expr) = es.expr() {
                    self.walk_expr(&expr, ctx);
                }
            }
            _ => {}
        }
    }
}

fn is_assign_op(op: ast::BinaryOp) -> bool {
    matches!(op, ast::BinaryOp::Assignment { .. })
}
