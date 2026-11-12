//! Chapter 21 challenges: counting distinct values, grouping with a hash table,
//! grouping input that is already sorted, and an aggregate that needs every row.

use crate::bind::{Bound, Scope, bind};
use crate::catalog::Table;
use crate::db::Database;
use crate::error::Result;
use crate::eval::{eval, order};
use crate::exec::{Operator, Row, Scan};
use crate::pager::Pager;
use crate::record::{Value, encode};
use crate::sort::{Key, Sort, SortKey};
use crate::sql::parse_expr;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

// @listing table-expr
/// A table, and an expression over its columns that was given as SQL text.
fn table_and_expr(db: &Database, table: &str, expr: &str) -> Result<(Table, Bound)> {
    let table = db.table(table)?.clone();
    let expr = bind(&parse_expr(expr)?, &Scope::of(&table))?;
    Ok((table, expr))
}
// @end

// @listing count-distinct
/// How many different values an expression has over a table, not counting NULL.
pub fn count_distinct(db: &mut Database, table: &str, expr: &str) -> Result<i64> {
    let (table, expr) = table_and_expr(db, table, expr)?;
    let mut scan = Scan::new(&table, &mut db.pager)?;
    let mut seen = BTreeSet::new();
    while let Some(row) = scan.next(&mut db.pager)? {
        let value = eval(&expr, &row)?;
        if value != Value::Null {
            seen.insert(Key { value, descending: false });
        }
    }
    Ok(seen.len() as i64)
}
// @end

// @listing fnv
/// The FNV-1a hash function: short, quick, and the same on every run.
#[derive(Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ *byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
// @end

// @listing hash-counts
/// How many rows of a table have each value of an expression, found with a hash
/// table and then put in order.
pub fn hash_counts(
    db: &mut Database,
    table: &str,
    expr: &str,
) -> Result<Vec<(Value, i64)>> {
    let (table, expr) = table_and_expr(db, table, expr)?;
    let mut scan = Scan::new(&table, &mut db.pager)?;
    // The key is the value as a record, since bytes can be hashed and compared
    // and a `Value` can't.
    let mut counts: HashMap<Vec<u8>, (Value, i64), BuildHasherDefault<Fnv>> =
        HashMap::default();
    while let Some(row) = scan.next(&mut db.pager)? {
        let value = eval(&expr, &row)?;
        let key = encode(std::slice::from_ref(&value));
        counts.entry(key).or_insert((value, 0)).1 += 1;
    }
    let mut groups: Vec<(Value, i64)> = counts.into_values().collect();
    groups.sort_by(|a, b| order(&a.0, &b.0));
    Ok(groups)
}
// @end

// @listing sorted-count
/// Counts the rows in each group of an input that arrives sorted by the group's
/// key. Hands on rows of two values, the key and the count, and holds one group
/// at a time.
pub struct SortedCount {
    input: Box<dyn Operator>,
    key: Bound,
    /// The group being counted, if the input has not run out.
    current: Option<(Value, i64)>,
    started: bool,
}

impl SortedCount {
    pub fn new(input: Box<dyn Operator>, key: Bound) -> SortedCount {
        SortedCount { input, key, current: None, started: false }
    }
}
// @end

// @listing sorted-count-next
impl Operator for SortedCount {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        if !self.started {
            self.started = true;
            if let Some(row) = self.input.next(pager)? {
                self.current = Some((eval(&self.key, &row)?, 1));
            }
        }
        let Some((key, count)) = &mut self.current else {
            return Ok(None);
        };
        // Count rows until one arrives with a different key, or none arrives.
        let mut following = None;
        while let Some(row) = self.input.next(pager)? {
            let value = eval(&self.key, &row)?;
            if order(&value, key) != Ordering::Equal {
                following = Some((value, 1));
                break;
            }
            *count += 1;
        }
        let finished = vec![key.clone(), Value::Integer(*count)];
        self.current = following;
        Ok(Some(finished))
    }
}
// @end

// @listing median
/// The middle value of an expression over a table, leaving out NULLs. With an
/// even number of values, the lower of the middle two.
pub fn median(db: &mut Database, table: &str, expr: &str) -> Result<Value> {
    let (table, expr) = table_and_expr(db, table, expr)?;
    let scan = Scan::new(&table, &mut db.pager)?;
    let keys = vec![SortKey { expr: expr.clone(), descending: false }];
    let mut sort = Sort::new(Box::new(scan), keys, db.sort_budget, db.temp.clone());
    // NULLs sort first, so the values are what comes after them.
    let mut values = Vec::new();
    while let Some(row) = sort.next(&mut db.pager)? {
        let value = eval(&expr, &row)?;
        if value != Value::Null {
            values.push(value);
        }
    }
    if values.is_empty() {
        return Ok(Value::Null);
    }
    Ok(values.swap_remove((values.len() - 1) / 2))
}
// @end
