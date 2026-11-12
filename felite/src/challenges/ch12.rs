//! Chapter 12 challenges: printing the schema, creating a table only if it is
//! missing, renaming a table, and accounting for every page of a database.

use crate::catalog::{fold, sql_error};
use crate::challenges::ch08::account_rows;
use crate::db::Database;
use crate::error::Result;
use crate::sql::{CreateTable, Statement, parse};

// @listing schema-dump
/// The statements that would recreate every table, one to a line, in order of
/// table name.
pub fn schema_dump(db: &Database) -> String {
    let mut out = String::new();
    for table in db.tables() {
        let create =
            CreateTable { name: table.name.clone(), columns: table.columns.clone() };
        out += &format!("{};\n", Statement::CreateTable(create));
    }
    out
}
// @end

// @listing create-if-missing
/// Runs a CREATE TABLE statement unless a table of that name exists. Returns
/// whether it created one.
pub fn create_if_missing(db: &mut Database, sql: &str) -> Result<bool> {
    let Statement::CreateTable(create) = parse(sql)? else {
        return sql_error("this is not a CREATE TABLE statement".to_string());
    };
    if db.table(&create.name).is_ok() {
        return Ok(false);
    }
    db.execute(sql)?;
    Ok(true)
}
// @end

// @listing rename
/// Gives a table a new name. Its rows and its pages are untouched.
pub fn rename_table(db: &mut Database, old: &str, new: &str) -> Result<()> {
    let mut table = db.table(old)?.clone();
    // @hide
    crate::index::no_indexes(&table)?;
    // @show
    if fold(old) != fold(new) && db.table(new).is_ok() {
        return sql_error(format!("there is already a table named {new}"));
    }
    // The parser decides what a name is, so ask it.
    let trial = parse(&format!("CREATE TABLE {new} (x INTEGER)"));
    let parsed =
        matches!(trial, Ok(Statement::CreateTable(create)) if create.name == new);
    if !parsed {
        return sql_error(format!("{new} cannot be the name of a table"));
    }
    table.name = new.to_string();
    db.schema.delete_row(&mut db.pager, table.schema_row)?;
    db.schema.insert_row(&mut db.pager, table.schema_row, &table.schema_record())?;
    db.tables.remove(&fold(old));
    db.tables.insert(fold(new), table);
    db.pager.flush()
}
// @end

// @listing account-database
/// Checks that every page of the file belongs to the header, the free list, the
/// schema table, a table, or one row's overflow chain, and to only one of them.
pub fn account_database(db: &mut Database) -> Result<()> {
    let mut trees = vec![db.schema];
    trees.extend(db.tables.values().map(|table| table.tree));
    for tree in &trees {
        tree.check(&mut db.pager)?;
    }
    account_rows(&mut db.pager, &trees)
}
// @end
