//! Reading comma-separated values, and loading them into a table.

use crate::catalog::{Table, sql_error};
use crate::db::Database;
use crate::error::{Error, Result};
use crate::record::Value;
use crate::sql::{ColumnType, Expr, ExprKind, Insert, Span};
use crate::transaction::Transaction;
use std::io::BufRead;

// @listing csv-record
/// Reads one record of comma-separated values: the fields of one row. A field
/// in double quotes may contain commas, line ends, and quotes written twice.
/// Returns `None` at the end of the input.
pub fn read_record(input: &mut impl BufRead) -> Result<Option<Vec<String>>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let (mut fields, mut field, mut quoted) = (Vec::new(), String::new(), false);
    loop {
        let mut chars = line.trim_end_matches(['\n', '\r']).chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                // Two quotes inside a quoted field are one quote.
                ('"', true) if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                ('"', _) => quoted = !quoted,
                (',', false) => fields.push(std::mem::take(&mut field)),
                _ => field.push(c),
            }
        }
        if !quoted {
            fields.push(field);
            return Ok(Some(fields));
        }
        // The line ended inside quotes, so the field goes on to the next line.
        field.push('\n');
        line.clear();
        if input.read_line(&mut line)? == 0 {
            let what = "the input ends inside a quoted field";
            return sql_error(what.to_string());
        }
    }
}
// @end

// @listing csv-value
/// A field of a record as a value for a column. An empty field is NULL.
fn value_of(field: &str, column: ColumnType, name: &str) -> Result<Value> {
    let bad =
        || Error::Sql(format!("{field:?} is not a value for the column {name}"));
    Ok(match column {
        _ if field.is_empty() => Value::Null,
        ColumnType::Integer => Value::Integer(field.parse().map_err(|_| bad())?),
        ColumnType::Real => match field.parse::<f64>() {
            Ok(x) if x.is_finite() => Value::Real(x),
            _ => return Err(bad()),
        },
        ColumnType::Text => Value::Text(field.to_string()),
    })
}

/// An INSERT of one row of values, as the parser would have produced it.
fn insert_of(table: &Table, values: Vec<Value>) -> Insert {
    let span = Span { offset: 0, line: 1, column: 1 };
    let literal = |value| Expr { kind: ExprKind::Literal(value), span };
    Insert {
        table: table.name.clone(),
        columns: Vec::new(),
        values: values.into_iter().map(literal).collect(),
    }
}
// @end

// @listing import-batch
/// How many rows `import` puts in each transaction.
pub const IMPORT_BATCH: usize = 10_000;
// @end

impl Database {
    // @listing import
    /// Reads records of comma-separated values and inserts each as a row of a
    /// table. If the first record is the names of the table's columns, it is
    /// passed over. Returns the number of rows inserted.
    pub fn import(&mut self, table: &str, input: &mut impl BufRead) -> Result<usize> {
        let imported = self.import_rows(table, input);
        // A failure partway through a batch takes the whole batch back.
        if imported.is_err() && self.transaction == Transaction::Open {
            self.execute("ROLLBACK")?;
        }
        imported
    }
    // @end

    // @listing import-rows
    fn import_rows(
        &mut self,
        table: &str,
        input: &mut impl BufRead,
    ) -> Result<usize> {
        let table = self.table(table)?.clone();
        let names: Vec<&str> =
            table.columns.iter().map(|c| c.name.as_str()).collect();
        let logged = self.pager.logged();
        let mut rows = 0;
        while let Some(fields) = read_record(input)? {
            let header = fields.iter().map(String::as_str).eq(names.iter().copied());
            if rows == 0 && header {
                continue;
            }
            if fields.len() != names.len() {
                let (row, found, wanted) = (rows + 1, fields.len(), names.len());
                return sql_error(format!(
                    "row {row} has {found} fields, and not {wanted}"
                ));
            }
            let mut values = Vec::with_capacity(fields.len());
            for (field, column) in fields.iter().zip(&table.columns) {
                values.push(value_of(field, column.kind, &column.name)?);
            }
            // Each batch of rows is one transaction: one sync, and all of the
            // batch or none of it if the program stops.
            if logged && rows % IMPORT_BATCH == 0 {
                if rows > 0 {
                    self.execute("COMMIT")?;
                }
                self.execute("BEGIN")?;
            }
            self.insert(&insert_of(&table, values))?;
            rows += 1;
        }
        if logged && rows > 0 {
            self.execute("COMMIT")?;
        } else if !logged {
            self.pager.flush()?;
        }
        Ok(rows)
    }
    // @end
}
