//! Adding a key: into a leaf if there is room, and by splitting pages if there is not.

use crate::btree::node::{INTERIOR, LEAF, interior_cell, leaf_cell};
use crate::btree::node::{parse_interior, parse_leaf};
use crate::btree::search::{MAX_DEPTH, Tree, child_for, find};
use crate::error::{Error, Result};
use crate::page::{Page, PageNo};
use crate::pager::Pager;
use crate::slotted::SLOT_SIZE;

// @listing max-cell
/// The longest leaf cell the tree will store. A page has room for four cells of
/// this size, so a split always has cells to put on both sides.
pub const MAX_CELL: usize = 1000;
// @end

// @listing split-point
/// Where to cut a list of cells in two so that the halves are as close in bytes as
/// they can be. Returns the number of cells that stay on the left, at least one
/// and not all.
fn split_point(cells: &[Vec<u8>]) -> usize {
    let cost = |cell: &Vec<u8>| cell.len() + SLOT_SIZE;
    let total: usize = cells.iter().map(cost).sum();
    let mut left = 0;
    for (i, cell) in cells.iter().enumerate() {
        if left + cost(cell) / 2 >= total / 2 {
            return i.clamp(1, cells.len() - 1);
        }
        left += cost(cell);
    }
    cells.len() - 1
}
// @end

// @listing key-of
/// The key of a cell from a page of the given kind.
fn key_of(kind: u8, cell: &[u8]) -> Result<&[u8]> {
    if kind == LEAF { Ok(parse_leaf(cell)?.0) } else { Ok(parse_interior(cell)?.1) }
}
// @end

// @listing page-of
/// A new page of the given kind holding the given cells, in order.
pub(crate) fn page_of(kind: u8, cells: &[Vec<u8>]) -> Result<Page> {
    let mut page = Page::new();
    page.init_slotted(kind);
    for (i, cell) in cells.iter().enumerate() {
        if !page.insert_cell(i, cell)? {
            return Err(Error::Corrupt("half of a split page does not fit".into()));
        }
    }
    Ok(page)
}
// @end

// @listing split
/// Puts `cell` at position `i` of page `n`, which has no room for it, by moving
/// the upper part of the page's cells to a new page. Returns the lowest key on the
/// new page and its number, which the caller must add to the parent.
fn split(
    pager: &mut Pager,
    n: PageNo,
    i: usize,
    cell: Vec<u8>,
) -> Result<(Vec<u8>, PageNo)> {
    let old = pager.page(n)?.clone();
    let mut cells = Vec::with_capacity(old.cell_count() + 1);
    for j in 0..old.cell_count() {
        cells.push(old.cell(j)?.to_vec());
    }
    cells.insert(i, cell);
    // A cell added at the very end of the tree's last page starts a page of its
    // own, so that keys arriving in order leave full pages behind them.
    let appending = old.right() == 0 && i == cells.len() - 1;
    let at = if appending { cells.len() - 1 } else { split_point(&cells) };

    let separator = key_of(old.kind(), &cells[at])?.to_vec();
    let mut right = page_of(old.kind(), &cells[at..])?;
    right.set_right(old.right());
    let new = pager.allocate(right)?;
    let mut left = page_of(old.kind(), &cells[..at])?;
    left.set_right(new);
    *pager.page_mut(n)? = left;
    Ok((separator, new))
}
// @end

// @listing add-cell
/// Puts `cell` at position `i` of page `n`, splitting the page if it is full.
/// Returns what `split` returns if there was a split.
fn add_cell(
    pager: &mut Pager,
    n: PageNo,
    i: usize,
    cell: Vec<u8>,
) -> Result<Option<(Vec<u8>, PageNo)>> {
    if pager.page_mut(n)?.insert_cell(i, &cell)? {
        return Ok(None);
    }
    split(pager, n, i, cell).map(Some)
}
// @end

// @listing insert-into
/// Inserts into the subtree whose root is page `n`. If that page had to split,
/// returns the lowest key and the number of its new right-hand neighbor.
fn insert_into(
    pager: &mut Pager,
    n: PageNo,
    key: &[u8],
    payload: &[u8],
    depth: usize,
) -> Result<Option<(Vec<u8>, PageNo)>> {
    if depth > MAX_DEPTH {
        return Err(Error::Corrupt(format!(
            "a tree more than {MAX_DEPTH} levels deep"
        )));
    }
    let page = pager.page(n)?;
    match page.kind() {
        LEAF => {
            let (i, found) = find(page, key)?;
            if found {
                return Err(Error::DuplicateKey);
            }
            add_cell(pager, n, i, leaf_cell(key, payload))
        }
        INTERIOR => {
            let i = child_for(page, key)?;
            let (child, _) = parse_interior(page.cell(i)?)?;
            match insert_into(pager, child, key, payload, depth + 1)? {
                None => Ok(None),
                Some((separator, new)) => {
                    add_cell(pager, n, i + 1, interior_cell(new, &separator))
                }
            }
        }
        kind => Err(Error::Corrupt(format!("page {n} has kind {kind} in a tree"))),
    }
}
// @end

impl Tree {
    // @listing create
    /// A new, empty tree: one leaf with no cells.
    pub fn create(pager: &mut Pager) -> Result<Tree> {
        let mut root = Page::new();
        root.init_slotted(LEAF);
        Ok(Tree::open(pager.allocate(root)?))
    }
    // @end

    // @listing insert
    /// Adds a key and its payload. The key must not be in the tree already.
    pub fn insert(
        &self,
        pager: &mut Pager,
        key: &[u8],
        payload: &[u8],
    ) -> Result<()> {
        let size = leaf_cell(key, payload).len();
        if size > MAX_CELL {
            return Err(Error::TooLarge(size));
        }
        let Some((separator, new)) =
            insert_into(pager, self.root(), key, payload, 1)?
        else {
            return Ok(());
        };
        // The root split. Its lower half moves to a new page, and the root becomes
        // an interior page with two children, so its page number never changes.
        let left = pager.page(self.root())?.clone();
        let first = key_of(left.kind(), left.cell(0)?)?.to_vec();
        let left = pager.allocate(left)?;
        let cells = [interior_cell(left, &first), interior_cell(new, &separator)];
        *pager.page_mut(self.root())? = page_of(INTERIOR, &cells)?;
        Ok(())
    }
    // @end
}
