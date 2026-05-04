use swc_ecma_ast::*;
use swc_ecma_visit::{Visit, VisitWith};

/// Count branch points in a block statement.
pub fn count_branches_block(block: &BlockStmt) -> usize {
    let mut visitor = BranchVisitor { count: 0 };
    block.visit_with(&mut visitor);
    visitor.count
}

/// Count branch points in a single expression (arrow body).
pub fn count_branches_expr(expr: &Expr) -> usize {
    let mut visitor = BranchVisitor { count: 0 };
    expr.visit_with(&mut visitor);
    visitor.count
}

struct BranchVisitor {
    count: usize,
}

impl Visit for BranchVisitor {
    fn visit_if_stmt(&mut self, n: &IfStmt) {
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_switch_stmt(&mut self, n: &SwitchStmt) {
        // Each case is a branch; subtract 1 for baseline (or use cases-1)
        let cases = n.cases.len();
        if cases > 1 {
            self.count += cases - 1;
        }
        n.visit_children_with(self);
    }

    fn visit_for_stmt(&mut self, n: &ForStmt) {
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_for_in_stmt(&mut self, n: &ForInStmt) {
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_for_of_stmt(&mut self, n: &ForOfStmt) {
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_while_stmt(&mut self, n: &WhileStmt) {
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_do_while_stmt(&mut self, n: &DoWhileStmt) {
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_cond_expr(&mut self, n: &CondExpr) {
        // Ternary operator: +1
        self.count += 1;
        n.visit_children_with(self);
    }

    fn visit_bin_expr(&mut self, n: &BinExpr) {
        match n.op {
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                self.count += 1;
            }
            _ => {}
        }
        // Do NOT count `??` (nullish coalescing) or `?.` (optional chaining)
        n.visit_children_with(self);
    }

    // Do NOT descend into nested function/arrow expressions
    fn visit_arrow_expr(&mut self, _n: &ArrowExpr) {}
    fn visit_fn_expr(&mut self, _n: &FnExpr) {}
    fn visit_fn_decl(&mut self, _n: &FnDecl) {}
}
