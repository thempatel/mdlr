use swc_common::SourceMap;
use swc_ecma_ast::*;
use swc_ecma_visit::{Visit, VisitWith};

/// Measure the largest nested BlockStmt within a function body,
/// excluding the function's own top-level block.
pub fn max_scope_lines_block(block: &BlockStmt, sm: &SourceMap) -> usize {
    let mut visitor = ScopeVisitor { sm, max: 0 };
    // Walk the block's statements directly — don't measure the top-level block itself.
    for stmt in &block.stmts {
        stmt.visit_with(&mut visitor);
    }
    visitor.max
}

struct ScopeVisitor<'a> {
    sm: &'a SourceMap,
    max: usize,
}

impl ScopeVisitor<'_> {
    fn record_span(&mut self, span: swc_common::Span) {
        let lo = self.sm.lookup_char_pos(span.lo);
        let hi = self.sm.lookup_char_pos(span.hi);
        let lines = hi.line.saturating_sub(lo.line) + 1;
        if lines > self.max {
            self.max = lines;
        }
    }
}

impl Visit for ScopeVisitor<'_> {
    fn visit_block_stmt(&mut self, n: &BlockStmt) {
        // Record this block as a scope
        self.record_span(n.span);
        // Recurse into nested blocks
        n.visit_children_with(self);
    }

    fn visit_if_stmt(&mut self, n: &IfStmt) {
        // The if body itself is a scope-creating construct
        n.visit_children_with(self);
    }

    fn visit_switch_stmt(&mut self, n: &SwitchStmt) {
        // Record the switch block span
        self.record_span(n.span);
        n.visit_children_with(self);
    }

    // Do NOT descend into nested function/arrow expressions
    fn visit_arrow_expr(&mut self, _n: &ArrowExpr) {}
    fn visit_fn_expr(&mut self, _n: &FnExpr) {}
    fn visit_fn_decl(&mut self, _n: &FnDecl) {}
}
