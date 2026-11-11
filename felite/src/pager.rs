//! The pager: pages by number, from a file.

use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::storage::Storage;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

// @listing header
/// The first 16 bytes of every felite database.
pub const MAGIC: &[u8; 16] = b"felite format 1\0";

// Where the header page keeps its fields.
const PAGE_SIZE_AT: usize = 16;
const PAGE_COUNT_AT: usize = 20;
// @end

// @listing free-header
/// The header's record of the free list: its first page, and how many it holds.
const FREE_HEAD_AT: usize = 24;
const FREE_COUNT_AT: usize = 28;

/// The kind of a page that belongs to no tree and is waiting to be used again.
pub const FREE: u8 = 3;
/// Where a free page keeps the number of the next free page.
const NEXT_FREE_AT: usize = 4;
// @end

// @listing pager
/// A page in memory, and whether it has changed since it was last written.
struct Frame {
    page: Page,
    dirty: bool,
}

pub struct Pager {
    storage: Box<dyn Storage>,
    frames: HashMap<PageNo, Frame>,
}
// @end

// @listing offset
/// Where page `n` starts in the file.
fn offset(n: PageNo) -> u64 {
    n as u64 * PAGE_SIZE as u64
}
// @end

impl Pager {
    // @listing open
    /// Opens the database in `storage`. An empty file becomes a new database.
    pub fn open(storage: Box<dyn Storage>) -> Result<Pager> {
        let mut pager = Pager { storage, frames: HashMap::new() };
        if pager.storage.size()? == 0 {
            let mut header = Page::new();
            header.bytes_mut()[..16].copy_from_slice(MAGIC);
            header.put_u32(PAGE_SIZE_AT, PAGE_SIZE as u32);
            header.put_u32(PAGE_COUNT_AT, 1);
            pager.frames.insert(0, Frame { page: header, dirty: true });
        } else {
            pager.check_header()?;
        }
        Ok(pager)
    }
    // @end

    // @listing check-header
    fn check_header(&mut self) -> Result<()> {
        let size = self.storage.size()?;
        if size < PAGE_SIZE as u64 {
            let what = format!("the file is {size} bytes, shorter than one page");
            return Err(Error::Corrupt(what));
        }
        let header = &self.load(0)?.page;
        if &header.bytes()[..16] != MAGIC {
            return Err(Error::Corrupt("this is not a felite database".into()));
        }
        let page_size = header.u32_at(PAGE_SIZE_AT);
        if page_size != PAGE_SIZE as u32 {
            let what = format!("its pages are {page_size} bytes, not {PAGE_SIZE}");
            return Err(Error::Corrupt(what));
        }
        Ok(())
    }
    // @end

    // @listing load
    /// The frame for page `n`, read from the file if it is not in memory yet.
    fn load(&mut self, n: PageNo) -> Result<&mut Frame> {
        match self.frames.entry(n) {
            Entry::Occupied(frame) => Ok(frame.into_mut()),
            Entry::Vacant(slot) => {
                let mut page = Page::new();
                self.storage.read_at(offset(n), page.bytes_mut())?;
                Ok(slot.insert(Frame { page, dirty: false }))
            }
        }
    }
    // @end

    // @listing page-count
    /// How many pages the database has, the header included.
    pub fn page_count(&mut self) -> Result<u32> {
        Ok(self.load(0)?.page.u32_at(PAGE_COUNT_AT))
    }

    fn check_in_file(&mut self, n: PageNo) -> Result<()> {
        let count = self.page_count()?;
        if n >= count {
            let what = format!("page {n} is past the end of a {count}-page file");
            return Err(Error::Corrupt(what));
        }
        Ok(())
    }
    // @end

    // @listing page-access
    /// Page `n`, to read.
    pub fn page(&mut self, n: PageNo) -> Result<&Page> {
        self.check_in_file(n)?;
        Ok(&self.load(n)?.page)
    }

    /// Page `n`, to change. The pager will write it back at the next flush.
    pub fn page_mut(&mut self, n: PageNo) -> Result<&mut Page> {
        self.check_in_file(n)?;
        let frame = self.load(n)?;
        frame.dirty = true;
        Ok(&mut frame.page)
    }
    // @end

    // @listing pages-in-memory
    /// How many pages the pager is holding in memory right now.
    pub fn pages_in_memory(&self) -> usize {
        self.frames.len()
    }
    // @end

    // @listing allocate
    /// Adds `page` to the end of the database and returns its number.
    pub fn allocate(&mut self, page: Page) -> Result<PageNo> {
        // @hide
        if self.load(0)?.page.u32_at(FREE_HEAD_AT) != 0 {
            return self.reuse(page);
        }
        // @show
        let n = self.page_count()?;
        let header = self.load(0)?;
        header.page.put_u32(PAGE_COUNT_AT, n + 1);
        header.dirty = true;
        self.frames.insert(n, Frame { page, dirty: true });
        Ok(n)
    }
    // @end

    // @listing free
    /// Gives page `n` up. It joins the front of the free list, and a later call to
    /// `allocate` will hand it out again.
    pub fn free(&mut self, n: PageNo) -> Result<()> {
        self.check_in_file(n)?;
        if n == 0 || self.load(n)?.page.bytes()[0] == FREE {
            return Err(Error::Corrupt(format!("page {n} cannot be freed")));
        }
        let header = self.load(0)?;
        let head = header.page.u32_at(FREE_HEAD_AT);
        let count = header.page.u32_at(FREE_COUNT_AT);
        header.page.put_u32(FREE_HEAD_AT, n);
        header.page.put_u32(FREE_COUNT_AT, count + 1);
        header.dirty = true;

        let mut page = Page::new();
        page.bytes_mut()[0] = FREE;
        page.put_u32(NEXT_FREE_AT, head);
        self.frames.insert(n, Frame { page, dirty: true });
        Ok(())
    }
    // @end

    // @listing reuse
    /// Takes the first page off the free list and gives it the contents `page`.
    fn reuse(&mut self, page: Page) -> Result<PageNo> {
        let n = self.load(0)?.page.u32_at(FREE_HEAD_AT);
        self.check_in_file(n)?;
        let frame = self.load(n)?;
        if frame.page.bytes()[0] != FREE {
            return Err(Error::Corrupt(format!(
                "page {n} is on the free list in use"
            )));
        }
        let next = frame.page.u32_at(NEXT_FREE_AT);
        *frame = Frame { page, dirty: true };

        let header = self.load(0)?;
        let count = header.page.u32_at(FREE_COUNT_AT);
        header.page.put_u32(FREE_HEAD_AT, next);
        header.page.put_u32(FREE_COUNT_AT, count.saturating_sub(1));
        header.dirty = true;
        Ok(n)
    }
    // @end

    // @listing free-count
    /// How many pages are on the free list.
    pub fn free_count(&mut self) -> Result<u32> {
        Ok(self.load(0)?.page.u32_at(FREE_COUNT_AT))
    }
    // @end

    // @listing flush
    /// Writes every changed page to the file, lowest page number first, and waits
    /// for the disk.
    pub fn flush(&mut self) -> Result<()> {
        let mut dirty: Vec<_> =
            self.frames.iter_mut().filter(|(_, frame)| frame.dirty).collect();
        dirty.sort_by_key(|(n, _)| **n);
        for (n, frame) in dirty {
            self.storage.write_at(offset(*n), frame.page.bytes())?;
            frame.dirty = false;
        }
        self.storage.sync()?;
        Ok(())
    }
    // @end
}
