//! Chapter 14 challenges: counting calls, DISTINCT, inserting values without SQL,
//! and filling one table from a query on another.

use crate::catalog::sql_error;
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::exec::{Operator, Row};
use crate::pager::Pager;
use crate::record::{Value, encode};
use crate::sql::{ColumnType, Statement, parse};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

// @listing counted
/// Wraps an operator and counts how many times it is asked for a row.
pub struct Counted {
    pub input: Box<dyn Operator>,
    pub calls: Rc<Cell<usize>>,
}

impl Operator for Counted {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        self.calls.set(self.calls.get() + 1);
        self.input.next(pager)
    }
}
// @end

// @listing distinct
/// Passes on each different row of its input once, the first time it appears.
pub struct Distinct {
    pub input: Box<dyn Operator>,
    /// The rows passed on so far, in their encoded form.
    pub seen: BTreeSet<Vec<u8>>,
}

impl Operator for Distinct {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        while let Some(row) = self.input.next(pager)? {
            if self.seen.insert(encode(&row)) {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }
}
// @end

// @listing insert-values
/// Adds one row to a table from values, with the checks INSERT makes. Nothing is
/// flushed. Returns the new row's id.
pub fn insert_values(db: &mut Database, table: &str, mut row: Row) -> Result<i64> {
    let table = db.table(table)?.clone();
    // @hide
    crate::index::no_indexes(&table)?;
    // @show
    if row.len() != table.columns.len() {
        return sql_error(format!(
            "{} values for {} columns",
            row.len(),
            table.columns.len()
        ));
    }
    for (position, column) in table.columns.iter().enumerate() {
        let value = &mut row[position];
        let fits = match (&*value, column.kind) {
            (Value::Null, _) => {
                !column.not_null || table.key_column == Some(position)
            }
            (Value::Integer(_), ColumnType::Integer) => true,
            (Value::Real(_), ColumnType::Real) => true,
            (Value::Text(_), ColumnType::Text) => true,
            (Value::Integer(i), ColumnType::Real) => {
                *value = Value::Real(*i as f64);
                true
            }
            _ => false,
        };
        if !fits {
            return sql_error(format!(
                "the column {} cannot hold {value}",
                column.name
            ));
        }
    }
    let id =
        match table.key_column.map(|k| std::mem::replace(&mut row[k], Value::Null)) {
            Some(Value::Integer(id)) => id,
            _ => table.tree.next_row_id(&mut db.pager)?,
        };
    match table.tree.insert_row(&mut db.pager, id, &encode(&row)) {
        Err(Error::DuplicateKey) => {
            sql_error(format!("there is already a row with id {id}"))
        }
        result => result.map(|()| id),
    }
}
// @end

// @listing insert-select
/// Runs a query and inserts every row of its result into a table. One flush at
/// the end covers them all. Returns how many rows were inserted.
pub fn insert_select(db: &mut Database, into: &str, query: &str) -> Result<usize> {
    // Look before running: this must not carry out some other kind of statement.
    if !matches!(parse(query)?, Statement::Select(_)) {
        return sql_error("that statement is not a query".to_string());
    }
    let Output::Rows(result) = db.execute(query)? else {
        return sql_error("that query returned no rows to insert".to_string());
    };
    let count = result.rows.len();
    for row in result.rows {
        insert_values(db, into, row)?;
    }
    db.pager.flush()?;
    Ok(count)
}
// @end
