//! Chapter 4 challenges: the path of a lookup, membership without a copy, the
//! ends of a tree, and short separator keys.

use crate::btree::search::{MAX_DEPTH, child_for, find};
use crate::btree::{INTERIOR, LEAF, Tree, parse_interior, parse_leaf};
use crate::error::{Error, Result};
use crate::page::PageNo;
use crate::pager::Pager;

// @listing path
/// The pages a lookup of `key` visits, each with the number of the cell it chose
/// there. The last entry is the leaf.
pub fn path(
    tree: &Tree,
    pager: &mut Pager,
    key: &[u8],
) -> Result<Vec<(PageNo, usize)>> {
    let mut steps = Vec::new();
    let mut n = tree.root();
    for _ in 0..MAX_DEPTH {
        let page = pager.page(n)?;
        match page.kind() {
            INTERIOR => {
                let i = child_for(page, key)?;
                steps.push((n, i));
                (n, _) = parse_interior(page.cell(i)?)?;
            }
            LEAF => {
                steps.push((n, find(page, key)?.0));
                return Ok(steps);
            }
            _ => return Err(Error::Corrupt(format!("page {n} is not a tree page"))),
        }
    }
    Err(Error::Corrupt("the tree is too deep".into()))
}
// @end

// @listing contains
/// Whether `key` is in the tree. Nothing is copied out of the leaf.
pub fn contains(tree: &Tree, pager: &mut Pager, key: &[u8]) -> Result<bool> {
    let steps = path(tree, pager, key)?;
    let (leaf, i) = steps[steps.len() - 1];
    let page = pager.page(leaf)?;
    Ok(i < page.cell_count() && parse_leaf(page.cell(i)?)?.0 == key)
}
// @end

// @listing ends
/// Walks down one edge of the tree and returns the key at the end of it.
fn edge_key(tree: &Tree, pager: &mut Pager, last: bool) -> Result<Option<Vec<u8>>> {
    let mut n = tree.root();
    for _ in 0..MAX_DEPTH {
        let page = pager.page(n)?;
        let count = page.cell_count();
        let i = if last { count.saturating_sub(1) } else { 0 };
        match page.kind() {
            LEAF if count == 0 => return Ok(None),
            LEAF => return Ok(Some(parse_leaf(page.cell(i)?)?.0.to_vec())),
            INTERIOR if count > 0 => (n, _) = parse_interior(page.cell(i)?)?,
            _ => return Err(Error::Corrupt(format!("page {n} is not a tree page"))),
        }
    }
    Err(Error::Corrupt("the tree is too deep".into()))
}

pub fn first_key(tree: &Tree, pager: &mut Pager) -> Result<Option<Vec<u8>>> {
    edge_key(tree, pager, false)
}

pub fn last_key(tree: &Tree, pager: &mut Pager) -> Result<Option<Vec<u8>>> {
    edge_key(tree, pager, true)
}
// @end

// @listing separator
/// The shortest byte string that is greater than `left` and not greater than
/// `right`. `left` must be less than `right`.
pub fn separator(left: &[u8], right: &[u8]) -> Vec<u8> {
    assert!(left < right);
    let shared = left.iter().zip(right).take_while(|(a, b)| a == b).count();
    right[..shared + 1].to_vec()
}
// @end
