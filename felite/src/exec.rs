//! Operators: the moving parts of a query, each handing rows to the next.

use crate::bind::Bound;
use crate::btree::{Cursor, row_id};
use crate::catalog::Table;
use crate::error::{Error, Result};
use crate::eval::{eval, truth};
use crate::overflow::load;
use crate::pager::Pager;
use crate::record::{Value, decode};

// @listing operator
/// One row of a table or of a result: a value for each column.
pub type Row = Vec<Value>;

/// A source of rows, asked for one at a time.
pub trait Operator {
    /// The next row, or `None` when there are no more.
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>>;
}
// @end

// @listing scan
/// Reads every row of a table, in order of row id.
pub struct Scan {
    cursor: Cursor,
    /// How many columns a row of this table has.
    columns: usize,
    /// The column whose value is the row id, if the table has one.
    key_column: Option<usize>,
    // @hide
    /// The row id of the row that `next` returned last.
    pub row_id: i64,
    // @show
}

impl Scan {
    pub fn new(table: &Table, pager: &mut Pager) -> Result<Scan> {
        let cursor = Cursor::first(&table.tree, pager)?;
        Ok(Scan {
            cursor,
            columns: table.columns.len(),
            key_column: table.key_column,
            // @hide
            row_id: 0,
            // @show
        })
    }
}

impl Operator for Scan {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        let Some((key, stored)) = self.cursor.next(pager)? else {
            return Ok(None);
        };
        // @hide
        self.row_id = row_id(&key)?;
        // @show
        let mut row = decode(&load(pager, stored)?)?;
        if row.len() != self.columns {
            let what = format!(
                "a row of {} values in a table of {}",
                row.len(),
                self.columns
            );
            return Err(Error::Corrupt(what));
        }
        if let Some(position) = self.key_column {
            row[position] = Value::Integer(row_id(&key)?);
        }
        Ok(Some(row))
    }
}
// @end

// @listing filter
/// Passes on the rows of its input for which a condition is true.
pub struct Filter {
    pub input: Box<dyn Operator>,
    pub condition: Bound,
}

impl Operator for Filter {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        while let Some(row) = self.input.next(pager)? {
            if truth(&eval(&self.condition, &row)?) == Some(true) {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }
}
// @end

// @listing project
/// Turns each row of its input into a new row, one value for each expression.
pub struct Project {
    pub input: Box<dyn Operator>,
    pub exprs: Vec<Bound>,
}

impl Operator for Project {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        let Some(row) = self.input.next(pager)? else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(self.exprs.len());
        for expr in &self.exprs {
            out.push(eval(expr, &row)?);
        }
        Ok(Some(out))
    }
}
// @end

// @listing limit
/// Passes over the first `skip` rows of its input, hands on the next `take`, and
/// then stops asking for more.
pub struct Limit {
    pub input: Box<dyn Operator>,
    pub skip: usize,
    /// How many rows are still to be handed on. `None` means all of them.
    pub take: Option<usize>,
}

impl Operator for Limit {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        if self.take == Some(0) {
            return Ok(None);
        }
        while self.skip > 0 {
            if self.input.next(pager)?.is_none() {
                return Ok(None);
            }
            self.skip -= 1;
        }
        let row = self.input.next(pager)?;
        if let (Some(_), Some(take)) = (&row, &mut self.take) {
            *take -= 1;
        }
        Ok(row)
    }
}
// @end
