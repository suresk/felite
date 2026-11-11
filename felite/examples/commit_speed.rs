//! How long a commit takes on a real disk, with and without waiting for it:
//! `cargo run --release --example commit_speed`.
use felite::{Database, NoSync, Pager, Storage};
use std::fs::{File, OpenOptions};
use std::time::Instant;

fn open(path: &str) -> File {
    let _ = std::fs::remove_file(path);
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap()
}

/// Inserts 500 rows, one statement and so one commit each, and returns the
/// number of commits per second.
fn run(file: Box<dyn Storage>, log: Box<dyn Storage>) -> f64 {
    let mut db =
        Database::with_pager(Pager::open_logged(file, log).unwrap()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, note TEXT)").unwrap();
    let start = Instant::now();
    for i in 0..500 {
        db.execute(&format!("INSERT INTO t (note) VALUES ('row number {i}')"))
            .unwrap();
    }
    500.0 / start.elapsed().as_secs_f64()
}

fn main() {
    let dir = std::env::temp_dir();
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let synced =
        run(Box::new(open(&path("speed.db"))), Box::new(open(&path("speed.log"))));
    let unsynced = run(
        Box::new(NoSync(open(&path("speed2.db")))),
        Box::new(NoSync(open(&path("speed2.log")))),
    );
    println!("with sync:    {synced:>9.0} commits a second");
    println!("without sync: {unsynced:>9.0} commits a second");
    for name in ["speed.db", "speed.log", "speed2.db", "speed2.log"] {
        let _ = std::fs::remove_file(path(name));
    }
}
