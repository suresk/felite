//! Verifying a whole database.

use crate::btree::Cursor;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::overflow::{SPILLED, chain, load};
use crate::record::decode;

impl Database {
    // @listing database-check
    /// Checks everything in the file: the free list, the schema table, every
    /// table's tree, every row, and that each page belongs to one of them.
    pub fn check(&mut self) -> Result<()> {
        // The header page, and the pages waiting to be used again.
        let mut pages = 1 + self.pager.check_free_list()? as usize;
        let mut trees = vec![(self.schema, 4)];
        for table in self.tables.values() {
            trees.push((table.tree, table.columns.len()));
        }
        for (tree, columns) in trees {
            let info = tree.check(&mut self.pager)?;
            pages += info.leaves + info.interior;
            let mut cursor = Cursor::first(&tree, &mut self.pager)?;
            while let Some((_, stored)) = cursor.next(&mut self.pager)? {
                if stored.len() == SPILLED {
                    pages += chain(&mut self.pager, &stored)?.1.len();
                }
                let row = decode(&load(&mut self.pager, stored)?)?;
                if row.len() != columns {
                    let what = format!("a row of {} values", row.len());
                    return Err(Error::Corrupt(what));
                }
            }
        }
        let count = self.pager.page_count()? as usize;
        if pages != count {
            let what = format!("{pages} pages are accounted for, of {count}");
            return Err(Error::Corrupt(what));
        }
        Ok(())
    }
    // @end
}
