//! Tests for the Chapter 8 challenges: `cargo test --test challenges_ch08`.
use felite::{MemFile, Pager, Tree, row_key, store};
use felite::{account_rows, load_range, update_row};

fn text(length: usize, seed: usize) -> Vec<u8> {
    (0..length).map(|i| b'a' + ((i + seed) % 26) as u8).collect()
}

#[test]
fn predict_overflow_pages() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let mut pages = |length: usize| {
        let before = pager.page_count().unwrap();
        store(&mut pager, &text(length, 0)).unwrap();
        pager.page_count().unwrap() - before
    };
    assert_eq!(pages(500), 0);
    assert_eq!(pages(1000), 1);
    assert_eq!(pages(983 + 4088), 1);
    assert_eq!(pages(983 + 4088 + 1), 2);
    assert_eq!(pages(1_000_000), 245);
    // The megabyte row puts 999,017 bytes on 245 pages of 4,096. The other 4,503
    // bytes of those pages are links, unused header bytes, and the empty end of
    // the last page.
    assert_eq!(245 * 4096 - (1_000_000 - 983), 4503);
    assert_eq!((245 * 8, 245 * 4088 - 999_017), (1960, 2543));
    // Rows that have spilled make cells of exactly 1,000 bytes: four to a leaf.
    assert_eq!(4084 / (1000 + 4), 4);
}

#[test]
fn part_of_a_large_row() {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let row = text(100_000, 3);
    let stored = store(&mut pager, &row).unwrap();
    pager.flush().unwrap();

    let read = |start: usize, length: usize| {
        let mut pager = Pager::open(Box::new(file.clone())).unwrap();
        let bytes = load_range(&mut pager, &stored, start, length).unwrap();
        (bytes, pager.pages_in_memory() - 1)
    };
    assert_eq!(read(0, 100), (row[..100].to_vec(), 0)); // all of it is in the tree's part
    assert_eq!(read(900, 200), (row[900..1100].to_vec(), 1)); // across the first boundary
    assert_eq!(read(983, 1), (row[983..984].to_vec(), 1));
    assert_eq!(read(5000, 10_000), (row[5000..15_000].to_vec(), 4));
    assert_eq!(read(99_990, 500), (row[99_990..].to_vec(), 25)); // the end needs every page
    assert_eq!(read(200_000, 10), (vec![], 0)); // past the end
    assert_eq!(read(0, usize::MAX).0, row);

    let short = text(300, 0);
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    assert_eq!(load_range(&mut pager, &short, 100, 50).unwrap(), short[100..150]);
    assert_eq!(load_range(&mut pager, &short, 290, 50).unwrap(), short[290..]);
    assert_eq!(load_range(&mut pager, &short, 400, 50).unwrap(), b"");
}

#[test]
fn replacing_a_row() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let table = Tree::create(&mut pager).unwrap();
    table.insert_row(&mut pager, 1, b"short").unwrap();
    table.insert_row(&mut pager, 2, &text(30_000, 0)).unwrap();
    let pages = pager.page_count().unwrap();

    assert!(update_row(&table, &mut pager, 1, &text(30_000, 5)).unwrap()); // short to long
    assert!(update_row(&table, &mut pager, 2, b"now short").unwrap()); // long to short
    assert!(!update_row(&table, &mut pager, 3, b"nobody").unwrap());
    assert_eq!(table.get_row(&mut pager, 1).unwrap().unwrap(), text(30_000, 5));
    assert_eq!(table.get_row(&mut pager, 2).unwrap().unwrap(), b"now short");
    assert_eq!(table.get(&mut pager, &row_key(3)).unwrap(), None);
    account_rows(&mut pager, &[table]).unwrap();
    // One long row before and one after, so the file grew only while both existed.
    assert_eq!(pager.page_count().unwrap(), pages + 8);
    assert_eq!(pager.free_count().unwrap(), 8);

    // Long to longer and back: the pages go round through the free list.
    assert!(update_row(&table, &mut pager, 1, &text(60_000, 1)).unwrap());
    assert!(update_row(&table, &mut pager, 1, &text(30_000, 2)).unwrap());
    account_rows(&mut pager, &[table]).unwrap();
}

#[test]
fn overflow_pages_are_accounted_for() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let a = Tree::create(&mut pager).unwrap();
    let b = Tree::create(&mut pager).unwrap();
    for id in 0..300 {
        let length = if id % 7 == 0 { 9000 } else { 60 };
        a.insert_row(&mut pager, id, &text(length, id as usize)).unwrap();
        b.insert_row(&mut pager, id, &text(2000, id as usize)).unwrap();
    }
    account_rows(&mut pager, &[a, b]).unwrap();
    for id in (0..300).step_by(3) {
        a.delete_row(&mut pager, id).unwrap();
    }
    account_rows(&mut pager, &[a, b]).unwrap();

    // Deleting a spilled row with the plain delete leaks its chain.
    assert!(a.delete(&mut pager, &row_key(7)).unwrap());
    let error = account_rows(&mut pager, &[a, b]).unwrap_err().to_string();
    assert_eq!(error, "corrupt database: 2 pages belong to nothing");
}
