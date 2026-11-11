//! Chapter 17 challenges: listing a log's frames, counting the commits in a log
//! that was cut short, a page as a list of changes, and a storage that never syncs.

use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::storage::Storage;
use crate::wal::{FRAME_SIZE, LOG_HEADER, LOG_MAGIC};
use std::io;

// @listing list-frames
/// The page number and commit mark of every whole frame in a log's bytes. The
/// checksums are not examined.
pub fn list_frames(log: &[u8]) -> Result<Vec<(PageNo, bool)>> {
    if log.len() < LOG_HEADER || &log[..8] != LOG_MAGIC {
        return Err(Error::Corrupt("this is not a felite log".into()));
    }
    let word = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    // Whole frames only: whatever is left over at the end is not one.
    let (frames, _) = log[LOG_HEADER..].as_chunks::<FRAME_SIZE>();
    Ok(frames.iter().map(|frame| (word(frame), word(&frame[4..]) != 0)).collect())
}
// @end

// @listing complete-commits
/// How many commits are entirely present in a log's bytes, and how many frames
/// they take up between them.
pub fn complete_commits(log: &[u8]) -> Result<(usize, usize)> {
    let frames = list_frames(log)?;
    let commits = frames.iter().filter(|(_, commit)| *commit).count();
    let covered = frames.iter().rposition(|(_, commit)| *commit).map_or(0, |i| i + 1);
    Ok((commits, covered))
}
// @end

// @listing page-delta
/// The differences between two versions of a page, as a list of runs: an offset
/// and a length, two bytes each, and then that many bytes of the new version.
pub fn page_delta(old: &Page, new: &Page) -> Vec<u8> {
    let (old, new) = (old.bytes(), new.bytes());
    let mut delta = Vec::new();
    let mut at = 0;
    while at < PAGE_SIZE {
        if old[at] == new[at] {
            at += 1;
            continue;
        }
        // A run goes on until eight bytes in a row are the same in both versions,
        // because a new run costs four bytes of its own.
        let mut end = at + 1;
        let mut same = 0;
        while end < PAGE_SIZE && same < 8 {
            same = if old[end] == new[end] { same + 1 } else { 0 };
            end += 1;
        }
        end -= same;
        delta.extend((at as u16).to_be_bytes());
        delta.extend(((end - at) as u16).to_be_bytes());
        delta.extend(&new[at..end]);
        at = end;
    }
    delta
}

/// Turns the old version of a page into the new one.
pub fn apply_delta(page: &mut Page, delta: &[u8]) -> Result<()> {
    let mut rest = delta;
    while !rest.is_empty() {
        let bad = || Error::Corrupt("a page delta that does not fit the page".into());
        let [a, b, c, d, tail @ ..] = rest else { return Err(bad()) };
        let at = u16::from_be_bytes([*a, *b]) as usize;
        let length = u16::from_be_bytes([*c, *d]) as usize;
        if tail.len() < length || at + length > PAGE_SIZE {
            return Err(bad());
        }
        page.bytes_mut()[at..at + length].copy_from_slice(&tail[..length]);
        rest = &tail[length..];
    }
    Ok(())
}
// @end

// @listing no-sync
/// A storage that returns from `sync` at once, without waiting for anything.
pub struct NoSync<S>(pub S);

impl<S: Storage> Storage for NoSync<S> {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_at(offset, buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.0.write_at(offset, buf)
    }

    fn size(&self) -> io::Result<u64> {
        self.0.size()
    }

    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}
// @end
