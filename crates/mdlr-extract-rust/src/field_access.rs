use ra_ap_syntax::ast;

use crate::walk::CstVisitor;

/// Extract field reads and writes from a function/method body.
///
/// Detects `self.field` and `param.field` patterns:
/// - Read: `self.field` or `param.field` in any position except assignment LHS
/// - Write: `self.field` or `param.field` in assignment LHS (`=`, `+=`, etc.)
/// - `self.method()` / `param.method()` is NOT a field read (it's a method call)
/// - `self.field.method()` — `field` IS a read
///
/// Self fields are recorded bare (`"x"`), param fields as `"param.field"`.
pub fn extract_field_access(
    body: &ast::BlockExpr,
    param_names: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();

    let mut visitor = FieldAccessVisitor {
        reads: &mut reads,
        writes: &mut writes,
        param_names,
    };
    visitor.walk_block(body, FieldContext::Read);
    (reads, writes)
}

/// Context for determining if a field access is a read or write.
#[derive(Clone, Copy, PartialEq, Default)]
enum FieldContext {
    #[default]
    Read,
    WriteLhs,
}

struct FieldAccessVisitor<'a> {
    reads: &'a mut Vec<String>,
    writes: &'a mut Vec<String>,
    param_names: &'a [String],
}

impl FieldAccessVisitor<'_> {
    fn record_field(&mut self, field_name: String, ctx: FieldContext) {
        let vec = match ctx {
            FieldContext::WriteLhs => &mut self.writes,
            FieldContext::Read => &mut self.reads,
        };
        if !vec.contains(&field_name) {
            vec.push(field_name);
        }
    }

    fn walk_write_and_read(&mut self, lhs: &ast::Expr, rhs: &ast::Expr) {
        self.walk_expr(lhs, FieldContext::WriteLhs);
        self.walk_expr(rhs, FieldContext::Read);
    }
}

impl CstVisitor for FieldAccessVisitor<'_> {
    type Ctx = FieldContext;

    fn visit_field(&mut self, expr: &ast::FieldExpr, ctx: FieldContext) {
        if let Some(base) = expr.expr() {
            if is_self_expr(&base) {
                if let Some(name_ref) = expr.name_ref() {
                    self.record_field(name_ref.text().to_string(), ctx);
                }
            } else if let Some(param) = is_param_expr(&base, self.param_names)
            {
                if let Some(name_ref) = expr.name_ref() {
                    self.record_field(
                        format!("{param}.{}", name_ref.text()),
                        ctx,
                    );
                }
            } else {
                // For chained access like self.field.subfield, the inner field IS a read
                self.walk_expr(&base, FieldContext::Read);
            }
        }
    }

    fn visit_assign(
        &mut self,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
        _ctx: FieldContext,
    ) {
        self.walk_write_and_read(lhs, rhs);
    }

    fn visit_index(&mut self, expr: &ast::IndexExpr, ctx: FieldContext) {
        // Propagate write context to base: `self.items[i] = v` → `items` is a write
        if let Some(base) = expr.base() {
            self.walk_expr(&base, ctx);
        }
        if let Some(idx) = expr.index() {
            self.walk_expr(&idx, FieldContext::Read);
        }
    }
}

/// Check if an expression refers to a known parameter name.
/// Returns the parameter name if matched.
fn is_param_expr(expr: &ast::Expr, param_names: &[String]) -> Option<String> {
    if let ast::Expr::PathExpr(path_expr) = expr {
        if let Some(path) = path_expr.path() {
            if let Some(segment) = path.segment() {
                if let Some(name_ref) = segment.name_ref() {
                    let name = name_ref.text().to_string();
                    if param_names.contains(&name) {
                        return Some(name);
                    }
                }
            }
        }
    }
    None
}

/// Check if an expression refers to `self`.
fn is_self_expr(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::PathExpr(path_expr) => {
            if let Some(path) = path_expr.path() {
                if let Some(segment) = path.segment() {
                    if let Some(name_ref) = segment.name_ref() {
                        return name_ref.text() == "self";
                    }
                    // Also check for `self` keyword token
                    return segment.self_token().is_some();
                }
            }
            false
        }
        // Handle deref: *self
        ast::Expr::PrefixExpr(prefix) => {
            if prefix.op_kind() == Some(ast::UnaryOp::Deref) {
                if let Some(inner) = prefix.expr() {
                    return is_self_expr(&inner);
                }
            }
            false
        }
        _ => false,
    }
}
