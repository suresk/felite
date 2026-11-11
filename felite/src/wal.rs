//! The write-ahead log: where a changed page goes before the database file sees it.

use crate::error::Result;
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::storage::Storage;
use std::collections::HashMap;

// @listing wal-layout
/// The first 8 bytes of every felite log.
pub const LOG_MAGIC: &[u8; 8] = b"felite-l";

/// The log's header: the magic, the page size, the salt, and a checksum of those.
pub const LOG_HEADER: usize = 24;
/// What comes before the page in each frame: the page's number, whether the frame
/// ends a commit, and a checksum.
pub const FRAME_HEADER: usize = 16;
/// One frame: its header, then a page.
pub const FRAME_SIZE: usize = FRAME_HEADER + PAGE_SIZE;

/// Where frame `i` starts in the log file. The first frame is frame 0.
fn frame_offset(i: u32) -> u64 {
    LOG_HEADER as u64 + i as u64 * FRAME_SIZE as u64
}
// @end

// @listing wal-checksum
/// Continues a checksum over `bytes`, whose length must be a multiple of 8.
pub fn checksum(mut sums: (u32, u32), bytes: &[u8]) -> (u32, u32) {
    let (words, _) = bytes.as_chunks::<4>();
    let mut words = words.iter().map(|word| u32::from_be_bytes(*word));
    while let (Some(first), Some(second)) = (words.next(), words.next()) {
        sums.0 = sums.0.wrapping_add(first).wrapping_add(sums.1);
        sums.1 = sums.1.wrapping_add(second).wrapping_add(sums.0);
    }
    sums
}
// @end

// @listing wal
pub(crate) struct Wal {
    storage: Box<dyn Storage>,
    /// A number that is different each time the log is started over.
    salt: u32,
    /// The checksum as it stands after the last frame written.
    sums: (u32, u32),
    /// How many frames have been written.
    frames: u32,
    /// For each page with a committed frame, the newest such frame.
    committed: HashMap<PageNo, u32>,
    /// The same, for the frames written since the last commit.
    pending: HashMap<PageNo, u32>,
}
// @end

impl Wal {
    // @listing wal-open
    /// Starts a log in `storage`.
    pub(crate) fn open(storage: Box<dyn Storage>) -> Result<Wal> {
        let mut wal = Wal {
            storage,
            salt: 1,
            sums: (0, 0),
            frames: 0,
            committed: HashMap::new(),
            pending: HashMap::new(),
        };
        wal.write_header()?;
        Ok(wal)
    }

    /// Writes the header of an empty log, with the current salt.
    fn write_header(&mut self) -> Result<()> {
        let mut header = Vec::with_capacity(LOG_HEADER);
        header.extend(LOG_MAGIC);
        header.extend((PAGE_SIZE as u32).to_be_bytes());
        header.extend(self.salt.to_be_bytes());
        self.sums = checksum((0, 0), &header);
        header.extend(self.sums.0.to_be_bytes());
        header.extend(self.sums.1.to_be_bytes());
        self.storage.write_at(0, &header)?;
        Ok(())
    }
    // @end

    // @listing wal-append
    /// Adds a frame holding page `n` to the end of the log. If `commit` is set, the
    /// frame ends a commit: the log is synced, and every frame since the last
    /// commit becomes part of the database.
    pub(crate) fn append(
        &mut self,
        n: PageNo,
        page: &Page,
        commit: bool,
    ) -> Result<()> {
        let mut frame = Vec::with_capacity(FRAME_SIZE);
        frame.extend(n.to_be_bytes());
        frame.extend((commit as u32).to_be_bytes());
        let sums = checksum(checksum(self.sums, &frame), page.bytes());
        frame.extend(sums.0.to_be_bytes());
        frame.extend(sums.1.to_be_bytes());
        frame.extend(page.bytes());
        self.storage.write_at(frame_offset(self.frames), &frame)?;

        self.sums = sums;
        self.pending.insert(n, self.frames);
        self.frames += 1;
        if commit {
            self.storage.sync()?;
            self.committed.extend(self.pending.drain());
        }
        Ok(())
    }
    // @end

    // @listing wal-read
    /// The newest version of page `n` in the log, if the log has one.
    pub(crate) fn read(&mut self, n: PageNo) -> Result<Option<Page>> {
        let Some(&frame) = self.pending.get(&n).or(self.committed.get(&n)) else {
            return Ok(None);
        };
        let mut page = Page::new();
        let at = frame_offset(frame) + FRAME_HEADER as u64;
        self.storage.read_at(at, page.bytes_mut())?;
        Ok(Some(page))
    }

    /// Whether any frame has been written since the last commit.
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// How many frames the log holds.
    pub(crate) fn frames(&self) -> u32 {
        self.frames
    }
    // @end
}
