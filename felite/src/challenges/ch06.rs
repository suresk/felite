//! Chapter 6 challenges: counting a range without copying, a cursor that survives
//! changes to the tree, scanning by prefix, and a cursor that runs backward.

use crate::btree::search::{MAX_DEPTH, child_for, find};
use crate::btree::{Cursor, Entry, INTERIOR, LEAF, Tree, parse_interior, parse_leaf};
use crate::error::{Error, Result};
use crate::page::PageNo;
use crate::pager::Pager;

// @listing count-range
/// How many keys are at least `low` and less than `high`. No key or payload is
/// copied out of a page.
pub fn count_range(
    tree: &Tree,
    pager: &mut Pager,
    low: &[u8],
    high: &[u8],
) -> Result<usize> {
    // Down to the leaf where `low` belongs.
    let mut n = tree.root();
    for _ in 0..MAX_DEPTH {
        let page = pager.page(n)?;
        if page.kind() != INTERIOR {
            break;
        }
        (n, _) = parse_interior(page.cell(child_for(page, low)?)?)?;
    }
    // Along the leaves. On each one, two binary searches give the positions of
    // `low` and `high`, and the keys between them are in range.
    let mut total = 0;
    for _ in 0..pager.page_count()? {
        let page = pager.page(n)?;
        if page.kind() != LEAF {
            return Err(Error::Corrupt(format!("page {n} is not a leaf")));
        }
        let (from, _) = find(page, low)?;
        let (to, _) = find(page, high)?;
        total += to.saturating_sub(from);
        if to < page.cell_count() || page.right() == 0 {
            return Ok(total);
        }
        n = page.right();
    }
    Err(Error::Corrupt("the leaves link in a circle".into()))
}
// @end

// @listing stable-cursor
/// A cursor that remembers the last key it returned, not a position in a page, so
/// it stays correct when the tree changes between calls.
#[derive(Debug, Default)]
pub struct StableCursor {
    last: Option<Vec<u8>>,
}

impl StableCursor {
    pub fn new() -> StableCursor {
        StableCursor { last: None }
    }

    /// The first entry after the last one returned.
    pub fn next(&mut self, tree: &Tree, pager: &mut Pager) -> Result<Option<Entry>> {
        let mut cursor = match &self.last {
            None => Cursor::first(tree, pager)?,
            Some(key) => Cursor::seek(tree, pager, key)?,
        };
        let mut entry = cursor.next(pager)?;
        // The seek lands on the last key itself, if it is still in the tree.
        if entry.as_ref().map(|(key, _)| key) == self.last.as_ref() && entry.is_some()
        {
            entry = cursor.next(pager)?;
        }
        if let Some((key, _)) = &entry {
            self.last = Some(key.clone());
        }
        Ok(entry)
    }
}
// @end

// @listing prefix-scan
/// The smallest byte string that is greater than every string starting with
/// `prefix`, or `None` if there is no such string.
pub fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < 0xff {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

/// Every entry whose key starts with `prefix`, in key order.
pub fn prefix_scan(
    tree: &Tree,
    pager: &mut Pager,
    prefix: &[u8],
) -> Result<Vec<Entry>> {
    if let Some(end) = prefix_end(prefix) {
        return tree.range(pager, prefix, &end);
    }
    // Nothing sorts after this prefix, so the scan runs to the end of the tree.
    let mut entries = Vec::new();
    let mut cursor = Cursor::seek(tree, pager, prefix)?;
    while let Some(entry) = cursor.next(pager)? {
        entries.push(entry);
    }
    Ok(entries)
}
// @end

// @listing reverse-cursor
/// A cursor that reads a tree from its largest key to its smallest. It keeps the
/// path from the root: at each interior page the cell it went through, and at the
/// leaf the number of cells still to be returned.
#[derive(Debug)]
pub struct ReverseCursor {
    path: Vec<(PageNo, usize)>,
}

/// Extends `path` from page `n` down the right-hand edge of its subtree.
fn descend_last(
    pager: &mut Pager,
    mut n: PageNo,
    path: &mut Vec<(PageNo, usize)>,
) -> Result<()> {
    while path.len() < MAX_DEPTH {
        let page = pager.page(n)?;
        let count = page.cell_count();
        match page.kind() {
            LEAF => {
                path.push((n, count));
                return Ok(());
            }
            INTERIOR if count > 0 => {
                path.push((n, count - 1));
                (n, _) = parse_interior(page.cell(count - 1)?)?;
            }
            _ => return Err(Error::Corrupt(format!("page {n} is not a tree page"))),
        }
    }
    Err(Error::Corrupt("the tree is too deep".into()))
}
// @end

impl ReverseCursor {
    // @listing reverse-prev
    /// A cursor after the last entry of the tree.
    pub fn last(tree: &Tree, pager: &mut Pager) -> Result<ReverseCursor> {
        let mut path = Vec::new();
        descend_last(pager, tree.root(), &mut path)?;
        Ok(ReverseCursor { path })
    }

    /// The entry before the cursor, which then moves back past it.
    pub fn prev(&mut self, pager: &mut Pager) -> Result<Option<Entry>> {
        loop {
            let Some(&(leaf, left)) = self.path.last() else { return Ok(None) };
            if left > 0 {
                let (key, payload) = parse_leaf(pager.page(leaf)?.cell(left - 1)?)?;
                let entry = (key.to_vec(), payload.to_vec());
                self.path.last_mut().unwrap().1 = left - 1;
                return Ok(Some(entry));
            }
            // This leaf is used up. Climb to the nearest ancestor that has a child
            // to the left of the one we came through, and go down that child.
            self.path.pop();
            while let Some((n, i)) = self.path.pop() {
                if i > 0 {
                    self.path.push((n, i - 1));
                    let (child, _) = parse_interior(pager.page(n)?.cell(i - 1)?)?;
                    descend_last(pager, child, &mut self.path)?;
                    break;
                }
            }
        }
    }
    // @end
}
