//! Chapter 5 challenges: insert-or-replace, counting entries along the leaves, and
//! building a tree from sorted entries.

use crate::btree::search::MAX_DEPTH;
use crate::btree::{
    INTERIOR, LEAF, MAX_CELL, Tree, interior_cell, leaf_cell, parse_interior,
};
use crate::challenges::ch04::path;
use crate::error::{Error, Result};
use crate::page::{Page, PageNo};
use crate::pager::Pager;

// @listing put
/// Stores `payload` under `key`, replacing what was there. Returns whether the key
/// was already present.
pub fn put(
    tree: &Tree,
    pager: &mut Pager,
    key: &[u8],
    payload: &[u8],
) -> Result<bool> {
    let size = leaf_cell(key, payload).len();
    if size > MAX_CELL {
        return Err(Error::TooLarge(size)); // before anything is removed
    }
    let steps = path(tree, pager, key)?;
    let (leaf, i) = steps[steps.len() - 1];
    let page = pager.page(leaf)?;
    let found =
        i < page.cell_count() && page.cell(i)?.starts_with(&leaf_cell(key, b""));
    if found {
        pager.page_mut(leaf)?.remove_cell(i)?;
    }
    tree.insert(pager, key, payload)?;
    Ok(found)
}
// @end

// @listing count
/// The number of entries in the tree, counted by walking along the leaves.
pub fn count(tree: &Tree, pager: &mut Pager) -> Result<usize> {
    // Down the left edge to the first leaf, no deeper than any tree can be.
    let mut n = tree.root();
    let mut depth = 1;
    while pager.page(n)?.kind() == INTERIOR {
        if depth == MAX_DEPTH {
            let message = format!("a tree more than {MAX_DEPTH} levels deep");
            return Err(Error::Corrupt(message));
        }
        (n, _) = parse_interior(pager.page(n)?.cell(0)?)?;
        depth += 1;
    }
    // Then from leaf to leaf. A file has only so many pages, so a walk longer than
    // that is following a cycle.
    let mut total = 0;
    for _ in 0..pager.page_count()? {
        let page = pager.page(n)?;
        if page.kind() != LEAF {
            return Err(Error::Corrupt(format!("page {n} is not a leaf")));
        }
        total += page.cell_count();
        n = page.right();
        if n == 0 {
            return Ok(total);
        }
    }
    Err(Error::Corrupt("the leaves link in a cycle".into()))
}
// @end

// @listing pack
/// Packs cells into full pages of one kind. Each cell comes with its key. Returns
/// each page with the lowest key on it.
fn pack(kind: u8, cells: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<(Vec<u8>, Page)>> {
    let mut pages = Vec::new();
    let mut page = Page::new();
    page.init_slotted(kind);
    let mut lowest = Vec::new();
    for (key, cell) in cells {
        if !page.insert_cell(page.cell_count(), cell)? {
            pages.push((std::mem::take(&mut lowest), page));
            page = Page::new();
            page.init_slotted(kind);
            if !page.insert_cell(0, cell)? {
                return Err(Error::TooLarge(cell.len())); // not even on an empty page
            }
        }
        if page.cell_count() == 1 {
            lowest = key.clone();
        }
    }
    pages.push((lowest, page));
    Ok(pages)
}
// @end

// @listing bulk-load
/// Builds a tree from entries that are already in key order, one level at a time
/// from the leaves up.
pub fn bulk_load(pager: &mut Pager, entries: &[(Vec<u8>, Vec<u8>)]) -> Result<Tree> {
    assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0), "keys must ascend");
    let mut cells = Vec::with_capacity(entries.len());
    for (key, payload) in entries {
        let cell = leaf_cell(key, payload);
        if cell.len() > MAX_CELL {
            return Err(Error::TooLarge(cell.len())); // the limit `insert` applies
        }
        cells.push((key.clone(), cell));
    }
    let tree = Tree::create(pager)?;
    let mut kind = LEAF;
    loop {
        let mut pages = pack(kind, &cells)?;
        if pages.len() == 1 {
            // The top level is one page, and it becomes the root.
            *pager.page_mut(tree.root())? = pages.remove(0).1;
            return Ok(tree);
        }
        // `allocate` may hand out any free page, so each page's number is known only
        // once it is allocated, and its left neighbor's link is set then.
        cells.clear();
        let mut previous: Option<PageNo> = None;
        for (key, page) in pages {
            let n = pager.allocate(page)?;
            if let Some(left) = previous {
                pager.page_mut(left)?.set_right(n);
            }
            previous = Some(n);
            let cell = interior_cell(n, &key);
            cells.push((key, cell));
        }
        kind = INTERIOR;
    }
}
// @end
