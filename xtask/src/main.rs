//! `cargo xtask check 2` runs Chapter 2's tests (the chapter's examples and its
//! challenges) and says which ones still fail. `cargo xtask check all` runs them all.
//! `cargo xtask dump file.db [page]` prints one page of a database file, and
//! `cargo xtask cells file.db page` lists the cells of a slotted page.
//! `cargo xtask data` fetches the example data set (Chapter 25), and
//! `cargo xtask slt [script …]` runs sqllogictest scripts against felite.

mod data;
mod slt;

use felite::{PAGE_SIZE, Page, hex_dump};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["check", "all"] => run(&["test", "--package", "felite"], "all chapters"),
        ["check", n] => match n.parse::<u32>() {
            Ok(n) => check(n),
            Err(_) => usage(),
        },
        ["dump", path] => report(dump(path, 0)),
        ["dump", path, page] => match page.parse() {
            Ok(page) => report(dump(path, page)),
            Err(_) => usage(),
        },
        ["cells", path, page] => match page.parse() {
            Ok(page) => report(cells(path, page)),
            Err(_) => usage(),
        },
        ["data"] => outcome(data::data()),
        ["slt", ..] => outcome(slt::slt(&args[1..])),
        _ => usage(),
    }
}

fn outcome(result: Result<(), Box<dyn std::error::Error>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: cargo xtask check <chapter>|all");
    eprintln!("       cargo xtask dump <file> [page]");
    eprintln!("       cargo xtask cells <file> <page>");
    eprintln!("       cargo xtask data");
    eprintln!("       cargo xtask slt [script ...]");
    ExitCode::from(2)
}

// @listing dump
/// Prints page `page` of the file at `path`.
fn dump(path: &str, page: u64) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let pages = file.metadata()?.len() / PAGE_SIZE as u64;
    let mut bytes = [0; PAGE_SIZE];
    file.seek(SeekFrom::Start(page * PAGE_SIZE as u64))?;
    file.read_exact(&mut bytes)?;
    println!("page {page} (the file has {pages} pages)");
    print!("{}", hex_dump(&bytes));
    Ok(())
}
// @end

// @listing cells
/// Prints the cells of page `page` of the file at `path`, if it is a slotted page.
fn cells(path: &str, page: u64) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let mut bytes = Page::new();
    file.seek(SeekFrom::Start(page * PAGE_SIZE as u64))?;
    file.read_exact(bytes.bytes_mut())?;
    match bytes.describe() {
        Ok(text) => print!("{text}"),
        Err(e) => println!("page {page} is not a healthy slotted page: {e}"),
    }
    Ok(())
}
// @end

fn report(result: std::io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The test binaries for one chapter: `book_ch07`, `challenges_ch07`, and any others
/// whose names start that way.
fn chapter_tests(n: u32) -> Vec<String> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../felite/tests");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".rs").map(String::from))
        .filter(|name| {
            [format!("book_ch{n:02}"), format!("challenges_ch{n:02}")]
                .iter()
                .any(|p| name.starts_with(p.as_str()))
        })
        .collect();
    names.sort();
    names
}

fn check(n: u32) -> ExitCode {
    let tests = chapter_tests(n);
    if tests.is_empty() {
        eprintln!("no tests for Chapter {n} in felite/tests");
        return ExitCode::from(2);
    }
    let mut args = vec!["test", "--package", "felite"];
    for t in &tests {
        args.extend(["--test", t.as_str()]);
    }
    run(&args, &format!("Chapter {n}"))
}

fn run(args: &[&str], what: &str) -> ExitCode {
    let status = Command::new(env!("CARGO")).args(args).status();
    match status {
        Ok(s) if s.success() => {
            println!("{what}: every test passes");
            ExitCode::SUCCESS
        }
        Ok(_) => {
            println!("{what}: some tests fail or do not compile yet (see above)");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("could not run cargo: {e}");
            ExitCode::FAILURE
        }
    }
}
