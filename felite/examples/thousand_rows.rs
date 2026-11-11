//! Makes `rows.db`, a tree of a thousand rows, to look at with `cargo xtask cells`.
use felite::{Pager, Tree, row_key};
use std::fs::OpenOptions;

// @listing main
fn main() -> felite::Result<()> {
    let mut options = OpenOptions::new();
    let file =
        options.read(true).write(true).create(true).truncate(true).open("rows.db")?;
    let mut pager = Pager::open(Box::new(file))?;
    let tree = Tree::create(&mut pager)?;
    for id in 1..=1000 {
        tree.insert(&mut pager, &row_key(id), &[id as u8; 40])?;
    }
    pager.flush()?;
    let info = tree.check(&mut pager)?;
    println!("rows.db: {} rows on {} pages", info.entries, pager.page_count()?);
    Ok(())
}
// @end
