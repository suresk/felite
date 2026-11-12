//! SELECT: from a statement to a chain of operators, and from that to rows.

use crate::aggregate::is_grouped;
use crate::bind::{Bound, BoundKind, Scope, bind, bind_condition};
use crate::catalog::fold;
use crate::db::{Database, Output};
use crate::error::Result;
use crate::exec::{Filter, Limit, Operator, Project, Row, Scan};
use crate::sort::{Sort, SortKey};
use crate::sql::{Expr, ExprKind, OrderKey, Select, SelectItem, Span};

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
    // @listing select
    pub(crate) fn select(&mut self, select: &Select) -> Result<Output> {
        // @hide
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
