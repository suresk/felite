//! The planner: choosing how a SELECT will be run.

use crate::aggregate::{Aggregate, AggregateCall, bind_grouping, has_aggregate};
use crate::aggregate::{is_grouped, rewrite_query};
use crate::bind::{Bound, BoundKind, Scope, Type, bind, bind_condition};
use crate::btree::{INTERIOR, Tree, parse_interior};
use crate::catalog::Table;
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::exec::{Filter, Limit, Operator, Project, Row, Scan};
use crate::index::{Index, IndexScan};
use crate::join::{HashJoin, NestedLoop, equi_keys, table_scope};
use crate::keys::{after, encode_key};
use crate::pager::Pager;
use crate::query::{ResultSet, key_expr, order_keys, select_list};
use crate::record::Value;
use crate::sort::{Sort, SortKey};
use crate::sql::UnaryOp;
use crate::sql::{BinaryOp, ColumnType, Expr, ExprKind, OrderKey, Select, Span};

// @listing plan
/// A way to run a query, as a tree. Each node says what one operator will do,
/// and carries what is needed to build it.
#[derive(Debug, Clone)]
pub enum Plan {
    /// Every row of a table, in order of row id.
    Scan {
        table: Table,
        name: String,
        rows: u64,
    },
    /// The one row of a table with a given id, if there is one.
    RowId {
        table: Table,
        name: String,
        id: i64,
    },
    /// The rows that a range of an index leads to.
    IndexScan {
        table: Table,
        name: String,
        index: Index,
        from: Vec<u8>,
        until: Option<Vec<u8>>,
        /// The conditions that the range stands for, as SQL.
        conditions: String,
        rows: u64,
    },
    Filter {
        input: Box<Plan>,
        condition: Bound,
        text: String,
    },
    HashJoin {
        outer: Box<Plan>,
        inner: Box<Plan>,
        keys: (Bound, Bound),
        inner_width: usize,
        left: bool,
        text: String,
    },
    NestedLoop {
        outer: Box<Plan>,
        inner: Table,
        condition: Bound,
        left: bool,
        text: String,
    },
    /// Puts the columns of joined rows back in the order the query wrote its
    /// tables in.
    Arrange {
        input: Box<Plan>,
        positions: Vec<usize>,
    },
    Aggregate {
        input: Box<Plan>,
        groups: Vec<Bound>,
        calls: Vec<AggregateCall>,
        text: String,
    },
    Sort {
        input: Box<Plan>,
        keys: Vec<SortKey>,
        text: String,
    },
    Project {
        input: Box<Plan>,
        exprs: Vec<Bound>,
        text: String,
    },
    Limit {
        input: Box<Plan>,
        skip: usize,
        take: Option<usize>,
    },
}
// @end

// @listing column-bound
/// An expression that is the value at one position of a row.
fn column(position: usize) -> Bound {
    let span = Span { offset: 0, line: 1, column: 1 };
    Bound { kind: BoundKind::Column(position), ty: Type::Null, span }
}
// @end

// @listing row-lookup
/// Hands on the row of a table with one id, if the table has it.
pub struct RowLookup {
    table: Table,
    /// The id to look for. `None` once it has been looked for.
    id: Option<i64>,
}

impl Operator for RowLookup {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        match self.id.take() {
            Some(id) => self.table.fetch(pager, id),
            None => Ok(None),
        }
    }
}
// @end

impl Plan {
    // @listing build
    /// The operators that carry the plan out.
    pub fn build(self, db: &mut Database) -> Result<Box<dyn Operator>> {
        Ok(match self {
            Plan::Scan { table, .. } => Box::new(Scan::new(&table, &mut db.pager)?),
            Plan::RowId { table, id, .. } => {
                Box::new(RowLookup { table, id: Some(id) })
            }
            Plan::IndexScan { table, index, from, until, .. } => {
                Box::new(IndexScan::new(&table, &index, &mut db.pager, &from, until)?)
            }
            Plan::Filter { input, condition, .. } => {
                Box::new(Filter { input: input.build(db)?, condition })
            }
            Plan::HashJoin { outer, inner, keys, inner_width, left, .. } => {
                let (outer, inner) = (outer.build(db)?, inner.build(db)?);
                Box::new(HashJoin::new(outer, inner, keys, inner_width, left))
            }
            Plan::NestedLoop { outer, inner, condition, left, .. } => {
                Box::new(NestedLoop::new(outer.build(db)?, &inner, condition, left))
            }
            Plan::Arrange { input, positions } => {
                let exprs = positions.into_iter().map(column).collect();
                Box::new(Project { input: input.build(db)?, exprs })
            }
            Plan::Aggregate { input, groups, calls, .. } => {
                Box::new(Aggregate::new(input.build(db)?, groups, calls))
            }
            Plan::Sort { input, keys, .. } => {
                let (budget, temp) = (db.sort_budget, db.temp.clone());
                Box::new(Sort::new(input.build(db)?, keys, budget, temp))
            }
            Plan::Project { input, exprs, .. } => {
                Box::new(Project { input: input.build(db)?, exprs })
            }
            Plan::Limit { input, skip, take } => {
                Box::new(Limit { input: input.build(db)?, skip, take })
            }
        })
    }
    // @end

    // @listing explain
    /// The plan as text: a line for each node, with a node's inputs indented
    /// beneath it.
    pub fn explain(&self) -> Vec<String> {
        let mut lines = Vec::new();
        self.describe(0, &mut lines);
        lines
    }

    fn describe(&self, depth: usize, lines: &mut Vec<String>) {
        let join = |left: &bool, kind: &str| match left {
            true => format!("Left {kind}"),
            false => format!("{}{}", kind[..1].to_uppercase(), &kind[1..]),
        };
        let (line, inputs): (String, Vec<&Plan>) = match self {
            Plan::Scan { name, rows, .. } => {
                (format!("Scan {name}, about {rows} rows"), vec![])
            }
            Plan::RowId { name, id, .. } => {
                (format!("Look up row {id} of {name}"), vec![])
            }
            Plan::IndexScan { name, index, conditions, rows, .. } => {
                let index = &index.name;
                let line = format!(
                    "Search {name} with {index} for {conditions}, about {rows} rows"
                );
                (line, vec![])
            }
            Plan::Filter { input, text, .. } => {
                (format!("Filter {text}"), vec![input])
            }
            Plan::HashJoin { outer, inner, left, text, .. } => {
                (format!("{} on {text}", join(left, "hash join")), vec![outer, inner])
            }
            Plan::NestedLoop { outer, left, text, .. } => {
                (format!("{} with {text}", join(left, "nested loop")), vec![outer])
            }
            Plan::Arrange { input, .. } => {
                ("Put the columns in the order written".to_string(), vec![input])
            }
            Plan::Aggregate { input, text, .. } => {
                (format!("Aggregate {text}"), vec![input])
            }
            Plan::Sort { input, text, .. } => {
                (format!("Sort by {text}"), vec![input])
            }
            Plan::Project { input, text, .. } => {
                (format!("Project {text}"), vec![input])
            }
            Plan::Limit { input, skip, take } => {
                let line = match take {
                    Some(take) if *skip == 0 => format!("Stop after {take} rows"),
                    Some(take) => format!("Skip {skip} rows, then stop after {take}"),
                    None => format!("Skip {skip} rows"),
                };
                (line, vec![input])
            }
        };
        lines.push(format!("{}{line}", "  ".repeat(depth)));
        for input in inputs {
            input.describe(depth + 1, lines);
        }
    }
    // @end
}

// @listing conjuncts
/// The conditions that an expression is an AND of. An expression with no AND at
/// its top is one condition.
pub fn conjuncts(expr: &Expr, found: &mut Vec<Expr>) {
    match &expr.kind {
        ExprKind::Binary { op: BinaryOp::And, left, right } => {
            conjuncts(left, found);
            conjuncts(right, found);
        }
        _ => found.push(expr.clone()),
    }
}

/// Takes out of `pool` the conditions that can be worked out from the columns
/// of `scope` alone.
fn take_bindable(pool: &mut Vec<Expr>, scope: &Scope) -> Vec<Expr> {
    let (taken, left) = pool.drain(..).partition(|c| bind(c, scope).is_ok());
    *pool = left;
    taken
}
// @end

impl Tree {
    // @listing estimate
    /// A guess at how many entries the tree holds, from the pages on the way to
    /// its first leaf: the product of how many cells each of them has.
    pub fn estimate(&self, pager: &mut Pager) -> Result<u64> {
        let (mut n, mut rows) = (self.root(), 1);
        for _ in 0..=pager.page_count()? {
            let page = pager.page(n)?;
            rows *= page.cell_count() as u64;
            if page.kind() != INTERIOR || page.cell_count() == 0 {
                return Ok(rows);
            }
            n = parse_interior(page.cell(0)?)?.0;
        }
        Err(Error::Corrupt("a tree deeper than the file has pages".to_string()))
    }
    // @end
}

// @listing test
/// A condition of the form `column op constant`, as the planner sees it: which
/// column of the table, which comparison, and the constant as a value of the
/// column's type.
struct Test {
    position: usize,
    op: BinaryOp,
    value: Value,
}

/// The value of an expression that is a constant written out, with or without
/// a minus sign.
fn constant(expr: &Expr) -> Option<Value> {
    match &expr.kind {
        ExprKind::Literal(value) => Some(value.clone()),
        ExprKind::Unary { op: UnaryOp::Negate, operand } => match &operand.kind {
            ExprKind::Literal(Value::Integer(i)) => {
                i.checked_neg().map(Value::Integer)
            }
            ExprKind::Literal(Value::Real(x)) => Some(Value::Real(-x)),
            _ => None,
        },
        _ => None,
    }
}

// @end

// @listing as-test
/// Reads a condition as a `Test` on a column of `table`, if it is one.
fn as_test(condition: &Expr, table: &Table, scope: &Scope) -> Option<Test> {
    use BinaryOp::*;
    let ExprKind::Binary { op, left, right } = &condition.kind else {
        return None;
    };
    // The column may be on either side. If it is on the right, the comparison
    // is turned around.
    let (side, value, op) = match (constant(left), constant(right)) {
        (None, Some(value)) => (left, value, *op),
        (Some(value), None) => {
            let turned = match op {
                Less => Greater,
                LessEqual => GreaterEqual,
                Greater => Less,
                GreaterEqual => LessEqual,
                other => *other,
            };
            (right, value, turned)
        }
        _ => return None,
    };
    let BoundKind::Column(position) = bind(side, scope).ok()?.kind else {
        return None;
    };
    // The constant has to become a value of exactly the column's type, because
    // an index compares bytes, and 2 and 2.0 are different bytes.
    let value = match (table.columns[position].kind, value) {
        (ColumnType::Integer, Value::Integer(i)) => Value::Integer(i),
        (ColumnType::Real, Value::Integer(i)) => Value::Real(i as f64),
        (ColumnType::Real, Value::Real(x)) => Value::Real(x),
        (ColumnType::Text, Value::Text(text)) => Value::Text(text),
        _ => return None,
    };
    matches!(op, Equal | Less | LessEqual | Greater | GreaterEqual).then_some(Test {
        position,
        op,
        value,
    })
}
// @end

// @listing index-range
/// A way to use an index for some of a table's conditions.
struct Range {
    index: Index,
    from: Vec<u8>,
    until: Option<Vec<u8>>,
    /// Which of the conditions the range takes care of, by their places in the
    /// list.
    used: Vec<usize>,
    /// How many of the index's leading columns are fixed to one value.
    fixed: usize,
    /// The part of the table that the range is guessed to select.
    fraction: f64,
}

/// The guesses the planner makes when it knows nothing about the data: the
/// part of a table that `column = constant` selects, and the part that one
/// bound on a column selects.
const EQUAL: f64 = 0.05;
const BOUND: f64 = 0.3;
/// An index is used if it is guessed to select no more than this much.
const WORTH_IT: f64 = 0.1;

// @end

// @listing range-for
/// The best use of `index` for a list of tests, if it has any use.
fn range_for(index: &Index, tests: &[Option<Test>], rows: u64) -> Option<Range> {
    let test_on = |position: usize, wanted: &[BinaryOp]| {
        tests.iter().position(|test| {
            test.as_ref()
                .is_some_and(|t| t.position == position && wanted.contains(&t.op))
        })
    };
    // As many leading columns as have a test for equality.
    let (mut used, mut values) = (Vec::new(), Vec::new());
    for position in &index.positions {
        let Some(found) = test_on(*position, &[BinaryOp::Equal]) else { break };
        used.push(found);
        values.extend(tests[found].as_ref().map(|test| test.value.clone()));
    }
    let fixed = used.len();
    let prefix = encode_key(&values);
    let mut fraction = EQUAL.powi(fixed as i32);
    if index.unique && fixed == index.positions.len() {
        fraction = 1.0 / rows.max(1) as f64;
    }
    let (mut from, mut until) = (prefix.clone(), after(&prefix));
    // On the column after those, a bound below, a bound above, or both.
    if let Some(next) = index.positions.get(fixed) {
        let bound = |found: usize| {
            let test = tests[found].as_ref()?;
            let mut key = prefix.clone();
            key.extend(encode_key(std::slice::from_ref(&test.value)));
            Some((key, test.op))
        };
        let below = test_on(*next, &[BinaryOp::Greater, BinaryOp::GreaterEqual]);
        let above = test_on(*next, &[BinaryOp::Less, BinaryOp::LessEqual]);
        if let Some((key, op)) = below.and_then(bound) {
            from = if op == BinaryOp::Greater { after(&key) } else { key };
            fraction *= BOUND;
        } else if above.is_some() {
            // A bound above only. NULLs sort first and are no part of it.
            let mut nulls = prefix.clone();
            nulls.push(0);
            from = after(&nulls);
        }
        if let Some((key, op)) = above.and_then(bound) {
            until = if op == BinaryOp::LessEqual { after(&key) } else { key };
            fraction *= BOUND;
        }
        used.extend(below);
        used.extend(above);
    }
    (!used.is_empty()).then_some(Range {
        index: index.clone(),
        from,
        until: Some(until),
        used,
        fixed,
        fraction,
    })
}
// @end

// @listing source
/// One table of a FROM clause, with the plan for reading it.
struct Source {
    table: Table,
    scope: Scope,
    /// Where this table's columns begin in a row with the tables as written.
    offset: usize,
    /// The conditions that mention this table only.
    conditions: Vec<Expr>,
    plan: Plan,
    /// A guess at how many rows the plan produces.
    rows: u64,
    /// The columns, by their position in the table, that the plan's rows are
    /// in order of.
    sorted_by: Vec<usize>,
    /// Whether the plan produces one row at most.
    single: bool,
}
// @end

// @listing ordering
/// What the planner knows about the order of a plan's rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ordering {
    /// The positions of the columns that the rows are in ascending order of.
    pub sorted_by: Vec<usize>,
    /// Whether there is one row at most, which is in every order at once.
    pub single: bool,
}

/// A plan for everything up to the grouping, if there is one, with what the
/// rest of the query is built from.
struct Shaped {
    plan: Plan,
    /// The SELECT list.
    exprs: Vec<Bound>,
    columns: Vec<String>,
    /// The ORDER BY.
    keys: Vec<SortKey>,
    ordering: Ordering,
}
// @end

impl Ordering {
    // @listing provides
    /// Whether rows in this order are already in the order that `keys` ask for.
    fn provides(&self, keys: &[SortKey]) -> bool {
        let wanted = keys.iter().map(|key| match key.expr.kind {
            BoundKind::Column(position) if !key.descending => Some(position),
            _ => None,
        });
        let wanted: Option<Vec<usize>> = wanted.collect();
        self.single
            || wanted.is_some_and(|wanted| self.sorted_by.starts_with(&wanted))
    }
    // @end
}

// @listing choose
/// How one table will be read, before the conditions that are left over are
/// put on top as filters.
struct Choice {
    plan: Plan,
    /// The places, in the list of conditions, of those the plan takes care of.
    used: Vec<usize>,
    /// The part of the table the plan is guessed to produce.
    fraction: f64,
    sorted_by: Vec<usize>,
}

/// A number of rows, from a table's size and a fraction of it. Never zero.
fn guess(table_rows: u64, fraction: f64) -> u64 {
    (table_rows as f64 * fraction).ceil().max(1.0) as u64
}

/// The row id, if a condition gives it. Failing that, the index that is guessed
/// to select the least, if that is little enough. Failing that, a scan.
fn choose(
    table: &Table,
    name: String,
    tests: &[Option<Test>],
    conditions: &[Expr],
    rows: u64,
) -> Choice {
    let table = table.clone();
    let key = table.key_column;
    let by_id = tests.iter().enumerate().find_map(|(at, test)| match test {
        Some(Test { position, op: BinaryOp::Equal, value: Value::Integer(id) })
            if key == Some(*position) =>
        {
            Some((at, *id))
        }
        _ => None,
    });
    if let Some((at, id)) = by_id {
        let plan = Plan::RowId { table, name, id };
        return Choice { plan, used: vec![at], fraction: 0.0, sorted_by: Vec::new() };
    }
    let ranges = table.indexes.iter().filter_map(|i| range_for(i, tests, rows));
    let best = ranges.min_by(|a, b| a.fraction.total_cmp(&b.fraction));
    match best.filter(|range| range.fraction <= WORTH_IT) {
        Some(range) => index_choice(table, name, range, conditions, rows),
        None => {
            let sorted_by = key.into_iter().collect();
            let plan = Plan::Scan { table, name, rows };
            Choice { plan, used: Vec::new(), fraction: 1.0, sorted_by }
        }
    }
}
// @end

// @listing index-choice
/// The choice to read a table through a range of an index.
fn index_choice(
    table: Table,
    name: String,
    range: Range,
    conditions: &[Expr],
    rows: u64,
) -> Choice {
    let text = |at: &usize| conditions[*at].to_string();
    let shown: Vec<String> = range.used.iter().map(text).collect();
    // Past the fixed columns, the entries are in order of the columns that
    // follow, and then of row id.
    let mut sorted_by = range.index.positions[range.fixed..].to_vec();
    sorted_by.extend(table.key_column);
    let plan = Plan::IndexScan {
        table,
        name,
        index: range.index,
        from: range.from,
        until: range.until,
        conditions: shown.join(" AND "),
        rows: guess(rows, range.fraction),
    };
    Choice { plan, used: range.used, fraction: range.fraction, sorted_by }
}
// @end

// @listing join-helpers
/// A condition that is true of every row.
fn always() -> Bound {
    let kind = BoundKind::Literal(Value::Integer(1));
    Bound { kind, ty: Type::Condition, ..column(0) }
}

/// Joins one more table to a plan: with a hash join if an equality links them,
/// which is given as its text and its two keys, and with a nested loop if not.
fn join_to(plan: Plan, next: Source, edge: Option<(String, (Bound, Bound))>) -> Plan {
    let outer = Box::new(plan);
    match edge {
        Some((text, keys)) => {
            let inner_width = next.table.columns.len();
            let inner = Box::new(next.plan);
            Plan::HashJoin { outer, inner, keys, inner_width, left: false, text }
        }
        None => {
            let text = next.scope.columns[0].table.clone();
            let (inner, condition) = (next.table, always());
            Plan::NestedLoop { outer, inner, condition, left: false, text }
        }
    }
}

/// If the tables were not joined in the order they were written in, puts the
/// columns back as the rest of the query expects them. `placed` gives, for
/// each table in the order joined, where its columns begin in the written
/// order and how many there are.
fn arrange(plan: Plan, placed: &[(usize, usize)]) -> Plan {
    let width = placed.iter().map(|(_, width)| width).sum();
    let mut positions = vec![0; width];
    let mut built = 0;
    for (offset, width) in placed {
        for column in 0..*width {
            positions[offset + column] = built;
            built += 1;
        }
    }
    if positions.iter().enumerate().all(|(written, built)| written == *built) {
        return plan;
    }
    Plan::Arrange { input: Box::new(plan), positions }
}
// @end

impl Database {
    // @listing access
    /// Chooses how to read one table, given the conditions that mention it alone.
    fn access(
        &mut self,
        table: &Table,
        alias: &Option<String>,
        offset: usize,
        conditions: Vec<Expr>,
    ) -> Result<Source> {
        let scope = table_scope(table, alias);
        let name = scope.columns[0].table.clone();
        let table_rows = table.tree.estimate(&mut self.pager)?;
        let tests: Vec<Option<Test>> =
            conditions.iter().map(|c| as_test(c, table, &scope)).collect();
        let Choice { mut plan, used, mut fraction, sorted_by } =
            choose(table, name, &tests, &conditions, table_rows);
        let single = matches!(plan, Plan::RowId { .. });
        // The conditions that the choice did not take care of are filters.
        for (at, condition) in conditions.iter().enumerate() {
            if !used.contains(&at) {
                let bound = bind_condition(condition, &scope, "WHERE")?;
                let text = condition.to_string();
                plan = Plan::Filter { input: Box::new(plan), condition: bound, text };
                fraction *= BOUND;
            }
        }
        let rows = guess(table_rows, fraction);
        let table = table.clone();
        Ok(Source { table, scope, offset, conditions, plan, rows, sorted_by, single })
    }
    // @end

    // @listing plan-written
    /// The plan for a FROM clause with a LEFT JOIN in it: the joins in the order
    /// they are written, and the WHERE clause above them all.
    fn plan_written(&mut self, select: &Select, scope: &Scope) -> Result<Plan> {
        let first = self.table(&select.from)?.clone();
        let source = self.access(&first, &select.alias, 0, Vec::new())?;
        let (mut plan, mut so_far) = (source.plan, source.scope);
        for join in &select.joins {
            let table = self.table(&join.table)?.clone();
            let inner = self.access(&table, &join.alias, 0, Vec::new())?;
            let keys = equi_keys(&join.on, &so_far, &inner.scope);
            so_far.columns.extend(inner.scope.columns);
            let condition = bind_condition(&join.on, &so_far, "ON")?;
            let (outer, left, text) =
                (Box::new(plan), join.left, join.on.to_string());
            plan = match keys {
                Some(keys) => {
                    let (inner, inner_width) =
                        (Box::new(inner.plan), table.columns.len());
                    Plan::HashJoin { outer, inner, keys, inner_width, left, text }
                }
                None => {
                    let text = format!(
                        "{} on {text}",
                        so_far.columns[so_far.columns.len() - 1].table
                    );
                    Plan::NestedLoop { outer, inner: table, condition, left, text }
                }
            };
        }
        if let Some(filter) = &select.filter {
            let condition = bind_condition(filter, scope, "WHERE")?;
            plan = Plan::Filter {
                input: Box::new(plan),
                condition,
                text: filter.to_string(),
            };
        }
        Ok(plan)
    }
    // @end

    // @listing plan-from
    /// Plans the FROM and WHERE clauses of a SELECT. Returns the plan, whose
    /// rows have the columns of the tables in the order written, with the scope
    /// that describes them and what is known of their order.
    pub(crate) fn plan_from(
        &mut self,
        select: &Select,
    ) -> Result<(Plan, Scope, Ordering)> {
        // Check every condition where it stands, so that a mistake is reported
        // as it always was.
        let scope = self.scope_of(select)?;
        let mut pool = Vec::new();
        let mut so_far = table_scope(self.table(&select.from)?, &select.alias);
        let mut tables =
            vec![(self.table(&select.from)?.clone(), select.alias.clone(), 0)];
        for join in &select.joins {
            let table = self.table(&join.table)?.clone();
            tables.push((table.clone(), join.alias.clone(), so_far.columns.len()));
            so_far.columns.extend(table_scope(&table, &join.alias).columns);
            bind_condition(&join.on, &so_far, "ON")?;
            conjuncts(&join.on, &mut pool);
        }
        if let Some(filter) = &select.filter {
            bind_condition(filter, &scope, "WHERE")?;
            conjuncts(filter, &mut pool);
        }
        if select.joins.iter().any(|join| join.left) {
            return Ok((
                self.plan_written(select, &scope)?,
                scope,
                Ordering::default(),
            ));
        }

        // Each table takes the conditions that are about it alone, and gets a
        // plan of its own.
        let mut sources = Vec::new();
        for (table, alias, offset) in tables {
            let mine = take_bindable(&mut pool, &table_scope(&table, &alias));
            sources.push(self.access(&table, &alias, offset, mine)?);
        }
        self.plan_joins(sources, pool, scope)
    }
    // @end

    // @listing plan-joins
    /// Joins the tables of a FROM clause. The table guessed to produce the most
    /// rows is read as a stream, and the others are joined to it one by one,
    /// the smallest first among those that an equality connects.
    fn plan_joins(
        &mut self,
        mut sources: Vec<Source>,
        mut pool: Vec<Expr>,
        scope: Scope,
    ) -> Result<(Plan, Scope, Ordering)> {
        let size = |i: &usize| (sources[*i].rows, usize::MAX - i);
        let largest = (0..sources.len()).max_by_key(size).unwrap_or(0);
        let first = sources.remove(largest);
        let mut placed = vec![(first.offset, first.scope.columns.len())];
        let ordering = Ordering {
            sorted_by: first.sorted_by.iter().map(|p| first.offset + p).collect(),
            single: first.single && sources.is_empty(),
        };
        let (mut plan, mut so_far) = (first.plan, first.scope);
        sources.sort_by_key(|source| source.rows);
        while !sources.is_empty() {
            // The first table, in order of size, that an equality joins to
            // what there is so far. If there is none, the smallest.
            let link = |source: &Source| {
                let keys = |c: &Expr| equi_keys(c, &so_far, &source.scope);
                pool.iter().enumerate().find_map(|(at, c)| Some((at, keys(c)?)))
            };
            let linked = sources.iter().position(|source| link(source).is_some());
            let mut next = sources.remove(linked.unwrap_or(0));
            let edge =
                link(&next).map(|(at, keys)| (pool.remove(at).to_string(), keys));
            if edge.is_none() {
                // A nested loop reads its table directly, so the table's own
                // conditions go back in the pool, to be filters.
                pool.append(&mut next.conditions);
            }
            placed.push((next.offset, next.scope.columns.len()));
            so_far.columns.extend(next.scope.columns.clone());
            plan = join_to(plan, next, edge);
            // Every condition that the columns so far are enough for.
            for condition in take_bindable(&mut pool, &so_far) {
                let text = condition.to_string();
                let condition = bind_condition(&condition, &so_far, "ON")?;
                plan = Plan::Filter { input: Box::new(plan), condition, text };
            }
        }
        Ok((arrange(plan, &placed), scope, ordering))
    }
    // @end

    // @listing plan-grouped
    /// Puts an aggregation, and the filter for a HAVING, on top of a plan.
    /// Returns the plan with everything the rest of the query needs, bound
    /// against the aggregation's output.
    fn plan_grouped(
        &mut self,
        select: &Select,
        plan: Plan,
        scope: &Scope,
    ) -> Result<Shaped> {
        let query = rewrite_query(select)?;
        let (groups, calls, output) =
            bind_grouping(&select.group_by, &query.calls, scope)?;
        let mut exprs = Vec::new();
        for item in &query.items {
            exprs.push(bind(item, &output)?);
        }
        let mut keys = Vec::new();
        for (expr, descending) in &query.order {
            keys.push(SortKey {
                expr: bind(expr, &output)?,
                descending: *descending,
            });
        }
        let list = |exprs: &[Expr]| {
            exprs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ")
        };
        let text = match list(&select.group_by) {
            by if by.is_empty() => list(&query.calls),
            by => format!("by {by}: {}", list(&query.calls)),
        };
        // The groups come out in the order of their keys.
        let ordering = Ordering {
            sorted_by: (0..groups.len()).collect(),
            single: groups.is_empty(),
        };
        let mut plan = Plan::Aggregate { input: Box::new(plan), groups, calls, text };
        if let (Some(rewritten), Some(having)) = (&query.having, &select.having) {
            let condition = bind_condition(rewritten, &output, "HAVING")?;
            let text = having.to_string();
            plan = Plan::Filter { input: Box::new(plan), condition, text };
        }
        Ok(Shaped { plan, exprs, columns: query.columns, keys, ordering })
    }
    // @end

    // @listing plan-select
    /// Plans a whole SELECT. Returns the plan and the names of the result's
    /// columns.
    pub fn plan_select(&mut self, select: &Select) -> Result<(Plan, Vec<String>)> {
        // Check the grouping as it was written, before anything is moved.
        if is_grouped(select) {
            rewrite_query(select)?;
        }
        let select = &move_having(select);
        let (plan, scope, ordering) = self.plan_from(select)?;
        let Shaped { mut plan, exprs, columns, keys, ordering } =
            if is_grouped(select) {
                self.plan_grouped(select, plan, &scope)?
            } else {
                let (exprs, columns) = select_list(&select.items, &scope)?;
                let keys = order_keys(select, &scope)?;
                Shaped { plan, exprs, columns, keys, ordering }
            };
        // A sort is needed only if the rows are not in the wanted order already.
        if !keys.is_empty() && !ordering.provides(&keys) {
            let key = |key: &OrderKey| {
                let direction = if key.descending { " DESC" } else { "" };
                format!("{}{direction}", key_expr(select, key))
            };
            let text = select.order_by.iter().map(key).collect::<Vec<_>>().join(", ");
            plan = Plan::Sort { input: Box::new(plan), keys, text };
        }
        let text = columns.join(", ");
        plan = Plan::Project { input: Box::new(plan), exprs, text };
        if select.limit.is_some() || select.offset.is_some() {
            let skip = select.offset.unwrap_or(0);
            plan = Plan::Limit { input: Box::new(plan), skip, take: select.limit };
        }
        Ok((plan, columns))
    }
    // @end

    // @listing select-planned
    /// Runs a SELECT by planning it first.
    pub(crate) fn select_planned(&mut self, select: &Select) -> Result<Output> {
        let (plan, columns) = self.plan_select(select)?;
        let mut operator = plan.build(self)?;
        let mut rows = Vec::new();
        while let Some(row) = operator.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(Output::Rows(ResultSet { columns, rows }))
    }

    /// The plan for a SELECT, as rows of one column for the prompt to print.
    pub(crate) fn explain(&mut self, select: &Select) -> Result<Output> {
        let (plan, _) = self.plan_select(select)?;
        let rows =
            plan.explain().into_iter().map(|line| vec![Value::Text(line)]).collect();
        Ok(Output::Rows(ResultSet { columns: vec!["plan".to_string()], rows }))
    }

    /// Turns the planner off or on. With it off, every SELECT runs as Chapters
    /// 14 to 22 built it.
    pub fn set_planner(&mut self, on: bool) {
        self.planner = on;
    }
    // @end
}

// @listing move-having
/// A copy of a SELECT in which each part of the HAVING clause that uses no
/// aggregate has moved to the WHERE clause, where it is applied to rows before
/// they are grouped.
fn move_having(select: &Select) -> Select {
    let mut select = select.clone();
    if select.group_by.is_empty() {
        return select;
    }
    let Some(having) = select.having.take() else {
        return select;
    };
    let mut parts = Vec::new();
    conjuncts(&having, &mut parts);
    let and = |a: Expr, b: Expr| {
        let span = a.span;
        let (left, right) = (Box::new(a), Box::new(b));
        Expr { kind: ExprKind::Binary { op: BinaryOp::And, left, right }, span }
    };
    for part in parts {
        let to = if has_aggregate(&part) {
            &mut select.having
        } else {
            &mut select.filter
        };
        *to = Some(match to.take() {
            Some(earlier) => and(earlier, part),
            None => part,
        });
    }
    select
}
// @end
