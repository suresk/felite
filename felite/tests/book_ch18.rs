//! The examples in Chapter 18: `cargo test --test book_ch18`.
use felite::{CrashDisk, Database, MemFile, Output, Pager, Storage, Value};
use felite::{FRAME_SIZE, LOG_HEADER};

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing helpers
/// Opens the database in a file and its log.
fn open(file: impl Storage + 'static, log: impl Storage + 'static) -> Database {
    let pager = Pager::open_logged(Box::new(file), Box::new(log)).unwrap();
    Database::with_pager(pager).unwrap()
}

/// The bank of Section 16.8, with a log: 600 accounts of 100 each, on a disk
/// that can crash. Returns the database, its file and its log.
fn bank() -> (Database, CrashDisk, CrashDisk) {
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let mut db = open(disk.clone(), log.clone());
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
    (db, disk, log)
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
fn opening_after_a_crash() {
    // @listing recover-test
    let (mut db, disk, log) = bank();
    db.execute(BONUS).unwrap();
    // Everything the bank has is in the log. Its file has never been written.
    assert_eq!(db.pager().log_frames(), 616);
    assert_eq!(disk.size().unwrap(), 0);

    // The program is killed, and a new one opens the two files it left.
    drop(db);
    let mut db = open(disk.kill(), log.kill());
    assert_eq!(db.pager().log_frames(), 616);
    assert_eq!(total(&mut db), 61_500);
    db.check().unwrap();
    // @end
}

#[test]
fn a_statement_is_all_or_nothing() {
    // @listing atomic-test
    let mut found = Vec::new();
    for writes in 0..=4 {
        let (mut db, disk, log) = bank();
        disk.stop_after(writes);
        assert!(db.execute(BONUS).is_err());
        let mut db = open(disk.kill(), log.kill());
        db.check().unwrap();
        found.push(total(&mut db));
    }
    // In Chapter 16 this was 60,000, 60,425, 60,850, 61,275, 61,500.
    assert_eq!(found, [60_000, 60_000, 60_000, 60_000, 61_500]);
    // @end

    // @listing atomic-power-test
    for writes in 0..=4 {
        let (mut db, disk, log) = bank();
        disk.stop_after(writes);
        assert!(db.execute(BONUS).is_err());
        // The same stopped disk, with the power failing in ten different ways.
        for seed in 0..10 {
            let mut db = open(disk.power_cut(seed), log.power_cut(seed + 1000));
            db.check().unwrap();
            let total = total(&mut db);
            assert!(total == 60_000 || total == 61_500);
        }
    }
    // @end
}

// @listing damaged
/// The bank after two bonuses, reopened with its log changed by `damage`.
fn reopen_damaged(damage: impl Fn(&mut Vec<u8>)) -> Database {
    let (mut db, disk, log) = bank();
    db.execute(BONUS).unwrap(); // frames 612 to 615
    db.execute(BONUS).unwrap(); // frames 616 to 619
    let mut bytes = log.kill().bytes();
    damage(&mut bytes);
    let mut damaged = MemFile::new();
    damaged.write_at(0, &bytes).unwrap();
    open(disk.kill(), damaged)
}

/// Where frame `i` starts in a log file.
fn frame_at(i: usize) -> usize {
    LOG_HEADER + i * FRAME_SIZE
}
// @end

#[test]
fn where_the_log_stops() {
    // @listing damage-test
    let mut db = reopen_damaged(|_| {});
    assert_eq!((db.pager().log_frames(), total(&mut db)), (620, 63_000));

    // One bit wrong in the second bonus: the log ends at the first bonus's commit.
    let mut db = reopen_damaged(|log| log[frame_at(617) + 100] ^= 1);
    assert_eq!((db.pager().log_frames(), total(&mut db)), (616, 61_500));

    // The last frame cut short, as a torn write would leave it. The same.
    let mut db = reopen_damaged(|log| log.truncate(frame_at(619) + 512));
    assert_eq!((db.pager().log_frames(), total(&mut db)), (616, 61_500));

    // Only the last frame's commit mark changed, and so its checksum fails.
    let mut db = reopen_damaged(|log| log[frame_at(619) + 7] = 0);
    assert_eq!((db.pager().log_frames(), total(&mut db)), (616, 61_500));

    // One bit wrong in the first bonus. The second is intact, and is lost with it.
    let mut db = reopen_damaged(|log| log[frame_at(613) + 100] ^= 1);
    assert_eq!((db.pager().log_frames(), total(&mut db)), (612, 60_000));
    // @end

    // @listing damage-start-test
    // One bit wrong in the very first frame, and there is no bank.
    let mut db = reopen_damaged(|log| log[frame_at(0) + 100] ^= 1);
    assert_eq!(db.pager().log_frames(), 0);
    assert!(db.table("accounts").is_err());
    // @end
}

#[test]
fn a_checkpoint() {
    // @listing checkpoint-test
    let (mut db, disk, log) = bank();
    db.execute(BONUS).unwrap();
    assert_eq!(log.size().unwrap(), 24 + 616 * 4112); // 2.5 MB of log
    let before = (disk.writes(), disk.syncs());

    assert_eq!(db.pager().checkpoint().unwrap(), 7);
    // Seven pages written to the database file and a new header to the log, with
    // one sync for each file.
    assert_eq!((disk.writes() - before.0, disk.syncs() - before.1), (8, 2));
    assert_eq!(disk.size().unwrap(), 7 * 4096);
    assert_eq!(db.pager().log_frames(), 0);
    assert_eq!(total(&mut db), 61_500);

    // The database file is now complete by itself.
    let mut alone = Database::open(Box::new(disk.kill())).unwrap();
    assert_eq!(total(&mut alone), 61_500);
    alone.check().unwrap();
    // @end

    // @listing restart-test
    // The log file is as long as it was. Only its header is different: salt 2.
    let bytes = log.kill().bytes();
    assert_eq!(bytes.len(), 24 + 616 * 4112);
    assert_eq!(bytes[12..16], [0, 0, 0, 2]);
    // Frame 5 of the old log is still there, and nobody will ever believe it.
    let mut db = open(disk.kill(), log.kill());
    assert_eq!(db.pager().log_frames(), 0);

    // New frames go over the old ones from the top.
    db.execute(BONUS).unwrap();
    assert_eq!(db.pager().log_frames(), 4);
    assert_eq!(total(&mut db), 63_000);
    // @end
}

#[test]
fn old_frames_after_new_ones() {
    // @listing stale-test
    let (mut db, disk, log) = bank();
    db.pager().checkpoint().unwrap();
    // Two frames of the next statement reach the log, and then the power fails.
    disk.stop_after(2);
    assert!(db.execute(BONUS).is_err());
    let after = log.kill().bytes();
    assert_eq!(after.len(), 24 + 612 * 4112);

    // The file holds two new frames followed by 610 old ones, every one of them
    // well formed. Recovery believes none of them.
    let mut db = open(disk.kill(), log.kill());
    assert_eq!(db.pager().log_frames(), 0);
    assert_eq!(total(&mut db), 60_000);
    // @end
}

#[test]
fn crashing_in_a_checkpoint() {
    // @listing checkpoint-crash-test
    for writes in 0..=8 {
        let (mut db, disk, log) = bank();
        db.execute(BONUS).unwrap();
        disk.stop_after(writes);
        assert!(db.pager().checkpoint().is_err());
        for seed in 0..10 {
            let mut db = open(disk.power_cut(seed), log.power_cut(seed + 1000));
            db.check().unwrap();
            assert_eq!(total(&mut db), 61_500);
            // And the checkpoint can simply be done again.
            db.pager().checkpoint().unwrap();
            assert_eq!(total(&mut db), 61_500);
        }
    }
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
fn checkpoints_by_themselves() {
    // @listing auto-test
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let mut db = open(disk.clone(), log.clone());
    db.pager().checkpoint_after(20);
    let mut longest = 0;
    for statement in &workload() {
        db.execute(statement).unwrap();
        longest = longest.max(db.pager().log_frames());
    }
    db.check().unwrap();
    // The log never held more than 19 frames after a statement, and its file never
    // grew past 22. In Chapter 17 the same statements left 192 frames.
    assert_eq!(longest, 19);
    assert_eq!(log.size().unwrap(), 24 + 22 * 4112);
    // The database file has its eight pages now, apart from the newest changes.
    assert_eq!(disk.size().unwrap(), 8 * 4096);
    assert_eq!(db.pager().log_frames(), 5);
    assert_eq!((disk.writes(), disk.syncs()), (253, 129));
    // @end
}

// @listing verdict
/// Every table and every row of a database, as text.
fn contents(db: &mut Database) -> felite::Result<String> {
    let names: Vec<String> = db.tables().map(|table| table.name.clone()).collect();
    let mut text = String::new();
    for name in names {
        let Output::Rows(result) = db.execute(&format!("SELECT * FROM {name}"))?
        else {
            panic!("not a query")
        };
        text += &format!("{name}: {:?}\n", result.rows);
    }
    Ok(text)
}

/// What is wrong with the two files a crash left, if anything. `allowed` holds
/// the contents the database is entitled to have.
fn verdict(file: MemFile, log: MemFile, allowed: &[String]) -> &'static str {
    let pager = Pager::open_logged(Box::new(file), Box::new(log));
    let Ok(mut db) = pager.and_then(Database::with_pager) else {
        return "will not open";
    };
    if db.check().is_err() {
        return "fails the check";
    }
    match contents(&mut db) {
        Ok(text) if allowed.contains(&text) => "correct",
        _ => "wrong contents",
    }
}
// @end

// @listing run
/// Runs the workload on a database in these two files, with a checkpoint every
/// 20 frames or so, until a statement fails. Returns how many statements
/// finished, and adds the contents of the database after each to `states`.
fn run(disk: &CrashDisk, log: &CrashDisk, states: &mut Vec<String>) -> usize {
    // On a disk that has been stopped, even opening can fail.
    let pager = Pager::open_logged(Box::new(disk.clone()), Box::new(log.clone()));
    let Ok(mut db) = pager.and_then(Database::with_pager) else { return 0 };
    db.pager().checkpoint_after(20);
    let mut done = 0;
    for statement in &workload() {
        if db.execute(statement).is_err() {
            break;
        }
        done += 1;
        states.push(contents(&mut db).unwrap());
    }
    done
}
// @end

// @listing crash-points
/// Runs the workload once for every point at which it could be stopped, and
/// gives the verdict on each. `cut` turns one file of a stopped disk into what
/// is left of it.
fn crash_points(cut: impl Fn(&CrashDisk, u64) -> MemFile) -> Vec<&'static str> {
    // What the database should hold after each statement, from a run with no crash.
    let disk = CrashDisk::new();
    let mut states = vec![String::new()];
    run(&disk, &disk.another_file(), &mut states);

    let mut verdicts = Vec::new();
    for writes in 0..=disk.writes() {
        let disk = CrashDisk::new();
        let log = disk.another_file();
        disk.stop_after(writes);
        let done = run(&disk, &log, &mut Vec::new());
        // The statements that finished, with or without the one that was stopped.
        let allowed = &states[done..(done + 2).min(states.len())];
        let (file, log) = (cut(&disk, writes), cut(&log, writes + 1000));
        verdicts.push(verdict(file, log, allowed));
    }
    verdicts
}

/// How many of each verdict: correct, wrong contents, fails the check, will not open.
fn tally(verdicts: &[&str]) -> [usize; 4] {
    ["correct", "wrong contents", "fails the check", "will not open"]
        .map(|which| verdicts.iter().filter(|v| **v == which).count())
}
// @end

#[test]
fn every_crash_point() {
    // @listing killed-test
    let killed = crash_points(|file, _| file.kill());
    assert_eq!(killed.len(), 254);
    assert_eq!(tally(&killed), [254, 0, 0, 0]);
    // @end

    // @listing power-test
    let cut = crash_points(|file, seed| file.power_cut(seed));
    assert_eq!(tally(&cut), [254, 0, 0, 0]);
    // @end
}
