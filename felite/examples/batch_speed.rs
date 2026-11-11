//! Rows inserted per second on a real disk, by the number of rows in each
//! transaction: `cargo run --release --example batch_speed`.
use felite::{Database, insert_in_batches};
use std::time::Instant;

fn main() {
    let path = std::env::temp_dir().join("batch.db").to_string_lossy().into_owned();
    println!("{:>8}  {:>12}", "batch", "rows/second");
    for (batch, rows) in
        [(1, 500), (10, 5000), (100, 20_000), (1000, 50_000), (10_000, 50_000)]
    {
        for suffix in ["", "-log"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
        let mut db = Database::open_file(&path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
        let start = Instant::now();
        insert_in_batches(&mut db, rows, batch).unwrap();
        let rate = rows as f64 / start.elapsed().as_secs_f64();
        println!("{batch:>8}  {rate:>12.0}");
    }
    for suffix in ["", "-log"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}
