//! Removing a key, and giving back the pages that are no longer needed.

use crate::btree::insert::page_of;
use crate::btree::node::{INTERIOR, LEAF, interior_cell, parse_interior};
use crate::btree::search::{MAX_DEPTH, Tree, child_for, find};
use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::pager::Pager;
use crate::slotted::{SLOT_SIZE, SLOTTED_HEADER};

// @listing merge-below
/// A page with fewer bytes in use than this is merged into a neighbor, if the two
/// fit in one page. It is a quarter of the usable space.
const MERGE_BELOW: usize = (PAGE_SIZE - SLOTTED_HEADER) / 4;
// @end

// @listing cells-of
/// The cells of a tree page, copied out. If `first_key` is given and the page is
/// an interior page, its first cell is rebuilt with that key.
fn cells_of(page: &Page, first_key: Option<&[u8]>) -> Result<Vec<Vec<u8>>> {
    let mut cells = Vec::with_capacity(page.cell_count());
    for i in 0..page.cell_count() {
        cells.push(page.cell(i)?.to_vec());
    }
    if let (INTERIOR, Some(key), Some(first)) =
        (page.kind(), first_key, cells.first_mut())
    {
        let (child, _) = parse_interior(first)?;
        *first = interior_cell(child, key);
    }
    Ok(cells)
}
// @end

// @listing merge
/// Child `i` of interior page `n` has just lost a cell. If it is now small, and it
/// and a neighbor fit in one page, the two become one and the parent loses a cell.
fn merge_if_small(pager: &mut Pager, n: PageNo, i: usize) -> Result<()> {
    let parent = pager.page(n)?;
    if parent.cell_count() < 2 {
        return Ok(()); // an only child has no neighbor to merge with
    }
    // The pair to merge: the child and the page on its left, or on its right if
    // the child is the first.
    let j = i.saturating_sub(1);
    let (small, _) = parse_interior(parent.cell(i)?)?;
    let (left_no, _) = parse_interior(parent.cell(j)?)?;
    let (right_no, separator) = parse_interior(parent.cell(j + 1)?)?;
    let separator = separator.to_vec();

    let used = PAGE_SIZE - SLOTTED_HEADER - pager.page(small)?.free_space()?;
    if used >= MERGE_BELOW {
        return Ok(());
    }
    let left = pager.page(left_no)?.clone();
    let right = pager.page(right_no)?.clone();
    if left.kind() != right.kind() {
        return Err(Error::Corrupt(format!(
            "pages {left_no} and {right_no} differ in kind"
        )));
    }
    let mut cells = cells_of(&left, None)?;
    cells.extend(cells_of(&right, Some(&separator))?);
    let size: usize = cells.iter().map(|cell| cell.len() + SLOT_SIZE).sum();
    if size > PAGE_SIZE - SLOTTED_HEADER {
        return Ok(()); // they do not fit, and we leave both as they are
    }
    let mut merged = page_of(left.kind(), &cells)?;
    merged.set_right(right.right());
    *pager.page_mut(left_no)? = merged;
    pager.page_mut(n)?.remove_cell(j + 1)?;
    pager.free(right_no)
}
// @end

// @listing delete-from
/// Removes `key` from the subtree at page `n`. Returns whether it was there.
fn delete_from(
    pager: &mut Pager,
    n: PageNo,
    key: &[u8],
    depth: usize,
) -> Result<bool> {
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
                pager.page_mut(n)?.remove_cell(i)?;
            }
            Ok(found)
        }
        INTERIOR => {
            let i = child_for(page, key)?;
            let (child, _) = parse_interior(page.cell(i)?)?;
            let found = delete_from(pager, child, key, depth + 1)?;
            if found {
                merge_if_small(pager, n, i)?;
            }
            Ok(found)
        }
        kind => Err(Error::Corrupt(format!("page {n} has kind {kind} in a tree"))),
    }
}
// @end

// @listing free-subtree
/// Frees page `n` and every page below it.
fn free_subtree(pager: &mut Pager, n: PageNo, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::Corrupt(format!(
            "a tree more than {MAX_DEPTH} levels deep"
        )));
    }
    let page = pager.page(n)?.clone();
    match page.kind() {
        LEAF => {}
        INTERIOR => {
            for i in 0..page.cell_count() {
                let (child, _) = parse_interior(page.cell(i)?)?;
                free_subtree(pager, child, depth + 1)?;
            }
        }
        kind => {
            return Err(Error::Corrupt(format!(
                "page {n} has kind {kind} in a tree"
            )));
        }
    }
    pager.free(n)
}
// @end

impl Tree {
    // @listing delete
    /// Removes `key` and its payload. Returns whether the key was in the tree.
    pub fn delete(&self, pager: &mut Pager, key: &[u8]) -> Result<bool> {
        if !delete_from(pager, self.root(), key, 1)? {
            return Ok(false);
        }
        // A root with one child is a level the tree no longer needs. The child's
        // contents move up into the root page, and the child's page is freed.
        for _ in 0..MAX_DEPTH {
            let root = pager.page(self.root())?;
            if root.kind() != INTERIOR || root.cell_count() != 1 {
                break;
            }
            let (child, _) = parse_interior(root.cell(0)?)?;
            if child == self.root() {
                return Err(Error::Corrupt("a root that is its own child".into()));
            }
            let contents = pager.page(child)?.clone();
            *pager.page_mut(self.root())? = contents;
            pager.free(child)?;
        }
        Ok(true)
    }
    // @end

    // @listing destroy
    /// Frees every page of the tree, the root included. The tree must not be used
    /// afterwards.
    pub fn destroy(self, pager: &mut Pager) -> Result<()> {
        free_subtree(pager, self.root(), 1)
    }
    // @end
}
