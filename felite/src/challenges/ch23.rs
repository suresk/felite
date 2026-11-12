//! Chapter 23 challenges: reading values back out of a key, answering a query
//! from an index alone, dropping an index, descending columns, and a join that
//! looks its inner rows up in an index.

use crate::bind::Bound;
use crate::btree::{Cursor, row_id};
use crate::catalog::{Table, fold, sql_error};
use crate::db::Database;
use crate::error::{Error, Result};
use crate::eval::eval;
use crate::exec::{Operator, Row};
use crate::index::{Index, IndexScan};
use crate::keys::{after, encode_key};
use crate::pager::Pager;
use crate::record::Value;

// @listing decode-key
/// The values that a key was made from. An index entry's key gives the indexed
/// values and then, as an integer with no tag, the row id.
pub fn decode_key(mut key: &[u8]) -> Result<Vec<Value>> {
    let bad = || Error::Corrupt("an index key that can't be read".to_string());
    let mut values = Vec::new();
    while let Some((tag, rest)) = key.split_first() {
        key = rest;
        if *tag == 0 {
            values.push(Value::Null);
        } else if *tag == 1 || *tag == 2 {
            let (bytes, rest) = key.split_first_chunk::<8>().ok_or_else(bad)?;
            key = rest;
            let bits = u64::from_be_bytes(*bytes);
            values.push(match *tag {
                1 => Value::Integer((bits ^ (1 << 63)) as i64),
                // The top bit says which of the two changes was made.
                _ if bits >> 63 == 1 => Value::Real(f64::from_bits(bits ^ (1 << 63))),
                _ => Value::Real(f64::from_bits(!bits)),
            });
        } else if *tag == 3 {
            let mut text = Vec::new();
            loop {
                let (byte, rest) = key.split_first().ok_or_else(bad)?;
                key = rest;
                if *byte != 0 {
                    text.push(*byte);
                    continue;
                }
                // A zero is followed by 0xFF if it is part of the text, and by
                // another zero if it is the end.
                let (second, rest) = key.split_first().ok_or_else(bad)?;
                key = rest;
                match second {
                    0 => break,
                    0xFF => text.push(0),
                    _ => return Err(bad()),
                }
            }
            values.push(Value::Text(String::from_utf8(text).map_err(|_| bad())?));
        } else if key.len() == 7 {
            // Not a tag: the first byte of the row id that ends an entry's key.
            let mut id = vec![*tag];
            id.extend(key);
            values.push(Value::Integer(row_id(&id)?));
            key = &[];
        } else {
            return Err(bad());
        }
    }
    Ok(values)
}
// @end

// @listing index-only
/// The indexed values and row id of every entry of an index between two keys,
/// read from the index and nothing else.
pub fn index_only(
    db: &mut Database,
    index: &Index,
    from: &[u8],
    until: &[u8],
) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    let mut cursor = Cursor::seek(&index.tree, &mut db.pager, from)?;
    while let Some((key, _)) = cursor.next(&mut db.pager)? {
        if key.as_slice() >= until {
            break;
        }
        rows.push(decode_key(&key)?);
    }
    Ok(rows)
}
// @end

// @listing drop-index
/// Removes an index: its pages, its row in the schema table, and its place in
/// its table's list.
pub fn drop_index(db: &mut Database, name: &str) -> Result<()> {
    let found = db.tables.values().find_map(|table| table.index(name).cloned());
    let Some(index) = found else {
        return sql_error(format!("there is no index named {name}"));
    };
    index.tree.destroy(&mut db.pager)?;
    db.schema.delete_row(&mut db.pager, index.schema_row)?;
    if let Some(table) = db.tables.get_mut(&fold(&index.table)) {
        table.indexes.retain(|other| other.name != index.name);
    }
    db.pager.flush()
}
// @end

// @listing descending
/// A key in which some of the values sort backwards. `descending` says which.
pub fn encode_key_desc(values: &[Value], descending: &[bool]) -> Vec<u8> {
    let mut key = Vec::new();
    for (value, descending) in values.iter().zip(descending) {
        let mut part = encode_key(std::slice::from_ref(value));
        if *descending {
            part.iter_mut().for_each(|byte| *byte = !*byte);
        }
        key.extend(part);
    }
    key
}
// @end

// @listing index-loop
/// A nested loop join whose inner side is an index. For each outer row, it
/// reads only the inner rows whose indexed value equals the outer row's key.
pub struct IndexLoop {
    outer: Box<dyn Operator>,
    inner: Table,
    index: Index,
    /// The value to look up, worked out from an outer row.
    key: Bound,
    /// The outer row being worked on, and the scan of the index for it.
    current: Option<(Row, IndexScan)>,
    /// How many index entries have been read.
    pub entries: u64,
}

impl IndexLoop {
    pub fn new(
        outer: Box<dyn Operator>,
        inner: &Table,
        index: &Index,
        key: Bound,
    ) -> IndexLoop {
        let (inner, index) = (inner.clone(), index.clone());
        IndexLoop { outer, inner, index, key, current: None, entries: 0 }
    }
}
// @end

// @listing index-loop-next
impl Operator for IndexLoop {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        loop {
            if let Some((outer, scan)) = &mut self.current
                && let Some(inner) = scan.next(pager)?
            {
                self.entries += 1;
                let mut pair = outer.clone();
                pair.extend(inner);
                return Ok(Some(pair));
            }
            let Some(outer) = self.outer.next(pager)? else {
                return Ok(None);
            };
            // A NULL matches nothing. Go round for the next outer row.
            let from = match eval(&self.key, &outer)? {
                Value::Null => continue,
                value => encode_key(&[value]),
            };
            let until = Some(after(&from));
            let scan = IndexScan::new(&self.inner, &self.index, pager, &from, until)?;
            self.current = Some((outer, scan));
        }
    }
}
// @end
