//! The slotted page: many cells of different lengths in one page.

use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};

// @listing layout
/// Bytes taken by the header at the start of a slotted page.
pub const SLOTTED_HEADER: usize = 12;
/// Bytes taken by one slot: a cell's offset and its length.
pub const SLOT_SIZE: usize = 4;

const KIND_AT: usize = 0;
const COUNT_AT: usize = 2;
const CONTENT_AT: usize = 4;
const FRAGMENTED_AT: usize = 6;
const RIGHT_AT: usize = 8;
// @end

impl Page {
    // @listing init
    /// Makes this an empty slotted page of the given kind.
    pub fn init_slotted(&mut self, kind: u8) {
        self.bytes_mut().fill(0);
        self.bytes_mut()[KIND_AT] = kind;
        self.put_u16(CONTENT_AT, PAGE_SIZE as u16);
    }

    pub fn kind(&self) -> u8 {
        self.bytes()[KIND_AT]
    }

    pub fn cell_count(&self) -> usize {
        self.u16_at(COUNT_AT) as usize
    }

    pub fn right(&self) -> PageNo {
        self.u32_at(RIGHT_AT)
    }

    pub fn set_right(&mut self, page: PageNo) {
        self.put_u32(RIGHT_AT, page);
    }
    // @end

    // @listing slots-end
    /// The offset just past the last slot, checked against the page.
    fn slots_end(&self) -> Result<usize> {
        let count = self.cell_count();
        let end = SLOTTED_HEADER + SLOT_SIZE * count;
        if end > PAGE_SIZE {
            return Err(Error::Corrupt(format!("a page claims {count} cells")));
        }
        Ok(end)
    }
    // @end

    // @listing slot
    /// Where cell `i` starts and how long it is, checked against the page.
    fn slot(&self, i: usize) -> Result<(usize, usize)> {
        let slots_end = self.slots_end()?;
        let count = self.cell_count();
        assert!(i < count, "cell {i} of a page with {count}");
        let at = SLOTTED_HEADER + SLOT_SIZE * i;
        let start = self.u16_at(at) as usize;
        let len = self.u16_at(at + 2) as usize;
        if start < slots_end || start + len > PAGE_SIZE {
            let end = start + len;
            return Err(Error::Corrupt(format!(
                "cell {i} claims bytes {start} to {end}"
            )));
        }
        Ok((start, len))
    }

    pub fn cell(&self, i: usize) -> Result<&[u8]> {
        let (start, len) = self.slot(i)?;
        Ok(&self.bytes()[start..start + len])
    }
    // @end

    // @listing free-space
    /// The unused bytes between the last slot and the first cell.
    fn gap(&self) -> Result<usize> {
        let slots_end = self.slots_end()?;
        let content = self.u16_at(CONTENT_AT) as usize;
        if content < slots_end || content > PAGE_SIZE {
            return Err(Error::Corrupt(format!(
                "cell content starts at byte {content}"
            )));
        }
        Ok(content - slots_end)
    }

    /// Every unused byte: the gap, and the holes that removed cells left behind.
    pub fn free_space(&self) -> Result<usize> {
        Ok(self.gap()? + self.u16_at(FRAGMENTED_AT) as usize)
    }
    // @end

    // @listing insert-cell
    /// Adds a cell as number `i`, moving the cells from `i` on up by one. Returns
    /// false, with the page untouched, if there is no room.
    pub fn insert_cell(&mut self, i: usize, cell: &[u8]) -> Result<bool> {
        let count = self.cell_count();
        assert!(i <= count, "cell {i} inserted into a page with {count}");
        let needed = cell.len() + SLOT_SIZE;
        if self.free_space()? < needed {
            return Ok(false);
        }
        if self.gap()? < needed {
            self.compact()?;
            if self.gap()? < needed {
                return Err(Error::Corrupt("a page miscounts its free space".into()));
            }
        }
        let start = self.u16_at(CONTENT_AT) as usize - cell.len();
        self.bytes_mut()[start..start + cell.len()].copy_from_slice(cell);

        let at = SLOTTED_HEADER + SLOT_SIZE * i;
        let slots_end = self.slots_end()?;
        self.bytes_mut().copy_within(at..slots_end, at + SLOT_SIZE);
        self.put_u16(at, start as u16);
        self.put_u16(at + 2, cell.len() as u16);
        self.put_u16(COUNT_AT, count as u16 + 1);
        self.put_u16(CONTENT_AT, start as u16);
        Ok(true)
    }
    // @end

    // @listing remove-cell
    /// Removes cell `i`. The cells after it move down by one.
    pub fn remove_cell(&mut self, i: usize) -> Result<()> {
        let (start, len) = self.slot(i)?;
        let count = self.cell_count();
        self.bytes_mut()[start..start + len].fill(0);
        if start == self.u16_at(CONTENT_AT) as usize {
            // The lowest cell: the gap grows to take its place.
            self.put_u16(CONTENT_AT, (start + len) as u16);
        } else {
            let fragmented = self.u16_at(FRAGMENTED_AT) as usize + len;
            self.put_u16(FRAGMENTED_AT, fragmented as u16);
        }
        let at = SLOTTED_HEADER + SLOT_SIZE * i;
        let slots_end = self.slots_end()?;
        self.bytes_mut().copy_within(at + SLOT_SIZE..slots_end, at);
        self.bytes_mut()[slots_end - SLOT_SIZE..slots_end].fill(0);
        self.put_u16(COUNT_AT, count as u16 - 1);
        Ok(())
    }
    // @end

    // @listing compact
    /// Packs the cells against the end of the page, leaving no holes.
    pub fn compact(&mut self) -> Result<()> {
        let slots_end = self.slots_end()?;
        let mut packed = self.clone();
        packed.bytes_mut()[slots_end..].fill(0);
        let mut start = PAGE_SIZE;
        for i in 0..self.cell_count() {
            let cell = self.cell(i)?;
            if cell.len() > start - slots_end {
                return Err(Error::Corrupt("the cells of a page overlap".into()));
            }
            start -= cell.len();
            packed.bytes_mut()[start..start + cell.len()].copy_from_slice(cell);
            packed.put_u16(SLOTTED_HEADER + SLOT_SIZE * i, start as u16);
        }
        packed.put_u16(CONTENT_AT, start as u16);
        packed.put_u16(FRAGMENTED_AT, 0);
        *self = packed;
        Ok(())
    }
    // @end

    // @listing describe
    /// The header and slots of a slotted page as text, one line per cell.
    pub fn describe(&self) -> Result<String> {
        let (kind, count, right) = (self.kind(), self.cell_count(), self.right());
        let free = self.free_space()?;
        let mut out =
            format!("kind {kind}, {count} cells, {free} bytes free, right {right}\n");
        for i in 0..count {
            let (start, len) = self.slot(i)?;
            let shown = &self.bytes()[start..start + len.min(12)];
            let hex: Vec<String> = shown.iter().map(|b| format!("{b:02x}")).collect();
            let more = if len > 12 { " …" } else { "" };
            out += &format!(
                "  cell {i}: {len:4} bytes at {start:4}  {}{more}\n",
                hex.join(" ")
            );
        }
        Ok(out)
    }
    // @end
}
