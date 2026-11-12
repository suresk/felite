//! Chapter 20 challenges: the first few rows of a sort without the sort, NULLs
//! at the other end, merging in more than one pass, longer runs, and sorting by
//! a column's position.

use crate::catalog::sql_error;
use crate::db::{Database, Output};
use crate::error::Result;
use crate::eval::{eval, order};
use crate::exec::{Operator, Row};
use crate::pager::Pager;
use crate::record::Value;
use crate::sort::SortKey;
use crate::sql::{ExprKind, Select, SelectItem, Statement, parse};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

// @listing ranked
/// A row with what it takes to rank it: its keys, and its place in the input
/// to settle ties.
struct Ranked {
    keys: Vec<(Value, bool)>,
    place: usize,
    row: Row,
}

impl Ord for Ranked {
    fn cmp(&self, other: &Ranked) -> Ordering {
        for ((a, descending), (b, _)) in self.keys.iter().zip(&other.keys) {
            let by = if *descending { order(b, a) } else { order(a, b) };
            if by != Ordering::Equal {
                return by;
            }
        }
        self.place.cmp(&other.place)
    }
}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Ranked) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Ranked) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ranked {}
// @end

// @listing top-n
/// The first `n` rows of its input in the order of their keys, found without
/// sorting the rest.
pub struct TopN {
    input: Box<dyn Operator>,
    keys: Vec<SortKey>,
    n: usize,
    /// The best rows, once the input has been read, with the first one last.
    found: Option<Vec<Ranked>>,
    /// The most rows that were held at once.
    pub held: usize,
}

impl TopN {
    pub fn new(input: Box<dyn Operator>, keys: Vec<SortKey>, n: usize) -> TopN {
        TopN { input, keys, n, found: None, held: 0 }
    }
}
// @end

impl TopN {
    // @listing top-n-find
    fn find(&mut self, pager: &mut Pager) -> Result<Vec<Ranked>> {
        // A max-heap: the row on top is the worst of the best so far.
        let mut best = BinaryHeap::new();
        let mut place = 0;
        while let Some(row) = self.input.next(pager)? {
            let key = |k: &SortKey| Ok((eval(&k.expr, &row)?, k.descending));
            let keys = self.keys.iter().map(key).collect::<Result<_>>()?;
            best.push(Ranked { keys, place, row });
            place += 1;
            self.held = self.held.max(best.len());
            if best.len() > self.n {
                best.pop();
            }
        }
        let mut found = best.into_sorted_vec();
        found.reverse();
        Ok(found)
    }
    // @end
}

// @listing top-n-next
impl Operator for TopN {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        if self.found.is_none() {
            self.found = Some(self.find(pager)?);
        }
        let found = self.found.as_mut().and_then(|found| found.pop());
        Ok(found.map(|ranked| ranked.row))
    }
}
// @end

// @listing nulls-last
/// How two values sort for one key of an ORDER BY, with a say in where the
/// NULLs go.
pub fn order_with(
    left: &Value,
    right: &Value,
    descending: bool,
    nulls_last: bool,
) -> Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        // Where the NULLs go does not depend on the direction.
        (Value::Null, _) if nulls_last => Ordering::Greater,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) if nulls_last => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        _ if descending => order(right, left),
        _ => order(left, right),
    }
}
// @end

// @listing merge-passes
/// Merges sorted runs, at most `fan_in` of them at a time, until one is left.
/// Returns it, and the number of passes over the data that it took.
pub fn merge_passes(mut runs: Vec<Vec<i64>>, fan_in: usize) -> (Vec<i64>, usize) {
    let mut passes = 0;
    while runs.len() > 1 {
        runs = runs.chunks(fan_in.max(2)).map(merge).collect();
        passes += 1;
    }
    (runs.pop().unwrap_or_default(), passes)
}

/// One merge, with a heap of each run's next number.
fn merge(runs: &[Vec<i64>]) -> Vec<i64> {
    let mut at = vec![0; runs.len()];
    let mut heap = BinaryHeap::new();
    for (run, numbers) in runs.iter().enumerate() {
        if let Some(first) = numbers.first() {
            heap.push(Reverse((*first, run)));
        }
    }
    let mut merged = Vec::new();
    while let Some(Reverse((number, run))) = heap.pop() {
        merged.push(number);
        at[run] += 1;
        if let Some(next) = runs[run].get(at[run]) {
            heap.push(Reverse((*next, run)));
        }
    }
    merged
}
// @end

// @listing replacement
/// Cuts `input` into sorted runs while holding at most `memory` numbers, by
/// replacement selection.
pub fn replacement_runs(input: &[i64], memory: usize) -> Vec<Vec<i64>> {
    let mut input = input.iter().copied();
    // Each number is in the heap with the run it will be written to.
    let mut heap = BinaryHeap::new();
    for number in input.by_ref().take(memory.max(1)) {
        heap.push(Reverse((0, number)));
    }
    let mut runs: Vec<Vec<i64>> = Vec::new();
    while let Some(Reverse((run, number))) = heap.pop() {
        if run == runs.len() {
            runs.push(Vec::new());
        }
        runs[run].push(number);
        // The number that takes its place can join this run if it is not
        // smaller than the one just written. Otherwise it waits for the next.
        if let Some(arrival) = input.next() {
            let goes_in = if arrival >= number { run } else { run + 1 };
            heap.push(Reverse((goes_in, arrival)));
        }
    }
    runs
}
// @end

// @listing by-position
/// Runs a SELECT in which an ORDER BY key may be a number: the position of a
/// column in the result, counting from 1.
pub fn query_by_position(db: &mut Database, sql: &str) -> Result<Output> {
    let Statement::Select(select) = parse(sql)? else {
        return sql_error("that statement is not a SELECT".to_string());
    };
    db.run(Statement::Select(by_position(select)?))
}

fn by_position(mut select: Select) -> Result<Select> {
    for key in &mut select.order_by {
        let ExprKind::Literal(Value::Integer(position)) = key.expr.kind else {
            continue;
        };
        let item = match usize::try_from(position) {
            Ok(position) if position >= 1 => select.items.get(position - 1),
            _ => None,
        };
        match item {
            Some(SelectItem::Expr { expr, .. }) => key.expr = expr.clone(),
            Some(SelectItem::Star) => {
                return sql_error(format!("column {position} is a *"));
            }
            None => {
                let message = format!("the result has no column {position}");
                return sql_error(message);
            }
        }
    }
    Ok(select)
}
// @end
