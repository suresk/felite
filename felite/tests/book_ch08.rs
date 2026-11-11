//! The examples in Chapter 8: `cargo test --test book_ch08`.
use felite::{Cursor, Error, MemFile, OVERFLOW, Pager, SPILLED, Tree};
use felite::{load, release, row_key, store};
use std::collections::BTreeMap;

/// A new database in memory with one empty tree in it.
fn new_tree() -> (Pager, Tree) {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    (pager, tree)
}

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing text
/// `length` bytes that are easy to tell apart: the alphabet, over and over,
/// starting at a letter chosen by `seed`.
fn text(length: usize, seed: usize) -> Vec<u8> {
    (0..length).map(|i| b'a' + ((i + seed) % 26) as u8).collect()
}
// @end

#[test]
fn the_limit_we_are_lifting() {
    // @listing the-limit
    let (mut pager, tree) = new_tree();
    let error = tree.insert(&mut pager, &row_key(1), &text(5000, 0)).unwrap_err();
    assert!(matches!(error, Error::TooLarge(5009)));
    // @end
}

#[test]
fn short_rows_are_stored_as_they_are() {
    // @listing store-short
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let row = text(100, 0);
    let stored = store(&mut pager, &row).unwrap();
    assert_eq!(stored, row);
    assert_eq!(pager.page_count().unwrap(), 1); // no page was allocated
    assert_eq!(load(&mut pager, stored).unwrap(), row);

    // The longest row that stays whole is one byte short of the spilled form.
    assert_eq!(SPILLED, 991);
    let stored = store(&mut pager, &text(990, 0)).unwrap();
    assert_eq!((stored.len(), pager.page_count().unwrap()), (990, 1));
    // @end
}

#[test]
fn a_long_row_spills() {
    // @listing store-long
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let row = text(10_000, 0);
    let stored = store(&mut pager, &row).unwrap();

    // 991 bytes for the tree: 983 of the row, its length, and its first page.
    assert_eq!(stored.len(), 991);
    assert_eq!(stored[..983], row[..983]);
    assert_eq!(stored[983..987], 10_000u32.to_be_bytes());
    assert_eq!(stored[987..], 3u32.to_be_bytes());

    // The other 9,017 bytes need three overflow pages of 4,088 bytes. They were
    // written last page first, so the chain runs 3, 2, 1.
    assert_eq!(pager.page_count().unwrap(), 4);
    let links: Vec<(u8, u32)> = [3, 2, 1]
        .map(|n| {
            let page = pager.page(n).unwrap();
            (page.kind(), page.u32_at(4))
        })
        .to_vec();
    assert_eq!(links, [(OVERFLOW, 2), (OVERFLOW, 1), (OVERFLOW, 0)]);
    // Page 3 carries on exactly where the tree's part stopped.
    assert_eq!(pager.page(3).unwrap().bytes()[8..12], row[983..987]);

    assert_eq!(load(&mut pager, stored.clone()).unwrap(), row);
    release(&mut pager, &stored).unwrap();
    assert_eq!(pager.free_count().unwrap(), 3);
    // @end
}

#[test]
fn the_edge_between_short_and_long() {
    // @listing edge
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    // (row length, bytes stored in the tree, overflow pages)
    let mut cases = Vec::new();
    for length in [990, 991, 5071, 5072] {
        let before = pager.page_count().unwrap();
        let row = text(length, length);
        let stored = store(&mut pager, &row).unwrap();
        cases.push((length, stored.len(), pager.page_count().unwrap() - before));
        assert_eq!(load(&mut pager, stored).unwrap(), row);
    }
    assert_eq!(cases, [(990, 990, 0), (991, 991, 1), (5071, 991, 1), (5072, 991, 2)]);
    // @end
}

#[test]
fn rows_of_any_size_in_a_table() {
    // @listing rows-test
    let (mut pager, tree) = new_tree();
    tree.insert_row(&mut pager, 1, b"a short row").unwrap();
    tree.insert_row(&mut pager, 2, &text(100_000, 7)).unwrap();
    tree.insert_row(&mut pager, 3, b"another short row").unwrap();

    assert_eq!(tree.get_row(&mut pager, 1).unwrap().unwrap(), b"a short row");
    assert_eq!(tree.get_row(&mut pager, 2).unwrap().unwrap(), text(100_000, 7));
    assert_eq!(tree.get_row(&mut pager, 4).unwrap(), None);
    // The header, the tree's one leaf, and 25 overflow pages.
    assert_eq!((100_000usize - 983).div_ceil(4088), 25);
    assert_eq!(pager.page_count().unwrap(), 2 + 25);

    // The tree itself holds three ordinary cells and passes its check.
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.entries), (1, 3));
    assert_eq!(tree.get(&mut pager, &row_key(2)).unwrap().unwrap().len(), 991);

    assert!(tree.delete_row(&mut pager, 2).unwrap());
    assert!(!tree.delete_row(&mut pager, 2).unwrap());
    assert_eq!(pager.free_count().unwrap(), 25);
    assert_eq!(tree.get_row(&mut pager, 3).unwrap().unwrap(), b"another short row");
    // @end
}

#[test]
fn a_refused_row_leaves_nothing_behind() {
    // @listing no-leak
    let (mut pager, tree) = new_tree();
    tree.insert_row(&mut pager, 1, b"the first row 1").unwrap();
    let pages = pager.page_count().unwrap();

    let error = tree.insert_row(&mut pager, 1, &text(50_000, 0)).unwrap_err();
    assert!(matches!(error, Error::DuplicateKey));
    // The 12 overflow pages were written before the tree refused the row. They
    // are on the free list, not lost.
    assert_eq!(pager.page_count().unwrap(), pages + 12);
    assert_eq!(pager.free_count().unwrap(), 12);
    assert_eq!(tree.get_row(&mut pager, 1).unwrap().unwrap(), b"the first row 1");
    // @end
}

#[test]
fn what_a_large_row_costs_to_read() {
    // @listing cost
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..2000 {
        // Every hundredth row is 100,000 bytes. The rest are 40.
        let length = if id % 100 == 0 { 100_000 } else { 40 };
        tree.insert_row(&mut pager, id, &text(length, id as usize)).unwrap();
    }
    pager.flush().unwrap();
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves), (2, 32));
    assert_eq!(pager.page_count().unwrap(), 1 + 33 + 20 * 25);

    let read = |id: i64| {
        let mut pager = Pager::open(Box::new(file.clone())).unwrap();
        tree.get_row(&mut pager, id).unwrap().unwrap();
        pager.pages_in_memory() - 1
    };
    assert_eq!(read(150), 2); // a small row: the root and a leaf
    assert_eq!(read(100), 2 + 25); // a large one: and its whole chain

    // A scan that only looks at keys never touches an overflow page.
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let mut rows = 0;
    while cursor.next(&mut pager).unwrap().is_some() {
        rows += 1;
    }
    assert_eq!((rows, pager.pages_in_memory() - 1), (2000, 33));
    // @end
}

#[test]
fn rows_of_every_size_against_a_model() {
    // @listing model
    let (mut pager, tree) = new_tree();
    let mut model: BTreeMap<i64, Vec<u8>> = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for step in 1..=4000 {
        let random = next(&mut state);
        let id = (random >> 8) as i64 % 500;
        if !random.is_multiple_of(3) {
            // Mostly short rows, some just around the limit, a few very long.
            let length = match random % 10 {
                0 => 980 + (random >> 20) as usize % 20,
                1 => (random >> 20) as usize % 30_000,
                _ => (random >> 20) as usize % 300,
            };
            let row = text(length, step);
            match tree.insert_row(&mut pager, id, &row) {
                Ok(()) => assert!(model.insert(id, row).is_none()),
                Err(Error::DuplicateKey) => assert!(model.contains_key(&id)),
                Err(error) => panic!("{error}"),
            }
        } else {
            let was_there = model.remove(&id).is_some();
            assert_eq!(tree.delete_row(&mut pager, id).unwrap(), was_there);
        }
    }
    assert_eq!(tree.check(&mut pager).unwrap().entries, model.len());
    for (id, row) in &model {
        assert_eq!(tree.get_row(&mut pager, *id).unwrap().as_ref(), Some(row));
    }
    // Delete what is left. Every page but the header and the root is free again.
    for id in model.keys() {
        assert!(tree.delete_row(&mut pager, *id).unwrap());
    }
    assert_eq!(pager.free_count().unwrap(), pager.page_count().unwrap() - 2);
    // @end
}

#[test]
fn a_damaged_chain_is_an_error() {
    // @listing damaged-chain
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let stored = store(&mut pager, &text(10_000, 0)).unwrap(); // pages 3, 2, 1
    let message = |pager: &mut Pager, stored: &[u8]| {
        load(pager, stored.to_vec()).unwrap_err().to_string()
    };

    pager.page_mut(2).unwrap().put_u32(4, 0); // cut the chain after two pages
    assert_eq!(
        message(&mut pager, &stored),
        "corrupt database: page 0 is not an overflow page"
    );
    pager.page_mut(2).unwrap().put_u32(4, 3); // or loop it back to the start
    assert_eq!(
        message(&mut pager, &stored),
        "corrupt database: an overflow chain runs on to page 2"
    );
    // @end
}

#[test]
fn dropping_a_table_with_large_rows() {
    // @listing destroy-rows-test
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let table = Tree::create(&mut pager).unwrap();
    for id in 0..50 {
        table.insert_row(&mut pager, id, &text(20_000, id as usize)).unwrap();
    }
    table.destroy_rows(&mut pager).unwrap();
    assert_eq!(pager.free_count().unwrap(), pager.page_count().unwrap() - 1);
    // @end
}
