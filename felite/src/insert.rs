//! INSERT: from a statement to one new row in a table's tree.

use crate::bind::{Scope, Type, bind};
use crate::catalog::{Table, fold, sql_error};
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::eval::eval;
use crate::record::{Value, encode};
use crate::sql::{ColumnDef, Expr, Insert};
use std::collections::BTreeSet;

// @listing targets
/// The position in the table of each column an INSERT names, in the order it
/// names them. With no list, that is every column in order.
fn targets(table: &Table, names: &[String]) -> Result<Vec<usize>> {
    if names.is_empty() {
        return Ok((0..table.columns.len()).collect());
    }
    let mut positions = Vec::new();
    for name in names {
        let found = table.columns.iter().position(|c| fold(&c.name) == fold(name));
        let Some(position) = found else {
            return sql_error(format!(
                "the table {} has no column named {name}",
                table.name
            ));
        };
        if positions.contains(&position) {
            return sql_error(format!("the column {name} is given a value twice"));
        }
        positions.push(position);
    }
    Ok(positions)
}
// @end

// @listing value-for
/// Evaluates the expression given for a column and checks that the column can
/// hold the result.
fn value_for(column: &ColumnDef, expr: &Expr) -> Result<Value> {
    // An INSERT's values stand alone: there is no row for them to refer to.
    let bound = bind(expr, &Scope { columns: Vec::new() })?;
    let wanted: Type = column.kind.into();
    let fits = bound.ty == wanted
        || bound.ty == Type::Null
        || (bound.ty == Type::Integer && wanted == Type::Real);
    if !fits {
        let message = format!(
            "the column {} is {wanted}, and this is {}",
            column.name, bound.ty
        );
        return Err(Error::Invalid(message, bound.span));
    }
    Ok(match eval(&bound, &[])? {
        Value::Integer(i) if wanted == Type::Real => Value::Real(i as f64),
        value => value,
    })
}
// @end

impl Database {
    // @listing insert
    pub(crate) fn insert(&mut self, insert: &Insert) -> Result<Output> {
        let table = self.table(&insert.table)?.clone();
        let positions = targets(&table, &insert.columns)?;
        if positions.len() != insert.values.len() {
            let (values, columns) = (insert.values.len(), positions.len());
            return sql_error(format!("{values} values for {columns} columns"));
        }
        // Columns that are not mentioned are NULL.
        let mut row = vec![Value::Null; table.columns.len()];
        for (position, expr) in positions.iter().zip(&insert.values) {
            row[*position] = value_for(&table.columns[*position], expr)?;
        }
        // The key column's value is the row id, and is not stored a second time.
        let key = table.key_column;
        let id = match key.map(|k| std::mem::replace(&mut row[k], Value::Null)) {
            Some(Value::Integer(id)) => id,
            _ => table.tree.next_row_id(&mut self.pager)?,
        };
        for (position, column) in table.columns.iter().enumerate() {
            let name = &column.name;
            if column.not_null
                && row[position] == Value::Null
                && key != Some(position)
            {
                return sql_error(format!("the column {name} cannot be NULL"));
            }
        }
        // @hide
        // The row goes into the indexes first, unless its id is taken, in which
        // case the insert below will refuse it.
        if table.tree.get_row(&mut self.pager, id)?.is_none() {
            let arriving = [(id, row.clone())];
            table.check_indexes(&mut self.pager, &BTreeSet::new(), &arriving)?;
            table.index_row(&mut self.pager, id, &row)?;
        }
        // @show
        let name = &table.name;
        match table.tree.insert_row(&mut self.pager, id, &encode(&row)) {
            Err(Error::DuplicateKey) => {
                sql_error(format!("the table {name} already has a row with id {id}"))
            }
            result => result.map(|()| Output::Count(1)),
        }
    }
    // @end
}
