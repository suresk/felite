//! `cargo xtask check 2` runs Chapter 2's tests (the chapter's examples and each of
//! its challenges) and says which pass. `cargo xtask check 2.3` runs one challenge,
//! and `cargo xtask check all` every chapter. Add `--crate <dir>` to run them against
//! your own crate. See `check.rs`.
//! `cargo xtask dump file.db [page]` prints one page of a database file, and
//! `cargo xtask cells file.db page` lists the cells of a slotted page.
//! `cargo xtask data` fetches the example data set (Chapter 25), and
//! `cargo xtask slt [script …]` runs sqllogictest scripts against felite.

mod check;
mod data;
mod slt;

use felite::{PAGE_SIZE, Page, hex_dump};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["check", ref rest @ ..] => check::main(rest),
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
    eprintln!(
        "usage: cargo xtask check <chapter | chapter.challenge | all> [--crate <dir>]"
    );
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
