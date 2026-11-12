//! Aggregates and GROUP BY: one row out for each group of rows in.

use crate::bind::{Bound, Scope, ScopeColumn, Type, bind, bind_condition};
use crate::catalog::{fold, sql_error};
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::eval::{eval, order};
use crate::exec::{Filter, Limit, Operator, Project, Row, Scan};
use crate::pager::Pager;
use crate::query::{ResultSet, key_expr};
use crate::record::Value;
use crate::sort::{Key, Sort, SortKey};
use crate::sql::{Expr, ExprKind, Function, Select, SelectItem, Span};
use std::cmp::Ordering;
use std::collections::BTreeMap;

// @listing call
/// One aggregate of a query: which function, what it is applied to, and where
/// in the statement it is written. Only `COUNT(*)` has no argument.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateCall {
    pub function: Function,
    pub arg: Option<Bound>,
    pub span: Span,
}
// @end

// @listing running
/// Where one aggregate has got to for one group, part of the way through the
/// group's rows.
#[derive(Debug, Clone)]
struct Running {
    /// How many values have been taken in.
    count: i64,
    /// The total, the smallest or the largest so far. NULL until there is one.
    value: Value,
}
// @end

// @listing running-add
impl Running {
    /// Takes in one row.
    fn add(&mut self, call: &AggregateCall, row: &Row) -> Result<()> {
        // COUNT(*) counts rows, whatever is in them.
        let Some(arg) = &call.arg else {
            self.count += 1;
            return Ok(());
        };
        // Every other aggregate passes over NULLs as if their rows weren't there.
        let value = eval(arg, row)?;
        if value == Value::Null {
            return Ok(());
        }
        self.count += 1;
        self.value = match (call.function, &self.value) {
            (Function::Count, _) => Value::Null,
            (Function::Avg, Value::Null) => Value::Real(real(&value)),
            (_, Value::Null) => value,
            (Function::Sum | Function::Avg, total) => plus(total, &value, call.span)?,
            (Function::Min, least) if order(least, &value) == Ordering::Less => {
                return Ok(());
            }
            (Function::Max, most) if order(most, &value) == Ordering::Greater => {
                return Ok(());
            }
            _ => value,
        };
        Ok(())
    }

    /// The aggregate's value, once every row of the group has been taken in.
    fn finish(self, function: Function) -> Value {
        match (function, self.value) {
            (Function::Count, _) => Value::Integer(self.count),
            (Function::Avg, Value::Real(total)) => {
                Value::Real(total / self.count as f64)
            }
            (_, value) => value,
        }
    }
}
// @end

// @listing plus
fn real(value: &Value) -> f64 {
    match value {
        Value::Integer(i) => *i as f64,
        Value::Real(x) => *x,
        _ => f64::NAN,
    }
}

/// Adds a value to a running total. Two integers make an integer, and anything
/// else makes a real.
fn plus(total: &Value, value: &Value, span: Span) -> Result<Value> {
    let fail = |what: &str| Err(Error::Invalid(what.to_string(), span));
    if let (Value::Integer(a), Value::Integer(b)) = (total, value) {
        return match a.checked_add(*b) {
            Some(sum) => Ok(Value::Integer(sum)),
            None => fail("the sum is too large for an integer"),
        };
    }
    let sum = real(total) + real(value);
    if !sum.is_finite() {
        return fail("the sum is too large for a real");
    }
    Ok(Value::Real(sum))
}
// @end

// @listing aggregate
/// Reads all of its input, sorts the rows into groups, and hands on one row for
/// each group: the values the group was formed by, then the value of each
/// aggregate over the group's rows.
pub struct Aggregate {
    input: Box<dyn Operator>,
    groups: Vec<Bound>,
    calls: Vec<AggregateCall>,
    /// The rows to hand on, once the input has been read, with the first one last.
    output: Option<Vec<Row>>,
}

impl Aggregate {
    pub fn new(
        input: Box<dyn Operator>,
        groups: Vec<Bound>,
        calls: Vec<AggregateCall>,
    ) -> Aggregate {
        Aggregate { input, groups, calls, output: None }
    }
}

impl Operator for Aggregate {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        if self.output.is_none() {
            self.output = Some(self.read_input(pager)?);
        }
        Ok(self.output.as_mut().and_then(|rows| rows.pop()))
    }
}
// @end

impl Aggregate {
    // @listing read-groups
    fn read_input(&mut self, pager: &mut Pager) -> Result<Vec<Row>> {
        let fresh = vec![Running { count: 0, value: Value::Null }; self.calls.len()];
        let mut running: BTreeMap<Vec<Key>, Vec<Running>> = BTreeMap::new();
        while let Some(row) = self.input.next(pager)? {
            let mut key = Vec::with_capacity(self.groups.len());
            for group in &self.groups {
                key.push(Key { value: eval(group, &row)?, descending: false });
            }
            let states = running.entry(key).or_insert_with(|| fresh.clone());
            for (state, call) in states.iter_mut().zip(&self.calls) {
                state.add(call, &row)?;
            }
        }
        // With no GROUP BY, the whole input is one group, and it is a group even
        // when it has no rows in it.
        if self.groups.is_empty() && running.is_empty() {
            running.insert(Vec::new(), fresh);
        }
        let mut rows = Vec::with_capacity(running.len());
        for (key, states) in running {
            let mut row: Row = key.into_iter().map(|key| key.value).collect();
            for (state, call) in states.into_iter().zip(&self.calls) {
                row.push(state.finish(call.function));
            }
            rows.push(row);
        }
        rows.reverse();
        Ok(rows)
    }
    // @end
}

// @listing same
/// Whether two expressions are the same expression, wherever each was written.
fn same(a: &Expr, b: &Expr) -> bool {
    use ExprKind::*;
    let same_box = |a: &Option<Box<Expr>>, b: &Option<Box<Expr>>| match (a, b) {
        (Some(a), Some(b)) => same(a, b),
        (None, None) => true,
        _ => false,
    };
    match (&a.kind, &b.kind) {
        (Literal(a), Literal(b)) => a == b,
        (Column { table: t, name: n }, Column { table: u, name: m }) => {
            fold(n) == fold(m) && t.as_deref().map(fold) == u.as_deref().map(fold)
        }
        (Unary { op: p, operand: a }, Unary { op: q, operand: b }) => {
            p == q && same(a, b)
        }
        (
            Binary { op: p, left: a, right: b },
            Binary { op: q, left: c, right: d },
        ) => p == q && same(a, c) && same(b, d),
        (IsNull { operand: a, negated: p }, IsNull { operand: b, negated: q }) => {
            p == q && same(a, b)
        }
        (Call { function: p, arg: a }, Call { function: q, arg: b }) => {
            p == q && same_box(a, b)
        }
        _ => false,
    }
}
// @end

// @listing has-aggregate
/// Whether an expression has an aggregate anywhere in it.
pub(crate) fn has_aggregate(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Call { .. } => true,
        ExprKind::Literal(_) | ExprKind::Column { .. } => false,
        ExprKind::Unary { operand, .. } | ExprKind::IsNull { operand, .. } => {
            has_aggregate(operand)
        }
        ExprKind::Binary { left, right, .. } => {
            has_aggregate(left) || has_aggregate(right)
        }
    }
}

/// Whether a SELECT forms groups: it says so, or it uses an aggregate.
pub(crate) fn is_grouped(select: &Select) -> bool {
    let in_item = |item: &SelectItem| match item {
        SelectItem::Expr { expr, .. } => has_aggregate(expr),
        SelectItem::Star => false,
    };
    !select.group_by.is_empty()
        || select.having.is_some()
        || select.items.iter().any(in_item)
        || select.order_by.iter().any(|key| has_aggregate(&key.expr))
}
// @end

// @listing grouping
/// What a grouped query's expressions may be built from: the GROUP BY
/// expressions, and every different aggregate found so far.
struct Grouping<'a> {
    groups: &'a [Expr],
    calls: Vec<Expr>,
}

/// A reference to column `n` of the aggregation's output.
fn output_column(n: usize, span: Span) -> Expr {
    Expr { kind: ExprKind::Column { table: None, name: format!("#{n}") }, span }
}
// @end

// @listing rewrite
impl Grouping<'_> {
    /// Rewrites an expression over the table's rows as one over the rows that
    /// the aggregation hands on. A column that is neither grouped nor inside an
    /// aggregate has no value there, and is an error.
    fn rewrite(&mut self, expr: &Expr) -> Result<Expr> {
        if let Some(n) = self.groups.iter().position(|group| same(group, expr)) {
            return Ok(output_column(n, expr.span));
        }
        let kind = match &expr.kind {
            ExprKind::Call { .. } => {
                let found = self.calls.iter().position(|call| same(call, expr));
                let n = found.unwrap_or(self.calls.len());
                if found.is_none() {
                    self.calls.push(expr.clone());
                }
                return Ok(output_column(self.groups.len() + n, expr.span));
            }
            ExprKind::Column { name, .. } => {
                let message =
                    format!("{name} must be in the GROUP BY or inside an aggregate");
                return Err(Error::Invalid(message, expr.span));
            }
            ExprKind::Literal(value) => ExprKind::Literal(value.clone()),
            ExprKind::Unary { op, operand } => {
                ExprKind::Unary { op: *op, operand: Box::new(self.rewrite(operand)?) }
            }
            ExprKind::IsNull { operand, negated } => ExprKind::IsNull {
                operand: Box::new(self.rewrite(operand)?),
                negated: *negated,
            },
            ExprKind::Binary { op, left, right } => {
                let left = Box::new(self.rewrite(left)?);
                let right = Box::new(self.rewrite(right)?);
                ExprKind::Binary { op: *op, left, right }
            }
        };
        Ok(Expr { kind, span: expr.span })
    }
}
// @end

// @listing bind-call
/// Checks one aggregate against the table's columns, and works out the type of
/// its result.
fn bind_call(call: &Expr, scope: &Scope) -> Result<(AggregateCall, Type)> {
    let ExprKind::Call { function, arg } = &call.kind else { unreachable!() };
    let arg = match arg {
        Some(arg) => Some(bind(arg, scope)?),
        None => None,
    };
    let of = arg.as_ref().map_or(Type::Null, |arg| arg.ty);
    let ty = match function {
        Function::Count => Type::Integer,
        Function::Min | Function::Max => of,
        Function::Sum | Function::Avg => {
            if !matches!(of, Type::Integer | Type::Real | Type::Null) {
                let message =
                    format!("{} needs a number, and this is {of}", function.name());
                return Err(Error::Invalid(message, call.span));
            }
            if *function == Function::Avg { Type::Real } else { of }
        }
    };
    Ok((AggregateCall { function: *function, arg, span: call.span }, ty))
}
// @end

// @listing column-name
/// The name of a result column, by the rule of Section 14.6.
fn column_name(expr: &Expr, alias: &Option<String>) -> String {
    match (alias, &expr.kind) {
        (Some(alias), _) => alias.clone(),
        (None, ExprKind::Column { name, .. }) => name.clone(),
        (None, _) => expr.to_string(),
    }
}
// @end

// @listing rewritten
/// The parts of a grouped query that come after the grouping, rewritten over
/// its output, and the aggregates they were found to use.
pub(crate) struct Rewritten {
    /// The names of the result's columns.
    pub(crate) columns: Vec<String>,
    pub(crate) items: Vec<Expr>,
    pub(crate) having: Option<Expr>,
    /// Each key of the ORDER BY, and whether it is descending.
    pub(crate) order: Vec<(Expr, bool)>,
    pub(crate) calls: Vec<Expr>,
}

pub(crate) fn rewrite_query(select: &Select) -> Result<Rewritten> {
    let mut grouping = Grouping { groups: &select.group_by, calls: Vec::new() };
    let (mut items, mut columns) = (Vec::new(), Vec::new());
    for item in &select.items {
        let SelectItem::Expr { expr, alias } = item else {
            return sql_error("a query with groups can't select *".to_string());
        };
        items.push(grouping.rewrite(expr)?);
        columns.push(column_name(expr, alias));
    }
    let having = match &select.having {
        Some(having) => Some(grouping.rewrite(having)?),
        None => None,
    };
    let mut order = Vec::new();
    for key in &select.order_by {
        order.push((grouping.rewrite(key_expr(select, key))?, key.descending));
    }
    Ok(Rewritten { columns, items, having, order, calls: grouping.calls })
}
// @end

// @listing output-scope
/// Binds the GROUP BY expressions and the aggregates against the table. Returns
/// them, with a scope that describes the rows the aggregation will hand on.
pub(crate) fn bind_grouping(
    groups: &[Expr],
    calls: &[Expr],
    scope: &Scope,
) -> Result<(Vec<Bound>, Vec<AggregateCall>, Scope)> {
    let (mut bound_groups, mut bound_calls) = (Vec::new(), Vec::new());
    let mut columns = Vec::new();
    let mut column = |ty: Type| {
        let name = format!("#{}", columns.len());
        columns.push(ScopeColumn { table: String::new(), name, ty });
    };
    for group in groups {
        let group = bind(group, scope)?;
        column(group.ty);
        bound_groups.push(group);
    }
    for call in calls {
        let (call, ty) = bind_call(call, scope)?;
        column(ty);
        bound_calls.push(call);
    }
    Ok((bound_groups, bound_calls, Scope { columns }))
}
// @end

impl Database {
    // @listing select-grouped
    pub(crate) fn select_grouped(&mut self, select: &Select) -> Result<Output> {
        let table = self.table(&select.from)?.clone();
        // @hide
        #[allow(unused_variables)]
        // @show
        let scope = Scope::of(&table);
        // @hide
        let scope = self.scope_of(select)?;
        // @show
        let condition = match &select.filter {
            Some(filter) => Some(bind_condition(filter, &scope, "WHERE")?),
            None => None,
        };
        let query = rewrite_query(select)?;
        let (groups, calls, scope) =
            bind_grouping(&select.group_by, &query.calls, &scope)?;
        // From here on, `scope` is the aggregation's output and not the table.
        let mut exprs = Vec::new();
        for item in &query.items {
            exprs.push(bind(item, &scope)?);
        }
        let having = match &query.having {
            Some(having) => Some(bind_condition(having, &scope, "HAVING")?),
            None => None,
        };
        let mut keys = Vec::new();
        for (expr, descending) in &query.order {
            keys.push(SortKey { expr: bind(expr, &scope)?, descending: *descending });
        }

        let mut plan: Box<dyn Operator> =
            Box::new(Scan::new(&table, &mut self.pager)?);
        // @hide
        plan = self.join_plan(select, plan)?;
        // @show
        if let Some(condition) = condition {
            plan = Box::new(Filter { input: plan, condition });
        }
        plan = Box::new(Aggregate::new(plan, groups, calls));
        if let Some(condition) = having {
            plan = Box::new(Filter { input: plan, condition });
        }
        self.finish(plan, keys, exprs, select, query.columns)
    }
    // @end

    // @listing finish
    /// Puts the last operators on a plan, which are the same for every SELECT,
    /// and runs it.
    fn finish(
        &mut self,
        mut plan: Box<dyn Operator>,
        keys: Vec<SortKey>,
        exprs: Vec<Bound>,
        select: &Select,
        columns: Vec<String>,
    ) -> Result<Output> {
        if !keys.is_empty() {
            let (budget, temp) = (self.sort_budget, self.temp.clone());
            plan = Box::new(Sort::new(plan, keys, budget, temp));
        }
        plan = Box::new(Project { input: plan, exprs });
        if select.limit.is_some() || select.offset.is_some() {
            let skip = select.offset.unwrap_or(0);
            plan = Box::new(Limit { input: plan, skip, take: select.limit });
        }
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(Output::Rows(ResultSet { columns, rows }))
    }
    // @end
}
