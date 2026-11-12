//! SELECT: from a statement to a chain of operators, and from that to rows.

use crate::bind::{Bound, BoundKind, Scope, bind, bind_condition};
use crate::catalog::fold;
use crate::db::{Database, Output};
use crate::error::Result;
use crate::exec::{Filter, Limit, Operator, Project, Row, Scan};
use crate::sort::{Sort, SortKey};
use crate::sql::{ExprKind, Select, SelectItem, Span};

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
fn select_list(
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
/// Checks an ORDER BY against the columns in scope. A key that is the name given
/// to something in the SELECT list means that expression.
fn order_keys(select: &Select, scope: &Scope) -> Result<Vec<SortKey>> {
    let named = |wanted: &str| {
        select.items.iter().find_map(|item| match item {
            SelectItem::Expr { expr, alias: Some(alias) }
                if fold(alias) == fold(wanted) =>
            {
                Some(expr)
            }
            _ => None,
        })
    };
    let mut keys = Vec::new();
    for key in &select.order_by {
        let expr = match &key.expr.kind {
            ExprKind::Column { table: None, name } => {
                named(name).unwrap_or(&key.expr)
            }
            _ => &key.expr,
        };
        keys.push(SortKey { expr: bind(expr, scope)?, descending: key.descending });
    }
    Ok(keys)
}
// @end

impl Database {
    // @listing select
    pub(crate) fn select(&mut self, select: &Select) -> Result<Output> {
        let table = self.table(&select.from)?.clone();
        let scope = Scope::of(&table);
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
