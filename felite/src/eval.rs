//! Evaluating a bound expression against a row.

use crate::bind::{Bound, BoundKind};
use crate::error::{Error, Result};
use crate::record::Value;
use crate::sql::{BinaryOp, Span, UnaryOp};
use std::cmp::Ordering;

// @listing truth
/// A condition's value as SQL sees it: true, false, or `None` for unknown.
pub fn truth(value: &Value) -> Option<bool> {
    match value {
        Value::Integer(i) => Some(*i != 0),
        _ => None,
    }
}

/// A truth value as a `Value`: 1, 0 or NULL.
fn condition(truth: Option<bool>) -> Value {
    match truth {
        Some(true) => Value::Integer(1),
        Some(false) => Value::Integer(0),
        None => Value::Null,
    }
}
// @end

// @listing fail
fn fail<T>(span: Span, message: &str) -> Result<T> {
    Err(Error::Invalid(message.to_string(), span))
}
// @end

// @listing arithmetic
/// `left op right` for two numbers. The binder has seen to it that they are
/// numbers, and that `op` is one of the five arithmetic operators.
fn arithmetic(
    op: BinaryOp,
    left: &Value,
    right: &Value,
    span: Span,
) -> Result<Value> {
    use BinaryOp::*;
    if let (Value::Integer(a), Value::Integer(b)) = (left, right) {
        let result = match op {
            Add => a.checked_add(*b),
            Subtract => a.checked_sub(*b),
            Multiply => a.checked_mul(*b),
            _ if *b == 0 => return fail(span, "division by zero"),
            Divide => a.checked_div(*b),
            _ => a.checked_rem(*b),
        };
        return match result {
            Some(value) => Ok(Value::Integer(value)),
            None => fail(span, "the result is too large for an integer"),
        };
    }
    let (a, b) = (as_real(left), as_real(right));
    let result = match op {
        Add => a + b,
        Subtract => a - b,
        Multiply => a * b,
        _ if b == 0.0 => return fail(span, "division by zero"),
        Divide => a / b,
        _ => a % b,
    };
    if !result.is_finite() {
        return fail(span, "the result is too large for a real");
    }
    Ok(Value::Real(result))
}

fn as_real(value: &Value) -> f64 {
    match value {
        Value::Integer(i) => *i as f64,
        Value::Real(x) => *x,
        _ => f64::NAN,
    }
}
// @end

// @listing compare
/// How two values that are not NULL compare: number with number, or text with
/// text.
fn compare(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        _ => as_real(left).total_cmp(&as_real(right)),
    }
}

fn comparison(op: BinaryOp, order: Ordering) -> bool {
    match op {
        BinaryOp::Equal => order == Ordering::Equal,
        BinaryOp::NotEqual => order != Ordering::Equal,
        BinaryOp::Less => order == Ordering::Less,
        BinaryOp::LessEqual => order != Ordering::Greater,
        BinaryOp::Greater => order == Ordering::Greater,
        _ => order != Ordering::Less,
    }
}
// @end

// @listing order
/// How two values sort. NULL comes before everything else, and two NULLs are
/// equal. The values must be of one type, apart from that.
pub fn order(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        _ => compare(left, right),
    }
}
// @end

// @listing logic
/// AND and OR, where either side may be unknown. The right side is a closure, so
/// that it is only worked out if the left side leaves the answer open.
fn logic(
    op: BinaryOp,
    left: Option<bool>,
    right: impl FnOnce() -> Result<Option<bool>>,
) -> Result<Option<bool>> {
    // AND is settled by a false, and OR by a true.
    let settles = op == BinaryOp::Or;
    if left == Some(settles) {
        return Ok(left);
    }
    Ok(match (left, right()?) {
        (_, Some(right)) if right == settles => Some(settles),
        (Some(_), Some(_)) => Some(!settles),
        _ => None,
    })
}
// @end

// @listing eval
/// The value of an expression for one row.
pub fn eval(expr: &Bound, row: &[Value]) -> Result<Value> {
    use BinaryOp::*;
    match &expr.kind {
        BoundKind::Literal(value) => Ok(value.clone()),
        BoundKind::Column(position) => Ok(row[*position].clone()),
        BoundKind::IsNull { operand, negated } => {
            let is_null = eval(operand, row)? == Value::Null;
            Ok(condition(Some(is_null != *negated)))
        }
        BoundKind::Unary { op, operand } => match (op, eval(operand, row)?) {
            (_, Value::Null) => Ok(Value::Null),
            (UnaryOp::Not, value) => Ok(condition(truth(&value).map(|t| !t))),
            (UnaryOp::Negate, Value::Integer(i)) => match i.checked_neg() {
                Some(negated) => Ok(Value::Integer(negated)),
                None => fail(expr.span, "the result is too large for an integer"),
            },
            (UnaryOp::Negate, value) => Ok(Value::Real(-as_real(&value))),
        },
        BoundKind::Binary { op: op @ (And | Or), left, right } => {
            let left = truth(&eval(left, row)?);
            let right = || Ok(truth(&eval(right, row)?));
            Ok(condition(logic(*op, left, right)?))
        }
        BoundKind::Binary { op, left, right } => {
            let (left, right) = (eval(left, row)?, eval(right, row)?);
            if left == Value::Null || right == Value::Null {
                return Ok(Value::Null);
            }
            match op {
                Add | Subtract | Multiply | Divide | Remainder => {
                    arithmetic(*op, &left, &right, expr.span)
                }
                _ => Ok(condition(Some(comparison(*op, compare(&left, &right))))),
            }
        }
    }
}
// @end
