//! A page: the unit the database file is read and written in.

// @listing page
pub const PAGE_SIZE: usize = 4096;

/// A page's position in the file. Page 0 is the header.
pub type PageNo = u32;

#[derive(Clone)]
pub struct Page(Box<[u8; PAGE_SIZE]>);
// @end

impl Page {
    // @listing page-new
    /// A page of zeros.
    pub fn new() -> Page {
        Page(Box::new([0; PAGE_SIZE]))
    }

    pub fn bytes(&self) -> &[u8; PAGE_SIZE] {
        &self.0
    }

    pub fn bytes_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
        &mut self.0
    }
    // @end

    // @listing page-integers
    pub fn u16_at(&self, at: usize) -> u16 {
        u16::from_be_bytes([self.0[at], self.0[at + 1]])
    }

    pub fn put_u16(&mut self, at: usize, value: u16) {
        self.0[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }

    pub fn u32_at(&self, at: usize) -> u32 {
        let b = &self.0[at..at + 4];
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }

    pub fn put_u32(&mut self, at: usize, value: u32) {
        self.0[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    // @end
}

// @listing page-default
impl Default for Page {
    fn default() -> Page {
        Page::new()
    }
}
// @end
