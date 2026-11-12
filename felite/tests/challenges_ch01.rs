//! Tests for the Chapter 1 challenges: `cargo test --test challenges_ch01`.
use felite::PAGE_SIZE;
use felite::{Error, MemFile, Page, Pager, Storage};

/// A flushed database with `pages` pages, the header included.
fn database(pages: u32) -> MemFile {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    for _ in 1..pages {
        pager.allocate(Page::new()).unwrap();
    }
    pager.flush().unwrap();
    file
}

// @challenge 2 6
use felite::Counting;

// @challenge 3
use felite::open_checked;

// @challenge 4 5
use felite::ReadOnly;

// @challenge 6
use felite::FailAfter;

// @challenge 1
#[test]
fn predict_sizes_and_offsets() {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let mut last = 0;
    for _ in 0..5 {
        last = pager.allocate(Page::new()).unwrap();
    }
    pager.page_mut(last).unwrap().put_u32(0, 1_000_000);
    pager.flush().unwrap();
    assert_eq!(last, 5);
    assert_eq!(file.size().unwrap(), 24_576);
    assert_eq!(file.bytes()[20_480..20_484], [0x00, 0x0f, 0x42, 0x40]);
    // The largest file a 32-bit page number can address: 16 TiB.
    assert_eq!((1u64 << 32) * PAGE_SIZE as u64, 17_592_186_044_416);
}

// @challenge 2
#[test]
fn counting_reads_writes_and_syncs() {
    let (storage, counts) = Counting::new(database(3));
    let mut pager = Pager::open(Box::new(storage)).unwrap();
    assert_eq!(counts.reads.get(), 1); // the header
    pager.page(1).unwrap();
    pager.page(1).unwrap();
    pager.page(0).unwrap();
    // Page 1 was read once, and the header was already in memory.
    assert_eq!(counts.reads.get(), 2);
    pager.flush().unwrap();
    assert_eq!((counts.writes.get(), counts.syncs.get()), (0, 1));
    pager.page_mut(2).unwrap().put_u32(0, 7);
    pager.page_mut(2).unwrap().put_u32(4, 8);
    pager.flush().unwrap();
    assert_eq!((counts.reads.get(), counts.writes.get()), (3, 1));
    assert_eq!(counts.syncs.get(), 2);
}

#[test]
fn allocating_reads_nothing() {
    let (storage, counts) = Counting::new(MemFile::new());
    let mut pager = Pager::open(Box::new(storage)).unwrap();
    for _ in 0..10 {
        pager.allocate(Page::new()).unwrap();
    }
    pager.flush().unwrap();
    assert_eq!((counts.reads.get(), counts.writes.get()), (0, 11));
}

// @challenge 3
#[test]
fn open_checked_compares_the_length_with_the_header() {
    let file = database(3);
    assert!(open_checked(Box::new(file.clone())).is_ok());
    assert!(open_checked(Box::new(MemFile::new())).is_ok());

    // A fourth page that the header does not know about.
    file.clone().write_at(3 * 4096, &[0; 4096]).unwrap();
    let Err(error) = open_checked(Box::new(file)) else {
        panic!("the file is too long");
    };
    assert_eq!(
        error.to_string(),
        "corrupt database: the header says 3 pages, but the file is 16384 bytes"
    );
}

#[test]
fn open_checked_notices_a_truncated_file() {
    let whole = database(3).bytes();
    let mut cut = MemFile::new();
    cut.write_at(0, &whole[..10_000]).unwrap();
    assert!(Pager::open(Box::new(cut.clone())).is_ok());
    assert!(matches!(open_checked(Box::new(cut)), Err(Error::Corrupt(_))));
}

// @challenge 4
#[test]
fn read_only_allows_reads_and_refuses_writes() {
    let file = database(2);
    let before = file.bytes();
    let mut pager = Pager::open(Box::new(ReadOnly(file.clone()))).unwrap();
    assert_eq!(pager.page_count().unwrap(), 2);
    pager.page(1).unwrap();
    pager.flush().unwrap(); // nothing is dirty, so nothing is written

    pager.page_mut(1).unwrap().put_u32(0, 1);
    assert!(matches!(pager.flush(), Err(Error::Io(_))));
    assert_eq!(file.bytes(), before);
}

// @challenge 5
#[test]
fn close_flushes() {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    pager.allocate(Page::new()).unwrap();
    pager.close().unwrap();
    assert_eq!(file.size().unwrap(), 8192);

    let mut pager = Pager::open(Box::new(ReadOnly(file))).unwrap();
    pager.page_mut(1).unwrap().put_u32(0, 1);
    assert!(pager.close().is_err());
}

// @challenge 6
#[test]
fn a_failed_flush_can_be_retried() {
    let file = MemFile::new();
    let (inner, counts) = Counting::new(file.clone());
    let (storage, writes_left) = FailAfter::new(inner, 2);
    let mut pager = Pager::open(Box::new(storage)).unwrap();
    for _ in 0..4 {
        pager.allocate(Page::new()).unwrap();
    }
    // Five dirty pages, and the disk takes two of them.
    assert!(matches!(pager.flush(), Err(Error::Io(_))));
    assert_eq!(file.size().unwrap(), 8192);
    assert_eq!((counts.writes.get(), counts.syncs.get()), (2, 0));

    writes_left.set(100);
    pager.flush().unwrap();
    assert_eq!(file.size().unwrap(), 5 * 4096);
    assert_eq!(counts.writes.get(), 5); // only the three that were still dirty
}
