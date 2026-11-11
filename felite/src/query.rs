//! SELECT: from a statement to a chain of operators, and from that to rows.

use crate::bind::{Bound, BoundKind, Scope, bind, bind_condition};
use crate::db::{Database, Output};
use crate::error::Result;
use crate::exec::{Filter, Operator, Project, Row, Scan};
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
        // Everything has been checked. Build the operators, innermost first.
        let mut plan: Box<dyn Operator> =
            Box::new(Scan::new(&table, &mut self.pager)?);
        if let Some(condition) = condition {
            plan = Box::new(Filter { input: plan, condition });
        }
        plan = Box::new(Project { input: plan, exprs });

        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(Output::Rows(ResultSet { columns, rows }))
    }
    // @end
}
