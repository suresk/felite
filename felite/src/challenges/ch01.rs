//! Chapter 1 challenges.

use crate::error::{Error, Result};
use crate::page::PAGE_SIZE;
use crate::pager::Pager;
use crate::storage::Storage;
use std::cell::Cell;
use std::io;
use std::rc::Rc;

// @listing counting
/// How many times each operation has been asked of a `Counting` storage.
#[derive(Default)]
pub struct Counts {
    pub reads: Cell<u64>,
    pub writes: Cell<u64>,
    pub syncs: Cell<u64>,
}

/// A storage that passes everything on to `inner` and keeps count.
pub struct Counting<S> {
    inner: S,
    counts: Rc<Counts>,
}

impl<S: Storage> Counting<S> {
    /// Wraps `inner`. The second value is the caller's handle to the counts.
    pub fn new(inner: S) -> (Counting<S>, Rc<Counts>) {
        let counts = Rc::new(Counts::default());
        (Counting { inner, counts: Rc::clone(&counts) }, counts)
    }
}
// @end

// @listing counting-storage
impl<S: Storage> Storage for Counting<S> {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.counts.reads.set(self.counts.reads.get() + 1);
        self.inner.read_at(offset, buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.counts.writes.set(self.counts.writes.get() + 1);
        self.inner.write_at(offset, buf)
    }

    fn size(&self) -> io::Result<u64> {
        self.inner.size()
    }

    fn sync(&mut self) -> io::Result<()> {
        self.counts.syncs.set(self.counts.syncs.get() + 1);
        self.inner.sync()
    }
}
// @end

// @listing open-checked
/// `Pager::open`, plus a check that the file is as long as its header says.
pub fn open_checked(storage: Box<dyn Storage>) -> Result<Pager> {
    let size = storage.size()?;
    let mut pager = Pager::open(storage)?;
    let pages = pager.page_count()? as u64;
    if size != 0 && size != pages * PAGE_SIZE as u64 {
        let what =
            format!("the header says {pages} pages, but the file is {size} bytes");
        return Err(Error::Corrupt(what));
    }
    Ok(pager)
}
// @end

// @listing read-only
/// A storage that refuses every write.
pub struct ReadOnly<S>(pub S);

impl<S: Storage> Storage for ReadOnly<S> {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_at(offset, buf)
    }

    fn write_at(&mut self, _offset: u64, _buf: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn size(&self) -> io::Result<u64> {
        self.0.size()
    }

    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}
// @end

impl Pager {
    // @listing close
    /// Flushes and gives up the pager. Unlike a `Drop`, this can report a failure.
    pub fn close(mut self) -> Result<()> {
        self.flush()
    }
    // @end
}

// @listing fail-after
/// A storage that performs a fixed number of writes and then fails every one after.
pub struct FailAfter<S> {
    inner: S,
    writes_left: Rc<Cell<u32>>,
}

impl<S: Storage> FailAfter<S> {
    /// Wraps `inner`. The second value lets the caller top the allowance up.
    pub fn new(inner: S, writes: u32) -> (FailAfter<S>, Rc<Cell<u32>>) {
        let writes_left = Rc::new(Cell::new(writes));
        (FailAfter { inner, writes_left: Rc::clone(&writes_left) }, writes_left)
    }
}

// @end

// @listing fail-after-storage
impl<S: Storage> Storage for FailAfter<S> {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.inner.read_at(offset, buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        match self.writes_left.get() {
            0 => Err(io::Error::other("the disk is full")),
            n => {
                self.writes_left.set(n - 1);
                self.inner.write_at(offset, buf)
            }
        }
    }

    fn size(&self) -> io::Result<u64> {
        self.inner.size()
    }

    fn sync(&mut self) -> io::Result<()> {
        self.inner.sync()
    }
}
// @end
