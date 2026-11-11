//! What felite asks of a file, and two things that can answer.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

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
