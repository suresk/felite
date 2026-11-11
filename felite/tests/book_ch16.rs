//! The examples in Chapter 16: `cargo test --test book_ch16`.
use felite::{CrashDisk, Database, Fate, MemFile, Output, Pager, Storage, Tree};
use felite::{PAGE_SIZE, Page, Value, row_key};

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn what_sync_promises() {
    // @listing sync-test
    let mut disk = CrashDisk::new();
    disk.write_at(0, &[1; PAGE_SIZE]).unwrap();
    disk.sync().unwrap();
    disk.write_at(0, &[2; PAGE_SIZE]).unwrap();
    disk.write_at(4096, &[3; PAGE_SIZE]).unwrap();
    assert_eq!((disk.writes(), disk.syncs(), disk.pending()), (3, 1, 2));

    // The program reads back what it wrote, synced or not.
    let mut page = [0; PAGE_SIZE];
    disk.read_at(4096, &mut page).unwrap();
    assert_eq!(page, [3; PAGE_SIZE]);

    // If the power fails now, only the synced write is certain to be there.
    let after = disk.crash(|_| Fate::Lost).bytes();
    assert_eq!(after, [1; PAGE_SIZE]);
    // The disk may also have kept either unsynced write and not the other.
    let after = disk.crash(|i| if i == 1 { Fate::Kept } else { Fate::Lost }).bytes();
    assert_eq!(after[..4096], [1; PAGE_SIZE]);
    assert_eq!(after[4096..], [3; PAGE_SIZE]);
    // Or part of one: here the first three sectors of the write to page 0.
    let after =
        disk.crash(|i| if i == 0 { Fate::Torn(3) } else { Fate::Lost }).bytes();
    assert_eq!(after[..1536], [2; 1536]);
    assert_eq!(after[1536..], [1; 2560]);

    // After a sync there is nothing left to lose.
    disk.sync().unwrap();
    assert_eq!(disk.pending(), 0);
    assert_eq!(disk.crash(|_| Fate::Lost).bytes(), disk.kill().bytes());
    // @end
}

#[test]
fn stopping_the_program() {
    // @listing stop-test
    let mut disk = CrashDisk::new();
    disk.stop_after(2);
    assert!(disk.write_at(0, &[1; PAGE_SIZE]).is_ok());
    assert!(disk.write_at(4096, &[2; PAGE_SIZE]).is_ok());
    assert!(disk.write_at(8192, &[3; PAGE_SIZE]).is_err());
    assert!(disk.sync().is_err());
    // A killed program loses nothing it wrote. A power failure can lose all of it.
    assert_eq!(disk.kill().bytes().len(), 2 * PAGE_SIZE);
    assert_eq!(disk.crash(|_| Fate::Lost).bytes().len(), 0);
    // @end
}

// @listing split-setup
/// A tree of 200 rows on a disk that can crash, flushed. Then one more row, which
/// splits a leaf, and has not been flushed.
fn about_to_split() -> (CrashDisk, Pager, Tree) {
    let disk = CrashDisk::new();
    let mut pager = Pager::open(Box::new(disk.clone())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..200 {
        tree.insert(&mut pager, &row_key(id * 10), &[id as u8; 40]).unwrap();
    }
    pager.flush().unwrap();
    tree.insert(&mut pager, &row_key(5), &[1; 40]).unwrap();
    (disk, pager, tree)
}

/// Opens the file a crash left behind, and says what `Tree::check` makes of it.
fn inspect(file: MemFile, tree: Tree) -> String {
    let mut pager = match Pager::open(Box::new(file)) {
        Ok(pager) => pager,
        Err(error) => return error.to_string(),
    };
    match tree.check(&mut pager) {
        Ok(info) => format!("{} rows", info.entries),
        Err(error) => error.to_string(),
    }
}
// @end

#[test]
fn the_writes_of_a_split() {
    // @listing split-writes
    let (disk, mut pager, _) = about_to_split();
    // Nothing has been written since the flush. The split is all in memory.
    assert_eq!(disk.pending(), 0);
    assert_eq!(pager.page_count().unwrap(), 6);
    let old = disk.kill().bytes();
    pager.flush().unwrap();
    let new = disk.kill().bytes();
    assert_eq!((old.len(), new.len()), (5 * PAGE_SIZE, 6 * PAGE_SIZE));

    // Which pages did the flush write? Page 5 is in the new file only.
    let differs = |n: &usize| {
        let at = n * PAGE_SIZE..(n + 1) * PAGE_SIZE;
        old.get(at.clone()) != new.get(at)
    };
    let changed: Vec<usize> = (0..6).filter(differs).collect();
    assert_eq!(changed, [0, 1, 3, 5]);
    assert_eq!(pager.page(5).unwrap().cell_count(), 39);
    // @end
}

#[test]
fn crashing_mid_split() {
    // @listing split-test
    let mut found = Vec::new();
    for writes in 0..=4 {
        let (disk, mut pager, tree) = about_to_split();
        disk.stop_after(writes);
        assert!(pager.flush().is_err());
        found.push(inspect(disk.kill(), tree));
    }
    assert_eq!(
        found,
        [
            "200 rows",
            "200 rows",
            "corrupt database: page 3: the key in cell 38 belongs on another page",
            "I/O error: unexpected end of file",
            "201 rows",
        ]
    );
    // @end
}

#[test]
fn a_lost_page_and_a_torn_page() {
    // @listing lost-test
    // The split's four writes all happen, and the power fails before the sync.
    let crash = |fate: &dyn Fn(usize) -> Fate| {
        let (disk, mut pager, tree) = about_to_split();
        disk.stop_after(4);
        assert!(pager.flush().is_err());
        assert_eq!(disk.pending(), 4);
        inspect(disk.crash(fate), tree)
    };
    let lose = |lost| crash(&|i| if i == lost { Fate::Lost } else { Fate::Kept });
    // Without the new header, page 5 is in the tree and not in the file's count.
    assert!(lose(0).ends_with("page 5 is past the end of a 5-page file"));
    // Without the new root, nothing leads to page 5, but page 3 links to it.
    assert!(lose(1).ends_with("page 3: its right link is 5, not 2"));
    // Without the new page 3, the rows that moved to page 5 are in both.
    assert!(lose(2).ends_with("page 3: the key in cell 38 belongs on another page"));
    // Without page 5, the root points past the end of the file.
    assert_eq!(lose(3), "I/O error: unexpected end of file");
    // @end

    // @listing torn-test
    let tear = |torn, sectors| {
        crash(&|i| if i == torn { Fate::Torn(sectors) } else { Fate::Kept })
    };
    // The header's fields are all in its first sector, so tearing it is harmless.
    assert_eq!(tear(0, 1), "201 rows");
    // Half of the new page 3 over half of the old one.
    assert!(
        tear(2, 4).ends_with("page 3: the key in cell 38 belongs on another page")
    );
    assert!(tear(2, 7).ends_with("page 3: the key in cell 10 is out of order"));
    // Half of the new root. The check blames a page that is perfectly good.
    assert!(
        tear(1, 4).ends_with("page 3: the key in cell 0 belongs on another page")
    );
    // @end
}

// @listing bank
/// 600 accounts with a balance of 100 each, half of them at each of two branches.
fn bank(disk: &CrashDisk) -> Database {
    let mut db = Database::open(Box::new(disk.clone())).unwrap();
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
    db
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
fn checking_a_whole_database() {
    // @listing check-test
    let disk = CrashDisk::new();
    let mut db = bank(&disk);
    db.check().unwrap();

    // Add a page that belongs to nothing, and the count no longer adds up.
    db.pager().allocate(Page::new()).unwrap();
    let error = db.check().unwrap_err().to_string();
    assert_eq!(error, "corrupt database: 7 pages are accounted for, of 8");
    // @end
}

#[test]
fn crashing_mid_statement() {
    // @listing statement-test
    let disk = CrashDisk::new();
    let mut db = bank(&disk);
    assert_eq!(total(&mut db), 60_000);
    let before = disk.writes();
    db.execute(BONUS).unwrap();
    assert_eq!(disk.writes() - before, 4); // the table's four leaves
    assert_eq!(total(&mut db), 61_500);

    let mut found = Vec::new();
    for writes in 0..=4 {
        let disk = CrashDisk::new();
        let mut db = bank(&disk);
        disk.stop_after(writes);
        assert!(db.execute(BONUS).is_err());
        // The file opens, and passes every check we have.
        let mut db = Database::open(Box::new(disk.kill())).unwrap();
        db.check().unwrap();
        found.push(total(&mut db));
    }
    assert_eq!(found, [60_000, 60_425, 60_850, 61_275, 61_500]);
    // @end
}

// @listing workload
/// 120 statements of every kind felite can run, the same on every call.
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

/// What is wrong with the file a crash left, if anything. `allowed` holds the
/// contents the database is entitled to have.
fn verdict(file: MemFile, allowed: &[String]) -> &'static str {
    let Ok(mut db) = Database::open(Box::new(file)) else {
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

// @listing crash-points
/// Runs the workload once for every point at which it could be stopped, and
/// gives the verdict on each. `cut` turns a stopped disk into the file that is left.
fn crash_points(cut: impl Fn(&CrashDisk, u64) -> MemFile) -> Vec<&'static str> {
    let sql = workload();
    // What the database should hold after each statement, from a run with no crash.
    let disk = CrashDisk::new();
    let mut db = Database::open(Box::new(disk.clone())).unwrap();
    let mut states = vec![contents(&mut db).unwrap()];
    for statement in &sql {
        db.execute(statement).unwrap();
        states.push(contents(&mut db).unwrap());
    }

    let mut verdicts = Vec::new();
    for writes in 0..=disk.writes() {
        let disk = CrashDisk::new();
        disk.stop_after(writes);
        let mut db = Database::open(Box::new(disk.clone())).unwrap();
        let done = sql.iter().take_while(|s| db.execute(s).is_ok()).count();
        // The statements that finished, with or without the one that was stopped.
        let allowed = &states[done..(done + 2).min(states.len())];
        verdicts.push(verdict(cut(&disk, writes), allowed));
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
fn the_shape_of_the_workload() {
    // @listing shape-test
    let disk = CrashDisk::new();
    let mut db = Database::open(Box::new(disk.clone())).unwrap();
    // Statements that write more than one page, and the places to stop inside them.
    let (mut several, mut inside) = (0, 0);
    for statement in &workload() {
        let before = disk.writes();
        db.execute(statement).unwrap();
        let writes = disk.writes() - before;
        if writes > 1 {
            several += 1;
            inside += writes - 1;
        }
    }
    assert_eq!((disk.writes(), disk.syncs()), (192, 120));
    assert_eq!((several, inside), (35, 82));
    db.check().unwrap();
    // @end
}

#[test]
fn every_crash_point() {
    // @listing killed-test
    // The program is killed at each point. Every write it made reaches the disk.
    let killed = crash_points(|disk, _| disk.kill());
    assert_eq!(killed.len(), 193);
    assert_eq!(tally(&killed), [121, 11, 60, 1]);
    // @end

    // @listing power-test
    // The power fails at each point, with a different seed for each.
    let cut = crash_points(|disk, writes| disk.power_cut(writes));
    assert_eq!(tally(&cut), [104, 8, 74, 7]);
    // @end

    // @listing none-test
    // A disk that keeps nothing it was not told to sync.
    let strict = crash_points(|disk, _| disk.crash(|_| Fate::Lost));
    assert_eq!(tally(&strict), [193, 0, 0, 0]);
    // @end
}
