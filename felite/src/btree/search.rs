//! Finding a key.

use crate::btree::node::{INTERIOR, LEAF, parse_interior, parse_leaf};
use crate::error::{Error, Result};
use crate::page::{Page, PageNo};
use crate::pager::Pager;

// @listing find
/// Where `key` is in a leaf, or where it would go: the number of the first cell
/// whose key is not less than `key`, and whether that cell's key equals it.
pub(crate) fn find(page: &Page, key: &[u8]) -> Result<(usize, bool)> {
    let (mut low, mut high) = (0, page.cell_count());
    while low < high {
        let mid = low + (high - low) / 2;
        let (mid_key, _) = parse_leaf(page.cell(mid)?)?;
        if mid_key < key {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let found = low < page.cell_count() && parse_leaf(page.cell(low)?)?.0 == key;
    Ok((low, found))
}
// @end

// @listing child-for
/// The number of the cell in an interior page whose child could hold `key`: the
/// last cell whose key is not greater than `key`, or cell 0 if there is none.
pub(crate) fn child_for(page: &Page, key: &[u8]) -> Result<usize> {
    if page.cell_count() == 0 {
        return Err(Error::Corrupt("an interior page with no cells".into()));
    }
    let (mut low, mut high) = (1, page.cell_count());
    while low < high {
        let mid = low + (high - low) / 2;
        let (_, mid_key) = parse_interior(page.cell(mid)?)?;
        if mid_key <= key {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    Ok(low - 1)
}
// @end

// @listing tree
/// The deepest tree we will follow. A healthy tree in a file of 2^32 pages cannot
/// be this deep, so reaching the limit means the pages point in a circle.
pub(crate) const MAX_DEPTH: usize = 40;

/// A B+tree, known by the page number of its root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tree {
    root: PageNo,
}
// @end

impl Tree {
    // @listing tree-open
    /// The tree whose root is page `root`.
    pub fn open(root: PageNo) -> Tree {
        Tree { root }
    }

    pub fn root(&self) -> PageNo {
        self.root
    }
    // @end

    // @listing get
    /// The payload stored under `key`, if there is one.
    pub fn get(&self, pager: &mut Pager, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut n = self.root;
        for _ in 0..MAX_DEPTH {
            let page = pager.page(n)?;
            match page.kind() {
                INTERIOR => {
                    let i = child_for(page, key)?;
                    (n, _) = parse_interior(page.cell(i)?)?;
                }
                LEAF => {
                    let (i, found) = find(page, key)?;
                    if !found {
                        return Ok(None);
                    }
                    let (_, payload) = parse_leaf(page.cell(i)?)?;
                    return Ok(Some(payload.to_vec()));
                }
                kind => {
                    let what =
                        format!("page {n} has kind {kind} and is not part of a tree");
                    return Err(Error::Corrupt(what));
                }
            }
        }
        Err(Error::Corrupt(format!("a tree more than {MAX_DEPTH} levels deep")))
    }
    // @end

    // @listing height
    /// The number of levels, counting the root and the leaves.
    pub fn height(&self, pager: &mut Pager) -> Result<usize> {
        let mut n = self.root;
        for level in 1..=MAX_DEPTH {
            let page = pager.page(n)?;
            match page.kind() {
                LEAF => return Ok(level),
                INTERIOR if page.cell_count() > 0 => {
                    (n, _) = parse_interior(page.cell(0)?)?;
                }
                _ => {
                    return Err(Error::Corrupt(format!(
                        "page {n} is not a tree page"
                    )));
                }
            }
        }
        Err(Error::Corrupt(format!("a tree more than {MAX_DEPTH} levels deep")))
    }
    // @end
}
