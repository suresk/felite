//! Reading a tree in key order, one entry at a time.

use crate::btree::node::{INTERIOR, LEAF, parse_interior, parse_leaf, row_id};
use crate::btree::search::{MAX_DEPTH, Tree, child_for, find};
use crate::error::{Error, Result};
use crate::page::{Page, PageNo};
use crate::pager::Pager;

// @listing cursor
/// A position in a tree: a leaf, and the number of the next cell to read there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    leaf: PageNo,
    slot: usize,
    /// How many times the cursor has stepped from one leaf to the next.
    steps: u32,
}

/// A key and its payload, copied out of a leaf.
pub type Entry = (Vec<u8>, Vec<u8>);
// @end

// @listing descend
/// Walks from the root to a leaf. At each interior page, `choose` picks the cell
/// whose child to follow. Returns the leaf's page number.
fn descend(
    tree: &Tree,
    pager: &mut Pager,
    choose: impl Fn(&Page) -> Result<usize>,
) -> Result<PageNo> {
    let mut n = tree.root();
    for _ in 0..MAX_DEPTH {
        let page = pager.page(n)?;
        match page.kind() {
            LEAF => return Ok(n),
            INTERIOR if page.cell_count() > 0 => {
                let i = choose(page)?;
                (n, _) = parse_interior(page.cell(i)?)?;
            }
            _ => return Err(Error::Corrupt(format!("page {n} is not a tree page"))),
        }
    }
    Err(Error::Corrupt(format!("a tree more than {MAX_DEPTH} levels deep")))
}
// @end

impl Cursor {
    // @listing first
    /// A cursor at the first entry of the tree.
    pub fn first(tree: &Tree, pager: &mut Pager) -> Result<Cursor> {
        let leaf = descend(tree, pager, |_| Ok(0))?;
        Ok(Cursor { leaf, slot: 0, steps: 0 })
    }
    // @end

    // @listing seek
    /// A cursor at the first entry whose key is not less than `key`.
    pub fn seek(tree: &Tree, pager: &mut Pager, key: &[u8]) -> Result<Cursor> {
        let leaf = descend(tree, pager, |page| child_for(page, key))?;
        let (slot, _) = find(pager.page(leaf)?, key)?;
        Ok(Cursor { leaf, slot, steps: 0 })
    }
    // @end

    // @listing next
    /// The entry at the cursor, which then moves to the one after it. `None` once
    /// the entries have run out.
    pub fn next(&mut self, pager: &mut Pager) -> Result<Option<Entry>> {
        loop {
            let page = pager.page(self.leaf)?;
            if page.kind() != LEAF {
                let n = self.leaf;
                return Err(Error::Corrupt(format!(
                    "page {n} is in a chain of leaves"
                )));
            }
            if self.slot < page.cell_count() {
                let (key, payload) = parse_leaf(page.cell(self.slot)?)?;
                self.slot += 1;
                return Ok(Some((key.to_vec(), payload.to_vec())));
            }
            if page.right() == 0 {
                return Ok(None);
            }
            (self.leaf, self.slot) = (page.right(), 0);
            // A file has only so many pages. More steps than that, and the leaves
            // must link in a circle.
            self.steps += 1;
            if self.steps > pager.page_count()? {
                return Err(Error::Corrupt("the leaves link in a circle".into()));
            }
        }
    }
    // @end
}

impl Tree {
    // @listing range
    /// Every entry whose key is at least `low` and less than `high`, in key order.
    pub fn range(
        &self,
        pager: &mut Pager,
        low: &[u8],
        high: &[u8],
    ) -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        let mut cursor = Cursor::seek(self, pager, low)?;
        while let Some((key, payload)) = cursor.next(pager)? {
            if key.as_slice() >= high {
                break;
            }
            entries.push((key, payload));
        }
        Ok(entries)
    }
    // @end

    // @listing last-key
    /// The largest key in the tree, or `None` if the tree is empty.
    pub fn last_key(&self, pager: &mut Pager) -> Result<Option<Vec<u8>>> {
        let leaf = descend(self, pager, |page| Ok(page.cell_count() - 1))?;
        let page = pager.page(leaf)?;
        match page.cell_count() {
            0 => Ok(None),
            count => Ok(Some(parse_leaf(page.cell(count - 1)?)?.0.to_vec())),
        }
    }
    // @end

    // @listing next-row-id
    /// A row id for a new row in a table: one more than the largest in use, or 1
    /// for an empty table.
    pub fn next_row_id(&self, pager: &mut Pager) -> Result<i64> {
        match self.last_key(pager)? {
            None => Ok(1),
            Some(key) => row_id(&key)?.checked_add(1).ok_or(Error::OutOfRowIds),
        }
    }
    // @end
}
