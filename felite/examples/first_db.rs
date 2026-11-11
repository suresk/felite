//! Chapter 1: makes `first.db`, a database of three pages.
//! `cargo run --example first_db`, then `cargo xtask dump first.db`.

use felite::{Page, Pager};
use std::fs::OpenOptions;

// @listing main
fn main() -> felite::Result<()> {
    let mut options = OpenOptions::new();
    let file = options.read(true).write(true).create(true).truncate(true);
    let mut pager = Pager::open(Box::new(file.open("first.db")?))?;

    let mut greeting = Page::new();
    greeting.bytes_mut()[..12].copy_from_slice(b"hello, pages");
    pager.allocate(Page::new())?;
    let n = pager.allocate(greeting)?;
    pager.page_mut(n)?.put_u32(4092, 2026); // the last four bytes of the page
    pager.flush()?;

    println!("first.db has {} pages", pager.page_count()?);
    Ok(())
}
// @end
