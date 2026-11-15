//! SELECT: from a statement to a chain of operators, and from that to rows.

use crate::aggregate::is_grouped;
use crate::bind::{Bound, BoundKind, Scope, ScopeColumn, bind, bind_condition};
use crate::catalog::fold;
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::exec::{Filter, Limit, Operator, Project, Row, Scan};
use crate::record::Value;
use crate::sort::{Sort, SortKey};
use crate::sql::{Expr, ExprKind, OrderKey, Select, SelectItem, Span};
use std::borrow::Cow;

// @listing result-set
/// The rows a query produced, and a name for each of their columns.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}
// @end

// @listing select-list
/// Works out what a SELECT list asks for: an expression and a name for each
/// column of the result.
pub(crate) fn select_list(
    items: &[SelectItem],
    scope: &Scope,
) -> Result<(Vec<Bound>, Vec<String>)> {
    let (mut exprs, mut names) = (Vec::new(), Vec::new());
    for item in items {
        match item {
            // A star stands for every column in scope, in order.
            SelectItem::Star => {
                for (position, column) in scope.columns.iter().enumerate() {
                    let span = Span { offset: 0, line: 1, column: 1 };
                    exprs.push(Bound {
                        kind: BoundKind::Column(position),
                        ty: column.ty,
                        span,
                    });
                    names.push(column.name.clone());
                }
            }
            SelectItem::Expr { expr, alias } => {
                exprs.push(bind(expr, scope)?);
                names.push(match (alias, &expr.kind) {
                    (Some(alias), _) => alias.clone(),
                    (None, ExprKind::Column { name, .. }) => name.clone(),
                    (None, _) => expr.to_string(),
                });
            }
        }
    }
    Ok((exprs, names))
}
// @end

// @listing order-keys
/// The expression that a key of an ORDER BY stands for. A key that is the name
/// given to something in the SELECT list means that expression.
pub(crate) fn key_expr<'a>(select: &'a Select, key: &'a OrderKey) -> &'a Expr {
    let ExprKind::Column { table: None, name } = &key.expr.kind else {
        return &key.expr;
    };
    let named = select.items.iter().find_map(|item| match item {
        SelectItem::Expr { expr, alias: Some(alias) }
            if fold(alias) == fold(name) =>
        {
            Some(expr)
        }
        _ => None,
    });
    named.unwrap_or(&key.expr)
}

/// Checks an ORDER BY against the columns in scope.
pub(crate) fn order_keys(select: &Select, scope: &Scope) -> Result<Vec<SortKey>> {
    let mut keys = Vec::new();
    for key in &select.order_by {
        let expr = bind(key_expr(select, key), scope)?;
        keys.push(SortKey { expr, descending: key.descending });
    }
    Ok(keys)
}
// @end

impl Database {
    // @listing order-position
    /// A copy of a SELECT in which each ORDER BY key that is a whole number has
    /// been replaced by the result column at that position, counting from 1. A
    /// star in the SELECT list stands for every column in scope.
    pub(crate) fn order_positions<'a>(
        &self,
        select: &'a Select,
    ) -> Result<Cow<'a, Select>> {
        let position = |key: &OrderKey| match key.expr.kind {
            ExprKind::Literal(Value::Integer(n)) => Some(n),
            _ => None,
        };
        if select.order_by.iter().all(|key| position(key).is_none()) {
            return Ok(Cow::Borrowed(select));
        }
        // What each column of the result is, in order.
        let mut columns = Vec::new();
        for item in &select.items {
            match item {
                SelectItem::Expr { expr, .. } => columns.push(expr.kind.clone()),
                SelectItem::Star => {
                    for column in self.scope_of(select)?.columns {
                        let ScopeColumn { table, name, .. } = column;
                        columns.push(ExprKind::Column { table: Some(table), name });
                    }
                }
            }
        }
        let mut select = select.clone();
        for key in &mut select.order_by {
            let Some(n) = position(key) else { continue };
            let span = key.expr.span;
            let at = usize::try_from(n).ok().and_then(|n| n.checked_sub(1));
            let Some(kind) = at.and_then(|at| columns.get(at)) else {
                let message = format!("the result has no column {n} to order by");
                return Err(Error::Invalid(message, span));
            };
            key.expr = Expr { kind: kind.clone(), span };
        }
        Ok(Cow::Owned(select))
    }
    // @end

    // @listing select
    pub(crate) fn select(&mut self, select: &Select) -> Result<Output> {
        // @hide
        let resolved = self.order_positions(select)?;
        let select = &*resolved;
        if self.planner {
            return self.select_planned(select);
        }
        if is_grouped(select) {
            return self.select_grouped(select);
        }
        // @show
        let table = self.table(&select.from)?.clone();
        // @hide
        #[allow(unused_variables)]
        // @show
        let scope = Scope::of(&table);
        // @hide
        let scope = self.scope_of(select)?;
        // @show
        let (exprs, columns) = select_list(&select.items, &scope)?;
        let condition = match &select.filter {
            Some(filter) => Some(bind_condition(filter, &scope, "WHERE")?),
            None => None,
        };
        // @hide
        let keys = order_keys(select, &scope)?;
        // @show
        // Everything has been checked. Build the operators, innermost first.
        let mut plan: Box<dyn Operator> =
            Box::new(Scan::new(&table, &mut self.pager)?);
        // @hide
        plan = self.join_plan(select, plan)?;
        // @show
        if let Some(condition) = condition {
            plan = Box::new(Filter { input: plan, condition });
        }
        // @hide
        if !keys.is_empty() {
            let (budget, temp) = (self.sort_budget, self.temp.clone());
            plan = Box::new(Sort::new(plan, keys, budget, temp));
        }
        // @show
        plan = Box::new(Project { input: plan, exprs });
        // @hide
        if select.limit.is_some() || select.offset.is_some() {
            let skip = select.offset.unwrap_or(0);
            plan = Box::new(Limit { input: plan, skip, take: select.limit });
        }
        // @show

        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(Output::Rows(ResultSet { columns, rows }))
    }
    // @end
}
