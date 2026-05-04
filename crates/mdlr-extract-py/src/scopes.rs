use ruff_python_ast::{self as ast, Stmt};
use ruff_text_size::Ranged;

use crate::visitor::LineIndex;

/// Measure the largest nested scope block within a function body,
/// excluding the function's own top-level block.
pub fn max_scope_lines_body(
    body: &[Stmt],
    source: &str,
    line_index: &LineIndex,
) -> usize {
    let mut max = 0;
    for stmt in body {
        max_scope_stmt(stmt, source, line_index, &mut max);
    }
    max
}

fn max_scope_stmt(
    stmt: &Stmt,
    source: &str,
    line_index: &LineIndex,
    max: &mut usize,
) {
    match stmt {
        Stmt::If(node) => {
            record_span(node.range(), line_index, max);
            for s in &node.body {
                max_scope_stmt(s, source, line_index, max);
            }
            for clause in &node.elif_else_clauses {
                record_span(clause.range(), line_index, max);
                for s in &clause.body {
                    max_scope_stmt(s, source, line_index, max);
                }
            }
        }
        Stmt::For(node) => {
            record_span(node.range(), line_index, max);
            for s in &node.body {
                max_scope_stmt(s, source, line_index, max);
            }
            for s in &node.orelse {
                max_scope_stmt(s, source, line_index, max);
            }
        }
        Stmt::While(node) => {
            record_span(node.range(), line_index, max);
            for s in &node.body {
                max_scope_stmt(s, source, line_index, max);
            }
            for s in &node.orelse {
                max_scope_stmt(s, source, line_index, max);
            }
        }
        Stmt::Try(node) => {
            record_span(node.range(), line_index, max);
            for s in &node.body {
                max_scope_stmt(s, source, line_index, max);
            }
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = handler;
                record_span(h.range(), line_index, max);
                for s in &h.body {
                    max_scope_stmt(s, source, line_index, max);
                }
            }
            for s in &node.orelse {
                max_scope_stmt(s, source, line_index, max);
            }
            for s in &node.finalbody {
                max_scope_stmt(s, source, line_index, max);
            }
        }
        Stmt::Match(node) => {
            record_span(node.range(), line_index, max);
            for case in &node.cases {
                record_span(case.range(), line_index, max);
                for s in &case.body {
                    max_scope_stmt(s, source, line_index, max);
                }
            }
        }
        Stmt::With(node) => {
            record_span(node.range(), line_index, max);
            for s in &node.body {
                max_scope_stmt(s, source, line_index, max);
            }
        }
        // Don't recurse into nested function/class defs
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
        _ => {}
    }
}

fn record_span(
    range: ruff_text_size::TextRange,
    line_index: &LineIndex,
    max: &mut usize,
) {
    let (start_line, _) = line_index.offset_to_line_col(range.start().into());
    let (end_line, _) = line_index.offset_to_line_col(range.end().into());
    let lines = end_line.saturating_sub(start_line) + 1;
    if lines > *max {
        *max = lines;
    }
}
