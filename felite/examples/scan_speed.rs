//! Times a full scan of a million rows three ways: `cargo run --release --example
//! scan_speed`.
use felite::{Cursor, MemFile, PageNo, Pager, Result, Tree, parse_leaf, row_key};
use std::time::Instant;

// @listing main
fn main() -> Result<()> {
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

    // By hand, along the leaves. Inserting in key order left the first leaf on
    // page 3, where the first root split moved it.
    let start = Instant::now();
    let (rows, bytes) = per_row(&mut pager, 3)?;
    println!("per row:   {rows} rows, {bytes} bytes, {:?}", start.elapsed());
    let start = Instant::now();
    let (rows, bytes) = per_leaf(&mut pager, 3)?;
    println!("in place:  {rows} rows, {bytes} bytes, {:?}", start.elapsed());
    Ok(())
}
// @end

// @listing walks
/// Reads every cell where it lies in the page, asking the pager for the leaf once
/// per row, as `Cursor::next` does, but copying nothing.
fn per_row(pager: &mut Pager, mut n: PageNo) -> Result<(usize, usize)> {
    let (mut rows, mut bytes) = (0, 0);
    while n != 0 {
        for i in 0..pager.page(n)?.cell_count() {
            let (key, payload) = parse_leaf(pager.page(n)?.cell(i)?)?;
            rows += 1;
            bytes += key.len() + payload.len();
        }
        n = pager.page(n)?.right();
    }
    Ok((rows, bytes))
}

/// The same, asking the pager for each leaf only once.
fn per_leaf(pager: &mut Pager, mut n: PageNo) -> Result<(usize, usize)> {
    let (mut rows, mut bytes) = (0, 0);
    while n != 0 {
        let page = pager.page(n)?;
        for i in 0..page.cell_count() {
            let (key, payload) = parse_leaf(page.cell(i)?)?;
            rows += 1;
            bytes += key.len() + payload.len();
        }
        n = page.right();
    }
    Ok((rows, bytes))
}
// @end
