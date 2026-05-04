use swc_ecma_ast::*;
use swc_ecma_visit::{Visit, VisitWith};

/// Extract `this.field` and `param.field` reads and writes from a block statement.
pub fn extract_field_access_block(
    block: &BlockStmt,
    param_names: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut visitor = FieldAccessVisitor {
        reads: Vec::new(),
        writes: Vec::new(),
        param_names: param_names.to_vec(),
    };
    block.visit_with(&mut visitor);
    (visitor.reads, visitor.writes)
}

/// Extract `this.field` and `param.field` reads and writes from a single expression.
pub fn extract_field_access_expr(
    expr: &Expr,
    param_names: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut visitor = FieldAccessVisitor {
        reads: Vec::new(),
        writes: Vec::new(),
        param_names: param_names.to_vec(),
    };
    expr.visit_with(&mut visitor);
    (visitor.reads, visitor.writes)
}

struct FieldAccessVisitor {
    reads: Vec<String>,
    writes: Vec<String>,
    param_names: Vec<String>,
}

/// Check if an expression is `this.field` and return the field name.
fn this_field_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Member(member) => {
            if matches!(&*member.obj, Expr::This(_)) {
                match &member.prop {
                    MemberProp::Ident(ident) => Some(ident.sym.to_string()),
                    _ => None,
                }
            } else {
                None
            }
        }
        // this?.field
        Expr::OptChain(opt) => match &*opt.base {
            OptChainBase::Member(member) => {
                if matches!(&*member.obj, Expr::This(_)) {
                    match &member.prop {
                        MemberProp::Ident(ident) => {
                            Some(ident.sym.to_string())
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Check if an expression is `param.field` where param is a known parameter name.
/// Returns `"param.field"` if matched.
fn param_field_name(expr: &Expr, param_names: &[String]) -> Option<String> {
    match expr {
        Expr::Member(member) => {
            if let Expr::Ident(ident) = &*member.obj {
                let name = ident.sym.to_string();
                if param_names.contains(&name) {
                    if let MemberProp::Ident(prop) = &member.prop {
                        return Some(format!("{name}.{}", prop.sym));
                    }
                }
            }
            None
        }
        Expr::OptChain(opt) => match &*opt.base {
            OptChainBase::Member(member) => {
                if let Expr::Ident(ident) = &*member.obj {
                    let name = ident.sym.to_string();
                    if param_names.contains(&name) {
                        if let MemberProp::Ident(prop) = &member.prop {
                            return Some(format!("{name}.{}", prop.sym));
                        }
                    }
                }
                None
            }
            _ => None,
        },
        _ => None,
    }
}

/// Check if expr is a call like `this.method(...)` or `param.method(...)`.
/// In that case `method` is NOT a field read.
fn is_receiver_method_call(expr: &Expr, param_names: &[String]) -> bool {
    match expr {
        Expr::Member(member) => {
            if matches!(&*member.obj, Expr::This(_)) {
                return true;
            }
            if let Expr::Ident(ident) = &*member.obj {
                return param_names.contains(&ident.sym.to_string());
            }
            false
        }
        _ => false,
    }
}

impl FieldAccessVisitor {
    fn record_read(&mut self, name: String) {
        if !self.reads.contains(&name) {
            self.reads.push(name);
        }
    }

    fn record_write(&mut self, name: String) {
        if !self.writes.contains(&name) {
            self.writes.push(name);
        }
    }

    /// Walk an expression collecting this.field reads, but skip the
    /// immediate this.field at the top (it was already recorded as write).
    fn walk_read_context(&mut self, expr: &Expr) {
        expr.visit_with(self);
    }
}

impl Visit for FieldAccessVisitor {
    fn visit_assign_expr(&mut self, n: &AssignExpr) {
        // LHS: check for this.field or param.field write
        match &n.left {
            AssignTarget::Simple(simple) => match simple {
                SimpleAssignTarget::Member(member) => {
                    let member_expr = Expr::Member(member.clone());
                    if let Some(name) = this_field_name(&member_expr) {
                        self.record_write(name);
                        self.walk_read_context(&n.right);
                        return;
                    }
                    if let Some(name) =
                        param_field_name(&member_expr, &self.param_names)
                    {
                        self.record_write(name);
                        self.walk_read_context(&n.right);
                        return;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        // Default recursion
        n.visit_children_with(self);
    }

    fn visit_update_expr(&mut self, n: &UpdateExpr) {
        // this.field++ / ++this.field → write
        if let Some(name) = this_field_name(&n.arg) {
            self.record_write(name);
            return;
        }
        // param.field++ / ++param.field → write
        if let Some(name) = param_field_name(&n.arg, &self.param_names) {
            self.record_write(name);
            return;
        }
        n.visit_children_with(self);
    }

    fn visit_member_expr(&mut self, n: &MemberExpr) {
        if matches!(&*n.obj, Expr::This(_)) {
            if let MemberProp::Ident(ident) = &n.prop {
                // Only record as read — writes are handled in visit_assign_expr
                self.record_read(ident.sym.to_string());
                return;
            }
        }
        // param.field → read (writes handled in visit_assign_expr)
        if let Expr::Ident(ident) = &*n.obj {
            if self.param_names.contains(&ident.sym.to_string()) {
                if let MemberProp::Ident(prop) = &n.prop {
                    self.record_read(format!("{}.{}", ident.sym, prop.sym));
                    return;
                }
            }
        }
        // For chained access like this.field.subfield, the inner `this.field`
        // IS a read even if the outer is a method call receiver
        n.visit_children_with(self);
    }

    fn visit_call_expr(&mut self, n: &CallExpr) {
        // this.method() / param.method() — `method` is NOT a field read, it's a call.
        // But this.field.method() — `field` IS a read.
        if let Callee::Expr(callee) = &n.callee {
            if is_receiver_method_call(callee, &self.param_names) {
                // Skip recording `method` as a field read — just recurse args
                for arg in &n.args {
                    arg.expr.visit_with(self);
                }
                return;
            }
        }
        n.visit_children_with(self);
    }
}
