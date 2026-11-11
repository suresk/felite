//! Chapter 13 challenges: folding constants, LIKE, CAST, and working out whether
//! an expression can be NULL.

use crate::bind::{Bound, BoundKind, Type};
use crate::error::{Error, Result};
use crate::eval::eval;
use crate::record::Value;

// @listing fold
/// Whether an expression mentions no column, so that its value is the same for
/// every row.
fn is_constant(expr: &Bound) -> bool {
    match &expr.kind {
        BoundKind::Literal(_) => true,
        BoundKind::Column(_) => false,
        BoundKind::Unary { operand, .. } | BoundKind::IsNull { operand, .. } => {
            is_constant(operand)
        }
        BoundKind::Binary { left, right, .. } => {
            is_constant(left) && is_constant(right)
        }
    }
}

/// Replaces every part of an expression that mentions no column with its value.
pub fn fold_constants(expr: Bound) -> Result<Bound> {
    if is_constant(&expr) {
        let value = eval(&expr, &[])?;
        return Ok(Bound { kind: BoundKind::Literal(value), ..expr });
    }
    let kind = match expr.kind {
        BoundKind::Unary { op, operand } => {
            BoundKind::Unary { op, operand: Box::new(fold_constants(*operand)?) }
        }
        BoundKind::IsNull { operand, negated } => BoundKind::IsNull {
            operand: Box::new(fold_constants(*operand)?),
            negated,
        },
        BoundKind::Binary { op, left, right } => BoundKind::Binary {
            op,
            left: Box::new(fold_constants(*left)?),
            right: Box::new(fold_constants(*right)?),
        },
        leaf => leaf,
    };
    Ok(Bound { kind, ..expr })
}
// @end

// @listing like
/// Whether `text` matches a LIKE pattern, in which `%` stands for any run of
/// characters, including none, and `_` for exactly one.
pub fn like(text: &str, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars();
    match pattern_chars.next() {
        None => text.is_empty(),
        // A % matches nothing, or one character and then whatever it would have.
        Some('%') => {
            let rest = pattern_chars.as_str();
            like(text, rest)
                || text
                    .chars()
                    .next()
                    .is_some_and(|c| like(&text[c.len_utf8()..], pattern))
        }
        Some(wanted) => match text.chars().next() {
            Some(c) if wanted == '_' || wanted == c => {
                like(&text[c.len_utf8()..], pattern_chars.as_str())
            }
            _ => false,
        },
    }
}
// @end

// @listing cast
/// Converts a value to a type, as `CAST(value AS type)` would. NULL stays NULL.
pub fn cast(value: &Value, to: Type) -> Result<Value> {
    let failed =
        |what: &str| Err(Error::Sql(format!("cannot turn {what} into {to}")));
    Ok(match (value, to) {
        (Value::Null, _) => Value::Null,
        (Value::Integer(i), Type::Integer) => Value::Integer(*i),
        (Value::Integer(i), Type::Real) => Value::Real(*i as f64),
        (Value::Real(x), Type::Real) => Value::Real(*x),
        // Toward zero, and only if the result is in range.
        (Value::Real(x), Type::Integer) => {
            let whole = x.trunc();
            let limit = 2f64.powi(63); // one more than the largest i64
            if !(-limit..limit).contains(&whole) {
                return failed(&format!("{x:?}"));
            }
            Value::Integer(whole as i64)
        }
        (Value::Integer(i), Type::Text) => Value::Text(i.to_string()),
        (Value::Real(x), Type::Text) => Value::Text(format!("{x:?}")),
        (Value::Text(s), Type::Text) => Value::Text(s.clone()),
        (Value::Text(s), Type::Integer) => match s.trim().parse() {
            Ok(i) => Value::Integer(i),
            Err(_) => return failed(&format!("'{s}'")),
        },
        (Value::Text(s), Type::Real) => match s.trim().parse::<f64>() {
            Ok(x) if x.is_finite() => Value::Real(x),
            _ => return failed(&format!("'{s}'")),
        },
        (_, Type::Condition | Type::Null) => return failed("a value"),
    })
}
// @end

// @listing can-be-null
/// Whether an expression could ever evaluate to NULL, given which columns of the
/// row are declared NOT NULL.
pub fn can_be_null(expr: &Bound, not_null: &[bool]) -> bool {
    match &expr.kind {
        BoundKind::Literal(value) => *value == Value::Null,
        BoundKind::Column(position) => !not_null[*position],
        BoundKind::IsNull { .. } => false,
        BoundKind::Unary { operand, .. } => can_be_null(operand, not_null),
        // Arithmetic and comparisons are NULL when either side is. AND and OR
        // can sometimes answer in spite of an unknown side, but not always.
        BoundKind::Binary { left, right, .. } => {
            can_be_null(left, not_null) || can_be_null(right, not_null)
        }
    }
}
// @end
