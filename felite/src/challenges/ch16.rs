//! Chapter 16 challenges: counting a statement's writes, a checksum that notices a
//! torn page, finding damage to the free list, and a storage that syncs every write.

use crate::crash::CrashDisk;
use crate::db::Database;
use crate::error::Result;
use crate::pager::Pager;
use crate::storage::Storage;
use std::io;

// @listing pages-written
/// Runs a statement and returns how many pages it wrote to `disk`, which must be
/// the disk that `db` is on.
pub fn pages_written(db: &mut Database, disk: &CrashDisk, sql: &str) -> Result<u64> {
    let before = disk.writes();
    db.execute(sql)?;
    Ok(disk.writes() - before)
}
// @end

// @listing checksum
/// Continues a checksum over `bytes`, whose length must be a multiple of 8.
/// Start with `(0, 0)`.
pub fn checksum(mut sums: (u32, u32), bytes: &[u8]) -> (u32, u32) {
    let word = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    for pair in bytes.chunks_exact(8) {
        sums.0 = sums.0.wrapping_add(word(&pair[..4])).wrapping_add(sums.1);
        sums.1 = sums.1.wrapping_add(word(&pair[4..])).wrapping_add(sums.0);
    }
    sums
}

/// The simplest checksum there is: every byte added up.
pub fn byte_sum(bytes: &[u8]) -> u32 {
    bytes.iter().map(|b| *b as u32).sum()
}
// @end

// @listing free-list-damage
/// Runs the statements with a crash after every write in turn, and returns the
/// first number of writes after which the file's free list is damaged.
pub fn free_list_damage(statements: &[&str]) -> Result<Option<u64>> {
    let clean = CrashDisk::new();
    let mut db = Database::open(Box::new(clean.clone()))?;
    for sql in statements {
        db.execute(sql)?;
    }
    for writes in 1..clean.writes() {
        let disk = CrashDisk::new();
        disk.stop_after(writes);
        let mut db = Database::open(Box::new(disk.clone()))?;
        for sql in statements {
            if db.execute(sql).is_err() {
                break;
            }
        }
        let Ok(mut pager) = Pager::open(Box::new(disk.kill())) else { continue };
        if pager.check_free_list().is_err() {
            return Ok(Some(writes));
        }
    }
    Ok(None)
}
// @end

// @listing sync-each
/// A storage that waits for the disk after every write.
pub struct SyncEach<S>(pub S);

impl<S: Storage> Storage for SyncEach<S> {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_at(offset, buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.0.write_at(offset, buf)?;
        self.0.sync()
    }

    fn size(&self) -> io::Result<u64> {
        self.0.size()
    }

    fn sync(&mut self) -> io::Result<()> {
        self.0.sync()
    }
}
// @end
