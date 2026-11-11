//! Chapter 8 challenges: part of a large row, replacing a row, and accounting for
//! overflow pages.

use crate::btree::{Cursor, Tree, row_key};
use crate::challenges::ch07::{free_pages, tree_pages};
use crate::error::{Error, Result};
use crate::overflow::{OVERFLOW, SPILLED, chain, load, release, store};
use crate::pager::Pager;
use std::collections::BTreeSet;

// @listing load-range
/// Up to `length` bytes of a stored row, starting `start` bytes in. Overflow pages
/// past the end of the range are not read.
pub fn load_range(
    pager: &mut Pager,
    stored: &[u8],
    start: usize,
    length: usize,
) -> Result<Vec<u8>> {
    let spilled = stored.len() == SPILLED;
    let kept = if spilled { SPILLED - 8 } else { stored.len() };
    let number = |at: usize| {
        u32::from_be_bytes([
            stored[at],
            stored[at + 1],
            stored[at + 2],
            stored[at + 3],
        ])
    };
    let total = if spilled { number(kept) as usize } else { kept };
    let end = total.min(start.saturating_add(length));
    let mut out = Vec::new();
    if start >= end {
        return Ok(out);
    }
    if start < kept {
        out.extend_from_slice(&stored[start..end.min(kept)]);
    }
    // `at` is the position in the row of the first byte on the next overflow page.
    let mut at = kept;
    let mut next = if spilled { number(kept + 4) } else { 0 };
    while at < end {
        let page = pager.page(next)?;
        if page.kind() != OVERFLOW {
            return Err(Error::Corrupt(format!(
                "page {next} is not an overflow page"
            )));
        }
        let (from, to) = (start.max(at), end.min(at + 4088));
        if from < to {
            out.extend_from_slice(&page.bytes()[8 + from - at..8 + to - at]);
        }
        next = page.u32_at(4);
        at += 4088;
    }
    Ok(out)
}
// @end

// @listing update-row
/// Replaces the row with this id. Returns false, changing nothing, if there is no
/// such row.
pub fn update_row(
    tree: &Tree,
    pager: &mut Pager,
    id: i64,
    row: &[u8],
) -> Result<bool> {
    let key = row_key(id);
    let Some(old) = tree.get(pager, &key)? else {
        return Ok(false);
    };
    let new = store(pager, row)?;
    tree.delete(pager, &key)?;
    tree.insert(pager, &key, &new)?;
    release(pager, &old)?;
    Ok(true)
}
// @end

// @listing account-rows
/// Checks that every page of the file is the header, on the free list, in one of
/// the given tables' trees, or in the overflow chain of exactly one row.
pub fn account_rows(pager: &mut Pager, tables: &[Tree]) -> Result<()> {
    let mut used = free_pages(pager)?;
    for table in tables {
        used.extend(tree_pages(table, pager)?);
        let mut cursor = Cursor::first(table, pager)?;
        while let Some((_, stored)) = cursor.next(pager)? {
            if stored.len() == SPILLED {
                used.extend(chain(pager, &stored)?.1);
                load(pager, stored)?; // and the chain can be read to the end
            }
        }
    }
    let mut seen = BTreeSet::from([0]);
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
