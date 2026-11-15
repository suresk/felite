//! Joins: rows of two tables, matched by a condition.

use crate::bind::{Bound, Scope, ScopeColumn, Type, bind, bind_condition};
use crate::catalog::{Table, fold, sql_error};
use crate::db::Database;
use crate::error::Result;
use crate::eval::{eval, truth};
use crate::exec::{Operator, Row, Scan};
use crate::pager::Pager;
use crate::record::{Value, encode};
use crate::sql::{BinaryOp, ColumnDef, Expr, ExprKind, Select};
use std::collections::HashMap;

// @listing nested-loop
/// Joins its outer input to a table by trying every pair of rows.
pub struct NestedLoop {
    outer: Box<dyn Operator>,
    inner: Table,
    /// The condition a pair must meet, over the outer row's values followed by
    /// the inner row's.
    condition: Bound,
    /// Whether to keep an outer row that no inner row matches.
    left: bool,
    current: Option<Current>,
    /// How many pairs of rows the condition has been tried on.
    pub pairs: u64,
}

/// The outer row a nested loop is working on.
struct Current {
    row: Row,
    /// The scan of the inner table for this row.
    scan: Scan,
    /// Whether any inner row has matched the row yet.
    matched: bool,
}

impl NestedLoop {
    pub fn new(
        outer: Box<dyn Operator>,
        inner: &Table,
        condition: Bound,
        left: bool,
    ) -> NestedLoop {
        let inner = inner.clone();
        NestedLoop { outer, inner, condition, left, current: None, pairs: 0 }
    }
}
// @end

// @listing nested-loop-next
impl Operator for NestedLoop {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        loop {
            let current = match &mut self.current {
                Some(current) => current,
                None => {
                    let Some(row) = self.outer.next(pager)? else {
                        return Ok(None);
                    };
                    // Start the inner table again from its first row.
                    let scan = Scan::new(&self.inner, pager)?;
                    self.current.insert(Current { row, scan, matched: false })
                }
            };
            while let Some(inner) = current.scan.next(pager)? {
                let mut pair = current.row.clone();
                pair.extend(inner);
                self.pairs += 1;
                if truth(&eval(&self.condition, &pair)?) == Some(true) {
                    current.matched = true;
                    return Ok(Some(pair));
                }
            }
            // The inner table is used up for this outer row.
            let Current { mut row, matched, .. } =
                self.current.take().expect("an outer row is in progress");
            if self.left && !matched {
                row.resize(row.len() + self.inner.columns.len(), Value::Null);
                return Ok(Some(row));
            }
        }
    }
}
// @end

// @listing hash-join
/// Joins two inputs on `outer_key = inner_key`, by putting the inner rows in a
/// hash table under their keys and looking each outer row's key up in it.
pub struct HashJoin {
    outer: Box<dyn Operator>,
    inner: Box<dyn Operator>,
    /// The key of an outer row, evaluated over that row alone.
    outer_key: Bound,
    /// The key of an inner row, evaluated over that row alone.
    inner_key: Bound,
    /// How many values an inner row has.
    inner_width: usize,
    left: bool,
    /// The inner rows under their keys, once the inner input has been read.
    hash_table: Option<HashMap<Vec<u8>, Vec<Row>>>,
    /// Joined rows that are ready to yield, with the first one last.
    ready: Vec<Row>,
    /// How many inner rows went into the hash table.
    pub built: u64,
    /// How many outer rows were looked up in it.
    pub probes: u64,
}
// @end

// @listing hash-key
/// A value as a hash table key: its bytes as a record. `None` for NULL, which
/// matches nothing and so is never put in the hash table or looked up.
fn hash_key(key: &Bound, row: &Row) -> Result<Option<Vec<u8>>> {
    Ok(match eval(key, row)? {
        Value::Null => None,
        value => Some(encode(&[value])),
    })
}
// @end

impl HashJoin {
    // @listing hash-join-new
    pub fn new(
        outer: Box<dyn Operator>,
        inner: Box<dyn Operator>,
        keys: (Bound, Bound),
        inner_width: usize,
        left: bool,
    ) -> HashJoin {
        HashJoin {
            outer,
            inner,
            outer_key: keys.0,
            inner_key: keys.1,
            inner_width,
            left,
            hash_table: None,
            ready: Vec::new(),
            built: 0,
            probes: 0,
        }
    }

    /// Reads the whole inner input into a hash table.
    fn build(&mut self, pager: &mut Pager) -> Result<HashMap<Vec<u8>, Vec<Row>>> {
        let mut hash_table: HashMap<Vec<u8>, Vec<Row>> = HashMap::new();
        while let Some(row) = self.inner.next(pager)? {
            if let Some(key) = hash_key(&self.inner_key, &row)? {
                hash_table.entry(key).or_default().push(row);
                self.built += 1;
            }
        }
        Ok(hash_table)
    }
    // @end
}

// @listing hash-join-next
impl Operator for HashJoin {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        let hash_table = match &self.hash_table {
            Some(hash_table) => hash_table,
            None => {
                let built = self.build(pager)?;
                self.hash_table.insert(built)
            }
        };
        loop {
            if let Some(row) = self.ready.pop() {
                return Ok(Some(row));
            }
            let Some(mut outer) = self.outer.next(pager)? else {
                return Ok(None);
            };
            self.probes += 1;
            let matches = match hash_key(&self.outer_key, &outer)? {
                Some(key) => hash_table.get(&key),
                None => None,
            };
            match matches {
                // Every inner row under the key makes a pair with the outer row.
                // They go in backwards, so that `pop` takes them in order.
                Some(inners) => {
                    for inner in inners.iter().rev() {
                        let mut pair = outer.clone();
                        pair.extend(inner.iter().cloned());
                        self.ready.push(pair);
                    }
                }
                None if self.left => {
                    outer.resize(outer.len() + self.inner_width, Value::Null);
                    return Ok(Some(outer));
                }
                None => {}
            }
        }
    }
}
// @end

// @listing table-scope
/// The columns of a table, under the name the query knows it by.
pub(crate) fn table_scope(table: &Table, alias: &Option<String>) -> Scope {
    let known_as = alias.as_ref().unwrap_or(&table.name);
    let column = |c: &ColumnDef| ScopeColumn {
        table: known_as.clone(),
        name: c.name.clone(),
        ty: c.kind.into(),
    };
    Scope { columns: table.columns.iter().map(column).collect() }
}
// @end

// @listing equi-keys
/// If a join's condition is `a = b` with one side worked out from the outer
/// row alone, the other from the inner row alone, and both of one type, returns
/// the two sides bound that way: the outer one first.
pub(crate) fn equi_keys(
    on: &Expr,
    outer: &Scope,
    inner: &Scope,
) -> Option<(Bound, Bound)> {
    let ExprKind::Binary { op: BinaryOp::Equal, left, right } = &on.kind else {
        return None;
    };
    for (a, b) in [(left, right), (right, left)] {
        if let (Ok(a), Ok(b)) = (bind(a, outer), bind(b, inner)) {
            let hashable = matches!(a.ty, Type::Integer | Type::Real | Type::Text);
            return (a.ty == b.ty && hashable).then_some((a, b));
        }
    }
    None
}
// @end

impl Database {
    // @listing from-scope
    /// Every column a SELECT's FROM clause provides: the columns of its first
    /// table, then those of each joined table.
    pub(crate) fn scope_of(&self, select: &Select) -> Result<Scope> {
        let mut scope = table_scope(self.table(&select.from)?, &select.alias);
        let mut names = vec![fold(&scope.columns[0].table)];
        for join in &select.joins {
            let inner = table_scope(self.table(&join.table)?, &join.alias);
            let name = fold(&inner.columns[0].table);
            if names.contains(&name) {
                let message = format!(
                    "{name} is in this query twice, and needs two different names"
                );
                return sql_error(message);
            }
            names.push(name);
            scope.columns.extend(inner.columns);
        }
        Ok(scope)
    }
    // @end

    // @listing join-plan
    /// Puts a join on top of `plan` for each JOIN of a SELECT. `plan` produces
    /// the rows of the first table.
    pub(crate) fn join_plan(
        &mut self,
        select: &Select,
        mut plan: Box<dyn Operator>,
    ) -> Result<Box<dyn Operator>> {
        let mut scope = table_scope(self.table(&select.from)?, &select.alias);
        for join in &select.joins {
            let table = self.table(&join.table)?.clone();
            let inner = table_scope(&table, &join.alias);
            let keys = equi_keys(&join.on, &scope, &inner);
            scope.columns.extend(inner.columns);
            let condition = bind_condition(&join.on, &scope, "ON")?;
            plan = match keys {
                Some(keys) => {
                    let scan = Box::new(Scan::new(&table, &mut self.pager)?);
                    let width = table.columns.len();
                    Box::new(HashJoin::new(plan, scan, keys, width, join.left))
                }
                None => Box::new(NestedLoop::new(plan, &table, condition, join.left)),
            };
        }
        Ok(plan)
    }
    // @end
}
