//! What the database knows about its tables.

use crate::btree::Tree;
use crate::error::{Error, Result};
use crate::page::PageNo;
use crate::record::{Value, decode, encode};
use crate::sql::{ColumnDef, ColumnType, CreateTable, Statement, parse};

// @listing table
/// A table, as the database needs to know it in order to use it.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    /// The tree that holds the table's rows.
    pub tree: Tree,
    /// The position of the INTEGER PRIMARY KEY column, if the table has one.
    pub key_column: Option<usize>,
    /// The row id of this table's row in the schema table.
    pub schema_row: i64,
}
// @end

// @listing sql-error
pub(crate) fn sql_error<T>(message: String) -> Result<T> {
    Err(Error::Sql(message))
}

/// The form in which a name is looked up. Names that differ only in the case of
/// their letters are the same name.
pub(crate) fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}
// @end

// @listing check-definition
/// Checks a table definition that the parser accepted, and finds its key column.
pub(crate) fn key_column(create: &CreateTable) -> Result<Option<usize>> {
    let mut key = None;
    for (i, column) in create.columns.iter().enumerate() {
        let name = &column.name;
        let earlier = &create.columns[..i];
        if earlier.iter().any(|other| fold(&other.name) == fold(name)) {
            return sql_error(format!("there are two columns named {name}"));
        }
        if !column.primary_key {
            continue;
        }
        if key.is_some() {
            return sql_error("a table can have only one PRIMARY KEY".to_string());
        }
        if column.kind != ColumnType::Integer {
            return sql_error(format!(
                "the PRIMARY KEY column {name} must be an INTEGER"
            ));
        }
        key = Some(i);
    }
    Ok(key)
}
// @end

impl Table {
    // @listing schema-record
    /// This table's row for the schema table: what it is, its name, the root page
    /// of its tree, and the statement that made it.
    pub(crate) fn schema_record(&self) -> Vec<u8> {
        let create =
            CreateTable { name: self.name.clone(), columns: self.columns.clone() };
        encode(&[
            Value::Text("table".to_string()),
            Value::Text(self.name.clone()),
            Value::Integer(self.tree.root() as i64),
            Value::Text(Statement::CreateTable(create).to_string()),
        ])
    }
    // @end

    // @listing from-schema
    /// The table that a row of the schema table describes.
    pub(crate) fn from_schema(schema_row: i64, record: &[u8]) -> Result<Table> {
        let bad =
            |what: &str| Error::Corrupt(format!("schema row {schema_row}: {what}"));
        let values = decode(record)?;
        let [
            Value::Text(kind),
            Value::Text(_),
            Value::Integer(root),
            Value::Text(sql),
        ] = values.as_slice()
        else {
            return Err(bad("not the four values of a schema row"));
        };
        if kind != "table" {
            return Err(bad("not a table"));
        }
        let Ok(root) = PageNo::try_from(*root) else {
            return Err(bad("a root page out of range"));
        };
        let Ok(Statement::CreateTable(create)) = parse(sql) else {
            return Err(bad("its SQL is not a CREATE TABLE statement"));
        };
        let Ok(key_column) = key_column(&create) else {
            return Err(bad("its table definition is not valid"));
        };
        Ok(Table {
            name: create.name,
            columns: create.columns,
            tree: Tree::open(root),
            key_column,
            schema_row,
        })
    }
    // @end
}
