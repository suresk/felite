//! Chapter 18's crash test on a longer run: 1,000 statements, stopped after every
//! write, with the program killed and with the power failing in three ways.
//! `cargo run --release --example crash_test`. It takes a few seconds.
use felite::{CrashDisk, Database, MemFile, Output, Pager};

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn workload(statements: usize) -> Vec<String> {
    let mut sql = vec![
        "CREATE TABLE flights (id INTEGER PRIMARY KEY, note TEXT, delay INTEGER)"
            .to_string(),
    ];
    let (mut state, mut extra) = (16, false);
    while sql.len() < statements {
        let r = next(&mut state);
        let delay = r >> 8 & 1023;
        sql.push(match r % 20 {
            0..=11 => {
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

/// Runs the statements until one fails. Returns how many finished, and records
/// the contents after each if `states` is given.
fn run(
    disk: &CrashDisk,
    log: &CrashDisk,
    sql: &[String],
    states: Option<&mut Vec<String>>,
) -> usize {
    let Ok(pager) = Pager::open_logged(Box::new(disk.clone()), Box::new(log.clone()))
    else {
        return 0;
    };
    let Ok(mut db) = Database::with_pager(pager) else { return 0 };
    db.pager().checkpoint_after(50);
    let mut states = states;
    let mut done = 0;
    for statement in sql {
        if db.execute(statement).is_err() {
            break;
        }
        done += 1;
        if let Some(states) = states.as_deref_mut() {
            states.push(contents(&mut db).unwrap());
        }
    }
    done
}

fn main() {
    let sql = workload(1000);
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let mut states = vec![String::new()];
    run(&disk, &log, &sql, Some(&mut states));
    let points = disk.writes();
    println!("{} statements, {points} writes, {} syncs", sql.len(), disk.syncs());

    let mut checked = 0;
    for writes in 0..=points {
        let disk = CrashDisk::new();
        let log = disk.another_file();
        disk.stop_after(writes);
        let done = run(&disk, &log, &sql, None);
        let allowed = &states[done..(done + 2).min(states.len())];
        // Killed, and then three power failures with different seeds.
        let mut cuts: Vec<(MemFile, MemFile)> = vec![(disk.kill(), log.kill())];
        for seed in 0..3 {
            let seed = writes * 3 + seed;
            cuts.push((disk.power_cut(seed), log.power_cut(seed + 1_000_000)));
        }
        for (i, (file, log)) in cuts.into_iter().enumerate() {
            let pager = Pager::open_logged(Box::new(file), Box::new(log));
            let mut db = Database::with_pager(pager.expect("opens")).expect("opens");
            if let Err(error) = db.check() {
                panic!("after {writes} writes, crash {i}: {error}");
            }
            let found = contents(&mut db).expect("reads");
            assert!(allowed.contains(&found), "after {writes} writes, crash {i}");
            checked += 1;
        }
    }
    println!("{checked} crashes, and every one left a correct database");
}
