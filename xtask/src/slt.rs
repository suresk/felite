//! `cargo xtask slt`: runs sqllogictest scripts against felite and prints how
//! many of their statements and queries it handled.

use crate::data::fetch;
use felite::{Database, MemFile, SltReport, run_slt};

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

/// A mirror of the sqllogictest scripts that the SQLite project wrote, and the
/// ones this book runs.
const MIRROR: &str =
    "https://raw.githubusercontent.com/gregrahn/sqllogictest/master/test/";
const SCRIPTS: [&str; 10] = [
    "select1.test",
    "select2.test",
    "select3.test",
    "select4.test",
    "select5.test",
    "random/select/slt_good_0.test",
    "index/commute/10/slt_good_0.test",
    "index/between/1/slt_good_0.test",
    "index/orderby/10/slt_good_0.test",
    "index/delete/10/slt_good_0.test",
];

fn row(name: &str, report: &SltReport) {
    let statements = report.statements_ok + report.statements_wrong;
    let queries = report.queries_ok + report.queries_wrong + report.queries_refused;
    println!(
        "{name:<34} {:>6} {statements:>6} {:>6} {:>6} {:>7} {queries:>6}",
        report.statements_ok,
        report.queries_ok,
        report.queries_wrong,
        report.queries_refused,
    );
}

/// Runs the scripts named, or the book's ten if none are, each against a new
/// database in memory.
pub fn slt(paths: &[String]) -> Outcome<()> {
    let mut paths = paths.to_vec();
    if paths.is_empty() {
        std::fs::create_dir_all("data/slt")?;
        for script in SCRIPTS {
            let path = format!("data/slt/{}", script.replace('/', "_"));
            fetch(&format!("{MIRROR}{script}"), &path)?;
            paths.push(path);
        }
    }
    println!("{:<34} {:>13} {:>27}", "", "statements", "queries");
    println!(
        "{:<34} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6}",
        "script", "ok", "of", "ok", "wrong", "refused", "of"
    );
    let mut total = SltReport::default();
    for path in &paths {
        let script = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
        let mut db = Database::open(Box::new(MemFile::new()))?;
        let report = run_slt(&mut db, &script);
        row(path.rsplit('/').next().unwrap_or(path), &report);
        total.add(&report);
    }
    row("all", &total);
    let mut reasons: Vec<(&String, &usize)> = total.refusals.iter().collect();
    reasons.sort_by_key(|(reason, count)| {
        (std::cmp::Reverse(**count), reason.to_string())
    });
    println!("\nwhy felite refused, most common first:");
    for (reason, count) in reasons.iter().take(20) {
        println!("{count:>8}  {reason}");
    }
    Ok(())
}
