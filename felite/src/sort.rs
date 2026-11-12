//! ORDER BY: sorting rows in memory, or in runs on a temporary file.

use crate::bind::Bound;
use crate::error::{Error, Result};
use crate::eval::{eval, order};
use crate::exec::{Operator, Row};
use crate::pager::Pager;
use crate::record::{Value, decode, encode};
use crate::storage::{Storage, TempSpace};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

// @listing sort-key
/// One key of a sort: what to sort by, and which way.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    pub expr: Bound,
    pub descending: bool,
}

/// A row's value for one key. It knows its key's direction, so that comparing
/// two of them gives the order their rows should come out in.
#[derive(Debug, Clone)]
pub(crate) struct Key {
    pub(crate) value: Value,
    pub(crate) descending: bool,
}

impl Ord for Key {
    fn cmp(&self, other: &Key) -> Ordering {
        let by_value = order(&self.value, &other.value);
        if self.descending { by_value.reverse() } else { by_value }
    }
}
// @end

// @listing key-traits
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Key) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Key) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Key {}
// @end

// @listing sort-stats
/// What a sort did, for tests and for the curious.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SortStats {
    /// Rows that went in.
    pub rows: usize,
    /// Sorted runs written to the temporary file. Zero if everything fit.
    pub runs: usize,
    /// The most bytes of records that were in memory at once.
    pub peak: usize,
    /// Bytes written to the temporary file.
    pub spilled: u64,
}
// @end

// @listing sort
/// How many bytes `Database` lets a sort hold in memory: 4 MiB.
pub const SORT_BUDGET: usize = 4 << 20;

/// The bytes in front of each record in a run, which hold its length.
const PREFIX: usize = 4;

/// Part of the temporary file: a sorted run, from `at` up to `end`. Reading the
/// run moves `at` forward.
struct Run {
    at: u64,
    end: u64,
}

/// Hands on the rows of its input in the order of their keys. Rows whose keys
/// are all equal stay in the order they arrived.
pub struct Sort {
    input: Box<dyn Operator>,
    keys: Vec<SortKey>,
    /// The most bytes of records to hold in memory.
    budget: usize,
    temp: TempSpace,
    /// Whether the input has been read to its end.
    ready: bool,
    /// The rows in memory: each one's keys, and the row as a record.
    buffer: Vec<(Vec<Key>, Vec<u8>)>,
    /// The size of the records in `buffer`, with a prefix for each.
    buffered: usize,
    /// The temporary file, once there has been a reason to create one.
    file: Option<Box<dyn Storage>>,
    runs: Vec<Run>,
    /// While merging: the keys of each run's next row, smallest on top.
    heap: BinaryHeap<Reverse<(Vec<Key>, usize)>>,
    /// While merging: each run's next row.
    heads: Vec<Option<Row>>,
    stats: SortStats,
}
// @end

impl Sort {
    // @listing sort-new
    pub fn new(
        input: Box<dyn Operator>,
        keys: Vec<SortKey>,
        budget: usize,
        temp: TempSpace,
    ) -> Sort {
        Sort {
            input,
            keys,
            budget,
            temp,
            ready: false,
            buffer: Vec::new(),
            buffered: 0,
            file: None,
            runs: Vec::new(),
            heap: BinaryHeap::new(),
            heads: Vec::new(),
            stats: SortStats::default(),
        }
    }

    pub fn stats(&self) -> SortStats {
        self.stats
    }

    /// The values a row is sorted by.
    fn keys_of(&self, row: &Row) -> Result<Vec<Key>> {
        let mut keys = Vec::with_capacity(self.keys.len());
        for key in &self.keys {
            let value = eval(&key.expr, row)?;
            keys.push(Key { value, descending: key.descending });
        }
        Ok(keys)
    }
    // @end

    // @listing read-input
    /// Reads the input to its end, and leaves it sorted: in `buffer` if it fit
    /// there, and otherwise in runs.
    fn read_input(&mut self, pager: &mut Pager) -> Result<()> {
        while let Some(row) = self.input.next(pager)? {
            let record = encode(&row);
            let size = PREFIX + record.len();
            // A row that would take the buffer over the budget goes in the next
            // run. A row that is over the budget by itself is a run of one.
            if self.buffered + size > self.budget && !self.buffer.is_empty() {
                self.write_run()?;
            }
            self.buffer.push((self.keys_of(&row)?, record));
            self.buffered += size;
            self.stats.rows += 1;
            self.stats.peak = self.stats.peak.max(self.buffered);
        }
        if self.runs.is_empty() {
            // `sort_by` keeps equal rows in order. The buffer is then turned
            // around, so that `pop` takes the first row and not the last.
            self.buffer.sort_by(|a, b| a.0.cmp(&b.0));
            self.buffer.reverse();
            return Ok(());
        }
        self.write_run()?;
        for run in 0..self.runs.len() {
            self.heads.push(None);
            self.advance(run)?;
        }
        Ok(())
    }
    // @end

    // @listing write-run
    /// Sorts the rows in memory and writes them to the end of the temporary file
    /// as one run.
    fn write_run(&mut self) -> Result<()> {
        if self.file.is_none() {
            self.file = Some(self.temp.create()?);
        }
        let Some(file) = &mut self.file else { unreachable!() };
        self.buffer.sort_by(|a, b| a.0.cmp(&b.0));
        let start = self.runs.last().map_or(0, |run| run.end);
        let mut end = start;
        for (_, record) in self.buffer.drain(..) {
            let mut bytes = (record.len() as u32).to_be_bytes().to_vec();
            bytes.extend_from_slice(&record);
            file.write_at(end, &bytes)?;
            end += bytes.len() as u64;
        }
        self.runs.push(Run { at: start, end });
        self.buffered = 0;
        self.stats.runs += 1;
        self.stats.spilled = end;
        Ok(())
    }
    // @end

    // @listing advance
    /// Reads the next row of a run, if it has one, and puts its keys in the heap.
    fn advance(&mut self, run: usize) -> Result<()> {
        let Run { at, end } = self.runs[run];
        if at == end {
            return Ok(());
        }
        let Some(file) = &mut self.file else { unreachable!() };
        let mut prefix = [0; PREFIX];
        file.read_at(at, &mut prefix)?;
        let next = at + (PREFIX + u32::from_be_bytes(prefix) as usize) as u64;
        if next > end {
            let what = "a sorted run ends in the middle of a row";
            return Err(Error::Corrupt(what.to_string()));
        }
        let mut record = vec![0; (next - at) as usize - PREFIX];
        file.read_at(at + PREFIX as u64, &mut record)?;
        self.runs[run].at = next;

        let row = decode(&record)?;
        self.heap.push(Reverse((self.keys_of(&row)?, run)));
        self.heads[run] = Some(row);
        Ok(())
    }
    // @end
}

// @listing sort-next
impl Operator for Sort {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Row>> {
        if !self.ready {
            self.read_input(pager)?;
            self.ready = true;
        }
        if self.runs.is_empty() {
            return match self.buffer.pop() {
                Some((_, record)) => Ok(Some(decode(&record)?)),
                None => Ok(None),
            };
        }
        // The top of the heap is the smallest row that any run has to offer.
        let Some(Reverse((_, run))) = self.heap.pop() else {
            return Ok(None);
        };
        let row = self.heads[run].take();
        self.advance(run)?;
        Ok(row)
    }
}
// @end
