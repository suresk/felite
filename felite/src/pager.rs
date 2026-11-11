//! The pager: pages by number, from a file.

use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::storage::Storage;
use crate::wal::Wal;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};

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

// @listing schema-header
/// Where the header keeps the root page of the schema table.
const SCHEMA_ROOT_AT: usize = 32;
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
    // @hide
    pool: Pool,
    wal: Option<Wal>,
    // @show
}
// @end

// @listing pool
/// How many pages `Pager::open` will keep in memory: about 8 MB of them.
pub const DEFAULT_FRAMES: usize = 2000;

/// What the pager keeps in order to choose which page to drop.
struct Pool {
    /// The most pages that may be in memory at once.
    capacity: usize,
    /// The pages in memory, in the order the clock's hand will come to them.
    clock: VecDeque<PageNo>,
    /// The pages that have been used since the hand last passed them.
    recent: HashSet<PageNo>,
    stats: PoolStats,
}

/// Counts of what the pager has done with the file.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PoolStats {
    /// Pages read from the file.
    pub reads: u64,
    /// Pages written to it, by `flush` or to make room.
    pub writes: u64,
    /// Pages dropped from memory to make room.
    pub evictions: u64,
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
        let mut pager = Pager {
            storage,
            frames: HashMap::new(),
            // @hide
            pool: Pool {
                capacity: DEFAULT_FRAMES,
                clock: VecDeque::new(),
                recent: HashSet::new(),
                stats: PoolStats::default(),
            },
            wal: None,
            // @show
        };
        if pager.storage.size()? == 0 {
            let mut header = Page::new();
            header.bytes_mut()[..16].copy_from_slice(MAGIC);
            header.put_u32(PAGE_SIZE_AT, PAGE_SIZE as u32);
            header.put_u32(PAGE_COUNT_AT, 1);
            pager.frames.insert(0, Frame { page: header, dirty: true });
            // @hide
            pager.pool.clock.push_back(0);
            // @show
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
        // @hide
        self.admit(n)?;
        // @show
        match self.frames.entry(n) {
            Entry::Occupied(frame) => Ok(frame.into_mut()),
            Entry::Vacant(slot) => {
                // @hide
                if let Some(wal) = &mut self.wal
                    && let Some(page) = wal.read(n)?
                {
                    return Ok(slot.insert(Frame { page, dirty: false }));
                }
                // @show
                let mut page = Page::new();
                self.storage.read_at(offset(n), page.bytes_mut())?;
                // @hide
                self.pool.stats.reads += 1;
                // @show
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

    // @listing with-capacity
    /// Opens a database with room in memory for `frames` pages, and no more.
    pub fn with_capacity(storage: Box<dyn Storage>, frames: usize) -> Result<Pager> {
        let mut pager = Pager::open(storage)?;
        pager.pool.capacity = frames.max(1);
        Ok(pager)
    }

    pub fn stats(&self) -> PoolStats {
        self.pool.stats
    }
    // @end

    // @listing admit
    /// Called before page `n` is used. If the page is not in memory, this makes
    /// sure there is room for it, dropping other pages if it has to.
    fn admit(&mut self, n: PageNo) -> Result<()> {
        if self.frames.contains_key(&n) {
            self.pool.recent.insert(n);
            return Ok(());
        }
        while self.frames.len() >= self.pool.capacity {
            let Some(victim) = self.pool.clock.pop_front() else { break };
            if self.pool.recent.remove(&victim) {
                // Used since the hand last came by: it gets another turn.
                self.pool.clock.push_back(victim);
            } else if let Err(error) = self.evict(victim) {
                self.pool.clock.push_front(victim);
                return Err(error);
            }
        }
        self.pool.clock.push_back(n);
        Ok(())
    }
    // @end

    // @listing evict
    /// Drops page `n` from memory, writing it to the file first if it has changed.
    fn evict(&mut self, n: PageNo) -> Result<()> {
        // @hide
        if self.wal.is_some() {
            return self.evict_to_log(n);
        }
        // @show
        if let Some(frame) = self.frames.get(&n) {
            if frame.dirty {
                self.storage.write_at(offset(n), frame.page.bytes())?;
                self.pool.stats.writes += 1;
            }
            self.frames.remove(&n);
            self.pool.stats.evictions += 1;
        }
        Ok(())
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
        // @hide
        self.admit(n)?;
        // @show
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
        // @hide
        self.admit(n)?;
        // @show
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

    // @listing check-free-list
    /// Walks the free list, checking each page on it, and returns its length.
    pub fn check_free_list(&mut self) -> Result<u32> {
        let count = self.free_count()?;
        let mut next = self.load(0)?.page.u32_at(FREE_HEAD_AT);
        for _ in 0..count {
            let page = self.page(next)?;
            if next == 0 || page.bytes()[0] != FREE {
                let what = format!("page {next} is on the free list and is not free");
                return Err(Error::Corrupt(what));
            }
            next = page.u32_at(NEXT_FREE_AT);
        }
        if next != 0 {
            let what = format!("the free list runs on past its {count} pages");
            return Err(Error::Corrupt(what));
        }
        Ok(count)
    }
    // @end

    // @listing schema-root
    /// The root page of the table that lists the tables, or 0 if there is none yet.
    pub fn schema_root(&mut self) -> Result<PageNo> {
        Ok(self.load(0)?.page.u32_at(SCHEMA_ROOT_AT))
    }

    pub fn set_schema_root(&mut self, root: PageNo) -> Result<()> {
        let header = self.load(0)?;
        header.page.put_u32(SCHEMA_ROOT_AT, root);
        header.dirty = true;
        Ok(())
    }
    // @end

    // @listing flush
    /// Writes every changed page to the file, lowest page number first, and waits
    /// for the disk.
    pub fn flush(&mut self) -> Result<()> {
        // @hide
        if self.wal.is_some() {
            return self.commit();
        }
        // @show
        let mut dirty: Vec<_> =
            self.frames.iter_mut().filter(|(_, frame)| frame.dirty).collect();
        dirty.sort_by_key(|(n, _)| **n);
        for (n, frame) in dirty {
            self.storage.write_at(offset(*n), frame.page.bytes())?;
            // @hide
            self.pool.stats.writes += 1;
            // @show
            frame.dirty = false;
        }
        self.storage.sync()?;
        Ok(())
    }
    // @end

    // @listing open-logged
    /// Opens the database in `storage`, with a write-ahead log in `log`.
    pub fn open_logged(
        storage: Box<dyn Storage>,
        log: Box<dyn Storage>,
    ) -> Result<Pager> {
        let mut pager = Pager::open(storage)?;
        pager.wal = Some(Wal::open(log)?);
        Ok(pager)
    }

    /// Changes how many pages the pager may keep in memory.
    pub fn set_capacity(&mut self, frames: usize) {
        self.pool.capacity = frames.max(1);
    }

    /// How many frames the log holds, or 0 if there is no log.
    pub fn log_frames(&self) -> u32 {
        self.wal.as_ref().map_or(0, Wal::frames)
    }
    // @end

    // @listing commit
    /// Makes every change since the last commit permanent, all of them or none.
    pub fn commit(&mut self) -> Result<()> {
        if self.wal.is_none() {
            return self.flush();
        }
        // A commit is marked on its last frame, so there has to be a frame. If the
        // only changes are in frames that were written earlier, log the header again.
        let nothing_dirty = self.frames.values().all(|frame| !frame.dirty);
        if nothing_dirty && self.wal.as_ref().is_some_and(Wal::has_pending) {
            self.load(0)?.dirty = true;
        }
        let Some(wal) = &mut self.wal else { return Ok(()) };
        let mut dirty: Vec<_> =
            self.frames.iter_mut().filter(|(_, frame)| frame.dirty).collect();
        dirty.sort_by_key(|(n, _)| **n);
        let last = dirty.len().saturating_sub(1);
        for (i, (n, frame)) in dirty.into_iter().enumerate() {
            wal.append(*n, &frame.page, i == last)?;
            frame.dirty = false;
        }
        Ok(())
    }
    // @end

    // @listing evict-to-log
    /// Drops page `n` from memory. If it has changed, it goes to the log first, in
    /// a frame that is not part of any commit yet.
    fn evict_to_log(&mut self, n: PageNo) -> Result<()> {
        if let (Some(frame), Some(wal)) = (self.frames.get(&n), &mut self.wal) {
            if frame.dirty {
                wal.append(n, &frame.page, false)?;
            }
            self.frames.remove(&n);
            self.pool.stats.evictions += 1;
        }
        Ok(())
    }
    // @end
}
