//! Transactions: several statements that happen together or not at all.

use crate::catalog::sql_error;
use crate::db::{Database, Output};
use crate::error::Result;
use crate::sql::{Statement, parse};

// @listing transaction-state
/// Where a database stands between `BEGIN` and `COMMIT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transaction {
    /// No transaction is open. Each statement commits by itself.
    Closed,
    /// `BEGIN` has been run, and nothing has gone wrong since.
    Open,
    /// A statement failed. The transaction's work is undone, and it accepts
    /// nothing more until it is ended.
    Failed,
}
// @end

impl Database {
    // @listing transact
    /// Runs one statement on a database that has a log.
    pub(crate) fn transact(&mut self, sql: &str) -> Result<Output> {
        use Transaction::{Closed, Failed, Open};
        let statement = parse(sql)?;
        match (&statement, self.transaction) {
            (Statement::Begin, Closed) => self.transaction = Open,
            (Statement::Begin, _) => {
                return sql_error("a transaction is already open".to_string());
            }
            (Statement::Commit | Statement::Rollback, Closed) => {
                return sql_error("no transaction is open".to_string());
            }
            (Statement::Commit, Open) => {
                self.transaction = Closed;
                self.commit_or_undo()?;
            }
            (Statement::Rollback, Open) => {
                self.transaction = Closed;
                self.rollback()?;
            }
            (Statement::Rollback, Failed) => self.transaction = Closed,
            (_, Failed) => {
                return sql_error(
                    "this transaction has failed, and only ROLLBACK can end it"
                        .to_string(),
                );
            }
            _ => return self.run_in_transaction(statement),
        }
        Ok(Output::Done)
    }
    // @end

    // @listing run-in-transaction
    /// Runs an ordinary statement, in the open transaction or in one of its own.
    fn run_in_transaction(&mut self, statement: Statement) -> Result<Output> {
        match self.run(statement) {
            Ok(output) => {
                if self.transaction == Transaction::Closed {
                    self.commit_or_undo()?;
                }
                Ok(output)
            }
            Err(error) => {
                // Whatever the statement had done so far is undone, along with
                // everything else in its transaction.
                self.rollback()?;
                if self.transaction == Transaction::Open {
                    self.transaction = Transaction::Failed;
                }
                Err(error)
            }
        }
    }

    /// Commits, and if that fails, makes sure nothing is left half done.
    fn commit_or_undo(&mut self) -> Result<()> {
        let committed = self.pager.commit();
        if committed.is_err() {
            self.rollback()?;
        }
        committed
    }
    // @end

    // @listing db-rollback
    /// Undoes everything since the last commit: the pages, and what the database
    /// remembers about its tables.
    pub(crate) fn rollback(&mut self) -> Result<()> {
        self.pager.rollback()?;
        let (schema, tables) = Database::read_catalog(&mut self.pager)?;
        self.schema = schema;
        self.tables = tables;
        Ok(())
    }

    /// Whether a transaction is open, or has failed and not been ended.
    pub fn transaction(&self) -> Transaction {
        self.transaction
    }
    // @end
}
