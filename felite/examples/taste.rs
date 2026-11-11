//! The snippet in the README, kept here so that it is compiled and run.
use felite::{MemFile, Page, Pager, Value, decode, encode};

fn main() {
    // A database in memory, with one page holding one row.
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let mut page = Page::new();
    page.init_slotted(1);
    let row = [Value::Integer(1432), Value::Text("SLC".to_string()), Value::Null];
    page.insert_cell(0, &encode(&row)).unwrap();
    let n = pager.allocate(page).unwrap();

    let stored = pager.page(n).unwrap().cell(0).unwrap();
    assert_eq!(decode(stored).unwrap(), row);
    println!("one row stored on page {n} and read back");
}
