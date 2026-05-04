use ra_ap_syntax::ast::{self, HasLoopBody};
use ra_ap_syntax::{AstNode, TextRange};

use crate::walk::CstVisitor;

/// Find the largest single scope block within a function body.
///
/// Measures the line count of each scope-creating expression:
/// - `if` then/else bodies
/// - `match` arm bodies
/// - `for`/`while`/`loop` bodies
/// - Block expressions (`{}`)
/// - Closures
///
/// The function's own top-level block is excluded (that's `function_size`).
/// Returns 0 for functions with no nested scope blocks.
pub fn max_scope_lines(body: &ast::BlockExpr, text: &str) -> usize {
    let line_index = SimpleLineIndex::new(text);
    let mut visitor = ScopeVisitor {
        line_index: &line_index,
        body_range: body.syntax().text_range(),
        max: 0,
    };
    // Walk into the top-level block's contents without measuring
    // the block itself (which would duplicate function_size).
    if let Some(stmt_list) = body.stmt_list() {
        visitor.walk_stmt_list(&stmt_list, ());
    }
    visitor.max
}

/// Minimal line index: maps byte offsets to 1-based line numbers.
struct SimpleLineIndex {
    line_starts: Vec<u32>,
}

impl SimpleLineIndex {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self { line_starts }
    }

    fn line_of(&self, offset: u32) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(line) => line + 1,
            Err(line) => line, // line is the index of the line_start *after* offset
        }
    }

    fn range_lines(&self, range: TextRange) -> usize {
        let start_line = self.line_of(u32::from(range.start()));
        let end_line = self.line_of(u32::from(range.end()));
        end_line.saturating_sub(start_line) + 1
    }
}

struct ScopeVisitor<'a> {
    line_index: &'a SimpleLineIndex,
    body_range: TextRange,
    max: usize,
}

impl ScopeVisitor<'_> {
    fn record_scope(&mut self, range: TextRange) {
        // Don't measure scopes from macro expansions or dummy ranges
        if !self.body_range.contains_range(range) {
            return;
        }
        let lines = self.line_index.range_lines(range);
        if lines > self.max {
            self.max = lines;
        }
    }
}

impl CstVisitor for ScopeVisitor<'_> {
    type Ctx = ();

    fn visit_if(&mut self, expr: &ast::IfExpr, _ctx: ()) {
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, ());
        }
        if let Some(then_branch) = expr.then_branch() {
            self.record_scope(then_branch.syntax().text_range());
            self.walk_block(&then_branch, ());
        }
        if let Some(else_branch) = expr.else_branch() {
            match else_branch {
                ast::ElseBranch::Block(block) => {
                    self.record_scope(block.syntax().text_range());
                    self.walk_block(&block, ());
                }
                ast::ElseBranch::IfExpr(elif) => {
                    self.record_scope(elif.syntax().text_range());
                    self.walk_expr(&ast::Expr::from(elif), ());
                }
            }
        }
    }

    fn visit_match(&mut self, expr: &ast::MatchExpr, _ctx: ()) {
        if let Some(scrutinee) = expr.expr() {
            self.walk_expr(&scrutinee, ());
        }
        if let Some(arm_list) = expr.match_arm_list() {
            for arm in arm_list.arms() {
                if let Some(body) = arm.expr() {
                    self.record_scope(body.syntax().text_range());
                }
                if let Some(guard) = arm.guard() {
                    if let Some(guard_expr) = guard.condition() {
                        self.walk_expr(&guard_expr, ());
                    }
                }
                if let Some(body) = arm.expr() {
                    self.walk_expr(&body, ());
                }
            }
        }
    }

    fn visit_for(&mut self, expr: &ast::ForExpr, _ctx: ()) {
        if let Some(iterable) = expr.iterable() {
            self.walk_expr(&iterable, ());
        }
        if let Some(body) = expr.loop_body() {
            self.record_scope(body.syntax().text_range());
            self.walk_block(&body, ());
        }
    }

    fn visit_while(&mut self, expr: &ast::WhileExpr, _ctx: ()) {
        if let Some(cond) = expr.condition() {
            self.walk_expr(&cond, ());
        }
        if let Some(body) = expr.loop_body() {
            self.record_scope(body.syntax().text_range());
            self.walk_block(&body, ());
        }
    }

    fn visit_loop(&mut self, expr: &ast::LoopExpr, _ctx: ()) {
        if let Some(body) = expr.loop_body() {
            self.record_scope(body.syntax().text_range());
            self.walk_block(&body, ());
        }
    }

    fn visit_block_expr(&mut self, block: &ast::BlockExpr, _ctx: ()) {
        self.record_scope(block.syntax().text_range());
        if let Some(stmt_list) = block.stmt_list() {
            self.walk_stmt_list(&stmt_list, ());
        }
    }

    fn visit_closure(&mut self, closure: &ast::ClosureExpr, _ctx: ()) {
        if let Some(body) = closure.body() {
            self.record_scope(body.syntax().text_range());
            self.walk_expr(&body, ());
        }
    }
}
