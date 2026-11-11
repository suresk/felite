//! The felite prompt: `cargo run --bin felite -- flights.db`.
use felite::{Database, MemFile, respond};
use std::io::{self, BufRead, Write};

// @listing main
fn main() -> felite::Result<()> {
    // A file named on the command line, or else a database that lives in memory.
    let mut db = match std::env::args().nth(1) {
        Some(path) => Database::open_file(&path)?,
        None => Database::open(Box::new(MemFile::new()))?,
    };
    let mut statement = String::new();
    loop {
        print!("{}", if statement.is_empty() { "felite> " } else { "   ...> " });
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 || line.trim() == ".quit" {
            // @hide
            db.pager().checkpoint()?;
            // @show
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
