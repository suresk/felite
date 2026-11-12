//! What felite asks of a file, and two things that can answer.

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

// @listing storage
/// A file, reduced to the four things a database asks of one.
pub trait Storage {
    /// Fills `buf` with the bytes that start at `offset`. Reading past the end of
    /// the file is an error.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;

    /// Writes `buf` at `offset`. The file grows if it has to.
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()>;

    /// The file's length in bytes.
    fn size(&self) -> io::Result<u64>;

    /// Returns once everything written so far has reached the disk itself.
    fn sync(&mut self) -> io::Result<()>;
}
// @end

// @listing storage-file
impl Storage for File {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(buf)
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}
// @end

// @listing mem-file
/// A file that lives in memory. Clones share the same bytes, the way two handles
/// to one file on disk do.
#[derive(Clone, Default)]
pub struct MemFile(Rc<RefCell<Vec<u8>>>);

impl MemFile {
    pub fn new() -> MemFile {
        MemFile::default()
    }

    /// A copy of the file's contents.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.borrow().clone()
    }
}
// @end

// @listing storage-mem
impl Storage for MemFile {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let data = self.0.borrow();
        let start = offset as usize;
        match data.get(start..start + buf.len()) {
            Some(bytes) => buf.copy_from_slice(bytes),
            None => return Err(io::ErrorKind::UnexpectedEof.into()),
        }
        Ok(())
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        let mut data = self.0.borrow_mut();
        let (start, end) = (offset as usize, offset as usize + buf.len());
        if data.len() < end {
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(buf);
        Ok(())
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.0.borrow().len() as u64)
    }

    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}
// @end

// @listing temp-file
/// A file for a query's scratch work, which is deleted when it is dropped.
pub struct TempFile {
    file: File,
    path: String,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // If this fails the file stays behind, which wastes space and harms nothing.
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Storage for TempFile {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.read_at(offset, buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.file.write_at(offset, buf)
    }

    fn size(&self) -> io::Result<u64> {
        self.file.size()
    }

    /// Nothing in a temporary file has to survive a crash, so there is nothing to
    /// wait for.
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}
// @end

// @listing temp-space
/// Where a query's temporary files go.
#[derive(Debug, Clone, PartialEq)]
pub enum TempSpace {
    /// In memory, for tests and for databases that are in memory themselves.
    Memory,
    /// In files beside the database file, whose path this is.
    Beside(String),
}

/// A number for each temporary file, so that no two have the same name.
static NEXT_TEMP: AtomicU32 = AtomicU32::new(0);

impl TempSpace {
    /// A new, empty temporary file.
    pub fn create(&self) -> io::Result<Box<dyn Storage>> {
        match self {
            TempSpace::Memory => Ok(Box::new(MemFile::new())),
            TempSpace::Beside(database) => {
                let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let path = format!("{database}-temp{n}");
                let mut options = OpenOptions::new();
                options.read(true).write(true).create(true).truncate(true);
                Ok(Box::new(TempFile { file: options.open(&path)?, path }))
            }
        }
    }
}
// @end
