//! Chapter 11 challenges: walking a tree, and printing it with only the
//! parentheses it needs.

use crate::sql::{BinaryOp, Expr, ExprKind, UnaryOp};

// @listing walk
/// The names of the columns an expression mentions, each once, in the order they
/// first appear.
pub fn columns_used(expr: &Expr) -> Vec<String> {
    fn walk(expr: &Expr, found: &mut Vec<String>) {
        match &expr.kind {
            ExprKind::Literal(_) => {}
            ExprKind::Column { name, .. } => {
                if !found.contains(name) {
                    found.push(name.clone());
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::IsNull { operand, .. } => {
                walk(operand, found);
            }
            ExprKind::Binary { left, right, .. } => {
                walk(left, found);
                walk(right, found);
            }
            // @hide
            ExprKind::Call { arg, .. } => {
                if let Some(arg) = arg {
                    walk(arg, found);
                }
            } // @show
        }
    }
    let mut found = Vec::new();
    walk(expr, &mut found);
    found
}

/// The number of levels in an expression's tree. A literal or a column is 1.
pub fn depth(expr: &Expr) -> usize {
    match &expr.kind {
        ExprKind::Literal(_) | ExprKind::Column { .. } => 1,
        ExprKind::Unary { operand, .. } | ExprKind::IsNull { operand, .. } => {
            1 + depth(operand)
        }
        ExprKind::Binary { left, right, .. } => 1 + depth(left).max(depth(right)),
        // @hide
        ExprKind::Call { arg, .. } => 1 + arg.as_deref().map_or(0, depth),
        // @show
    }
}
// @end

// @listing power
/// How tightly the operation at the top of an expression binds. These are the
/// parser's binding powers, with a value above them all for leaves.
fn power(expr: &Expr) -> u8 {
    use BinaryOp::*;
    match &expr.kind {
        ExprKind::Literal(_) | ExprKind::Column { .. } => 9,
        // @hide
        ExprKind::Call { .. } => 9,
        // @show
        ExprKind::Unary { op: UnaryOp::Negate, .. } => 7,
        ExprKind::Unary { op: UnaryOp::Not, .. } => 3,
        ExprKind::IsNull { .. } => 4,
        ExprKind::Binary { op, .. } => match op {
            Or => 1,
            And => 2,
            Equal | NotEqual | Less | LessEqual | Greater | GreaterEqual => 4,
            Add | Subtract => 5,
            Multiply | Divide | Remainder => 6,
        },
    }
}
// @end

// @listing minimal
/// An expression as SQL with no parentheses that could be left out.
pub fn to_minimal_sql(expr: &Expr) -> String {
    // `inner` as text, in parentheses if it binds less tightly than `needed`.
    let child = |inner: &Expr, needed: u8| {
        let text = to_minimal_sql(inner);
        if power(inner) < needed { format!("({text})") } else { text }
    };
    let own = power(expr);
    match &expr.kind {
        ExprKind::Literal(_) | ExprKind::Column { .. } => expr.to_string(),
        // @hide
        ExprKind::Call { function, arg: Some(arg) } => {
            format!("{}({})", function.name(), to_minimal_sql(arg))
        }
        ExprKind::Call { arg: None, .. } => expr.to_string(),
        // @show
        ExprKind::Unary { op: UnaryOp::Negate, operand } => {
            let text = child(operand, own);
            // Two minus signs in a row would begin a comment.
            if text.starts_with('-') {
                format!("-({text})")
            } else {
                format!("-{text}")
            }
        }
        ExprKind::Unary { op: UnaryOp::Not, operand } => {
            format!("NOT {}", child(operand, own))
        }
        ExprKind::IsNull { operand, negated } => {
            let not = if *negated { "NOT " } else { "" };
            format!("{} IS {not}NULL", child(operand, own))
        }
        // Operators group from the left, so a right-hand side of the same
        // strength needs its parentheses and a left-hand side does not.
        ExprKind::Binary { op, left, right } => {
            format!(
                "{} {} {}",
                child(left, own),
                op.spelling(),
                child(right, own + 1)
            )
        }
    }
}
// @end
