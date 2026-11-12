//! Binding: looking up every name in an expression and giving every part a type.

use crate::catalog::{Table, fold};
use crate::error::{Error, Result};
use crate::record::Value;
use crate::sql::{BinaryOp, ColumnDef, ColumnType, Expr, ExprKind, Span, UnaryOp};
use std::fmt;

// @listing type
/// What kind of value an expression produces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Type {
    Integer,
    Real,
    Text,
    /// True, false or unknown: the result of a comparison.
    Condition,
    /// The type of a bare NULL, which fits anywhere.
    Null,
}

impl Type {
    fn is_number(self) -> bool {
        matches!(self, Type::Integer | Type::Real | Type::Null)
    }
}

impl From<ColumnType> for Type {
    fn from(kind: ColumnType) -> Type {
        match kind {
            ColumnType::Integer => Type::Integer,
            ColumnType::Real => Type::Real,
            ColumnType::Text => Type::Text,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Type::Integer => "an INTEGER",
            Type::Real => "a REAL",
            Type::Text => "TEXT",
            Type::Condition => "a condition",
            Type::Null => "NULL",
        })
    }
}
// @end

// @listing scope
/// The columns that an expression may refer to, in the order their values will
/// appear in a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub columns: Vec<ScopeColumn>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeColumn {
    pub table: String,
    pub name: String,
    pub ty: Type,
}

impl Scope {
    /// The columns of one table.
    pub fn of(table: &Table) -> Scope {
        let column = |c: &ColumnDef| ScopeColumn {
            table: table.name.clone(),
            name: c.name.clone(),
            ty: c.kind.into(),
        };
        Scope { columns: table.columns.iter().map(column).collect() }
    }
}
// @end

// @listing resolve
fn invalid<T>(span: Span, message: String) -> Result<T> {
    Err(Error::Invalid(message, span))
}

impl Scope {
    /// The position and type of the column that a name refers to.
    fn resolve(
        &self,
        table: Option<&str>,
        name: &str,
        span: Span,
    ) -> Result<(usize, Type)> {
        let wanted = |c: &&ScopeColumn| {
            fold(&c.name) == fold(name)
                && table.is_none_or(|t| fold(&c.table) == fold(t))
        };
        let mut found = self.columns.iter().enumerate().filter(|(_, c)| wanted(c));
        match (found.next(), found.next()) {
            (Some((position, column)), None) => Ok((position, column.ty)),
            (Some(_), Some(_)) => {
                invalid(span, format!("the name {name} could mean two columns"))
            }
            (None, _) => match table {
                Some(table) => {
                    invalid(span, format!("there is no column {table}.{name}"))
                }
                None => invalid(span, format!("there is no column named {name}")),
            },
        }
    }
}
// @end

// @listing bound
/// An expression that has been checked: its columns are positions in a row, and
/// every part of it has a type.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub kind: BoundKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoundKind {
    Literal(Value),
    /// The value at this position in the row.
    Column(usize),
    Unary {
        op: UnaryOp,
        operand: Box<Bound>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Bound>,
        right: Box<Bound>,
    },
    IsNull {
        operand: Box<Bound>,
        negated: bool,
    },
}
// @end

// @listing binary-type
/// The type of `left op right`, or an error that points at the operand which
/// does not fit.
fn binary_type(op: BinaryOp, left: &Bound, right: &Bound) -> Result<Type> {
    use BinaryOp::*;
    let sign = op.spelling();
    match op {
        Add | Subtract | Multiply | Divide | Remainder => {
            for side in [left, right] {
                if !side.ty.is_number() {
                    let message =
                        format!("{sign} needs a number, and this is {}", side.ty);
                    return invalid(side.span, message);
                }
            }
            Ok(match (left.ty, right.ty) {
                (Type::Null, other) | (other, Type::Null) => other,
                (Type::Integer, Type::Integer) => Type::Integer,
                _ => Type::Real,
            })
        }
        Equal | NotEqual | Less | LessEqual | Greater | GreaterEqual => {
            let comparable = (left.ty.is_number() && right.ty.is_number())
                || matches!(
                    (left.ty, right.ty),
                    (Type::Text | Type::Null, Type::Text | Type::Null)
                );
            if !comparable {
                let message = format!(
                    "this is {}, and {sign} compares it with {}",
                    right.ty, left.ty
                );
                return invalid(right.span, message);
            }
            Ok(Type::Condition)
        }
        And | Or => {
            for side in [left, right] {
                if !matches!(side.ty, Type::Condition | Type::Null) {
                    let message =
                        format!("{sign} needs a condition, and this is {}", side.ty);
                    return invalid(side.span, message);
                }
            }
            Ok(Type::Condition)
        }
    }
}
// @end

// @listing bind
/// Checks an expression against the columns in scope.
pub fn bind(expr: &Expr, scope: &Scope) -> Result<Bound> {
    let span = expr.span;
    let (kind, ty) = match &expr.kind {
        ExprKind::Literal(value) => {
            let ty = match value {
                Value::Null => Type::Null,
                Value::Integer(_) => Type::Integer,
                Value::Real(_) => Type::Real,
                Value::Text(_) => Type::Text,
            };
            (BoundKind::Literal(value.clone()), ty)
        }
        ExprKind::Column { table, name } => {
            let (position, ty) = scope.resolve(table.as_deref(), name, span)?;
            (BoundKind::Column(position), ty)
        }
        ExprKind::Unary { op, operand } => {
            let operand = Box::new(bind(operand, scope)?);
            let ty = match op {
                UnaryOp::Negate if operand.ty.is_number() => operand.ty,
                UnaryOp::Not
                    if matches!(operand.ty, Type::Condition | Type::Null) =>
                {
                    Type::Condition
                }
                UnaryOp::Negate => {
                    let message =
                        format!("- needs a number, and this is {}", operand.ty);
                    return invalid(operand.span, message);
                }
                UnaryOp::Not => {
                    let message =
                        format!("NOT needs a condition, and this is {}", operand.ty);
                    return invalid(operand.span, message);
                }
            };
            (BoundKind::Unary { op: *op, operand }, ty)
        }
        ExprKind::Binary { op, left, right } => {
            let (left, right) =
                (Box::new(bind(left, scope)?), Box::new(bind(right, scope)?));
            let ty = binary_type(*op, &left, &right)?;
            (BoundKind::Binary { op: *op, left, right }, ty)
        }
        ExprKind::IsNull { operand, negated } => {
            let operand = Box::new(bind(operand, scope)?);
            (BoundKind::IsNull { operand, negated: *negated }, Type::Condition)
        }
        // @hide
        ExprKind::Call { function, .. } => {
            let name = function.name();
            return invalid(
                span,
                format!("{name} is an aggregate, and can't be used here"),
            );
        } // @show
    };
    Ok(Bound { kind, ty, span })
}
// @end

// @listing bind-condition
/// Checks an expression that has to be a condition, such as the one after WHERE.
/// `place` names where it is, for the error message.
pub fn bind_condition(expr: &Expr, scope: &Scope, place: &str) -> Result<Bound> {
    let bound = bind(expr, scope)?;
    if !matches!(bound.ty, Type::Condition | Type::Null) {
        let message = format!("{place} needs a condition, and this is {}", bound.ty);
        return invalid(bound.span, message);
    }
    Ok(bound)
}
// @end
