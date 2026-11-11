//! Tests for the Chapter 18 challenges: `cargo test --test challenges_ch18`.
use felite::{CrashDisk, Database, MemFile, Output, Pager, Storage, Value};
use felite::{FRAME_SIZE, LOG_HEADER};
use felite::{LogReport, log_report, recovery_reads, to_disks};

fn open(file: impl Storage + 'static, log: impl Storage + 'static) -> Database {
    let pager = Pager::open_logged(Box::new(file), Box::new(log)).unwrap();
    Database::with_pager(pager).unwrap()
}

fn mem(bytes: &[u8]) -> MemFile {
    let mut file = MemFile::new();
    if !bytes.is_empty() {
        file.write_at(0, bytes).unwrap();
    }
    file
}

fn count(db: &mut Database, table: &str) -> Option<usize> {
    match db.execute(&format!("SELECT * FROM {table}")) {
        Ok(Output::Rows(result)) => Some(result.rows.len()),
        _ => None,
    }
}

/// Three commits: a table `a` (frames 0 to 2), a row in it (frame 3), and a
/// table `b` (frames 4 to 6).
fn three_commits() -> Vec<u8> {
    let log = MemFile::new();
    let mut db = open(MemFile::new(), log.clone());
    db.execute("CREATE TABLE a (x INTEGER)").unwrap();
    db.execute("INSERT INTO a VALUES (1)").unwrap();
    db.execute("CREATE TABLE b (x INTEGER)").unwrap();
    log.bytes()
}

#[test]
fn predict_the_recovery() {
    let frame = |i: usize| LOG_HEADER + i * FRAME_SIZE;
    // What a database recovered from this log holds: rows in a, rows in b.
    let recovered = |log: &[u8]| {
        let mut db = open(MemFile::new(), mem(log));
        (count(&mut db, "a"), count(&mut db, "b"))
    };
    let log = three_commits();
    assert_eq!(recovered(&log), (Some(1), Some(0)));
    // The log ends partway through frame 5.
    assert_eq!(recovered(&log[..frame(5) + 2000]), (Some(1), None));
    // A bit of frame 3 is wrong.
    let mut damaged = log.clone();
    damaged[frame(3) + 50] ^= 4;
    assert_eq!(recovered(&damaged), (Some(0), None));
    // Frame 6 has lost its commit mark.
    let mut damaged = log.clone();
    damaged[frame(6) + 7] = 0;
    assert_eq!(recovered(&damaged), (Some(1), None));
    // Frame 1 claims to be a commit.
    let mut damaged = log.clone();
    damaged[frame(1) + 7] = 1;
    assert_eq!(recovered(&damaged), (None, None));
    // The salt in the header is different.
    let mut damaged = log.clone();
    damaged[15] = 2;
    assert_eq!(recovered(&damaged), (None, None));
}

#[test]
fn reporting_on_a_log() {
    let frame = |i: usize| LOG_HEADER + i * FRAME_SIZE;
    let log = three_commits();
    let report = log_report(&log).unwrap();
    assert_eq!(report, LogReport { salt: 1, valid: 7, committed: 7, commits: 3 });
    let report = log_report(&log[..frame(6)]).unwrap();
    assert_eq!(report, LogReport { salt: 1, valid: 6, committed: 4, commits: 2 });
    let mut damaged = log.clone();
    damaged[frame(2) + 4000] ^= 1;
    let report = log_report(&damaged).unwrap();
    assert_eq!(report, LogReport { salt: 1, valid: 2, committed: 0, commits: 0 });
    damaged[9] = 9;
    assert!(log_report(&damaged).is_err());
    assert!(log_report(&[]).is_err());

    // After a checkpoint the file is as long as before, and nothing in it is valid.
    let (file, log) = (MemFile::new(), MemFile::new());
    let mut db = open(file, log.clone());
    db.execute("CREATE TABLE a (x INTEGER)").unwrap();
    db.pager().checkpoint().unwrap();
    assert_eq!(log.bytes().len(), frame(3));
    let report = log_report(&log.bytes()).unwrap();
    assert_eq!(report, LogReport { salt: 2, valid: 0, committed: 0, commits: 0 });
}

#[test]
fn what_recovery_reads() {
    let (file, log) = (MemFile::new(), MemFile::new());
    let mut db = open(file.clone(), log.clone());
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
    for n in 0..200 {
        db.execute(&format!("INSERT INTO t (n) VALUES ({n})")).unwrap();
    }
    assert_eq!(db.pager().log_frames(), 203);
    // The header, every frame, and then the header page of the database, which
    // the pager fetches from the log to check it.
    let (before, log_before) = (mem(&file.bytes()), mem(&log.bytes()));
    assert_eq!(recovery_reads(before, log_before).unwrap(), 1 + 203 + 1);

    // After a checkpoint: the header, and one old frame that fails its checksum.
    db.pager().checkpoint().unwrap();
    assert_eq!(recovery_reads(mem(&file.bytes()), mem(&log.bytes())).unwrap(), 2);
}

#[test]
fn crashing_twice() {
    let bonus = "UPDATE accounts SET balance = balance + 5 WHERE branch = 'north'";
    let total = |db: &mut Database| -> i64 {
        let Output::Rows(result) =
            db.execute("SELECT balance FROM accounts").unwrap()
        else {
            panic!("not a query")
        };
        let balance = |row: &Vec<Value>| match row[0] {
            Value::Integer(n) => n,
            _ => 0,
        };
        result.rows.iter().map(balance).sum()
    };
    for first in 0..=4 {
        let disk = CrashDisk::new();
        let log = disk.another_file();
        let mut db = open(disk.clone(), log.clone());
        db.execute(
            "CREATE TABLE accounts (\
             id INTEGER PRIMARY KEY, branch TEXT, balance INTEGER)",
        )
        .unwrap();
        for id in 1..=100 {
            let branch = if id % 2 == 0 { "north" } else { "south" };
            db.execute(&format!(
                "INSERT INTO accounts VALUES ({id}, '{branch}', 100)"
            ))
            .unwrap();
        }
        db.pager().checkpoint_after(60);
        disk.stop_after(first);
        let _ = db.execute(bonus);
        for seed in 0..5 {
            let (file, wal) = (disk.power_cut(seed), log.power_cut(seed + 50));
            let mut recovered = open(mem(&file.bytes()), mem(&wal.bytes()));
            let once = total(&mut recovered);
            assert!(once == 10_000 || once == 10_250);

            // The same wreckage on a new disk, a second bonus, a second crash.
            for second in 0..=6 {
                let (disk, log) = to_disks(&file, &wal).unwrap();
                let mut db = open(disk.clone(), log.clone());
                db.pager().checkpoint_after(60);
                disk.stop_after(second);
                let finished = db.execute(bonus).is_ok();
                let mut db = open(disk.power_cut(seed + 7), log.power_cut(seed + 9));
                db.check().unwrap();
                let twice = total(&mut db);
                assert!(twice == once + 250 || (!finished && twice == once));
            }
        }
    }
}

#[test]
fn the_header_that_came_back() {
    let log = three_commits();
    // The header is wiped out, and the frames are left as they were.
    let mut wiped = log.clone();
    wiped[..LOG_HEADER].fill(0);
    let (file, wiped) = (MemFile::new(), mem(&wiped));
    let mut db = open(file.clone(), wiped.clone());
    // With no header there is no log, so this is a new database.
    assert_eq!(count(&mut db, "a"), None);
    drop(db);
    // But opening it wrote a new header, the same as the old one byte for byte.
    assert_eq!(wiped.bytes(), log);
    // So the next program to open these files finds three commits.
    let mut db = open(file, wiped);
    assert_eq!(count(&mut db, "a"), Some(1));
}
