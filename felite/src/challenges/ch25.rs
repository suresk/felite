//! Chapter 25 challenges: writing a database out as SQL, and reading a table
//! from a file that SQLite made.

use crate::db::Database;
use crate::error::{Error, Result};
use crate::exec::{Operator, Row, Scan};
use crate::record::Value;
use crate::sql::{CreateTable, Expr, ExprKind, Span, Statement};

// @listing dump-sql
/// The statements that would rebuild a database: each table, its rows, and
/// then its indexes.
pub fn dump_sql(db: &mut Database) -> Result<String> {
    let span = Span { offset: 0, line: 1, column: 1 };
    // `Display for Expr` already writes a value as SQL would have it.
    let literal =
        |value: Value| Expr { kind: ExprKind::Literal(value), span }.to_string();
    let mut out = String::new();
    let tables: Vec<_> = db.tables().cloned().collect();
    for table in tables {
        let (name, columns) = (table.name.clone(), table.columns.clone());
        out +=
            &format!("{};\n", Statement::CreateTable(CreateTable { name, columns }));
        let mut scan = Scan::new(&table, &mut db.pager)?;
        while let Some(row) = scan.next(&mut db.pager)? {
            let values: Vec<String> = row.into_iter().map(literal).collect();
            out += &format!(
                "INSERT INTO {} VALUES ({});\n",
                table.name,
                values.join(", ")
            );
        }
        for index in &table.indexes {
            out += &format!("{};\n", index.sql());
        }
    }
    Ok(out)
}
// @end

// @listing sqlite-varint
fn bad<T>(what: &str) -> Result<T> {
    Err(Error::Corrupt(format!("SQLite file: {what}")))
}

/// Reads one of SQLite's variable-length integers from the front of `bytes`:
/// seven bits to a byte, most significant first, and all eight bits of a
/// ninth byte. Returns the number and how many bytes it took.
fn sqlite_varint(bytes: &[u8]) -> Result<(i64, usize)> {
    let mut value: u64 = 0;
    for (i, byte) in bytes.iter().take(9).enumerate() {
        if i == 8 {
            return Ok((((value << 8) | *byte as u64) as i64, 9));
        }
        value = (value << 7) | (*byte & 0x7f) as u64;
        if *byte & 0x80 == 0 {
            return Ok((value as i64, i + 1));
        }
    }
    bad("a number that runs off the end")
}
// @end

// @listing sqlite-record
/// Decodes one of SQLite's records: a header of serial types, then the values.
fn sqlite_record(payload: &[u8]) -> Result<Row> {
    let (header_size, mut at) = sqlite_varint(payload)?;
    let mut body = header_size as usize;
    let mut row = Vec::new();
    while at < header_size as usize {
        let (serial, used) = sqlite_varint(payload.get(at..).unwrap_or(&[]))?;
        at += used;
        // How many bytes of the body this value has.
        let size = match serial {
            0 | 8 | 9 => 0,
            1..=4 => serial as usize,
            5 => 6,
            6 | 7 => 8,
            n if n >= 12 => (n as usize - 12) / 2,
            _ => return bad("a serial type that means nothing"),
        };
        let Some(bytes) = payload.get(body..body + size) else {
            return bad("a record shorter than its header says");
        };
        body += size;
        row.push(match serial {
            0 => Value::Null,
            8 => Value::Integer(0),
            9 => Value::Integer(1),
            7 => Value::Real(f64::from_bits(
                bytes.iter().fold(0, |n, b| (n << 8) | *b as u64),
            )),
            1..=6 => {
                // A signed integer of 1, 2, 3, 4, 6 or 8 bytes, big-endian.
                let sign = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
                Value::Integer(bytes.iter().fold(sign, |n, b| (n << 8) | *b as i64))
            }
            n if n % 2 == 1 => match std::str::from_utf8(bytes) {
                Ok(text) => Value::Text(text.to_string()),
                Err(_) => return bad("text that is not UTF-8"),
            },
            _ => return bad("a blob, which felite has no type for"),
        });
    }
    Ok(row)
}
// @end

// @listing sqlite-tree
/// A SQLite file, and what the walk of a table needs to know about it.
struct SqliteFile<'a> {
    bytes: &'a [u8],
    page_size: usize,
}

impl SqliteFile<'_> {
    /// Collects the rows of the table b-tree rooted at page `n`, with their
    /// row ids, in order.
    fn rows(
        &self,
        n: usize,
        depth: usize,
        found: &mut Vec<(i64, Row)>,
    ) -> Result<()> {
        if n == 0 || self.page_size < 512 {
            return bad("a page number or a page size of nothing");
        }
        let start = (n - 1) * self.page_size;
        let Some(page) = self.bytes.get(start..start + self.page_size) else {
            return bad("a page past the end of the file");
        };
        if depth > 20 {
            return bad("a tree deeper than any real one");
        }
        // Page 1 begins with the file's 100-byte header.
        let header = if n == 1 { 100 } else { 0 };
        let u16_at =
            |at: usize| u16::from_be_bytes([page[at], page[at + 1]]) as usize;
        let u32_at = |at: usize| {
            u32::from_be_bytes([page[at], page[at + 1], page[at + 2], page[at + 3]])
                as usize
        };
        let (kind, cells) = (page[header], u16_at(header + 3));
        let pointers = header + if kind == 5 { 12 } else { 8 };
        if pointers + 2 * cells > page.len() {
            return bad("more cells than a page can hold");
        }
        for i in 0..cells {
            let Some(cell) = page.get(u16_at(pointers + 2 * i)..) else {
                return bad("a cell outside its page");
            };
            match kind {
                // An interior page: a child, and the largest row id in it.
                5 if cell.len() >= 4 => {
                    let child =
                        u32::from_be_bytes([cell[0], cell[1], cell[2], cell[3]]);
                    self.rows(child as usize, depth + 1, found)?
                }
                // A leaf: the payload's size, the row id, and the payload.
                13 => {
                    let (size, a) = sqlite_varint(cell)?;
                    let (id, b) = sqlite_varint(&cell[a..])?;
                    let Some(payload) = cell.get(a + b..a + b + size as usize) else {
                        return bad("a row that spills onto overflow pages");
                    };
                    found.push((id, sqlite_record(payload)?));
                }
                _ => return bad("a page that is no part of a table"),
            }
        }
        if kind == 5 {
            self.rows(u32_at(header + 8), depth + 1, found)?;
        }
        Ok(())
    }
}
// @end

// @listing sqlite-table
/// The rows of the first table in a file made by SQLite, with their row ids.
pub fn sqlite_first_table(bytes: &[u8]) -> Result<Vec<(i64, Row)>> {
    if bytes.get(..16) != Some(b"SQLite format 3\0") {
        return bad("the first sixteen bytes are not SQLite's");
    }
    let page_size = match u16::from_be_bytes([bytes[16], bytes[17]]) {
        1 => 65536,
        size => size as usize,
    };
    let file = SqliteFile { bytes, page_size };
    // Page 1 is the schema table: type, name, table name, root page, SQL.
    let mut schema = Vec::new();
    file.rows(1, 0, &mut schema)?;
    for (_, row) in schema {
        if let [Value::Text(kind), _, _, Value::Integer(root), _] = row.as_slice()
            && kind == "table"
        {
            let mut found = Vec::new();
            file.rows(*root as usize, 0, &mut found)?;
            return Ok(found);
        }
    }
    bad("there is no table in it")
}
// @end
