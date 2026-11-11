//! Syntax trees: what a statement is, once its words have been given a shape.

use crate::record::Value;
use crate::sql::token::Span;
use std::fmt;

// @listing statement
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    CreateTable(CreateTable),
    Insert(Insert),
    Select(Select),
}
// @end

// @listing create-table
/// `CREATE TABLE name (column, …)`
#[derive(Debug, Clone, PartialEq)]
pub struct CreateTable {
    pub name: String,
    pub columns: Vec<ColumnDef>,
}

/// One column of a new table: `name TYPE [PRIMARY KEY] [NOT NULL]`
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub kind: ColumnType,
    pub primary_key: bool,
    pub not_null: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnType {
    Integer,
    Real,
    Text,
}
// @end

// @listing insert-select
/// `INSERT INTO table [(column, …)] VALUES (expression, …)`
#[derive(Debug, Clone, PartialEq)]
pub struct Insert {
    pub table: String,
    /// The columns named, or none if the statement gave no list.
    pub columns: Vec<String>,
    pub values: Vec<Expr>,
}

/// `SELECT item, … FROM table [WHERE expression]`
#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub items: Vec<SelectItem>,
    pub from: String,
    pub filter: Option<Expr>,
}

/// One thing a SELECT asks for: every column, or an expression and perhaps a
/// name to give it.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectItem {
    Star,
    Expr { expr: Expr, alias: Option<String> },
}
// @end

// @listing expr
/// An expression, and where in the text it began.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A number, a string or NULL, written out in the statement.
    Literal(Value),
    /// A column, as `name` or `table.name`.
    Column {
        table: Option<String>,
        name: String,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `operand IS NULL`, or `IS NOT NULL` if `negated`.
    IsNull {
        operand: Box<Expr>,
        negated: bool,
    },
}
// @end

// @listing operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[rustfmt::skip]
pub enum BinaryOp {
    Add, Subtract, Multiply, Divide, Remainder,
    Equal, NotEqual, Less, LessEqual, Greater, GreaterEqual,
    And, Or,
}

impl BinaryOp {
    /// How the operator is written.
    pub fn spelling(self) -> &'static str {
        use BinaryOp::*;
        match self {
            Add => "+",
            Subtract => "-",
            Multiply => "*",
            Divide => "/",
            Remainder => "%",
            Equal => "=",
            NotEqual => "<>",
            Less => "<",
            LessEqual => "<=",
            Greater => ">",
            GreaterEqual => ">=",
            And => "AND",
            Or => "OR",
        }
    }
}
// @end

// @listing display-expr
/// An expression as SQL, with parentheses around every operation, so that the
/// grouping the parser chose can be read off.
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.kind {
            ExprKind::Literal(Value::Null) => write!(f, "NULL"),
            ExprKind::Literal(Value::Integer(i)) => write!(f, "{i}"),
            ExprKind::Literal(Value::Real(x)) => write!(f, "{x:?}"),
            ExprKind::Literal(Value::Text(s)) => {
                write!(f, "'{}'", s.replace('\'', "''"))
            }
            ExprKind::Column { table: Some(table), name } => {
                write!(f, "{table}.{name}")
            }
            ExprKind::Column { table: None, name } => write!(f, "{name}"),
            ExprKind::Unary { op: UnaryOp::Negate, operand } => {
                write!(f, "(-{operand})")
            }
            ExprKind::Unary { op: UnaryOp::Not, operand } => {
                write!(f, "(NOT {operand})")
            }
            ExprKind::Binary { op, left, right } => {
                write!(f, "({left} {} {right})", op.spelling())
            }
            ExprKind::IsNull { operand, negated: false } => {
                write!(f, "({operand} IS NULL)")
            }
            ExprKind::IsNull { operand, negated: true } => {
                write!(f, "({operand} IS NOT NULL)")
            }
        }
    }
}
// @end

// @listing display-statement
/// Writes `items` with commas between them.
fn list<T: fmt::Display>(f: &mut fmt::Formatter, items: &[T]) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        let comma = if i > 0 { ", " } else { "" };
        write!(f, "{comma}{item}")?;
    }
    Ok(())
}

impl fmt::Display for ColumnDef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let kind = match self.kind {
            ColumnType::Integer => "INTEGER",
            ColumnType::Real => "REAL",
            ColumnType::Text => "TEXT",
        };
        let key = if self.primary_key { " PRIMARY KEY" } else { "" };
        let not_null = if self.not_null { " NOT NULL" } else { "" };
        write!(f, "{} {kind}{key}{not_null}", self.name)
    }
}

impl fmt::Display for SelectItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SelectItem::Star => write!(f, "*"),
            SelectItem::Expr { expr, alias: Some(alias) } => {
                write!(f, "{expr} AS {alias}")
            }
            SelectItem::Expr { expr, alias: None } => write!(f, "{expr}"),
        }
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Statement::CreateTable(create) => {
                write!(f, "CREATE TABLE {} (", create.name)?;
                list(f, &create.columns)?;
                write!(f, ")")
            }
            Statement::Insert(insert) => {
                write!(f, "INSERT INTO {}", insert.table)?;
                if !insert.columns.is_empty() {
                    write!(f, " (")?;
                    list(f, &insert.columns)?;
                    write!(f, ")")?;
                }
                write!(f, " VALUES (")?;
                list(f, &insert.values)?;
                write!(f, ")")
            }
            Statement::Select(select) => {
                write!(f, "SELECT ")?;
                list(f, &select.items)?;
                write!(f, " FROM {}", select.from)?;
                match &select.filter {
                    Some(filter) => write!(f, " WHERE {filter}"),
                    None => Ok(()),
                }
            }
        }
    }
}
// @end
