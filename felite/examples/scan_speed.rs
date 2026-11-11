//! Times a full scan of a million rows two ways: `cargo run --release --example
//! scan_speed`.
use felite::{Cursor, MemFile, Pager, Tree, parse_leaf, row_key};
use std::time::Instant;

// @listing main
fn main() -> felite::Result<()> {
    let mut pager = Pager::open(Box::new(MemFile::new()))?;
    let tree = Tree::create(&mut pager)?;
    for id in 0..1_000_000 {
        tree.insert(&mut pager, &row_key(id), &[id as u8; 40])?;
    }

    // With the cursor: every key and payload is copied into a new Vec.
    let start = Instant::now();
    let mut cursor = Cursor::first(&tree, &mut pager)?;
    let (mut rows, mut bytes) = (0, 0);
    while let Some((key, payload)) = cursor.next(&mut pager)? {
        rows += 1;
        bytes += key.len() + payload.len();
    }
    println!("cursor:    {rows} rows, {bytes} bytes, {:?}", start.elapsed());

    // By hand, a leaf at a time, looking at each cell where it lies in the page.
    // Bulk loading in key order put the first leaf on page 3.
    let start = Instant::now();
    let (mut rows, mut bytes, mut n) = (0, 0, 3);
    while n != 0 {
        let page = pager.page(n)?;
        for i in 0..page.cell_count() {
            let (key, payload) = parse_leaf(page.cell(i)?)?;
            rows += 1;
            bytes += key.len() + payload.len();
        }
        n = page.right();
    }
    println!("in place:  {rows} rows, {bytes} bytes, {:?}", start.elapsed());
    Ok(())
}
// @end
