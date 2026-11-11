//! Chapter 18 challenges: a report on a log, counting what recovery reads, and
//! putting a crashed file back on a disk that can crash again.

use crate::challenges::ch01::Counting;
use crate::crash::CrashDisk;
use crate::error::{Error, Result};
use crate::pager::Pager;
use crate::storage::{MemFile, Storage};
use crate::wal::{FRAME_HEADER, FRAME_SIZE, LOG_HEADER, LOG_MAGIC, checksum};

// @listing log-report
/// What recovery would make of a log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogReport {
    /// The salt in the header.
    pub salt: u32,
    /// Frames whose checksums continue the chain from the header.
    pub valid: usize,
    /// How many of those are covered by a commit, and so would be recovered.
    pub committed: usize,
    /// The number of commit marks among them.
    pub commits: usize,
}

/// Examines the bytes of a log file the way recovery does.
pub fn log_report(log: &[u8]) -> Result<LogReport> {
    let word = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let bad = |what: &str| Err(Error::Corrupt(what.to_string()));
    if log.len() < LOG_HEADER || &log[..8] != LOG_MAGIC {
        return bad("this is not a felite log");
    }
    let mut sums = checksum((0, 0), &log[..16]);
    if sums != (word(&log[16..]), word(&log[20..])) {
        return bad("the log's header fails its checksum");
    }
    let mut report =
        LogReport { salt: word(&log[12..]), valid: 0, committed: 0, commits: 0 };
    let (frames, _) = log[LOG_HEADER..].as_chunks::<FRAME_SIZE>();
    for frame in frames {
        sums = checksum(checksum(sums, &frame[..8]), &frame[FRAME_HEADER..]);
        if sums != (word(&frame[8..]), word(&frame[12..])) {
            break;
        }
        report.valid += 1;
        if word(&frame[4..]) != 0 {
            report.committed = report.valid;
            report.commits += 1;
        }
    }
    Ok(report)
}
// @end

// @listing recovery-reads
/// Opens a database and its log, and returns how many reads of the log that took.
pub fn recovery_reads(file: MemFile, log: MemFile) -> Result<u64> {
    let (log, counts) = Counting::new(log);
    Pager::open_logged(Box::new(file), Box::new(log))?;
    Ok(counts.reads.get())
}
// @end

// @listing to-disks
/// Puts a database file and its log, as a crash left them, on a new disk that can
/// crash in its turn. Both arrive synced.
pub fn to_disks(file: &MemFile, log: &MemFile) -> Result<(CrashDisk, CrashDisk)> {
    let mut disk = CrashDisk::new();
    let mut second = disk.another_file();
    for (from, to) in [(file, &mut disk), (log, &mut second)] {
        let bytes = from.bytes();
        if !bytes.is_empty() {
            to.write_at(0, &bytes)?;
            to.sync()?;
        }
    }
    Ok((disk, second))
}
// @end
