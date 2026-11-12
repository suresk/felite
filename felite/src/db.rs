//! A database: a pager, the schema table, and the tables it lists.

use crate::btree::{Cursor, Tree, row_id};
use crate::catalog::{Table, fold, key_column, sql_error};
use crate::error::Result;
use crate::overflow::load;
use crate::pager::Pager;
use crate::query::ResultSet;
use crate::sort::SORT_BUDGET;
use crate::sql::{CreateTable, Statement, parse};
use crate::storage::{Storage, TempSpace};
use crate::transaction::Transaction;
use std::collections::BTreeMap;
use std::fs::OpenOptions;

// @listing database
pub struct Database {
    pub(crate) pager: Pager,
    /// The table of tables.
    pub(crate) schema: Tree,
    /// Every table, under its name in lower case.
    pub(crate) tables: BTreeMap<String, Table>,
    // @hide
    pub(crate) transaction: Transaction,
    /// Where queries put their temporary files.
    pub(crate) temp: TempSpace,
    /// How many bytes a sort may hold in memory.
    pub(crate) sort_budget: usize,
    // @show
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
        let (schema, tables) = Database::read_catalog(&mut pager)?;
        Ok(Database {
            pager,
            schema,
            tables,
            // @hide
            transaction: Transaction::Closed,
            temp: TempSpace::Memory,
            sort_budget: SORT_BUDGET,
            // @show
        })
    }

    /// Finds the schema table, creating it in a new database, and learns every
    /// table from its row there.
    pub(crate) fn read_catalog(
        pager: &mut Pager,
    ) -> Result<(Tree, BTreeMap<String, Table>)> {
        let schema = match pager.schema_root()? {
            0 => {
                let schema = Tree::create(pager)?;
                pager.set_schema_root(schema.root())?;
                schema
            }
            root => Tree::open(root),
        };
        let mut tables = BTreeMap::new();
        let mut cursor = Cursor::first(&schema, pager)?;
        while let Some((key, stored)) = cursor.next(pager)? {
            let record = load(pager, stored)?;
            let table = Table::from_schema(row_id(&key)?, &record)?;
            tables.insert(fold(&table.name), table);
        }
        Ok((schema, tables))
    }
    // @end

    // @listing open-file
    /// Opens the database in the file at `path`, creating it if there is none. Its
    /// log is the file beside it whose name has `-log` on the end.
    pub fn open_file(path: &str) -> Result<Database> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        let file = options.open(path)?;
        let log = options.open(format!("{path}-log"))?;
        Database::with_pager(Pager::open_logged(Box::new(file), Box::new(log))?)
            // @hide
            .map(|db| Database { temp: TempSpace::Beside(path.to_string()), ..db })
        // @show
    }
    // @end

    // @listing sort-budget
    /// Sets how many bytes of rows a sort may hold in memory before it starts to
    /// write them to a temporary file.
    pub fn set_sort_budget(&mut self, bytes: usize) {
        self.sort_budget = bytes;
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
        // @hide
        if self.pager.logged() {
            return self.transact(sql);
        }
        // @show
        let output = self.run(parse(sql)?)?;
        // @hide
        if let Output::Rows(_) = output {
            return Ok(output);
        }
        // @show
        self.pager.flush()?;
        Ok(output)
    }

    /// Carries out a statement that has been parsed.
    pub(crate) fn run(&mut self, statement: Statement) -> Result<Output> {
        match statement {
            Statement::CreateTable(create) => self.create_table(create),
            Statement::DropTable(name) => self.drop_table(&name),
            // @hide
            Statement::Insert(insert) => self.insert(&insert),
            Statement::Select(select) => self.select(&select),
            Statement::Delete(delete) => self.delete(&delete),
            Statement::Update(update) => self.update(&update),
            Statement::Begin | Statement::Commit | Statement::Rollback => {
                sql_error("a database with no log has no transactions".to_string())
            }
            #[allow(unreachable_patterns)]
            // @show
            other => sql_error(format!("felite cannot run this yet: {other}")),
        }
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
