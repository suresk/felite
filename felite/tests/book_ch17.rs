//! The examples in Chapter 17: `cargo test --test book_ch17`.
use felite::{CrashDisk, Database, MemFile, Output, Pager, Storage, Tree, Value};
use felite::{FRAME_HEADER, FRAME_SIZE, LOG_HEADER, PAGE_SIZE};
use felite::{hex_dump, log_checksum, row_key};

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing helpers
/// A new database with a log, and handles to both files.
fn logged() -> (Pager, MemFile, MemFile) {
    let (file, log) = (MemFile::new(), MemFile::new());
    let pager = Pager::open_logged(Box::new(file.clone()), Box::new(log.clone()));
    (pager.unwrap(), file, log)
}

/// The big-endian number in four bytes.
fn word(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Frame `i` of a log: its page number, its commit mark, and all of its bytes.
fn frame(log: &[u8], i: usize) -> (u32, u32, &[u8]) {
    let bytes = &log[LOG_HEADER + i * FRAME_SIZE..][..FRAME_SIZE];
    (word(bytes), word(&bytes[4..]), bytes)
}
// @end

#[test]
fn a_first_commit() {
    // @listing first-commit
    let (mut pager, file, log) = logged();
    let tree = Tree::create(&mut pager).unwrap();
    tree.insert(&mut pager, &row_key(1), b"first").unwrap();
    pager.commit().unwrap();

    // Two pages changed, the header and the tree's root, so there are two frames.
    assert_eq!(pager.log_frames(), 2);
    assert_eq!(FRAME_SIZE, 4112);
    assert_eq!(log.bytes().len(), 24 + 2 * 4112);
    // And the database file has not been written at all.
    assert_eq!(file.bytes().len(), 0);
    // @end

    // @listing first-bytes
    let bytes = log.bytes();
    let expected = "\
0000  66 65 6c 69 74 65 2d 6c 00 00 10 00 00 00 00 01  |felite-l........|
0010  41 30 16 3e 1b fa b0 14 00 00 00 00 00 00 00 00  |A0.>............|
0020  89 ef 1a 25 0b 54 b6 de 66 65 6c 69 74 65 20 66  |...%.T..felite f|
0030  6f 72 6d 61 74 20 31 00 00 00 10 00 00 00 00 02  |ormat 1.........|
";
    assert_eq!(hex_dump(&bytes[..64]), expected);
    // @end

    // @listing first-frames
    let (page, commit, _) = frame(&bytes, 0);
    assert_eq!((page, commit), (0, 0));
    let (page, commit, _) = frame(&bytes, 1);
    assert_eq!((page, commit), (1, 1)); // the last frame carries the commit

    // A second commit that changes one page adds one frame.
    tree.insert(&mut pager, &row_key(2), b"second").unwrap();
    pager.commit().unwrap();
    let bytes = log.bytes();
    assert_eq!(pager.log_frames(), 3);
    let (page, commit, _) = frame(&bytes, 2);
    assert_eq!((page, commit), (1, 1));

    // A commit with nothing to save writes nothing.
    pager.commit().unwrap();
    assert_eq!(log.bytes().len(), bytes.len());
    // @end
}

#[test]
fn checksums_chain() {
    // @listing chain-test
    let (mut pager, _, log) = logged();
    let tree = Tree::create(&mut pager).unwrap();
    tree.insert(&mut pager, &row_key(1), b"first").unwrap();
    pager.commit().unwrap();
    let bytes = log.bytes();

    // The stored checksum of a frame, and the one computed from what precedes it.
    let stored = |frame: &[u8]| (word(&frame[8..]), word(&frame[12..]));
    let computed = |before: (u32, u32), frame: &[u8]| {
        log_checksum(log_checksum(before, &frame[..8]), &frame[FRAME_HEADER..])
    };
    let header = log_checksum((0, 0), &bytes[..16]);
    assert_eq!(header, (word(&bytes[16..]), word(&bytes[20..])));
    let (_, _, first) = frame(&bytes, 0);
    let (_, _, second) = frame(&bytes, 1);
    assert_eq!(computed(header, first), stored(first));
    assert_eq!(computed(stored(first), second), stored(second));

    // The second frame is intact, and still it is only valid after the first.
    assert_ne!(computed(header, second), stored(second));
    // One changed bit in the first frame's page, and neither frame checks out.
    let mut damaged = first.to_vec();
    damaged[2000] ^= 1;
    let sums = computed(header, &damaged);
    assert_ne!(sums, stored(first));
    assert_ne!(computed(sums, second), stored(second));
    // @end
}

// @listing bank
/// The bank of Section 16.8, in a file with no log: 600 accounts of 100 each.
fn bank_file() -> MemFile {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    db.execute(
        "CREATE TABLE accounts (
            id INTEGER PRIMARY KEY, branch TEXT, balance INTEGER
        )",
    )
    .unwrap();
    for id in 1..=600 {
        let branch = if id % 2 == 0 { "north" } else { "south" };
        let insert = format!("INSERT INTO accounts VALUES ({id}, '{branch}', 100)");
        db.execute(&insert).unwrap();
    }
    file
}

/// The sum of every balance.
fn total(db: &mut Database) -> i64 {
    let Output::Rows(result) = db.execute("SELECT balance FROM accounts").unwrap()
    else {
        panic!("not a query")
    };
    let balance = |row: &Vec<Value>| match row[0] {
        Value::Integer(n) => n,
        _ => 0,
    };
    result.rows.iter().map(balance).sum()
}
// @end

const BONUS: &str =
    "UPDATE accounts SET balance = balance + 5 WHERE branch = 'north'";

#[test]
fn the_database_file_has_not_changed() {
    // @listing unchanged-test
    let file = bank_file();
    let before = file.bytes();
    let log = MemFile::new();
    let pager = Pager::open_logged(Box::new(file.clone()), Box::new(log.clone()));
    let mut db = Database::with_pager(pager.unwrap()).unwrap();
    db.execute(BONUS).unwrap();

    // The statement changed four leaves. All four are in the log.
    assert_eq!(db.pager().log_frames(), 4);
    assert_eq!(total(&mut db), 61_500);
    // Not one byte of the database file is different.
    assert!(file.bytes() == before);

    // Someone who opens the file without the log sees the bank as it was.
    let mut stale = Database::open(Box::new(file.clone())).unwrap();
    assert_eq!(total(&mut stale), 60_000);
    // @end
}

#[test]
fn reading_through_the_log() {
    // @listing read-test
    let (mut pager, file, _) = logged();
    pager.set_capacity(4);
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..5000 {
        tree.insert(&mut pager, &row_key(id), &[id as u8; 40]).unwrap();
    }
    // 67 pages in the database, room for 4 in memory, and no commit yet. The other
    // pages are in the log, in frames that belong to no commit.
    assert_eq!(pager.page_count().unwrap(), 67);
    assert_eq!(pager.stats().evictions, 105);
    assert_eq!(pager.log_frames(), 105);

    // The pager finds them there.
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.entries, info.leaves), (5000, 65));
    assert_eq!(tree.get(&mut pager, &row_key(1234)).unwrap(), Some(vec![210; 40]));

    // The commit adds the pages still in memory. Nothing was ever read from the
    // database file or written to it, because nothing is in it.
    pager.commit().unwrap();
    assert_eq!(pager.log_frames(), 109);
    assert_eq!((pager.stats().reads, pager.stats().writes), (0, 0));
    assert_eq!(file.bytes().len(), 0);
    // @end
}

#[test]
fn a_commit_is_one_sync() {
    // @listing sync-test
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let pager = Pager::open_logged(Box::new(disk.clone()), Box::new(log.clone()));
    let mut db = Database::with_pager(pager.unwrap()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, note TEXT)").unwrap();
    // The log's header, with a sync of its own, and then three frames and a sync.
    assert_eq!((disk.writes(), disk.syncs()), (4, 2));

    // A row too long for a page: the leaf, an overflow page and the header change.
    let long = "x".repeat(3000);
    db.execute(&format!("INSERT INTO t (note) VALUES ('{long}')")).unwrap();
    assert_eq!((disk.writes(), disk.syncs()), (7, 3));
    // Everything written is synced, and all of it went to the log.
    assert_eq!((disk.pending(), log.pending()), (0, 0));
    assert_eq!(disk.size().unwrap(), 0);
    assert_eq!(log.size().unwrap(), 24 + 6 * 4112);
    // @end
}

// @listing workload
/// The 120 statements of Section 16.9.
fn workload() -> Vec<String> {
    let mut sql = vec![
        "CREATE TABLE flights (id INTEGER PRIMARY KEY, note TEXT, delay INTEGER)"
            .to_string(),
    ];
    let (mut state, mut extra) = (16, false);
    while sql.len() < 120 {
        let r = next(&mut state);
        let delay = r >> 8 & 1023;
        sql.push(match r % 20 {
            0..=11 => {
                // One note in sixteen is too long for a page, so it spills.
                let length =
                    if r >> 20 & 15 == 0 { 3000 } else { 20 + (r >> 24 & 255) };
                let note = "x".repeat(length as usize);
                format!(
                    "INSERT INTO flights (note, delay) VALUES ('{note}', {delay})"
                )
            }
            12..=14 => {
                let end = delay + 200;
                format!(
                    "DELETE FROM flights WHERE delay >= {delay} AND delay < {end}"
                )
            }
            15..=17 => {
                format!("UPDATE flights SET delay = delay + 1 WHERE delay >= {delay}")
            }
            _ => {
                extra = !extra;
                let create = "CREATE TABLE extra (a INTEGER, b TEXT)";
                (if extra { create } else { "DROP TABLE extra" }).to_string()
            }
        });
    }
    sql
}
// @end

#[test]
fn the_log_after_a_workload() {
    // @listing size-test
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let pager = Pager::open_logged(Box::new(disk.clone()), Box::new(log.clone()));
    let mut db = Database::with_pager(pager.unwrap()).unwrap();
    for statement in &workload() {
        db.execute(statement).unwrap();
    }
    db.check().unwrap();

    // The same 192 page writes as in Chapter 16, each one now a frame in the log.
    assert_eq!(db.pager().log_frames(), 192);
    // One sync for the log's header, and one for each statement that changed a page.
    assert_eq!(disk.syncs(), 111);
    // The database is 8 pages. Its log is 24 times that size, and its file is empty.
    assert_eq!(db.pager().page_count().unwrap(), 8);
    assert_eq!(log.size().unwrap(), 24 + 192 * 4112);
    assert_eq!(log.size().unwrap() / (8 * PAGE_SIZE as u64), 24);
    assert_eq!(disk.size().unwrap(), 0);
    // @end
}
