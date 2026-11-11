//! The examples in Chapter 9: `cargo test --test book_ch09`.
use felite::{
    Cursor, DEFAULT_FRAMES, MemFile, Page, Pager, PoolStats, Storage, Tree, row_key,
};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing table
/// A flushed file holding one table of 100,000 rows of 40 bytes, and its tree.
/// The file has 1,307 pages: a header, 1,299 leaves and 7 interior pages.
fn big_table() -> (MemFile, Tree) {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..100_000 {
        tree.insert(&mut pager, &row_key(id), &[id as u8; 40]).unwrap();
    }
    pager.flush().unwrap();
    (file, tree)
}
// @end

#[test]
fn how_much_the_pager_was_holding() {
    // @listing before
    let (file, tree) = big_table();
    let mut pager = Pager::open(Box::new(file)).unwrap();
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    while cursor.next(&mut pager).unwrap().is_some() {}
    // Every page the scan touched is still in memory: 5.3 MB for a 5.4 MB file.
    assert_eq!(pager.pages_in_memory(), 1302);
    assert_eq!(1302 * 4096, 5_332_992);
    assert_eq!(DEFAULT_FRAMES, 2000);
    // @end
}

#[test]
fn memory_has_a_limit() {
    // @listing bounded
    let (file, tree) = big_table();
    let mut pager = Pager::with_capacity(Box::new(file), 8).unwrap();
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let mut rows = 0;
    while cursor.next(&mut pager).unwrap().is_some() {
        rows += 1;
        assert!(pager.pages_in_memory() <= 8);
    }
    assert_eq!(rows, 100_000);
    assert_eq!(pager.pages_in_memory(), 8); // 32 kB, for the same scan
    // @end
}

#[test]
fn the_clock_at_work() {
    // @listing clock-test
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    for _ in 0..6 {
        pager.allocate(Page::new()).unwrap(); // pages 1 to 6
    }
    pager.flush().unwrap();

    // Four frames. The header takes one, which leaves three for other pages.
    let mut pager = Pager::with_capacity(Box::new(file), 4).unwrap();
    // Asks for a page, and says whether it had to be read from the file.
    let mut read = |n: u32| {
        let before = pager.stats().reads;
        pager.page(n).unwrap();
        pager.stats().reads > before
    };
    assert!(read(1) && read(2) && read(3)); // the pool is now full
    assert!(!read(1)); // already in memory
    assert!(read(4)); // something has to go, and it is page 2
    assert!(!read(1) && !read(3));
    assert!(read(2)); // page 2 comes back, and page 4 goes
    assert!(!read(1) && !read(3));
    assert!(read(4));
    // @end
}

#[test]
fn dirty_pages_are_written_on_the_way_out() {
    // @listing dirty-eviction
    let file = MemFile::new();
    let mut pager = Pager::with_capacity(Box::new(file.clone()), 4).unwrap();
    for n in 1..=20u32 {
        let mut page = Page::new();
        page.put_u32(0, n * 1000);
        assert_eq!(pager.allocate(page).unwrap(), n);
    }
    // No flush has happened, yet most of the pages are already in the file,
    // because there was nowhere else for them to go.
    let stats = pager.stats();
    assert_eq!((stats.reads, pager.pages_in_memory()), (0, 4));
    assert!(file.size().unwrap() > 0);
    assert_eq!(stats.writes, stats.evictions); // every page dropped was dirty

    // Nothing was lost on the way.
    for n in 1..=20u32 {
        assert_eq!(pager.page(n).unwrap().u32_at(0), n * 1000);
    }
    // Reading them back dropped pages too, and a page that has not changed since
    // it was read is dropped without being written.
    let after = pager.stats();
    assert!(after.evictions > stats.evictions);
    pager.flush().unwrap();
    assert_eq!(file.size().unwrap(), 21 * 4096);
    // @end
}

/// Runs a workload of inserts and deletes, with rows of all sizes, on a pager with
/// `frames` frames, and returns the bytes of the flushed file and the statistics.
// @listing workload
fn workload(frames: usize) -> (Vec<u8>, PoolStats) {
    let file = MemFile::new();
    let mut pager = Pager::with_capacity(Box::new(file.clone()), frames).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    let mut model: BTreeMap<i64, Vec<u8>> = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..3000 {
        let random = next(&mut state);
        let id = (random >> 8) as i64 % 400;
        if random.is_multiple_of(3) {
            let was_there = model.remove(&id).is_some();
            assert_eq!(tree.delete_row(&mut pager, id).unwrap(), was_there);
        } else if let Entry::Vacant(slot) = model.entry(id) {
            // One row in ten is long enough to need overflow pages.
            let limit = if random.is_multiple_of(10) { 20_000 } else { 300 };
            let row = vec![id as u8; (random >> 20) as usize % limit];
            tree.insert_row(&mut pager, id, &row).unwrap();
            slot.insert(row);
        }
    }
    tree.check(&mut pager).unwrap();
    for (id, row) in &model {
        assert_eq!(tree.get_row(&mut pager, *id).unwrap().as_ref(), Some(row));
    }
    pager.flush().unwrap();
    (file.bytes(), pager.stats())
}
// @end

#[test]
fn nothing_above_the_pager_can_tell() {
    // @listing same-file
    let (roomy, stats) = workload(DEFAULT_FRAMES);
    assert_eq!((stats.reads, stats.evictions), (0, 0)); // everything fitted in memory
    assert_eq!(roomy.len() / 4096, 118); // pages in the file

    for frames in [1, 2, 3, 8, 64] {
        let (cramped, stats) = workload(frames);
        assert!(stats.evictions > 0);
        // The same workload leaves the same file, byte for byte.
        assert!(cramped == roomy, "{frames} frames");
    }
    // @end
}

#[test]
fn a_scan_reads_each_page_once() {
    // @listing scan
    let (file, tree) = big_table();
    // 128 frames for a file of 1,307 pages: a tenth of it.
    let mut pager = Pager::with_capacity(Box::new(file), 128).unwrap();
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    while cursor.next(&mut pager).unwrap().is_some() {}
    let stats = pager.stats();
    // The header, two interior pages on the way down, and the 1,299 leaves.
    assert_eq!(stats.reads, 1302);
    assert_eq!(stats.evictions, 1302 - 128);
    assert_eq!(stats.writes, 0); // nothing was changed, so nothing was written
    // @end
}

#[test]
fn reads_per_lookup() {
    // @listing lookups
    let (file, tree) = big_table();
    // (frames, pages read by 20,000 random lookups)
    let mut results = Vec::new();
    for frames in [4, 8, 16, 32, 128, 512, 2000] {
        let mut pager = Pager::with_capacity(Box::new(file.clone()), frames).unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..20_000 {
            let id = (next(&mut state) % 100_000) as i64;
            assert!(tree.get(&mut pager, &row_key(id)).unwrap().is_some());
        }
        results.push((frames, pager.stats().reads));
    }
    assert_eq!(
        results,
        [
            (4, 36_172),
            (8, 29_643),
            (16, 21_667),
            (32, 19_912),
            (128, 18_203),
            (512, 12_417),
            (2000, 1307)
        ]
    );
    // @end
}
