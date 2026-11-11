//! A disk whose power can fail, for seeing what a crash leaves behind.

use crate::storage::{MemFile, Storage};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;

// @listing crash-disk
/// The unit a disk writes at one time. A write can stop between two of them.
pub const SECTOR: usize = 512;

/// A file on a disk whose power can fail. Clones share the one file.
#[derive(Clone, Default)]
pub struct CrashDisk(Rc<RefCell<Disk>>);

#[derive(Default)]
struct Disk {
    /// What the program sees when it reads: every write made so far.
    cache: Vec<u8>,
    /// What had reached the disk itself at the last sync.
    durable: Vec<u8>,
    /// The writes made since the last sync, oldest first.
    pending: Vec<(u64, Vec<u8>)>,
    writes: u64,
    syncs: u64,
    /// The number of writes after which the program is stopped, if it is to be.
    stop_after: Option<u64>,
}

/// Copies `data` into `bytes` at `offset`, growing `bytes` if it has to.
fn put(bytes: &mut Vec<u8>, offset: u64, data: &[u8]) {
    let (start, end) = (offset as usize, offset as usize + data.len());
    if bytes.len() < end {
        bytes.resize(end, 0);
    }
    bytes[start..end].copy_from_slice(data);
}
// @end

// @listing crash-stop
impl Disk {
    /// Fails if the program was to be stopped by now.
    fn running(&self) -> io::Result<()> {
        match self.stop_after {
            Some(limit) if self.writes >= limit => Err(io::Error::other("stopped")),
            _ => Ok(()),
        }
    }
}

impl CrashDisk {
    pub fn new() -> CrashDisk {
        CrashDisk::default()
    }

    /// Stops the program after `writes` more writes: every write and every sync
    /// after that fails.
    pub fn stop_after(&self, writes: u64) {
        let mut disk = self.0.borrow_mut();
        disk.stop_after = Some(disk.writes + writes);
    }

    /// How many writes the disk has accepted.
    pub fn writes(&self) -> u64 {
        self.0.borrow().writes
    }

    /// How many times it has been synced.
    pub fn syncs(&self) -> u64 {
        self.0.borrow().syncs
    }

    /// How many writes have been made since the last sync.
    pub fn pending(&self) -> usize {
        self.0.borrow().pending.len()
    }
}
// @end

// @listing crash-storage
impl Storage for CrashDisk {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let disk = self.0.borrow();
        let start = offset as usize;
        match disk.cache.get(start..start + buf.len()) {
            Some(bytes) => buf.copy_from_slice(bytes),
            None => return Err(io::ErrorKind::UnexpectedEof.into()),
        }
        Ok(())
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        let mut disk = self.0.borrow_mut();
        disk.running()?;
        disk.writes += 1;
        put(&mut disk.cache, offset, buf);
        disk.pending.push((offset, buf.to_vec()));
        Ok(())
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.0.borrow().cache.len() as u64)
    }

    fn sync(&mut self) -> io::Result<()> {
        let disk = &mut *self.0.borrow_mut();
        disk.running()?;
        disk.syncs += 1;
        for (offset, data) in disk.pending.drain(..) {
            put(&mut disk.durable, offset, &data);
        }
        Ok(())
    }
}
// @end

// @listing crash-fate
/// What became of one unsynced write when the power failed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fate {
    /// All of it reached the disk.
    Kept,
    /// None of it did.
    Lost,
    /// Only this many of its sectors did, counting from the start.
    Torn(usize),
}

impl CrashDisk {
    /// The file as it would be found after the power failed now. Everything synced
    /// is in it. `fate` is asked about each unsynced write, oldest first.
    pub fn crash(&self, mut fate: impl FnMut(usize) -> Fate) -> MemFile {
        let disk = self.0.borrow();
        let mut bytes = disk.durable.clone();
        for (i, (offset, data)) in disk.pending.iter().enumerate() {
            let kept = match fate(i) {
                Fate::Kept => data.len(),
                Fate::Lost => 0,
                Fate::Torn(sectors) => data.len().min(sectors * SECTOR),
            };
            if kept > 0 {
                put(&mut bytes, *offset, &data[..kept]);
            }
        }
        let mut file = MemFile::new();
        file.write_at(0, &bytes).expect("a MemFile takes any write");
        file
    }
}
// @end

// @listing crash-kinds
impl CrashDisk {
    /// The file after the program is killed and the computer keeps running: the
    /// operating system still has every write, and gets them all to the disk.
    pub fn kill(&self) -> MemFile {
        self.crash(|_| Fate::Kept)
    }

    /// The file after the power fails. Each unsynced write is kept, lost or torn,
    /// as `seed` decides.
    pub fn power_cut(&self, seed: u64) -> MemFile {
        let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.crash(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            match state % 8 {
                0..=2 => Fate::Kept,
                3..=5 => Fate::Lost,
                _ => Fate::Torn(1 + (state >> 8) as usize % 7),
            }
        })
    }
}
// @end
