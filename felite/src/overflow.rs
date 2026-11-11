//! Rows too large for a tree cell: the first part stays in the tree, and the rest
//! goes to a chain of pages of its own.

use crate::btree::{Cursor, MAX_CELL, Tree, row_key};
use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::pager::Pager;

// @listing layout
/// The kind of a page that holds part of a large row.
pub const OVERFLOW: u8 = 4;
/// Where an overflow page keeps the number of the next page in its chain.
const NEXT_AT: usize = 4;
/// Where its data starts, and how much of it there is room for.
const DATA_AT: usize = 8;
const DATA_SIZE: usize = PAGE_SIZE - DATA_AT;

/// The length of what the tree stores for a row that has spilled. With a row id
/// for a key, it makes a cell of exactly `MAX_CELL` bytes.
pub const SPILLED: usize = MAX_CELL - 9;
/// How much of a spilled row stays in the tree. The other eight bytes are the
/// row's full length and the number of its first overflow page.
const KEPT: usize = SPILLED - 8;
// @end

// @listing store
/// Turns a row into what the tree should store for it. A short row is stored as
/// it is. A long one keeps its first bytes and moves the rest to overflow pages.
pub fn store(pager: &mut Pager, row: &[u8]) -> Result<Vec<u8>> {
    if row.len() < SPILLED {
        return Ok(row.to_vec());
    }
    let Ok(length) = u32::try_from(row.len()) else {
        return Err(Error::TooLarge(row.len()));
    };
    // Written from the last page back, so that each page can name the one after.
    let mut next: PageNo = 0;
    for chunk in row[KEPT..].chunks(DATA_SIZE).rev() {
        let mut page = Page::new();
        page.bytes_mut()[0] = OVERFLOW;
        page.put_u32(NEXT_AT, next);
        page.bytes_mut()[DATA_AT..DATA_AT + chunk.len()].copy_from_slice(chunk);
        next = pager.allocate(page)?;
    }
    let mut stored = row[..KEPT].to_vec();
    stored.extend_from_slice(&length.to_be_bytes());
    stored.extend_from_slice(&next.to_be_bytes());
    Ok(stored)
}
// @end

// @listing chain
/// The full length of a spilled row and the pages of its chain, in order, checked
/// as far as they can be without reading the data.
pub(crate) fn chain(
    pager: &mut Pager,
    stored: &[u8],
) -> Result<(usize, Vec<PageNo>)> {
    let tail = &stored[KEPT..];
    let length = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) as usize;
    let mut next = PageNo::from_be_bytes([tail[4], tail[5], tail[6], tail[7]]);
    // A row this long has this many overflow pages. The file must be able to hold
    // them, or the length is not to be believed.
    let needed = length.saturating_sub(KEPT).div_ceil(DATA_SIZE);
    if length < SPILLED || needed > pager.page_count()? as usize {
        return Err(Error::Corrupt(format!("a spilled row of {length} bytes")));
    }
    let mut pages = Vec::with_capacity(needed);
    for _ in 0..needed {
        let page = pager.page(next)?;
        if page.kind() != OVERFLOW {
            return Err(Error::Corrupt(format!(
                "page {next} is not an overflow page"
            )));
        }
        pages.push(next);
        next = page.u32_at(NEXT_AT);
    }
    if next != 0 {
        return Err(Error::Corrupt(format!(
            "an overflow chain runs on to page {next}"
        )));
    }
    Ok((length, pages))
}
// @end

// @listing load
/// The row that `stored` stands for. The reverse of `store`.
pub fn load(pager: &mut Pager, stored: Vec<u8>) -> Result<Vec<u8>> {
    if stored.len() != SPILLED {
        return Ok(stored);
    }
    let (length, pages) = chain(pager, &stored)?;
    let mut row = Vec::with_capacity(length);
    row.extend_from_slice(&stored[..KEPT]);
    for n in pages {
        let wanted = (length - row.len()).min(DATA_SIZE);
        row.extend_from_slice(&pager.page(n)?.bytes()[DATA_AT..DATA_AT + wanted]);
    }
    Ok(row)
}
// @end

// @listing release
/// Frees the overflow pages of a stored row, if it has any.
pub fn release(pager: &mut Pager, stored: &[u8]) -> Result<()> {
    if stored.len() != SPILLED {
        return Ok(());
    }
    for n in chain(pager, stored)?.1 {
        pager.free(n)?;
    }
    Ok(())
}
// @end

impl Tree {
    // @listing insert-row
    /// Adds a row of any length to a table, under its row id.
    pub fn insert_row(&self, pager: &mut Pager, id: i64, row: &[u8]) -> Result<()> {
        let stored = store(pager, row)?;
        let result = self.insert(pager, &row_key(id), &stored);
        if result.is_err() {
            // The row did not go in, so nothing points at its overflow pages.
            release(pager, &stored)?;
        }
        result
    }
    // @end

    // @listing get-row
    /// The row with this id, whole, if there is one.
    pub fn get_row(&self, pager: &mut Pager, id: i64) -> Result<Option<Vec<u8>>> {
        match self.get(pager, &row_key(id))? {
            Some(stored) => load(pager, stored).map(Some),
            None => Ok(None),
        }
    }
    // @end

    // @listing delete-row
    /// Removes the row with this id and frees its overflow pages. Returns whether
    /// there was such a row.
    pub fn delete_row(&self, pager: &mut Pager, id: i64) -> Result<bool> {
        let Some(stored) = self.get(pager, &row_key(id))? else {
            return Ok(false);
        };
        self.delete(pager, &row_key(id))?;
        release(pager, &stored)?;
        Ok(true)
    }
    // @end

    // @listing destroy-rows
    /// Frees every page of a table: its tree and the overflow pages of its rows.
    pub fn destroy_rows(self, pager: &mut Pager) -> Result<()> {
        let mut cursor = Cursor::first(&self, pager)?;
        while let Some((_, stored)) = cursor.next(pager)? {
            release(pager, &stored)?;
        }
        self.destroy(pager)
    }
    // @end
}
