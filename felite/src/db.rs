//! A database: a pager, the schema table, and the tables it lists.

use crate::btree::{Cursor, Tree, row_id};
use crate::catalog::{Table, fold, key_column, sql_error};
use crate::error::Result;
use crate::overflow::load;
use crate::pager::Pager;
use crate::query::ResultSet;
use crate::sql::{CreateTable, Statement, parse};
use crate::storage::Storage;
use std::collections::BTreeMap;

// @listing database
pub struct Database {
    pub(crate) pager: Pager,
    /// The table of tables.
    pub(crate) schema: Tree,
    /// Every table, under its name in lower case.
    pub(crate) tables: BTreeMap<String, Table>,
}

/// What a statement produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// The statement did its work and has nothing to show for it.
    Done,
    // @hide
    /// The statement changed this many rows.
    Count(usize),
    /// The statement was a query, and these are its rows.
    Rows(ResultSet),
    // @show
}
// @end

impl Database {
    // @listing open
    /// Opens the database in `storage`, creating its schema table if it is new.
    pub fn open(storage: Box<dyn Storage>) -> Result<Database> {
        Database::with_pager(Pager::open(storage)?)
    }

    /// Opens the database that `pager` holds.
    pub fn with_pager(mut pager: Pager) -> Result<Database> {
        let schema = match pager.schema_root()? {
            0 => {
                let schema = Tree::create(&mut pager)?;
                pager.set_schema_root(schema.root())?;
                schema
            }
            root => Tree::open(root),
        };
        // Read the schema table, and learn each table from its row.
        let mut tables = BTreeMap::new();
        let mut cursor = Cursor::first(&schema, &mut pager)?;
        while let Some((key, stored)) = cursor.next(&mut pager)? {
            let record = load(&mut pager, stored)?;
            let table = Table::from_schema(row_id(&key)?, &record)?;
            tables.insert(fold(&table.name), table);
        }
        Ok(Database { pager, schema, tables })
    }
    // @end

    // @listing lookups
    /// The table with this name, in any mix of upper and lower case.
    pub fn table(&self, name: &str) -> Result<&Table> {
        match self.tables.get(&fold(name)) {
            Some(table) => Ok(table),
            None => sql_error(format!("there is no table named {name}")),
        }
    }

    /// Every table, in order of name.
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.tables.values()
    }

    pub fn schema(&self) -> Tree {
        self.schema
    }

    /// The pager underneath, for tests and tools that want to look at pages.
    pub fn pager(&mut self) -> &mut Pager {
        &mut self.pager
    }
    // @end

    // @listing execute
    /// Runs one statement, and writes what it changed to the file.
    pub fn execute(&mut self, sql: &str) -> Result<Output> {
        let output = match parse(sql)? {
            Statement::CreateTable(create) => self.create_table(create)?,
            Statement::DropTable(name) => self.drop_table(&name)?,
            // @hide
            Statement::Insert(insert) => self.insert(&insert)?,
            Statement::Select(select) => return self.select(&select),
            Statement::Delete(delete) => self.delete(&delete)?,
            Statement::Update(update) => self.update(&update)?,
            #[allow(unreachable_patterns)]
            // @show
            other => {
                return sql_error(format!("felite cannot run this yet: {other}"));
            }
        };
        self.pager.flush()?;
        Ok(output)
    }
    // @end

    // @listing create-table
    fn create_table(&mut self, create: CreateTable) -> Result<Output> {
        if let Ok(existing) = self.table(&create.name) {
            return sql_error(format!(
                "there is already a table named {}",
                existing.name
            ));
        }
        let key_column = key_column(&create)?;
        let table = Table {
            name: create.name,
            columns: create.columns,
            tree: Tree::create(&mut self.pager)?,
            key_column,
            schema_row: self.schema.next_row_id(&mut self.pager)?,
        };
        self.schema.insert_row(
            &mut self.pager,
            table.schema_row,
            &table.schema_record(),
        )?;
        self.tables.insert(fold(&table.name), table);
        Ok(Output::Done)
    }
    // @end

    // @listing drop-table
    fn drop_table(&mut self, name: &str) -> Result<Output> {
        let table = self.table(name)?.clone();
        table.tree.destroy_rows(&mut self.pager)?;
        self.schema.delete_row(&mut self.pager, table.schema_row)?;
        self.tables.remove(&fold(name));
        Ok(Output::Done)
    }
    // @end
}
