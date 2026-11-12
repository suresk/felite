//! Secondary indexes: a second tree for a table, in the order of other columns.

use crate::btree::{Cursor, Tree, row_id, row_key};
use crate::catalog::{Table, fold, sql_error};
use crate::db::{Database, Output};
use crate::error::{Error, Result};
use crate::exec::{Operator, Row, Scan};
use crate::keys::{after, encode_key};
use crate::page::PageNo;
use crate::pager::Pager;
use crate::record::{Value, decode, encode};
use crate::sql::{CreateIndex, Statement, parse};
use std::collections::{BTreeMap, BTreeSet};

// @listing index
/// An index on a table: a tree with an entry for each of the table's rows, whose
/// key is the row's values in the indexed columns, followed by its row id.
#[derive(Debug, Clone, PartialEq)]
pub struct Index {
    pub name: String,
    /// The name of the table it is an index on.
    pub table: String,
    pub columns: Vec<String>,
    /// Where each of those columns is in a row of the table.
    pub positions: Vec<usize>,
    /// The position of the table's key column, whose value is the row id.
    pub key_column: Option<usize>,
    pub unique: bool,
    pub tree: Tree,
    /// The row id of this index's row in the schema table.
    pub schema_row: i64,
}
// @end

impl Index {
    // @listing index-keys
    /// A row's values in the indexed columns. `id` is the row's id, which is the
    /// value of the table's key column.
    pub fn values(&self, id: i64, row: &[Value]) -> Vec<Value> {
        let value = |position: &usize| match self.key_column == Some(*position) {
            true => Value::Integer(id),
            false => row[*position].clone(),
        };
        self.positions.iter().map(value).collect()
    }

    /// The key of the entry for a row: its values, then its row id.
    pub fn key(&self, id: i64, row: &[Value]) -> Vec<u8> {
        let mut key = encode_key(&self.values(id, row));
        key.extend(row_key(id));
        key
    }
    // @end

    // @listing index-new
    /// The index that a CREATE INDEX statement describes, for a table.
    pub(crate) fn new(
        create: CreateIndex,
        table: &Table,
        tree: Tree,
        schema_row: i64,
    ) -> Result<Index> {
        let mut positions = Vec::new();
        for name in &create.columns {
            let found =
                table.columns.iter().position(|c| fold(&c.name) == fold(name));
            let Some(position) = found else {
                let table = &table.name;
                return sql_error(format!("the table {table} has no column {name}"));
            };
            if positions.contains(&position) {
                return sql_error(format!("the column {name} is in the index twice"));
            }
            positions.push(position);
        }
        Ok(Index {
            name: create.name,
            table: table.name.clone(),
            columns: create.columns,
            positions,
            key_column: table.key_column,
            unique: create.unique,
            tree,
            schema_row,
        })
    }
    // @end

    // @listing index-schema
    /// The statement that would make this index.
    pub fn sql(&self) -> String {
        let create = CreateIndex {
            name: self.name.clone(),
            table: self.table.clone(),
            columns: self.columns.clone(),
            unique: self.unique,
        };
        Statement::CreateIndex(create).to_string()
    }

    /// This index's row for the schema table, in the same four parts as a
    /// table's.
    fn schema_record(&self) -> Vec<u8> {
        encode(&[
            Value::Text("index".to_string()),
            Value::Text(self.name.clone()),
            Value::Integer(self.tree.root() as i64),
            Value::Text(self.sql()),
        ])
    }
    // @end
}

// @listing index-load
/// What a row of the schema table says about an index: its statement, the root
/// of its tree, and the schema row itself.
type Described = (CreateIndex, PageNo, i64);

/// If a row of the schema table describes an index, returns the description.
pub(crate) fn index_in_schema(
    schema_row: i64,
    record: &[u8],
) -> Result<Option<Described>> {
    let bad = |what: &str| Error::Corrupt(format!("schema row {schema_row}: {what}"));
    let values = decode(record)?;
    let [Value::Text(kind), _, Value::Integer(root), Value::Text(sql)] =
        values.as_slice()
    else {
        return Ok(None);
    };
    if kind != "index" {
        return Ok(None);
    }
    let Ok(root) = PageNo::try_from(*root) else {
        return Err(bad("a root page out of range"));
    };
    let Ok(Statement::CreateIndex(create)) = parse(sql) else {
        return Err(bad("its SQL is not a CREATE INDEX statement"));
    };
    Ok(Some((create, root, schema_row)))
}

// @end

// @listing index-attach
/// Gives each index that the schema table described to its table.
pub(crate) fn attach_indexes(
    tables: &mut BTreeMap<String, Table>,
    described: Vec<Described>,
) -> Result<()> {
    for (create, root, schema_row) in described {
        let bad =
            Error::Corrupt(format!("schema row {schema_row}: an index on nothing"));
        let Some(table) = tables.get_mut(&fold(&create.table)) else {
            return Err(bad);
        };
        let Ok(index) = Index::new(create, table, Tree::open(root), schema_row)
        else {
            return Err(bad);
        };
        table.indexes.push(index);
    }
    Ok(())
}
// @end

// @listing has-prefix
/// The row ids of the entries of an index whose keys begin with `prefix`.
fn ids_with_prefix(
    index: &Index,
    pager: &mut Pager,
    prefix: &[u8],
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for (key, _) in index.tree.range(pager, prefix, &after(prefix))? {
        ids.push(entry_id(&key)?);
    }
    Ok(ids)
}

/// The row id at the end of an index entry's key.
fn entry_id(key: &[u8]) -> Result<i64> {
    match key.len().checked_sub(8) {
        Some(start) => row_id(&key[start..]),
        None => {
            Err(Error::Corrupt("an index key too short for a row id".to_string()))
        }
    }
}
// @end

impl Table {
    // @listing check-indexes
    /// Makes sure that a statement's new rows can go into every index. `leaving`
    /// holds the ids of the rows that the statement is removing, whose entries
    /// will be gone by the time the new ones arrive.
    pub(crate) fn check_indexes(
        &self,
        pager: &mut Pager,
        leaving: &BTreeSet<i64>,
        arriving: &[(i64, Row)],
    ) -> Result<()> {
        for index in self.indexes.iter().filter(|index| index.unique) {
            let mut seen = BTreeSet::new();
            for (id, row) in arriving {
                let values = index.values(*id, row);
                // A NULL equals nothing, so a row with one in an indexed column
                // can't be a duplicate of anything.
                if values.contains(&Value::Null) {
                    continue;
                }
                let prefix = encode_key(&values);
                let staying = ids_with_prefix(index, pager, &prefix)?;
                let taken = staying.iter().any(|id| !leaving.contains(id));
                if taken || !seen.insert(prefix) {
                    let (name, columns) = (&index.name, index.columns.join(", "));
                    return sql_error(format!(
                        "two rows would have the same {columns}, and {name} is UNIQUE"
                    ));
                }
            }
        }
        Ok(())
    }
    // @end

    // @listing index-row
    /// Adds a row's entry to every index of the table.
    pub(crate) fn index_row(
        &self,
        pager: &mut Pager,
        id: i64,
        row: &[Value],
    ) -> Result<()> {
        for index in &self.indexes {
            index.tree.insert(pager, &index.key(id, row), &[])?;
        }
        Ok(())
    }

    /// Removes a row's entry from every index of the table.
    pub(crate) fn unindex_row(
        &self,
        pager: &mut Pager,
        id: i64,
        row: &[Value],
    ) -> Result<()> {
        for index in &self.indexes {
            if !index.tree.delete(pager, &index.key(id, row))? {
                let what =
                    format!("the index {} has no entry for row {id}", index.name);
                return Err(Error::Corrupt(what));
            }
        }
        Ok(())
    }
    // @end

    // @listing fetch
    /// The row with a given id, as a scan would return it.
    pub fn fetch(&self, pager: &mut Pager, id: i64) -> Result<Option<Row>> {
        let Some(record) = self.tree.get_row(pager, id)? else {
            return Ok(None);
        };
        let mut row = decode(&record)?;
        if row.len() != self.columns.len() {
            let what = format!(
                "a row of {} values in a table of {}",
                row.len(),
                self.columns.len()
            );
            return Err(Error::Corrupt(what));
        }
        if let Some(position) = self.key_column {
            row[position] = Value::Integer(id);
        }
        Ok(Some(row))
    }
    // @end

    // @listing find-index
    /// The index of this table with a given name.
    pub fn index(&self, name: &str) -> Option<&Index> {
        self.indexes.iter().find(|index| fold(&index.name) == fold(name))
    }
    // @end
}

/// Refuses to go on if a table has indexes. For the challenge solutions of
/// earlier chapters, which change a table's tree and know nothing of indexes.
pub(crate) fn no_indexes(table: &Table) -> Result<()> {
    if table.indexes.is_empty() {
        return Ok(());
    }
    sql_error(format!(
        "the table {} has indexes, which this would not maintain",
        table.name
    ))
}

impl Database {
    // @listing create-index
    pub(crate) fn create_index(&mut self, create: CreateIndex) -> Result<Output> {
        let name = create.name.clone();
        let taken = |table: &Table| table.index(&name).is_some();
        if self.tables.values().any(taken) {
            return sql_error(format!("there is already an index named {name}"));
        }
        let table = self.table(&create.table)?.clone();
        let schema_row = self.schema.next_row_id(&mut self.pager)?;
        let tree = Tree::create(&mut self.pager)?;
        let index = Index::new(create, &table, tree, schema_row)?;
        if let Err(error) = self.fill_index(&table, &index) {
            // Give back the pages of the half-built tree.
            index.tree.destroy(&mut self.pager)?;
            return Err(error);
        }
        self.schema.insert_row(
            &mut self.pager,
            schema_row,
            &index.schema_record(),
        )?;
        if let Some(table) = self.tables.get_mut(&fold(&table.name)) {
            table.indexes.push(index);
        }
        Ok(Output::Done)
    }

    // @end

    // @listing fill-index
    /// Gives a new index an entry for every row its table has.
    fn fill_index(&mut self, table: &Table, index: &Index) -> Result<()> {
        // The table as it will be, with only this index to check and add to.
        let only = Table { indexes: vec![index.clone()], ..table.clone() };
        let mut scan = Scan::new(table, &mut self.pager)?;
        while let Some(row) = scan.next(&mut self.pager)? {
            let id = scan.row_id;
            only.check_indexes(
                &mut self.pager,
                &BTreeSet::new(),
                &[(id, row.clone())],
            )?;
            only.index_row(&mut self.pager, id, &row)?;
        }
        Ok(())
    }
    // @end
}

// @listing index-scan
/// Reads the rows of a table through one of its indexes: the entries of the
/// index from one key up to another, and for each, the row it points to.
pub struct IndexScan {
    table: Table,
    cursor: Cursor,
    /// The scan stops at the first key that is not less than this.
    until: Option<Vec<u8>>,
    done: bool,
    /// How many entries of the index have been read.
    pub entries: u64,
}

impl IndexScan {
    /// A scan of the entries whose keys are at least `from` and less than
    /// `until`, or to the end of the index if `until` is `None`.
    pub fn new(
        table: &Table,
        index: &Index,
        pager: &mut Pager,
        from: &[u8],
        until: Option<Vec<u8>>,
    ) -> Result<IndexScan> {
        let cursor = Cursor::seek(&index.tree, pager, from)?;
        Ok(IndexScan { table: table.clone(), cursor, until, done: false, entries: 0 })
    }
}

// @end

// @listing index-scan-next
impl Operator for IndexScan {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        if self.done {
            return Ok(None);
        }
        let entry = self.cursor.next(pager)?;
        let key = match (entry, &self.until) {
            (Some((key, _)), Some(until)) if key < *until => key,
            (Some((key, _)), None) => key,
            _ => {
                self.done = true;
                return Ok(None);
            }
        };
        self.entries += 1;
        let id = entry_id(&key)?;
        match self.table.fetch(pager, id)? {
            Some(row) => Ok(Some(row)),
            None => Err(Error::Corrupt(format!(
                "an index entry for row {id}, which is gone"
            ))),
        }
    }
}
// @end

impl Database {
    // @listing check-index
    /// Checks an index against its table: the tree is sound, and it has exactly
    /// one entry for each row, under the key that row should have. Returns the
    /// number of pages in the index.
    pub(crate) fn check_index(
        &mut self,
        table: &Table,
        index: &Index,
    ) -> Result<usize> {
        let info = index.tree.check(&mut self.pager)?;
        let mut rows = 0;
        let mut scan = Scan::new(table, &mut self.pager)?;
        while let Some(row) = scan.next(&mut self.pager)? {
            let key = index.key(scan.row_id, &row);
            if index.tree.get(&mut self.pager, &key)?.is_none() {
                let what =
                    format!("row {} is not in the index {}", scan.row_id, index.name);
                return Err(Error::Corrupt(what));
            }
            rows += 1;
        }
        if info.entries != rows {
            let what = format!("{} entries for {rows} rows", info.entries);
            return Err(Error::Corrupt(format!(
                "the index {} has {what}",
                index.name
            )));
        }
        Ok(info.leaves + info.interior)
    }
    // @end
}
