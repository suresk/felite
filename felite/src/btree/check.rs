//! Verifying a whole tree.

use crate::btree::node::{INTERIOR, LEAF, parse_interior, parse_leaf};
use crate::btree::search::{MAX_DEPTH, Tree};
use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::pager::Pager;
use crate::slotted::SLOTTED_HEADER;
use std::collections::BTreeSet;

// @listing tree-info
/// What `Tree::check` learned about a tree that passed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeInfo {
    /// Levels, counting the root and the leaves.
    pub height: usize,
    pub leaves: usize,
    pub interior: usize,
    /// Keys stored in the leaves.
    pub entries: usize,
    /// Bytes in use on all the tree's pages, slots included, headers not.
    pub used: usize,
}

impl TreeInfo {
    /// The share of the tree's usable space that is in use, from 0 to 1.
    pub fn fill(&self) -> f64 {
        let usable = (self.leaves + self.interior) * (PAGE_SIZE - SLOTTED_HEADER);
        self.used as f64 / usable as f64
    }
}
// @end

// @listing checker
/// What the check has seen so far.
struct Checker {
    info: TreeInfo,
    seen: BTreeSet<PageNo>,
    /// For each level, the page visited last and the right link it carried.
    last: Vec<(PageNo, PageNo)>,
}

fn bad<T>(n: PageNo, what: &str) -> Result<T> {
    Err(Error::Corrupt(format!("page {n}: {what}")))
}

/// Whether `key` is at least `low` and less than `high`, where those are given.
fn in_range(key: &[u8], low: Option<&[u8]>, high: Option<&[u8]>) -> bool {
    low.is_none_or(|l| key >= l) && high.is_none_or(|h| key < h)
}
// @end

impl Checker {
    // @listing check-visit
    /// What every page must satisfy, whatever its kind. `level` is 0 at the root.
    fn visit(&mut self, pager: &mut Pager, n: PageNo, level: usize) -> Result<()> {
        if level >= MAX_DEPTH {
            return bad(n, "more levels than a tree can have");
        }
        if !self.seen.insert(n) {
            return bad(n, "reached twice");
        }
        let page = pager.page(n)?;
        let used = PAGE_SIZE - SLOTTED_HEADER - page.free_space()?;
        let right = page.right();
        // Pages on one level are visited from left to right, so each must be the
        // right-hand neighbor of the one before.
        if let Some(&(before, link)) = self.last.get(level) {
            if link != n {
                return bad(before, &format!("its right link is {link}, not {n}"));
            }
            self.last[level] = (n, right);
        } else {
            self.last.push((n, right));
        }
        self.info.used += used;
        Ok(())
    }
    // @end

    // @listing check-page
    /// Checks the subtree at page `n`. Every key in it must be at least `low` and
    /// less than `high`, where those are given.
    fn page(
        &mut self,
        pager: &mut Pager,
        n: PageNo,
        level: usize,
        low: Option<&[u8]>,
        high: Option<&[u8]>,
    ) -> Result<()> {
        self.visit(pager, n, level)?;
        let page = pager.page(n)?.clone();
        match page.kind() {
            LEAF => self.leaf(n, &page, level, low, high),
            INTERIOR => self.interior(pager, n, &page, level, low, high),
            kind => bad(n, &format!("kind {kind} is not a tree page")),
        }
    }
    // @end

    // @listing check-leaf
    fn leaf(
        &mut self,
        n: PageNo,
        page: &Page,
        level: usize,
        low: Option<&[u8]>,
        high: Option<&[u8]>,
    ) -> Result<()> {
        if self.info.height == 0 {
            self.info.height = level + 1;
        } else if self.info.height != level + 1 {
            return bad(n, "a leaf at a different depth from the others");
        }
        self.info.leaves += 1;
        self.info.entries += page.cell_count();
        let mut previous: Option<&[u8]> = None;
        for i in 0..page.cell_count() {
            let (key, _) = parse_leaf(page.cell(i)?)?;
            if !in_range(key, low, high) {
                return bad(
                    n,
                    &format!("the key in cell {i} belongs on another page"),
                );
            }
            if previous.is_some_and(|p| p >= key) {
                return bad(n, &format!("the key in cell {i} is out of order"));
            }
            previous = Some(key);
        }
        Ok(())
    }
    // @end

    // @listing check-interior
    fn interior(
        &mut self,
        pager: &mut Pager,
        n: PageNo,
        page: &Page,
        level: usize,
        low: Option<&[u8]>,
        high: Option<&[u8]>,
    ) -> Result<()> {
        self.info.interior += 1;
        let count = page.cell_count();
        if count == 0 {
            return bad(n, "an interior page with no cells");
        }
        for i in 0..count {
            let (child, key) = parse_interior(page.cell(i)?)?;
            // The child's keys run from this cell's key up to the next cell's. The
            // first key is never compared, so the first child inherits `low`.
            let from = if i == 0 { low } else { Some(key) };
            let to = match i + 1 < count {
                true => Some(parse_interior(page.cell(i + 1)?)?.1),
                false => high,
            };
            if i > 0 && (!in_range(key, low, high) || to.is_some_and(|t| key >= t)) {
                return bad(n, &format!("the key in cell {i} is out of order"));
            }
            self.page(pager, child, level + 1, from, to)?;
        }
        Ok(())
    }
    // @end
}

impl Tree {
    // @listing check
    /// Verifies every property the tree relies on, and describes the tree if they
    /// all hold.
    pub fn check(&self, pager: &mut Pager) -> Result<TreeInfo> {
        let info =
            TreeInfo { height: 0, leaves: 0, interior: 0, entries: 0, used: 0 };
        let mut checker = Checker { info, seen: BTreeSet::new(), last: Vec::new() };
        checker.page(pager, self.root(), 0, None, None)?;
        for &(n, link) in &checker.last {
            if link != 0 {
                return bad(
                    n,
                    &format!("the last page on its level has right link {link}"),
                );
            }
        }
        Ok(checker.info)
    }
    // @end
}
