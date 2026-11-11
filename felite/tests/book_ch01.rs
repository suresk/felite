//! The examples in Chapter 1: `cargo test --test book_ch01`.
use felite::{Error, MAGIC, MemFile, Page, Pager, Storage, hex_dump};
use std::fs::OpenOptions;

#[test]
fn integers_are_big_endian() {
    // @listing big-endian
    let mut page = Page::new();
    page.put_u32(8, 4096);
    assert_eq!(page.bytes()[8..12], [0x00, 0x00, 0x10, 0x00]);
    assert_eq!(page.u32_at(8), 4096);
    // The same four bytes, read as two 16-bit numbers.
    assert_eq!((page.u16_at(8), page.u16_at(10)), (0, 4096));
    // @end
}

#[test]
fn a_file_in_memory() {
    // @listing mem-file-test
    let mut file = MemFile::new();
    file.write_at(10, &[1, 2, 3]).unwrap();
    assert_eq!(file.size().unwrap(), 13); // ten zeros, then our three bytes

    let mut buf = [0; 4];
    file.read_at(9, &mut buf).unwrap();
    assert_eq!(buf, [0, 1, 2, 3]);
    assert!(file.read_at(10, &mut buf).is_err()); // there is no byte 13

    let mut other = file.clone(); // a second handle to the same file
    other.write_at(0, &[9]).unwrap();
    assert_eq!(file.bytes()[0], 9);
    // @end
}

#[test]
fn a_new_database() {
    // @listing new-database
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    assert_eq!(pager.page_count().unwrap(), 1);
    assert_eq!(file.size().unwrap(), 0); // nothing is written until we flush

    pager.flush().unwrap();
    let bytes = file.bytes();
    assert_eq!(bytes.len(), 4096);
    assert_eq!(&bytes[..16], MAGIC);
    assert_eq!(bytes[16..24], [0, 0, 0x10, 0, 0, 0, 0, 1]); // 4,096 and 1
    // @end
}

#[test]
fn pages_are_found_by_multiplying() {
    // @listing allocate-test
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let mut page = Page::new();
    page.bytes_mut()[..5].copy_from_slice(b"hello");
    assert_eq!(pager.allocate(Page::new()).unwrap(), 1);
    assert_eq!(pager.allocate(page).unwrap(), 2);
    assert_eq!(pager.page_count().unwrap(), 3);
    pager.flush().unwrap();

    let bytes = file.bytes();
    assert_eq!(bytes.len(), 12_288); // three pages
    assert_eq!(&bytes[8192..8197], b"hello"); // page 2 starts at 2 * 4,096
    // @end
}

#[test]
fn reopening() {
    // @listing reopen
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let n = pager.allocate(Page::new()).unwrap();
    pager.page_mut(n).unwrap().put_u32(100, 2026);
    pager.flush().unwrap();
    drop(pager);

    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    assert_eq!(pager.page_count().unwrap(), 2);
    assert_eq!(pager.page(n).unwrap().u32_at(100), 2026);
    // @end
}

#[test]
fn changes_wait_for_flush() {
    // @listing no-flush
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    pager.flush().unwrap();
    pager.allocate(Page::new()).unwrap();
    drop(pager); // without a flush

    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    assert_eq!(pager.page_count().unwrap(), 1); // the new page never reached the file
    // @end
}

#[test]
fn files_that_are_not_ours() {
    // @listing not-ours
    let message = |bytes: &[u8]| {
        let mut file = MemFile::new();
        file.write_at(0, bytes).unwrap();
        match Pager::open(Box::new(file)) {
            Err(error) => error.to_string(),
            Ok(_) => "it opened".to_string(),
        }
    };
    assert_eq!(
        message(b"hello"),
        "corrupt database: the file is 5 bytes, shorter than one page"
    );
    assert_eq!(
        message(&[b'x'; 5000]),
        "corrupt database: this is not a felite database"
    );
    let mut header = [0; 4096];
    header[..16].copy_from_slice(MAGIC);
    header[16..20].copy_from_slice(&8192u32.to_be_bytes());
    assert_eq!(
        message(&header),
        "corrupt database: its pages are 8192 bytes, not 4096"
    );
    // @end
}

#[test]
fn a_page_past_the_end() {
    // @listing past-the-end
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    pager.allocate(Page::new()).unwrap();
    let Err(error) = pager.page(7) else {
        panic!("there is no page 7");
    };
    assert!(matches!(error, Error::Corrupt(_)));
    assert_eq!(
        error.to_string(),
        "corrupt database: page 7 is past the end of a 2-page file"
    );
    // @end
}

#[test]
fn a_real_file() {
    // @listing real-file
    let name = format!("felite-ch01-{}.db", std::process::id());
    let path = std::env::temp_dir().join(name);
    let open = || {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false).open(&path)
    };

    let mut pager = Pager::open(Box::new(open().unwrap())).unwrap();
    let n = pager.allocate(Page::new()).unwrap();
    pager.page_mut(n).unwrap().put_u32(100, 2026);
    pager.flush().unwrap();
    drop(pager); // closes the file

    assert_eq!(std::fs::metadata(&path).unwrap().len(), 8192);
    let mut pager = Pager::open(Box::new(open().unwrap())).unwrap();
    assert_eq!(pager.page(n).unwrap().u32_at(100), 2026);
    drop(pager);
    std::fs::remove_file(&path).unwrap();
    // @end
}

#[test]
fn the_header_page_as_hex() {
    // @listing dump-test
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    pager.allocate(Page::new()).unwrap();
    pager.allocate(Page::new()).unwrap();
    pager.flush().unwrap();
    let expected = "\
0000  66 65 6c 69 74 65 20 66 6f 72 6d 61 74 20 31 00  |felite format 1.|
0010  00 00 10 00 00 00 00 03 00 00 00 00 00 00 00 00  |................|
0020  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  |................|
*
";
    assert_eq!(hex_dump(&file.bytes()[..4096]), expected);
    // @end
}
