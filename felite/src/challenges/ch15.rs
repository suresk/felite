//! Chapter 15 challenges: emptying a table at once, getting back what was deleted,
//! and counting what a statement would change without changing it.

use crate::btree::Tree;
use crate::catalog::{fold, sql_error};
use crate::db::Database;
use crate::error::Result;
use crate::exec::Row;
use crate::sql::{Statement, parse, parse_expr};

// @listing truncate
/// Removes every row of a table by freeing its whole tree and starting a new one.
/// Returns nothing about the rows, because it never looks at them.
pub fn truncate_table(db: &mut Database, name: &str) -> Result<()> {
    let mut table = db.table(name)?.clone();
    table.tree.destroy_rows(&mut db.pager)?;
    table.tree = Tree::create(&mut db.pager)?;
    // The new tree may have a different root, so the catalog is told.
    db.schema.delete_row(&mut db.pager, table.schema_row)?;
    db.schema.insert_row(&mut db.pager, table.schema_row, &table.schema_record())?;
    db.tables.insert(fold(name), table);
    db.pager.flush()
}
// @end

// @listing returning
/// Deletes the rows of a table that satisfy a condition, given as SQL text, and
/// returns them.
pub fn delete_returning(
    db: &mut Database,
    table: &str,
    condition: &str,
) -> Result<Vec<Row>> {
    let table = db.table(table)?.clone();
    let doomed = db.matching(&table, &Some(parse_expr(condition)?))?;
    for (id, _) in &doomed {
        table.tree.delete_row(&mut db.pager, *id)?;
    }
    db.pager.flush()?;
    Ok(doomed.into_iter().map(|(_, row)| row).collect())
}
// @end

// @listing would-affect
/// How many rows a DELETE or UPDATE would change, without changing them.
pub fn would_affect(db: &mut Database, sql: &str) -> Result<usize> {
    let (table, filter) = match parse(sql)? {
        Statement::Delete(delete) => (delete.table, delete.filter),
        Statement::Update(update) => (update.table, update.filter),
        _ => {
            return sql_error(
                "that statement is not a DELETE or an UPDATE".to_string(),
            );
        }
    };
    let table = db.table(&table)?.clone();
    Ok(db.matching(&table, &filter)?.len())
}
// @end
