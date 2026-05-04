use ra_ap_hir::{ModuleDef, PathResolution, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::ast;
use ra_ap_syntax::AstNode;

use crate::path_util::qualified_path;

/// Extract all call targets from a function/method body.
///
/// Uses `Semantics` to resolve calls to their fully-qualified paths.
/// This is the primary advantage over tree-sitter: trait method calls are resolved
/// to their concrete implementations.
///
/// Returns `(calls, partial)` where `partial` is true if semantic analysis
/// was not available and call resolution could not be performed.
pub fn extract_calls(
    sema: &Semantics<'_, RootDatabase>,
    db: &RootDatabase,
    body: &ast::BlockExpr,
) -> (Vec<String>, bool) {
    let mut calls = Vec::new();

    // Walk all descendants to find call expressions and method calls
    for node in body.syntax().descendants() {
        if let Some(method_call) = ast::MethodCallExpr::cast(node.clone()) {
            if let Some(func) = sema.resolve_method_call(&method_call) {
                let def: ModuleDef = func.into();
                let path = qualified_path(db, def);
                if !calls.contains(&path) {
                    calls.push(path);
                }
            }
        } else if let Some(call_expr) = ast::CallExpr::cast(node.clone()) {
            if let Some(callee) = call_expr.expr() {
                if let Some(path_expr) = match &callee {
                    ast::Expr::PathExpr(p) => Some(p.clone()),
                    _ => None,
                } {
                    if let Some(path) = path_expr.path() {
                        if let Some(resolution) = sema.resolve_path(&path) {
                            let def = match resolution {
                                PathResolution::Def(def) => Some(def),
                                _ => None,
                            };
                            if let Some(def) = def {
                                let path_str = qualified_path(db, def);
                                if !calls.contains(&path_str) {
                                    calls.push(path_str);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    (calls, false)
}
