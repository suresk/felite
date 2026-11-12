//! What the prompt does with a line of input.

use crate::catalog::Table;
use crate::db::{Database, Output};
use crate::query::ResultSet;
use crate::sql::{CreateTable, Statement, explain};

// @listing format-rows
/// A result as a table of text: a line of column names, a line of dashes, and a
/// line for each row.
pub fn format_rows(result: &ResultSet) -> String {
    let mut lines: Vec<Vec<String>> = vec![result.columns.clone()];
    lines.push(
        result.columns.iter().map(|name| "-".repeat(name.chars().count())).collect(),
    );
    for row in &result.rows {
        lines.push(row.iter().map(|value| value.to_string()).collect());
    }
    // Each column is as wide as the widest thing in it.
    let mut widths = vec![0; result.columns.len()];
    for line in &lines {
        for (width, cell) in widths.iter_mut().zip(line) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for line in &lines {
        let cells: Vec<String> = line
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        out += cells.join("  ").trim_end();
        out.push('\n');
    }
    out
}
// @end

// @listing respond
/// Carries out one line typed at the prompt, and returns what should be printed.
pub fn respond(db: &mut Database, input: &str) -> String {
    match input.trim() {
        ".tables" => db.tables().map(|table| format!("{}\n", table.name)).collect(),
        ".schema" => {
            let define = |table: &Table| {
                let (name, columns) = (table.name.clone(), table.columns.clone());
                let create = Statement::CreateTable(CreateTable { name, columns });
                format!("{create};\n")
            };
            // @hide
            let indexes = |table: &Table| -> String {
                table
                    .indexes
                    .iter()
                    .map(|index| format!("{};\n", index.sql()))
                    .collect()
            };
            let define = |table: &Table| define(table) + &indexes(table);
            // @show
            db.tables().map(define).collect()
        }
        // @hide
        command if command.starts_with(".import ") => import_command(db, command),
        // @show
        sql => match db.execute(sql) {
            Ok(Output::Done) => "ok\n".to_string(),
            Ok(Output::Count(1)) => "1 row\n".to_string(),
            Ok(Output::Count(count)) => format!("{count} rows\n"),
            Ok(Output::Rows(result)) => {
                let count = match result.rows.len() {
                    1 => "(1 row)".to_string(),
                    count => format!("({count} rows)"),
                };
                format!("{}{count}\n", format_rows(&result))
            }
            // An error with a position comes back as several lines. Others are
            // one message, with no line end.
            Err(error) => match explain(sql, &error) {
                text if text.ends_with('\n') => text,
                text => format!("error: {text}\n"),
            },
        },
    }
}
// @end

// @listing import-command
/// `.import FILE TABLE`: loads a file of comma-separated values into a table.
fn import_command(db: &mut Database, command: &str) -> String {
    let words: Vec<&str> = command.split_whitespace().collect();
    let [_, path, table] = words.as_slice() else {
        return "usage: .import FILE TABLE\n".to_string();
    };
    let imported = match std::fs::File::open(path) {
        Ok(file) => db.import(table, &mut std::io::BufReader::new(file)),
        Err(error) => Err(error.into()),
    };
    match imported {
        Ok(rows) => format!("{rows} rows\n"),
        Err(error) => format!("error: {error}\n"),
    }
}
// @end
