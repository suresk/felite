//! Chapter 19 challenges: running a script, a transaction around a closure, a dry
//! run, and inserting in batches.

use crate::catalog::sql_error;
use crate::db::{Database, Output};
use crate::error::Result;
use crate::sql::{Symbol, Token, tokenize};
use crate::transaction::Transaction;

// @listing script
/// Runs every statement in `text`, which holds any number of them with a
/// semicolon after each. Stops at the first that fails.
pub fn execute_script(db: &mut Database, text: &str) -> Result<Vec<Output>> {
    let mut outputs = Vec::new();
    let mut start = 0;
    for (token, span) in tokenize(text)? {
        let end = match token {
            Token::Symbol(Symbol::Semicolon) => span.offset,
            Token::End => text.len(),
            _ => continue,
        };
        // Between two semicolons, or after the last, there may be nothing at all.
        let statement = &text[start..end];
        if !statement.trim().is_empty() {
            outputs.push(db.execute(statement)?);
        }
        start = end + 1;
    }
    Ok(outputs)
}
// @end

// @listing atomically
/// Runs `work` inside a transaction. If it returns `Ok` the transaction is
/// committed, and if it returns an error the transaction is rolled back.
pub fn atomically<T>(
    db: &mut Database,
    work: impl FnOnce(&mut Database) -> Result<T>,
) -> Result<T> {
    db.execute("BEGIN")?;
    match work(db) {
        Ok(value) => {
            db.execute("COMMIT")?;
            Ok(value)
        }
        Err(error) => {
            // The error may have come from a statement, which has already undone
            // the work, or from the closure's own code, which has not.
            db.execute("ROLLBACK")?;
            Err(error)
        }
    }
}
// @end

// @listing dry-run
/// Runs a statement, notes what it produced, and undoes it.
pub fn dry_run(db: &mut Database, sql: &str) -> Result<Output> {
    if db.transaction() != Transaction::Closed {
        return sql_error(
            "a dry run needs to be outside any transaction".to_string(),
        );
    }
    db.execute("BEGIN")?;
    let output = db.execute(sql);
    db.execute("ROLLBACK")?;
    output
}
// @end

// @listing batches
/// Inserts the numbers below `rows` into table `t`, committing after every
/// `batch` of them.
pub fn insert_in_batches(db: &mut Database, rows: usize, batch: usize) -> Result<()> {
    for n in 0..rows {
        if n % batch == 0 {
            db.execute("BEGIN")?;
        }
        db.execute(&format!("INSERT INTO t (n) VALUES ({n})"))?;
        if n % batch == batch - 1 || n == rows - 1 {
            db.execute("COMMIT")?;
        }
    }
    Ok(())
}
// @end
