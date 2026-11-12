//! Tests for the Chapter 17 challenges: `cargo test --test challenges_ch17`.
use felite::{CrashDisk, Database, MemFile, Page, Pager, Tree, row_key};
use felite::{FRAME_SIZE, LEAF, LOG_HEADER};

fn logged() -> (Database, MemFile) {
    let log = MemFile::new();
    let pager = Pager::open_logged(Box::new(MemFile::new()), Box::new(log.clone()));
    (Database::with_pager(pager.unwrap()).unwrap(), log)
}

// @challenge 1 2
use felite::list_frames;

// @challenge 3
use felite::{apply_delta, page_delta};

// @challenge 4
use felite::NoSync;

// @challenge 5
use felite::complete_commits;

// @challenge 1
#[test]
fn predict_the_log() {
    let (mut db, log) = logged();
    let frames = |log: &MemFile| list_frames(&log.bytes()).unwrap();
    db.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)").unwrap();
    // The header, the schema table's page, the new table's root.
    assert_eq!(frames(&log), [(0, false), (1, false), (2, true)]);
    db.execute("INSERT INTO notes (body) VALUES ('one')").unwrap();
    db.execute("INSERT INTO notes (body) VALUES ('two')").unwrap();
    // The same page twice. The log keeps both versions.
    assert_eq!(frames(&log)[3..], [(2, true), (2, true)]);
    db.execute("SELECT * FROM notes").unwrap();
    db.execute("DELETE FROM notes WHERE id = 99").unwrap();
    assert_eq!(frames(&log).len(), 5);
    db.execute("DROP TABLE notes").unwrap();
    // The header, the schema table's page, and page 2, which is now a free page.
    assert_eq!(frames(&log)[5..], [(0, false), (1, false), (2, true)]);
    assert_eq!(log.bytes().len(), LOG_HEADER + 8 * FRAME_SIZE);
}

// @challenge 2
#[test]
fn reading_a_log() {
    let (mut db, log) = logged();
    db.execute("CREATE TABLE t (a INTEGER)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();
    let bytes = log.bytes();
    assert_eq!(list_frames(&bytes).unwrap().len(), 4);
    // Half a frame at the end is not a frame.
    assert_eq!(list_frames(&bytes[..bytes.len() - 100]).unwrap().len(), 3);
    assert!(list_frames(&bytes[..10]).is_err());
    assert!(list_frames(&[0; 5000]).is_err());
}

// @challenge 3
#[test]
fn a_page_as_its_changes() {
    // A leaf with 50 rows, before and after one more is added.
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..50 {
        tree.insert(&mut pager, &row_key(id), &[id as u8; 40]).unwrap();
    }
    let mut old = Page::new();
    old.bytes_mut().copy_from_slice(pager.page(tree.root()).unwrap().bytes());
    tree.insert(&mut pager, &row_key(50), &[50; 40]).unwrap();
    let new = pager.page(tree.root()).unwrap();
    assert_eq!(new.kind(), LEAF);

    let delta = page_delta(&old, new);
    // Three runs: the cell count and the start of the cells, which are neighbors
    // in the header; the new slot; and the new cell of 49 bytes. Each run costs 4
    // bytes besides, which makes 68 bytes where the page is 4,096.
    assert_eq!(delta.len(), 3 + 4 + 49 + 3 * 4);
    assert_eq!(delta.len(), 68);
    apply_delta(&mut old, &delta).unwrap();
    assert!(old.bytes() == new.bytes());

    // No change, no delta. And a delta that runs off the page is refused.
    assert!(page_delta(&old, new).is_empty());
    assert!(apply_delta(&mut old, &[0x0f, 0xff, 0, 2, 1, 1]).is_err());
    assert!(apply_delta(&mut old, &[0, 0, 0]).is_err());
}

// @challenge 4
#[test]
fn a_storage_that_never_syncs() {
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let pager = Pager::open_logged(
        Box::new(NoSync(disk.clone())),
        Box::new(NoSync(log.clone())),
    );
    let mut db = Database::with_pager(pager.unwrap()).unwrap();
    db.execute("CREATE TABLE t (a INTEGER)").unwrap();
    db.execute("INSERT INTO t VALUES (1)").unwrap();
    // Two commits, and the disk was never asked to make anything safe.
    assert_eq!(disk.syncs(), 0);
    assert_eq!(log.pending(), 5);
    // So a power failure may leave nothing at all.
    assert_eq!(log.crash(|_| felite::Fate::Lost).bytes().len(), 0);
}

// @challenge 5
#[test]
fn commits_in_a_log_cut_short() {
    let (mut db, log) = logged();
    db.execute("CREATE TABLE t (a INTEGER)").unwrap(); // frames 0 to 2
    db.execute("INSERT INTO t VALUES (1)").unwrap(); // frame 3
    db.execute("CREATE TABLE u (a INTEGER)").unwrap(); // frames 4 to 6
    let bytes = log.bytes();
    let cut =
        |frames: usize| complete_commits(&bytes[..LOG_HEADER + frames * FRAME_SIZE]);
    assert_eq!(cut(7).unwrap(), (3, 7));
    // Cut anywhere inside the last commit, and it was never made.
    assert_eq!(cut(6).unwrap(), (2, 4));
    assert_eq!(cut(5).unwrap(), (2, 4));
    assert_eq!(cut(4).unwrap(), (2, 4));
    assert_eq!(cut(3).unwrap(), (1, 3));
    assert_eq!(cut(2).unwrap(), (0, 0));
    assert_eq!(cut(0).unwrap(), (0, 0));
}
