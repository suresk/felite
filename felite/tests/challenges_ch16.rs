//! Tests for the Chapter 16 challenges: `cargo test --test challenges_ch16`.
use felite::{CrashDisk, Database, Fate, Output, PAGE_SIZE, SECTOR, Storage, Value};
use std::collections::BTreeSet;

// @challenge 2
use felite::pages_written;

// @challenge 3
use felite::{byte_sum, checksum};

// @challenge 4
use felite::free_list_damage;

// @challenge 5
use felite::SyncEach;

// @challenge 1
#[test]
fn predict_what_survives() {
    let mut disk = CrashDisk::new();
    disk.write_at(0, &[b'A'; PAGE_SIZE]).unwrap();
    disk.sync().unwrap();
    disk.write_at(0, &[b'B'; PAGE_SIZE]).unwrap();
    disk.write_at(4096, &[b'C'; PAGE_SIZE]).unwrap();
    disk.write_at(0, &[b'D'; PAGE_SIZE]).unwrap();

    // Every way of keeping or losing the three unsynced writes.
    let mut files = BTreeSet::new();
    for choice in 0..8 {
        let keep =
            |i: usize| if choice >> i & 1 == 1 { Fate::Kept } else { Fate::Lost };
        let bytes = disk.crash(keep).bytes();
        // Describe the file by the first byte of each of its pages.
        let pages: String = bytes.chunks(PAGE_SIZE).map(|p| p[0] as char).collect();
        files.insert(pages);
    }
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    assert_eq!(files, ["A", "AC", "B", "BC", "D", "DC"]);
}

// @challenge 2
#[test]
fn writes_of_a_statement() {
    let disk = CrashDisk::new();
    let mut db = Database::open(Box::new(disk.clone())).unwrap();
    let mut written = |sql: &str| pages_written(&mut db, &disk, sql).unwrap();
    // The header, the schema table's page and the new table's root.
    assert_eq!(written("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"), 3);
    assert_eq!(written("INSERT INTO notes (body) VALUES ('short')"), 1);
    assert_eq!(written("SELECT * FROM notes"), 0);
    assert_eq!(written("DELETE FROM notes WHERE id = 99"), 0);
    // The leaf, one overflow page, and the header that counts the new page.
    let long = "y".repeat(3000);
    assert_eq!(written(&format!("INSERT INTO notes (body) VALUES ('{long}')")), 3);
    // The header, the schema table's page, and the two pages that are now free.
    assert_eq!(written("DROP TABLE notes"), 4);
}

// @challenge 3
#[test]
fn detecting_a_torn_page() {
    let old: Vec<u8> = (0..PAGE_SIZE).map(|i| (i * 7 % 251) as u8).collect();
    let new: Vec<u8> = (0..PAGE_SIZE).map(|i| (i * 13 % 241) as u8).collect();
    let expected = checksum((0, 0), &new);
    for sectors in 1..8 {
        let mut torn = old.clone();
        torn[..sectors * SECTOR].copy_from_slice(&new[..sectors * SECTOR]);
        assert_ne!(checksum((0, 0), &torn), expected);
    }
    // The checksum can be continued: two halves give what the whole gives.
    let half = checksum((0, 0), &new[..2048]);
    assert_eq!(checksum(half, &new[2048..]), expected);

    // Two sectors written to each other's places. Adding the bytes up cannot see
    // it, because a sum does not care about order.
    let mut swapped = new.clone();
    swapped[..SECTOR].copy_from_slice(&new[SECTOR..2 * SECTOR]);
    swapped[SECTOR..2 * SECTOR].copy_from_slice(&new[..SECTOR]);
    assert_ne!(swapped, new);
    assert_eq!(byte_sum(&swapped), byte_sum(&new));
    assert_ne!(checksum((0, 0), &swapped), expected);
}

// @challenge 4
#[test]
fn damaging_the_free_list() {
    let statements = ["CREATE TABLE t (a INTEGER)", "DROP TABLE t"];
    // Three writes create the table. The fourth is the header that DROP TABLE
    // changed, which now says page 2 is free, while page 2 is still a leaf.
    assert_eq!(free_list_damage(&statements).unwrap(), Some(4));
    assert_eq!(free_list_damage(&statements[..1]).unwrap(), None);
}

// @challenge 5
#[test]
fn syncing_every_write() {
    let bonus = "UPDATE accounts SET balance = balance + 5 WHERE branch = 'north'";
    let mut totals = Vec::new();
    for writes in 1..=4 {
        let disk = CrashDisk::new();
        let mut db = Database::open(Box::new(SyncEach(disk.clone()))).unwrap();
        db.execute("CREATE TABLE accounts (id INTEGER PRIMARY KEY, branch TEXT, balance INTEGER)")
            .unwrap();
        for id in 1..=600 {
            let branch = if id % 2 == 0 { "north" } else { "south" };
            db.execute(&format!(
                "INSERT INTO accounts VALUES ({id}, '{branch}', 100)"
            ))
            .unwrap();
        }
        // Nothing is ever waiting: every write was synced as it was made.
        assert_eq!(disk.pending(), 0);
        let syncs = disk.syncs();
        disk.stop_after(writes);
        let finished = db.execute(bonus).is_ok();
        assert!(!finished);
        assert!(disk.syncs() - syncs < 4);

        // The worst a power failure can do now is lose the write in flight.
        let mut db = Database::open(Box::new(disk.crash(|_| Fate::Lost))).unwrap();
        db.check().unwrap();
        let Output::Rows(result) =
            db.execute("SELECT balance FROM accounts").unwrap()
        else {
            panic!("not a query")
        };
        let balance = |row: &Vec<Value>| match row[0] {
            Value::Integer(n) => n,
            _ => 0,
        };
        totals.push(result.rows.iter().map(balance).sum::<i64>());
    }
    assert_eq!(totals, [60_000, 60_425, 60_850, 61_275]);
}
