use swc_ecma_ast::*;
use swc_ecma_visit::{Visit, VisitWith};

/// Extract call targets from a block statement.
pub fn extract_calls_block(block: &BlockStmt) -> Vec<String> {
    let mut visitor = CallVisitor { calls: Vec::new() };
    block.visit_with(&mut visitor);
    visitor.calls
}

/// Extract call targets from a single expression (arrow expression body).
pub fn extract_calls_expr(expr: &Expr) -> Vec<String> {
    let mut visitor = CallVisitor { calls: Vec::new() };
    expr.visit_with(&mut visitor);
    visitor.calls
}

struct CallVisitor {
    calls: Vec<String>,
}

/// Build a call target string from a callee expression.
fn callee_to_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(ident) => Some(ident.sym.to_string()),
        Expr::Member(member) => {
            let obj = expr_to_string(&member.obj)?;
            let prop = match &member.prop {
                MemberProp::Ident(ident) => ident.sym.to_string(),
                MemberProp::Computed(_) => return None,
                MemberProp::PrivateName(p) => p.name.to_string(),
            };
            Some(format!("{obj}.{prop}"))
        }
        // Unwrap optional chaining: foo?.bar() → foo.bar
        Expr::OptChain(opt) => match &*opt.base {
            OptChainBase::Member(member) => {
                let obj = expr_to_string(&member.obj)?;
                let prop = match &member.prop {
                    MemberProp::Ident(ident) => ident.sym.to_string(),
                    MemberProp::Computed(_) => return None,
                    MemberProp::PrivateName(p) => p.name.to_string(),
                };
                Some(format!("{obj}.{prop}"))
            }
            OptChainBase::Call(call) => callee_to_string(&call.callee),
        },
        // Unwrap TS type assertions: foo as Bar, foo!
        Expr::TsAs(ts) => callee_to_string(&ts.expr),
        Expr::TsNonNull(ts) => callee_to_string(&ts.expr),
        _ => None,
    }
}

/// Convert an expression to a simple string for object parts.
fn expr_to_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(ident) => Some(ident.sym.to_string()),
        Expr::This(_) => Some("this".to_string()),
        Expr::Member(member) => {
            let obj = expr_to_string(&member.obj)?;
            let prop = match &member.prop {
                MemberProp::Ident(ident) => ident.sym.to_string(),
                _ => return None,
            };
            Some(format!("{obj}.{prop}"))
        }
        Expr::OptChain(opt) => match &*opt.base {
            OptChainBase::Member(member) => {
                let obj = expr_to_string(&member.obj)?;
                let prop = match &member.prop {
                    MemberProp::Ident(ident) => ident.sym.to_string(),
                    _ => return None,
                };
                Some(format!("{obj}.{prop}"))
            }
            _ => None,
        },
        _ => None,
    }
}

impl CallVisitor {
    fn record(&mut self, name: String) {
        if !self.calls.contains(&name) {
            self.calls.push(name);
        }
    }
}

impl Visit for CallVisitor {
    fn visit_call_expr(&mut self, n: &CallExpr) {
        if let Callee::Expr(expr) = &n.callee {
            if let Some(name) = callee_to_string(expr) {
                self.record(name);
            }
        }
        // Recurse into arguments but NOT into nested function/arrow bodies
        for arg in &n.args {
            arg.expr.visit_with(self);
        }
        // Recurse into callee for chained calls
        if let Callee::Expr(expr) = &n.callee {
            expr.visit_with(self);
        }
    }

    fn visit_new_expr(&mut self, n: &NewExpr) {
        if let Some(name) = callee_to_string(&n.callee) {
            self.record(name);
        }
        n.visit_children_with(self);
    }

    // Do NOT descend into nested functions/arrows — their calls belong
    // to their own unit (or roll up if they're callbacks).
    fn visit_arrow_expr(&mut self, _n: &ArrowExpr) {}
    fn visit_fn_expr(&mut self, _n: &FnExpr) {}
    fn visit_fn_decl(&mut self, _n: &FnDecl) {}
}
