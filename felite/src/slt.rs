//! Running sqllogictest scripts: SQL statements and queries with the results
//! that some other database produced for them.

use crate::db::{Database, Output};
use crate::record::Value;
use std::collections::BTreeMap;

// @listing md5
/// The MD5 digest of some bytes, as 32 hexadecimal digits. sqllogictest records
/// a long result as its MD5, so checking one means computing it.
pub fn md5(input: &[u8]) -> String {
    const SHIFTS: [[u32; 4]; 4] =
        [[7, 12, 17, 22], [5, 9, 14, 20], [4, 11, 16, 23], [6, 10, 15, 21]];
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    // The message, then a 1 bit, then zeros up to 8 bytes short of a multiple
    // of 64, then its length in bits.
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend((input.len() as u64 * 8).to_le_bytes());
    for block in message.as_chunks::<64>().0 {
        let word = |i: usize| u32::from_le_bytes(block.as_chunks::<4>().0[i]);
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (mixed, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            // The constant for step i is the first 32 bits of |sin(i + 1)|.
            let constant = (((i + 1) as f64).sin().abs() * 4_294_967_296.0) as u32;
            let sum =
                a.wrapping_add(mixed).wrapping_add(constant).wrapping_add(word(g));
            (a, d, c) = (d, c, b);
            b = b.wrapping_add(sum.rotate_left(SHIFTS[i / 16][i % 4]));
        }
        for (total, part) in state.iter_mut().zip([a, b, c, d]) {
            *total = total.wrapping_add(part);
        }
    }
    state
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
// @end

// @listing slt-report
/// What happened when a script was run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SltReport {
    /// Statements that did what the script said they would: succeeded, or
    /// failed where failure was expected.
    pub statements_ok: usize,
    pub statements_wrong: usize,
    /// Queries whose results matched.
    pub queries_ok: usize,
    /// Queries that ran and gave different results.
    pub queries_wrong: usize,
    /// Queries that felite refused to run.
    pub queries_refused: usize,
    /// Records that are marked as being for one particular database.
    pub skipped: usize,
    /// Each kind of refusal, of a query or of a statement that should have
    /// worked, with how many times it happened.
    pub refusals: BTreeMap<String, usize>,
}

impl SltReport {
    /// Adds another report's counts to this one's.
    pub fn add(&mut self, other: &SltReport) {
        self.statements_ok += other.statements_ok;
        self.statements_wrong += other.statements_wrong;
        self.queries_ok += other.queries_ok;
        self.queries_wrong += other.queries_wrong;
        self.queries_refused += other.queries_refused;
        self.skipped += other.skipped;
        for (reason, count) in &other.refusals {
            *self.refusals.entry(reason.clone()).or_default() += count;
        }
    }
}
// @end

// @listing slt-format
/// One value of a result, as sqllogictest writes it for a column of a given
/// type: `I` for an integer, `R` for a real, `T` for text.
fn show(value: &Value, column: char) -> String {
    match (value, column) {
        (Value::Null, _) => "NULL".to_string(),
        (Value::Integer(i), 'R') => format!("{:.3}", *i as f64),
        (Value::Real(x), 'R') => format!("{x:.3}"),
        (Value::Real(x), 'I') => format!("{}", *x as i64),
        (Value::Text(text), 'I') => format!("{}", text.parse::<i64>().unwrap_or(0)),
        (Value::Text(text), _) if text.is_empty() => "(empty)".to_string(),
        (value, _) => value.to_string(),
    }
}

/// A query's rows as the lines the script should have: one value to a line,
/// sorted as the script asks, and replaced by a count and a hash if there are
/// more than `threshold` of them.
fn result_lines(
    rows: &[Vec<Value>],
    types: &str,
    sort: &str,
    threshold: usize,
) -> Vec<String> {
    let line = |row: &Vec<Value>| -> Vec<String> {
        row.iter()
            .zip(types.chars().chain(std::iter::repeat('T')))
            .map(|(v, t)| show(v, t))
            .collect()
    };
    let mut shown: Vec<Vec<String>> = rows.iter().map(line).collect();
    if sort == "rowsort" {
        shown.sort();
    }
    let mut values: Vec<String> = shown.into_iter().flatten().collect();
    if sort == "valuesort" {
        values.sort();
    }
    if threshold > 0 && values.len() > threshold {
        let text: String = values.iter().map(|value| format!("{value}\n")).collect();
        return vec![format!(
            "{} values hashing to {}",
            values.len(),
            md5(text.as_bytes())
        )];
    }
    values
}
// @end

// @listing slt-reason
/// A short name for why felite refused a query: its error message with the
/// particulars taken out, so that like refusals can be counted together.
fn reason(message: &str) -> String {
    let message = message.rsplit(": ").next().unwrap_or(message);
    let mut kept = String::new();
    let mut quoted = false;
    for c in message.chars() {
        match c {
            '`' => quoted = !quoted,
            // Names and numbers vary from one query to the next.
            c if quoted || !c.is_ascii_digit() => kept.push(c),
            _ => {}
        }
    }
    kept
}
// @end

// @listing slt-run
/// Runs a sqllogictest script against a database, and reports how it went.
pub fn run_slt(db: &mut Database, script: &str) -> SltReport {
    let mut report = SltReport::default();
    // A result of more than this many values is recorded as a hash. Eight is
    // the default, and a script can change it.
    let mut threshold = 8;
    // Records are separated by blank lines. Some scripts have Windows line ends.
    for record in script.replace("\r\n", "\n").split("\n\n") {
        let lines: Vec<&str> =
            record.lines().filter(|line| !line.starts_with('#')).collect();
        let Some(first) = lines.iter().position(|line| !line.contains("if ")) else {
            continue;
        };
        if lines[..first].iter().any(|line| line.starts_with("onlyif")) {
            report.skipped += 1;
            continue;
        }
        let words: Vec<&str> = lines[first].split_whitespace().collect();
        let body = &lines[first + 1..];
        match words.as_slice() {
            ["hash-threshold", n] => threshold = n.parse().unwrap_or(0),
            ["halt"] => break,
            ["statement", expected] => {
                let outcome = db.execute(&body.join("\n"));
                match outcome.is_ok() == (*expected == "ok") {
                    true => report.statements_ok += 1,
                    false => report.statements_wrong += 1,
                }
                if let (Err(error), "ok") = (outcome, *expected) {
                    *report
                        .refusals
                        .entry(reason(&error.to_string()))
                        .or_default() += 1;
                }
            }
            ["query", types, rest @ ..] => {
                let split = body
                    .iter()
                    .position(|line| *line == "----")
                    .unwrap_or(body.len());
                let expected: Vec<&str> =
                    body.iter().skip(split + 1).copied().collect();
                let sort = rest.first().copied().unwrap_or("nosort");
                match db.execute(&body[..split].join("\n")) {
                    Ok(Output::Rows(result)) => {
                        let got = result_lines(&result.rows, types, sort, threshold);
                        match got == expected {
                            true => report.queries_ok += 1,
                            false => report.queries_wrong += 1,
                        }
                    }
                    Ok(_) => report.queries_wrong += 1,
                    Err(error) => {
                        report.queries_refused += 1;
                        *report
                            .refusals
                            .entry(reason(&error.to_string()))
                            .or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    report
}
// @end
