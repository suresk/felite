//! DELETE and UPDATE: finding the rows first, and only then changing them.

use crate::bind::{Bound, Scope, Type, bind, bind_condition};
use crate::catalog::{Table, fold, sql_error};
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::eval::{eval, truth};
use crate::exec::{Operator, Row, Scan};
use crate::record::{Value, encode};
use crate::sql::{ColumnType, Delete, Expr, Update};
use std::collections::BTreeSet;

impl Database {
    // @listing matching
    /// Every row of a table that a WHERE clause selects, with its row id, read to
    /// the end before anything is returned.
    pub(crate) fn matching(
        &mut self,
        table: &Table,
        filter: &Option<Expr>,
    ) -> Result<Vec<(i64, Row)>> {
        let condition = match filter {
            Some(filter) => Some(bind_condition(filter, &Scope::of(table), "WHERE")?),
            None => None,
        };
        let mut scan = Scan::new(table, &mut self.pager)?;
        let mut found = Vec::new();
        while let Some(row) = scan.next(&mut self.pager)? {
            let wanted = match &condition {
                Some(condition) => truth(&eval(condition, &row)?) == Some(true),
                None => true,
            };
            if wanted {
                found.push((scan.row_id, row));
            }
        }
        Ok(found)
    }
    // @end

    // @listing delete
    pub(crate) fn delete(&mut self, delete: &Delete) -> Result<Output> {
        let table = self.table(&delete.table)?.clone();
        let doomed = self.matching(&table, &delete.filter)?;
        // @hide
        for (id, row) in &doomed {
            table.unindex_row(&mut self.pager, *id, row)?;
        }
        // @show
        for (id, _) in &doomed {
            table.tree.delete_row(&mut self.pager, *id)?;
        }
        Ok(Output::Count(doomed.len()))
    }
    // @end
}

// @listing assignments
/// Checks the SET clause of an UPDATE. Returns, for each assignment, the position
/// of its column and the bound expression for the new value.
fn assignments(table: &Table, update: &Update) -> Result<Vec<(usize, Bound)>> {
    let scope = Scope::of(table);
    let mut checked: Vec<(usize, Bound)> = Vec::new();
    for (name, expr) in &update.assignments {
        let found = table.columns.iter().position(|c| fold(&c.name) == fold(name));
        let Some(position) = found else {
            return sql_error(format!(
                "the table {} has no column named {name}",
                table.name
            ));
        };
        if checked.iter().any(|(earlier, _)| *earlier == position) {
            return sql_error(format!("the column {name} is set twice"));
        }
        let bound = bind(expr, &scope)?;
        let wanted: Type = table.columns[position].kind.into();
        let fits = bound.ty == wanted
            || bound.ty == Type::Null
            || (bound.ty == Type::Integer && wanted == Type::Real);
        if !fits {
            let message =
                format!("the column {name} is {wanted}, and this is {}", bound.ty);
            return Err(Error::Invalid(message, bound.span));
        }
        checked.push((position, bound));
    }
    Ok(checked)
}
// @end

// @listing new-row
/// The row that an UPDATE turns `old` into, and its row id, which is `id` unless
/// the key column was assigned to.
fn new_row(
    table: &Table,
    sets: &[(usize, Bound)],
    id: i64,
    old: &Row,
) -> Result<(i64, Row)> {
    let mut row = old.clone();
    for (position, expr) in sets {
        // Every expression sees the row as it was, not as it is becoming.
        let to_real = table.columns[*position].kind == ColumnType::Real;
        row[*position] = match eval(expr, old)? {
            Value::Integer(i) if to_real => Value::Real(i as f64),
            value => value,
        };
    }
    let mut new_id = id;
    for (position, column) in table.columns.iter().enumerate() {
        let name = &column.name;
        if table.key_column == Some(position) {
            match std::mem::replace(&mut row[position], Value::Null) {
                Value::Integer(given) => new_id = given,
                _ => {
                    return sql_error(format!(
                        "the key column {name} cannot be set to NULL"
                    ));
                }
            }
        } else if column.not_null && row[position] == Value::Null {
            return sql_error(format!("the column {name} cannot be NULL"));
        }
    }
    Ok((new_id, row))
}
// @end

impl Database {
    // @listing update
    pub(crate) fn update(&mut self, update: &Update) -> Result<Output> {
        let table = self.table(&update.table)?.clone();
        let sets = assignments(&table, update)?;
        let old = self.matching(&table, &update.filter)?;
        // Work out every new row before changing any, so that a row which cannot
        // be computed stops the statement while the table is still untouched.
        let mut new = Vec::with_capacity(old.len());
        for (id, row) in &old {
            new.push(new_row(&table, &sets, *id, row)?);
        }
        self.check_new_ids(&table, &old, &new)?;
        // @hide
        let leaving: BTreeSet<i64> = old.iter().map(|(id, _)| *id).collect();
        table.check_indexes(&mut self.pager, &leaving, &new)?;
        for (id, row) in &old {
            table.unindex_row(&mut self.pager, *id, row)?;
        }
        for (id, row) in &new {
            table.index_row(&mut self.pager, *id, row)?;
        }
        // @show
        for (id, _) in &old {
            table.tree.delete_row(&mut self.pager, *id)?;
        }
        for (id, row) in &new {
            table.tree.insert_row(&mut self.pager, *id, &encode(row))?;
        }
        Ok(Output::Count(new.len()))
    }
    // @end

    // @listing check-new-ids
    /// Makes sure that the row ids an UPDATE is about to use will not collide: not
    /// with each other, and not with a row that the UPDATE leaves alone.
    fn check_new_ids(
        &mut self,
        table: &Table,
        old: &[(i64, Row)],
        new: &[(i64, Row)],
    ) -> Result<()> {
        let leaving: BTreeSet<i64> = old.iter().map(|(id, _)| *id).collect();
        let mut arriving = BTreeSet::new();
        for (id, _) in new {
            let taken = !arriving.insert(*id)
                || (!leaving.contains(id)
                    && table.tree.get_row(&mut self.pager, *id)?.is_some());
            if taken {
                return sql_error(format!(
                    "two rows of {} would have the id {id}",
                    table.name
                ));
            }
        }
        Ok(())
    }
    // @end
}
