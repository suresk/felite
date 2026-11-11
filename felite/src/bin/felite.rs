//! The felite prompt: `cargo run --bin felite -- flights.db`.
use felite::{Database, MemFile, Storage, respond};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};

// @listing main
fn main() -> felite::Result<()> {
    // A file named on the command line, or else a database that lives in memory.
    let storage: Box<dyn Storage> = match std::env::args().nth(1) {
        Some(path) => {
            let mut options = OpenOptions::new();
            Box::new(
                options
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path)?,
            )
        }
        None => Box::new(MemFile::new()),
    };
    let mut db = Database::open(storage)?;
    let mut statement = String::new();
    loop {
        print!("{}", if statement.is_empty() { "felite> " } else { "   ...> " });
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 || line.trim() == ".quit" {
            return Ok(());
        }
        statement += &line;
        // A command that starts with a dot is one line. A statement runs to its
        // semicolon, however many lines that takes.
        let text = statement.trim();
        if text.starts_with('.') || text.ends_with(';') {
            print!("{}", respond(&mut db, text));
            statement.clear();
        } else if text.is_empty() {
            statement.clear();
        }
    }
}
// @end
