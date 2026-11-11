//! Tests for the Chapter 9 challenges: `cargo test --test challenges_ch09`.
use felite::{Counting, Error, FailAfter, MemFile, Page, Pager, SharedPool};
use felite::{clock_misses, lru_misses};

/// A flushed file of `pages` pages. Page `n` holds the number `n` in its first
/// four bytes.
fn numbered(pages: u32) -> MemFile {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    for n in 1..pages {
        let mut page = Page::new();
        page.put_u32(0, n);
        pager.allocate(page).unwrap();
    }
    pager.flush().unwrap();
    file
}

#[test]
fn predict_reads_and_evictions() {
    let mut pager = Pager::with_capacity(Box::new(numbered(10)), 4).unwrap();
    for n in [1, 2, 3, 1, 4, 5, 1, 2, 6, 1] {
        pager.page(n).unwrap();
    }
    let stats = pager.stats();
    // The header, then seven misses among the ten requests.
    assert_eq!(stats.reads, 1 + 7);
    assert_eq!(stats.evictions, 8 - 4);
    assert_eq!(stats.writes, 0);
    // The same answer from the simulator, which has no header to carry: three
    // frames, the same requests.
    assert_eq!(clock_misses(3, &[1, 2, 3, 1, 4, 5, 1, 2, 6, 1], false), 7);
}

#[test]
fn one_frame_is_enough() {
    let file = MemFile::new();
    let mut pager = Pager::with_capacity(Box::new(file.clone()), 1).unwrap();
    let tree = felite::Tree::create(&mut pager).unwrap();
    for id in 0..2000 {
        tree.insert(&mut pager, &felite::row_key(id), &[id as u8; 40]).unwrap();
        assert_eq!(pager.pages_in_memory(), 1);
    }
    assert_eq!(tree.check(&mut pager).unwrap().entries, 2000);
    let stats = pager.stats();
    // It works, at a price: six page reads for every row inserted, where a pool
    // with room for the tree does none.
    assert_eq!((stats.reads, stats.writes), (12_130, 2104));
}

#[test]
fn clock_and_lru_on_the_same_traces() {
    // A loop over five pages in four frames is the worst case for both.
    let looping: Vec<u32> = (0..100).map(|i| i % 5 + 1).collect();
    assert_eq!(lru_misses(4, &looping), 100);
    assert_eq!(clock_misses(4, &looping, false), 100);

    // Three hot pages used constantly, and a long scan passing through.
    let mut trace = Vec::new();
    for page in 100..600 {
        trace.extend([1, 2, 3, page]);
    }
    let all = (
        lru_misses(8, &trace),
        clock_misses(8, &trace, false),
        clock_misses(8, &trace, true),
    );
    assert_eq!(all, (503, 503, 506));

    // The same, but the hot pages are used only once for every five of the scan,
    // so the hot pages and one round of the scan fill the pool exactly.
    let mut trace = Vec::new();
    for page in (100..600).step_by(5) {
        trace.extend([1, 2, 3]);
        trace.extend(page..page + 5);
    }
    let all = (
        lru_misses(8, &trace),
        clock_misses(8, &trace, false),
        clock_misses(8, &trace, true),
    );
    assert_eq!(all, (503, 503, 650));
}

#[test]
fn a_failed_eviction_loses_nothing() {
    // Two writes allowed, then the disk is full.
    let (counting, counts) = Counting::new(MemFile::new());
    let (storage, allowance) = FailAfter::new(counting, 2);
    let mut pager = Pager::with_capacity(Box::new(storage), 3).unwrap();
    let mut failed_at = 0;
    for n in 1..=10u32 {
        let mut page = Page::new();
        page.put_u32(0, n);
        match pager.allocate(page) {
            Ok(_) => {}
            Err(Error::Io(_)) => {
                failed_at = n;
                break;
            }
            Err(error) => panic!("{error}"),
        }
    }
    // Three frames hold the header and pages 1 and 2. Pages 3 and 4 each push a
    // dirty page out, and page 5 needs a third write that the disk refuses.
    assert_eq!((failed_at, counts.writes.get()), (5, 2));
    // The page that could not be written is still in memory, still dirty.
    assert_eq!(pager.pages_in_memory(), 3);
    allowance.set(100);
    pager.flush().unwrap();
    for n in 1..=4u32 {
        assert_eq!(pager.page(n).unwrap().u32_at(0), n);
    }
}

#[test]
fn a_pool_with_pins() {
    let mut pool = SharedPool::new(Box::new(numbered(10)), 2);
    let one = pool.get(1).unwrap();
    let two = pool.get(2).unwrap();
    assert_eq!((one.borrow().u32_at(0), two.borrow().u32_at(0)), (1, 2));

    // Both frames are held, so there is nowhere to put a third page.
    let Err(error) = pool.get(3) else { panic!("there was no room") };
    assert_eq!(
        error.to_string(),
        "corrupt database: every page in the pool is pinned"
    );
    drop(two);
    assert_eq!(pool.get(3).unwrap().borrow().u32_at(0), 3);

    // Two handles to one page, one writing and one reading, is a mistake that
    // only shows up when the program runs.
    let again = pool.get(1).unwrap();
    let writing = one.borrow_mut();
    assert!(again.try_borrow().is_err());
    drop(writing);
    assert!(again.try_borrow().is_ok());
}
