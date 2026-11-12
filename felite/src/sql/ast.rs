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
    // @hide
    /// `DROP TABLE name`
    DropTable(String),
    Delete(Delete),
    Update(Update),
    Begin,
    Commit,
    Rollback,
    // @show
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
    // @hide
    /// What to sort the result by. Empty if the statement has no ORDER BY.
    pub order_by: Vec<OrderKey>,
    /// The most rows to return, if there is a LIMIT.
    pub limit: Option<usize>,
    /// How many rows to pass over first, if there is an OFFSET.
    pub offset: Option<usize>,
    /// What to group the rows by. Empty if the statement has no GROUP BY.
    pub group_by: Vec<Expr>,
    /// The condition on groups, if there is a HAVING.
    pub having: Option<Expr>,
    /// Another name for the table in `from`, if the statement gives it one.
    pub alias: Option<String>,
    /// The tables joined to it, in the order they are written.
    pub joins: Vec<Join>,
    // @show
}

/// One thing a SELECT asks for: every column, or an expression and perhaps a
/// name to give it.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectItem {
    Star,
    Expr { expr: Expr, alias: Option<String> },
}
// @end

// @listing join
/// `[LEFT] JOIN table [alias] ON condition`
#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub table: String,
    pub alias: Option<String>,
    /// Whether this is a LEFT JOIN, which keeps the rows that match nothing.
    pub left: bool,
    pub on: Expr,
}
// @end

// @listing order-key
/// One key of an ORDER BY: an expression, and which way to sort by it.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderKey {
    pub expr: Expr,
    pub descending: bool,
}
// @end

// @listing delete-update
/// `DELETE FROM table [WHERE expression]`
#[derive(Debug, Clone, PartialEq)]
pub struct Delete {
    pub table: String,
    pub filter: Option<Expr>,
}

/// `UPDATE table SET column = expression, … [WHERE expression]`
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub table: String,
    /// Each column to change, with the expression for its new value.
    pub assignments: Vec<(String, Expr)>,
    pub filter: Option<Expr>,
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
    // @hide
    /// An aggregate function and what it is applied to: `SUM(delay)`. Only
    /// `COUNT(*)` has no argument.
    Call {
        function: Function,
        arg: Option<Box<Expr>>,
    },
    // @show
}
// @end

// @listing function
/// The aggregate functions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Function {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

impl Function {
    const ALL: [(&str, Function); 5] = [
        ("COUNT", Function::Count),
        ("SUM", Function::Sum),
        ("MIN", Function::Min),
        ("MAX", Function::Max),
        ("AVG", Function::Avg),
    ];

    /// The function a word names, in any mix of upper and lower case.
    pub fn from_word(word: &str) -> Option<Function> {
        let found =
            Function::ALL.iter().find(|(name, _)| name.eq_ignore_ascii_case(word));
        found.map(|(_, function)| *function)
    }

    /// How the function is written, in capitals.
    pub fn name(self) -> &'static str {
        let found = Function::ALL.iter().find(|(_, function)| *function == self);
        found.map_or("?", |(name, _)| name)
    }
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
            // @hide
            ExprKind::Call { function, arg: Some(arg) } => {
                write!(f, "{}({arg})", function.name())
            }
            ExprKind::Call { function, arg: None } => {
                write!(f, "{}(*)", function.name())
            } // @show
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

// @listing display-joins
impl Select {
    /// Writes the first table's other name, if it has one, and every JOIN.
    fn write_joins(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let named = |alias: &Option<String>| match alias {
            Some(alias) => format!(" {alias}"),
            None => String::new(),
        };
        write!(f, "{}", named(&self.alias))?;
        for join in &self.joins {
            let kind = if join.left { " LEFT JOIN" } else { " JOIN" };
            write!(f, "{kind} {}{} ON {}", join.table, named(&join.alias), join.on)?;
        }
        Ok(())
    }
}
// @end

// @listing display-order
impl Select {
    /// Whether anything comes after the WHERE clause.
    fn has_tail(&self) -> bool {
        // @hide
        if !self.group_by.is_empty() || self.having.is_some() {
            return true;
        }
        // @show
        !self.order_by.is_empty() || self.limit.is_some() || self.offset.is_some()
    }

    /// Writes the WHERE clause and everything after it.
    fn write_tail(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(filter) = &self.filter {
            write!(f, " WHERE {filter}")?;
        }
        // @hide
        for (i, group) in self.group_by.iter().enumerate() {
            let before = if i == 0 { " GROUP BY " } else { ", " };
            write!(f, "{before}{group}")?;
        }
        if let Some(having) = &self.having {
            write!(f, " HAVING {having}")?;
        }
        // @show
        for (i, key) in self.order_by.iter().enumerate() {
            let before = if i == 0 { " ORDER BY " } else { ", " };
            let direction = if key.descending { " DESC" } else { "" };
            write!(f, "{before}{}{direction}", key.expr)?;
        }
        if let Some(limit) = self.limit {
            write!(f, " LIMIT {limit}")?;
        }
        if let Some(offset) = self.offset {
            write!(f, " OFFSET {offset}")?;
        }
        Ok(())
    }
}
// @end

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
            // @hide
            Statement::DropTable(name) => write!(f, "DROP TABLE {name}"),
            Statement::Delete(delete) => {
                write!(f, "DELETE FROM {}", delete.table)?;
                match &delete.filter {
                    Some(filter) => write!(f, " WHERE {filter}"),
                    None => Ok(()),
                }
            }
            Statement::Update(update) => {
                write!(f, "UPDATE {} SET ", update.table)?;
                for (i, (column, value)) in update.assignments.iter().enumerate() {
                    let comma = if i > 0 { ", " } else { "" };
                    write!(f, "{comma}{column} = {value}")?;
                }
                match &update.filter {
                    Some(filter) => write!(f, " WHERE {filter}"),
                    None => Ok(()),
                }
            }
            Statement::Begin => write!(f, "BEGIN"),
            Statement::Commit => write!(f, "COMMIT"),
            Statement::Rollback => write!(f, "ROLLBACK"),
            // @show
            Statement::Select(select) => {
                write!(f, "SELECT ")?;
                list(f, &select.items)?;
                write!(f, " FROM {}", select.from)?;
                // @hide
                select.write_joins(f)?;
                // @show
                // @hide
                if select.has_tail() {
                    return select.write_tail(f);
                }
                // @show
                match &select.filter {
                    Some(filter) => write!(f, " WHERE {filter}"),
                    None => Ok(()),
                }
            }
        }
    }
}
// @end
