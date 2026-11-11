//! Chapter 3 challenges: replacing a cell, checking a page, dropping the cells at
//! the end, and reusing a hole.

use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page};
use crate::slotted::{SLOT_SIZE, SLOTTED_HEADER};

const COUNT_AT: usize = 2;
const CONTENT_AT: usize = 4;
const FRAGMENTED_AT: usize = 6;

// @listing replace-cell
/// Gives cell `i` new contents. Returns false, with the page untouched, if the new
/// contents are longer and there is no room for them.
pub fn replace_cell(page: &mut Page, i: usize, cell: &[u8]) -> Result<bool> {
    let old = page.cell(i)?.len();
    if cell.len() > old {
        if page.free_space()? + old < cell.len() {
            return Ok(false);
        }
        page.remove_cell(i)?;
        return page.insert_cell(i, cell);
    }
    let at = SLOTTED_HEADER + SLOT_SIZE * i;
    let start = page.u16_at(at) as usize;
    page.bytes_mut()[start..start + old].fill(0);
    page.bytes_mut()[start..start + cell.len()].copy_from_slice(cell);
    page.put_u16(at + 2, cell.len() as u16);
    let fragmented = page.u16_at(FRAGMENTED_AT) as usize + old - cell.len();
    page.put_u16(FRAGMENTED_AT, fragmented as u16);
    Ok(true)
}
// @end

// @listing extents
/// Where each cell starts and ends, in the order the cells lie in the page.
fn extents(page: &Page) -> Result<Vec<(usize, usize)>> {
    let mut extents = Vec::new();
    for i in 0..page.cell_count() {
        let len = page.cell(i)?.len(); // checks the slot
        let start = page.u16_at(SLOTTED_HEADER + SLOT_SIZE * i) as usize;
        extents.push((start, start + len));
    }
    extents.sort();
    Ok(extents)
}
// @end

// @listing check-slotted
/// Checks everything a slotted page promises about itself.
pub fn check_slotted(page: &Page) -> Result<()> {
    page.free_space()?; // checks the cell count and the start of cell content
    let mut at = page.u16_at(CONTENT_AT) as usize;
    let mut holes = 0;
    for (start, end) in extents(page)? {
        if start < at {
            return Err(Error::Corrupt(format!("the cell at byte {start} overlaps")));
        }
        holes += start - at;
        at = end;
    }
    holes += PAGE_SIZE - at;
    let fragmented = page.u16_at(FRAGMENTED_AT) as usize;
    if holes != fragmented {
        let what =
            format!("{holes} bytes in holes, and the header says {fragmented}");
        return Err(Error::Corrupt(what));
    }
    Ok(())
}
// @end

// @listing truncate-cells
/// Removes every cell from number `keep` on, in one pass.
pub fn truncate_cells(page: &mut Page, keep: usize) -> Result<()> {
    assert!(keep <= page.cell_count());
    page.free_space()?;
    page.put_u16(COUNT_AT, keep as u16);
    page.compact()
}
// @end

// @listing reuse-hole
/// As `insert_cell`, but puts the cell in a hole of exactly its size if there is
/// one, so that the page need not be compacted.
pub fn insert_cell_reusing(page: &mut Page, i: usize, cell: &[u8]) -> Result<bool> {
    let count = page.cell_count();
    let slots_end = SLOTTED_HEADER + SLOT_SIZE * count;
    let content = page.u16_at(CONTENT_AT) as usize;
    if page.free_space()? < cell.len() + SLOT_SIZE || content - slots_end < SLOT_SIZE
    {
        return page.insert_cell(i, cell);
    }
    let mut at = content;
    let mut hole = None;
    for (start, end) in extents(page)?.into_iter().chain([(PAGE_SIZE, PAGE_SIZE)]) {
        if start >= at && start - at == cell.len() && !cell.is_empty() {
            hole = Some(at);
        }
        at = end;
    }
    let Some(start) = hole else { return page.insert_cell(i, cell) };
    page.bytes_mut()[start..start + cell.len()].copy_from_slice(cell);
    let slot = SLOTTED_HEADER + SLOT_SIZE * i;
    page.bytes_mut().copy_within(slot..slots_end, slot + SLOT_SIZE);
    page.put_u16(slot, start as u16);
    page.put_u16(slot + 2, cell.len() as u16);
    page.put_u16(COUNT_AT, count as u16 + 1);
    let fragmented = page.u16_at(FRAGMENTED_AT) as usize - cell.len();
    page.put_u16(FRAGMENTED_AT, fragmented as u16);
    Ok(true)
}
// @end
