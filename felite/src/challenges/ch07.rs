//! Chapter 7 challenges: deleting a range, listing where every page is, rebuilding
//! a tree, and accounting for the whole file.

use crate::btree::{Cursor, INTERIOR, LEAF, Tree, parse_interior};
use crate::error::{Error, Result};
use crate::page::{Page, PageNo};
use crate::pager::{FREE, Pager};
use std::collections::BTreeSet;

// @listing delete-range
/// Deletes every key that is at least `low` and less than `high`. Returns how many
/// there were.
pub fn delete_range(
    tree: &Tree,
    pager: &mut Pager,
    low: &[u8],
    high: &[u8],
) -> Result<usize> {
    let mut deleted = 0;
    loop {
        // The first key still in the range, found afresh each time, because the
        // deletion before it may have merged pages.
        let mut cursor = Cursor::seek(tree, pager, low)?;
        match cursor.next(pager)? {
            Some((key, _)) if key.as_slice() < high => {
                tree.delete(pager, &key)?;
                deleted += 1;
            }
            _ => return Ok(deleted),
        }
    }
}
// @end

// @listing free-pages
/// The page numbers on the free list, in the order they would be handed out.
pub fn free_pages(pager: &mut Pager) -> Result<Vec<PageNo>> {
    let mut pages = Vec::new();
    let mut n = pager.page(0)?.u32_at(24);
    while n != 0 {
        if pages.len() as u32 >= pager.page_count()? {
            return Err(Error::Corrupt("the free list links in a circle".into()));
        }
        let page = pager.page(n)?;
        if page.kind() != FREE {
            return Err(Error::Corrupt(format!(
                "page {n} is on the free list in use"
            )));
        }
        pages.push(n);
        n = page.u32_at(4);
    }
    Ok(pages)
}
// @end

// @listing tree-pages
/// Every page of a tree, the root first.
pub fn tree_pages(tree: &Tree, pager: &mut Pager) -> Result<Vec<PageNo>> {
    let mut pages = Vec::new();
    collect(pager, tree.root(), 1, &mut pages)?;
    Ok(pages)
}

fn collect(
    pager: &mut Pager,
    n: PageNo,
    depth: usize,
    pages: &mut Vec<PageNo>,
) -> Result<()> {
    let page = pager.page(n)?.clone();
    if depth > 40 || !matches!(page.kind(), LEAF | INTERIOR) {
        return Err(Error::Corrupt(format!(
            "page {n} is not where a tree page should be"
        )));
    }
    pages.push(n);
    if page.kind() == INTERIOR {
        for i in 0..page.cell_count() {
            let (child, _) = parse_interior(page.cell(i)?)?;
            collect(pager, child, depth + 1, pages)?;
        }
    }
    Ok(())
}
// @end

// @listing rebuild
/// Rewrites a tree so that its pages are full again. The root keeps its number.
pub fn rebuild(tree: &Tree, pager: &mut Pager) -> Result<()> {
    let mut entries = Vec::new();
    let mut cursor = Cursor::first(tree, pager)?;
    while let Some(entry) = cursor.next(pager)? {
        entries.push(entry);
    }
    for n in tree_pages(tree, pager)? {
        if n != tree.root() {
            pager.free(n)?;
        }
    }
    let mut root = Page::new();
    root.init_slotted(LEAF);
    *pager.page_mut(tree.root())? = root;
    // In key order, so that the append rule of Section 5.9 fills every page.
    for (key, payload) in entries {
        tree.insert(pager, &key, &payload)?;
    }
    Ok(())
}
// @end

// @listing account
/// Checks that every page of the file is the header, or on the free list, or in
/// exactly one of the given trees.
pub fn account(pager: &mut Pager, trees: &[Tree]) -> Result<()> {
    let mut seen = BTreeSet::from([0]);
    let free = free_pages(pager)?;
    if free.len() as u32 != pager.free_count()? {
        return Err(Error::Corrupt(
            "the free list is not as long as the header says".into(),
        ));
    }
    let mut used = free;
    for tree in trees {
        used.extend(tree_pages(tree, pager)?);
    }
    for n in used {
        if !seen.insert(n) {
            return Err(Error::Corrupt(format!("page {n} is in use twice")));
        }
    }
    let lost = pager.page_count()? as usize - seen.len();
    if lost > 0 {
        return Err(Error::Corrupt(format!("{lost} pages belong to nothing")));
    }
    Ok(())
}
// @end
