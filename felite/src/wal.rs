//! The write-ahead log: where a changed page goes before the database file sees it.

use crate::error::{Error, Result};
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
    // @hide
    /// The values of `frames` and `sums` as they were after the last commit.
    committed_end: (u32, (u32, u32)),
    // @show
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
            // @hide
            committed_end: (0, (0, 0)),
            // @show
        };
        // @hide
        if wal.recover()? {
            return Ok(wal);
        }
        // @show
        wal.write_header()?;
        Ok(wal)
    }

    /// Writes the header of an empty log, with the current salt, and syncs it. No
    /// frame is ever written after a header that might not be on the disk.
    fn write_header(&mut self) -> Result<()> {
        let mut header = Vec::with_capacity(LOG_HEADER);
        header.extend(LOG_MAGIC);
        header.extend((PAGE_SIZE as u32).to_be_bytes());
        header.extend(self.salt.to_be_bytes());
        let sums = checksum((0, 0), &header);
        header.extend(sums.0.to_be_bytes());
        header.extend(sums.1.to_be_bytes());
        self.storage.write_at(0, &header)?;
        self.storage.sync()?;
        self.sums = sums;
        // @hide
        self.committed_end = (0, sums);
        // @show
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
            // @hide
            self.committed_end = (self.frames, self.sums);
            // @show
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

    // @listing wal-recover
    /// Reads the log that is in the file, as far as it can be believed. Returns
    /// false if there is no log there to read.
    fn recover(&mut self) -> Result<bool> {
        let word = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let size = self.storage.size()?;
        let mut header = [0; LOG_HEADER];
        if size < LOG_HEADER as u64 {
            return Ok(false);
        }
        self.storage.read_at(0, &mut header)?;
        let mut sums = checksum((0, 0), &header[..16]);
        let stored = (word(&header[16..]), word(&header[20..]));
        if &header[..8] != LOG_MAGIC || sums != stored {
            return Ok(false);
        }
        if word(&header[8..]) != PAGE_SIZE as u32 {
            return Err(Error::Corrupt("the log is for another page size".into()));
        }
        self.salt = word(&header[12..]);
        self.sums = sums;

        // Frames are good for as long as each one continues the chain. Those that
        // come before a commit mark are in the database. The rest never were.
        let mut frame = vec![0; FRAME_SIZE];
        let mut pending = HashMap::new();
        let whole = (size - LOG_HEADER as u64) / FRAME_SIZE as u64;
        for i in 0..whole as u32 {
            self.storage.read_at(frame_offset(i), &mut frame)?;
            sums = checksum(checksum(sums, &frame[..8]), &frame[FRAME_HEADER..]);
            if sums != (word(&frame[8..]), word(&frame[12..])) {
                break;
            }
            pending.insert(word(&frame), i);
            if word(&frame[4..]) != 0 {
                self.committed.extend(pending.drain());
                self.frames = i + 1;
                self.sums = sums;
            }
        }
        // @hide
        self.committed_end = (self.frames, self.sums);
        // @show
        Ok(true)
    }
    // @end

    // @listing wal-holds
    /// Whether the log has a version of page `n`.
    pub(crate) fn holds(&self, n: PageNo) -> bool {
        self.pending.contains_key(&n) || self.committed.contains_key(&n)
    }
    // @end

    // @listing wal-checkpoint
    /// Copies the newest committed version of every page in the log into the
    /// database file, and then starts the log again. Returns how many pages it
    /// copied. Does nothing while frames are waiting for a commit.
    pub(crate) fn checkpoint(&mut self, database: &mut dyn Storage) -> Result<u32> {
        if !self.pending.is_empty() || self.committed.is_empty() {
            return Ok(0);
        }
        let mut newest: Vec<(PageNo, u32)> =
            self.committed.iter().map(|(n, frame)| (*n, *frame)).collect();
        newest.sort();
        let mut page = [0; PAGE_SIZE];
        for (n, frame) in &newest {
            let at = frame_offset(*frame) + FRAME_HEADER as u64;
            self.storage.read_at(at, &mut page)?;
            database.write_at(*n as u64 * PAGE_SIZE as u64, &page)?;
        }
        // The pages must be safe in the database file before the log gives them up.
        database.sync()?;
        self.restart()?;
        Ok(newest.len() as u32)
    }

    /// Empties the log, by writing a header with a salt that no frame in the file
    /// was written under.
    fn restart(&mut self) -> Result<()> {
        self.salt = self.salt.wrapping_add(1);
        self.write_header()?;
        self.frames = 0;
        self.committed.clear();
        Ok(())
    }
    // @end

    // @listing wal-rollback
    /// Forgets every frame written since the last commit. The next frame will be
    /// written over the first of them.
    pub(crate) fn rollback(&mut self) {
        self.pending.clear();
        (self.frames, self.sums) = self.committed_end;
    }
    // @end
}
