//! Chapter 24 challenges: measuring what the planner guesses at.

use crate::bind::{Scope, bind_condition};
use crate::db::Database;
use crate::error::Result;
use crate::eval::{eval, truth};
use crate::exec::{Operator, Scan};
use crate::sort::Key;
use crate::sql::parse_expr;
use std::collections::BTreeSet;

// @listing selectivity
/// The part of a table's rows for which a condition, given as SQL, is true.
pub fn selectivity(db: &mut Database, table: &str, condition: &str) -> Result<f64> {
    let table = db.table(table)?.clone();
    let condition =
        bind_condition(&parse_expr(condition)?, &Scope::of(&table), "WHERE")?;
    let mut scan = Scan::new(&table, &mut db.pager)?;
    let (mut rows, mut matching) = (0, 0);
    while let Some(row) = scan.next(&mut db.pager)? {
        rows += 1;
        if truth(&eval(&condition, &row)?) == Some(true) {
            matching += 1;
        }
    }
    Ok(if rows == 0 { 0.0 } else { matching as f64 / rows as f64 })
}
// @end

// @listing distinct-counts
/// For each column of a table, its name and how many different values it has,
/// not counting NULL.
pub fn distinct_counts(db: &mut Database, table: &str) -> Result<Vec<(String, u64)>> {
    let table = db.table(table)?.clone();
    let mut seen = vec![BTreeSet::new(); table.columns.len()];
    let mut scan = Scan::new(&table, &mut db.pager)?;
    while let Some(row) = scan.next(&mut db.pager)? {
        for (value, seen) in row.into_iter().zip(&mut seen) {
            if value != crate::record::Value::Null {
                seen.insert(Key { value, descending: false });
            }
        }
    }
    let names = table.columns.iter().map(|column| column.name.clone());
    Ok(names.zip(seen.iter().map(|seen| seen.len() as u64)).collect())
}
// @end

// @listing join-orders
/// The number of orders in which `tables` tables can be joined one after
/// another, or `None` if it is too large for a `u64`.
pub fn join_orders(tables: u32) -> Option<u64> {
    (1..=tables as u64).try_fold(1u64, |orders, n| orders.checked_mul(n))
}
// @end
