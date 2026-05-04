use mdlr_core::{Span, Unit, UnitKind};
use std::path::PathBuf;
use swc_common::SourceMap;
use swc_ecma_ast::*;
use swc_ecma_visit::{Visit, VisitWith};

use crate::branches;
use crate::calls;
use crate::cognitive;
use crate::field_access;
use crate::scopes;

/// Extract all units from a parsed module.
pub fn extract_units(
    module: &Module,
    rel_path: &str,
    sm: &SourceMap,
) -> Vec<Unit> {
    let mut extractor = UnitExtractor {
        rel_path: rel_path.to_string(),
        sm,
        units: Vec::new(),
        scope_stack: Vec::new(),
    };
    module.visit_with(&mut extractor);
    extractor.units
}

struct UnitExtractor<'a> {
    rel_path: String,
    sm: &'a SourceMap,
    units: Vec<Unit>,
    /// Stack of enclosing named scopes for building compound IDs.
    scope_stack: Vec<String>,
}

impl<'a> UnitExtractor<'a> {
    /// Build an ID like `src/utils.ts::outer::inner`.
    fn make_id(&self, name: &str) -> String {
        let mut parts = vec![self.rel_path.clone()];
        parts.extend(self.scope_stack.iter().cloned());
        parts.push(name.to_string());
        parts.join("::")
    }

    /// Build a parent ID from the current scope stack (for methods).
    fn parent_id(&self) -> Option<String> {
        if self.scope_stack.is_empty() {
            return None;
        }
        let mut parts = vec![self.rel_path.clone()];
        parts.extend(self.scope_stack.iter().cloned());
        Some(parts.join("::"))
    }

    fn make_span(&self, span: swc_common::Span) -> Span {
        let lo = self.sm.lookup_char_pos(span.lo);
        let hi = self.sm.lookup_char_pos(span.hi);
        Span {
            start_line: lo.line,
            start_col: lo.col_display,
            end_line: hi.line,
            end_col: hi.col_display,
        }
    }

    /// Analyze a function/arrow body and produce a Unit.
    fn extract_fn_unit(
        &self,
        name: &str,
        kind: UnitKind,
        params: usize,
        span: swc_common::Span,
        body: Option<&BlockStmt>,
        expr_body: Option<&Expr>,
        parent: Option<String>,
        param_names: &[String],
    ) -> Unit {
        let id = self.make_id(name);
        let unit_span = self.make_span(span);

        let (
            call_targets,
            reads,
            writes,
            branch_count,
            max_scope,
            cognitive_complexity,
        ) = if let Some(block) = body {
            let call_targets = calls::extract_calls_block(block);
            let (reads, writes) =
                field_access::extract_field_access_block(block, param_names);
            let branch_count = branches::count_branches_block(block);
            let max_scope = scopes::max_scope_lines_block(block, self.sm);
            let cog = cognitive::compute_cognitive_block(block);
            (call_targets, reads, writes, branch_count, max_scope, cog)
        } else if let Some(expr) = expr_body {
            let call_targets = calls::extract_calls_expr(expr);
            let (reads, writes) =
                field_access::extract_field_access_expr(expr, param_names);
            let branch_count = branches::count_branches_expr(expr);
            let cog = cognitive::compute_cognitive_expr(expr);
            // Arrow with expression body has no block scopes
            (call_targets, reads, writes, branch_count, 0, cog)
        } else {
            (vec![], vec![], vec![], 0, 0, 0)
        };

        Unit {
            id,
            kind,
            file: PathBuf::from(&self.rel_path),
            span: unit_span,
            reads,
            writes,
            calls: call_targets,
            tags: vec![],
            params,
            branches: branch_count,
            max_scope_lines: max_scope,
            parent,
            cognitive_complexity,
            partial: false,
        }
    }

    /// Count function parameters (excluding patterns like `this` in TS).
    fn count_params(params: &[Param]) -> usize {
        params
            .iter()
            .filter(|p| !matches!(&p.pat, Pat::Ident(ident) if ident.sym == "this"))
            .count()
    }

    /// Check if a function/arrow expression is being assigned to a variable
    /// declaration. Returns None if the init expression doesn't match.
    fn try_extract_var_fn(
        &mut self,
        name: &str,
        init: &Expr,
        decl_span: swc_common::Span,
    ) -> bool {
        match init {
            Expr::Arrow(arrow) => {
                let params = arrow.params.len();
                let param_names = extract_pat_names(&arrow.params);
                let (body_block, body_expr) = match &*arrow.body {
                    BlockStmtOrExpr::BlockStmt(block) => (Some(block), None),
                    BlockStmtOrExpr::Expr(expr) => (None, Some(expr.as_ref())),
                };
                let unit = self.extract_fn_unit(
                    name,
                    UnitKind::Function,
                    params,
                    decl_span,
                    body_block,
                    body_expr,
                    None,
                    &param_names,
                );
                self.units.push(unit);
                true
            }
            Expr::Fn(fn_expr) => {
                let params = Self::count_params(&fn_expr.function.params);
                let param_names =
                    extract_param_names(&fn_expr.function.params);
                let unit = self.extract_fn_unit(
                    name,
                    UnitKind::Function,
                    params,
                    decl_span,
                    fn_expr.function.body.as_ref(),
                    None,
                    None,
                    &param_names,
                );
                self.units.push(unit);
                true
            }
            // Unwrap `expr as Type` or `expr satisfies Type` wrappers
            Expr::TsAs(ts_as) => {
                self.try_extract_var_fn(name, &ts_as.expr, decl_span)
            }
            Expr::TsSatisfies(ts_sat) => {
                self.try_extract_var_fn(name, &ts_sat.expr, decl_span)
            }
            _ => false,
        }
    }

    /// Visit class members after pushing the class name onto the scope stack.
    fn visit_class_members(&mut self, class: &Class) {
        for member in &class.body {
            match member {
                ClassMember::Method(method) => {
                    let name = prop_name_to_string(&method.key);
                    if let Some(raw_name) = name {
                        // Prefix getter/setter names per plan
                        let name = match method.kind {
                            MethodKind::Getter => {
                                format!("get_{raw_name}")
                            }
                            MethodKind::Setter => {
                                format!("set_{raw_name}")
                            }
                            MethodKind::Method => raw_name,
                        };
                        let params =
                            Self::count_params(&method.function.params);
                        let param_names =
                            extract_param_names(&method.function.params);
                        let parent = self.parent_id();
                        let unit = self.extract_fn_unit(
                            &name,
                            UnitKind::Method,
                            params,
                            method.span,
                            method.function.body.as_ref(),
                            None,
                            parent,
                            &param_names,
                        );
                        self.units.push(unit);
                    }
                }
                ClassMember::Constructor(ctor) => {
                    let params = ctor
                        .params
                        .iter()
                        .filter(|p| matches!(p, ParamOrTsParamProp::Param(_)))
                        .count();
                    let ctor_params: Vec<Param> = ctor
                        .params
                        .iter()
                        .filter_map(|p| match p {
                            ParamOrTsParamProp::Param(param) => {
                                Some(param.clone())
                            }
                            _ => None,
                        })
                        .collect();
                    let param_names = extract_param_names(&ctor_params);
                    let parent = self.parent_id();
                    let unit = self.extract_fn_unit(
                        "constructor",
                        UnitKind::Method,
                        params,
                        ctor.span,
                        ctor.body.as_ref(),
                        None,
                        parent,
                        &param_names,
                    );
                    self.units.push(unit);
                }
                ClassMember::PrivateMethod(method) => {
                    let name = method.key.name.to_string();
                    let params = Self::count_params(&method.function.params);
                    let param_names =
                        extract_param_names(&method.function.params);
                    let parent = self.parent_id();
                    let unit = self.extract_fn_unit(
                        &name,
                        UnitKind::Method,
                        params,
                        method.span,
                        method.function.body.as_ref(),
                        None,
                        parent,
                        &param_names,
                    );
                    self.units.push(unit);
                }
                _ => {}
            }
        }
    }
}

impl Visit for UnitExtractor<'_> {
    fn visit_fn_decl(&mut self, n: &FnDecl) {
        let name = n.ident.sym.to_string();
        let params = Self::count_params(&n.function.params);
        let param_names = extract_param_names(&n.function.params);
        let unit = self.extract_fn_unit(
            &name,
            UnitKind::Function,
            params,
            n.function.span,
            n.function.body.as_ref(),
            None,
            None,
            &param_names,
        );
        self.units.push(unit);
        // Do NOT recurse into the function body for nested declarations —
        // nested named functions get their own visit via the scope stack.
        self.scope_stack.push(name);
        n.function.body.visit_with(self);
        self.scope_stack.pop();
    }

    fn visit_class_decl(&mut self, n: &ClassDecl) {
        let name = n.ident.sym.to_string();

        // Emit struct unit for the class itself
        let unit = Unit {
            id: self.make_id(&name),
            kind: UnitKind::Struct,
            file: PathBuf::from(&self.rel_path),
            span: self.make_span(n.class.span),
            reads: vec![],
            writes: vec![],
            calls: vec![],
            tags: vec![],
            params: 0,
            branches: 0,
            max_scope_lines: 0,
            parent: None,
            cognitive_complexity: 0,
            partial: false,
        };
        self.units.push(unit);

        // Visit members with class name on the scope stack
        self.scope_stack.push(name);
        self.visit_class_members(&n.class);
        self.scope_stack.pop();
    }

    fn visit_var_declarator(&mut self, n: &VarDeclarator) {
        if let Some(init) = &n.init {
            if let Pat::Ident(ident) = &n.name {
                let name = ident.sym.to_string();
                if self.try_extract_var_fn(&name, init, n.span) {
                    // Extracted — recurse into body for nested declarations
                    self.scope_stack.push(name);
                    init.visit_with(self);
                    self.scope_stack.pop();
                    return;
                }
            }
        }
        // Default recursion for non-function var declarators
        n.visit_children_with(self);
    }

    fn visit_export_default_decl(&mut self, n: &ExportDefaultDecl) {
        match &n.decl {
            DefaultDecl::Fn(fn_expr) => {
                let name = fn_expr
                    .ident
                    .as_ref()
                    .map(|id| id.sym.to_string())
                    .unwrap_or_else(|| "default".to_string());
                let params = Self::count_params(&fn_expr.function.params);
                let param_names =
                    extract_param_names(&fn_expr.function.params);
                let unit = self.extract_fn_unit(
                    &name,
                    UnitKind::Function,
                    params,
                    fn_expr.function.span,
                    fn_expr.function.body.as_ref(),
                    None,
                    None,
                    &param_names,
                );
                self.units.push(unit);
                self.scope_stack.push(name);
                fn_expr.function.body.visit_with(self);
                self.scope_stack.pop();
            }
            DefaultDecl::Class(class_expr) => {
                let name = class_expr
                    .ident
                    .as_ref()
                    .map(|id| id.sym.to_string())
                    .unwrap_or_else(|| "default".to_string());

                let unit = Unit {
                    id: self.make_id(&name),
                    kind: UnitKind::Struct,
                    file: PathBuf::from(&self.rel_path),
                    span: self.make_span(class_expr.class.span),
                    reads: vec![],
                    writes: vec![],
                    calls: vec![],
                    tags: vec![],
                    params: 0,
                    branches: 0,
                    max_scope_lines: 0,
                    parent: None,
                    cognitive_complexity: 0,
                    partial: false,
                };
                self.units.push(unit);

                self.scope_stack.push(name);
                self.visit_class_members(&class_expr.class);
                self.scope_stack.pop();
            }
            _ => {
                n.visit_children_with(self);
            }
        }
    }

    fn visit_export_default_expr(&mut self, n: &ExportDefaultExpr) {
        match &*n.expr {
            Expr::Arrow(arrow) => {
                let params = arrow.params.len();
                let param_names = extract_pat_names(&arrow.params);
                let (body_block, body_expr) = match &*arrow.body {
                    BlockStmtOrExpr::BlockStmt(block) => (Some(block), None),
                    BlockStmtOrExpr::Expr(expr) => (None, Some(expr.as_ref())),
                };
                let unit = self.extract_fn_unit(
                    "default",
                    UnitKind::Function,
                    params,
                    n.span,
                    body_block,
                    body_expr,
                    None,
                    &param_names,
                );
                self.units.push(unit);
            }
            Expr::Fn(fn_expr) => {
                let name = fn_expr
                    .ident
                    .as_ref()
                    .map(|id| id.sym.to_string())
                    .unwrap_or_else(|| "default".to_string());
                let params = Self::count_params(&fn_expr.function.params);
                let param_names =
                    extract_param_names(&fn_expr.function.params);
                let unit = self.extract_fn_unit(
                    &name,
                    UnitKind::Function,
                    params,
                    fn_expr.function.span,
                    fn_expr.function.body.as_ref(),
                    None,
                    None,
                    &param_names,
                );
                self.units.push(unit);
            }
            _ => {
                n.visit_children_with(self);
            }
        }
    }

    // Do NOT descend into nested arrow/function expressions as top-level
    // declarations — they're callbacks whose analysis rolls up to the
    // enclosing function. The visitor only fires for module-level items.
    fn visit_arrow_expr(&mut self, _n: &ArrowExpr) {
        // Intentionally empty — arrows are handled by visit_var_declarator
        // and visit_export_default_expr. Callback arrows are not extracted.
    }

    fn visit_fn_expr(&mut self, _n: &FnExpr) {
        // Intentionally empty — function expressions are handled by
        // visit_var_declarator and visit_export_default_expr.
    }
}

/// Extract parameter names from function params (excluding `this`).
fn extract_param_names(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .filter_map(|p| match &p.pat {
            Pat::Ident(ident) if ident.sym != "this" => {
                Some(ident.sym.to_string())
            }
            _ => None,
        })
        .collect()
}

/// Extract parameter names from arrow function patterns (excluding `this`).
fn extract_pat_names(pats: &[Pat]) -> Vec<String> {
    pats.iter()
        .filter_map(|p| match p {
            Pat::Ident(ident) if ident.sym != "this" => {
                Some(ident.sym.to_string())
            }
            _ => None,
        })
        .collect()
}

/// Extract a string name from a property key.
fn prop_name_to_string(key: &PropName) -> Option<String> {
    match key {
        PropName::Ident(ident) => Some(ident.sym.to_string()),
        PropName::Str(s) => Some(s.value.to_string_lossy().into_owned()),
        PropName::Num(n) => Some(n.value.to_string()),
        PropName::Computed(_) => None,
        PropName::BigInt(b) => Some(b.value.to_string()),
    }
}
