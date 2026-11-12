# felite

felite is a small SQL database written in plain Rust, organized the way SQLite is: a single
file of fixed-size pages, B+trees for tables and indexes, a write-ahead log, and a SQL front
end. It has no dependencies.

It is the code for the book *Build a Database in Rust* by Spencer Uresk, **which is still
being written**. The book builds felite one chapter at a time, and every listing in it is
printed from this repository, so the code here is the code on the page. Chapter 1 is a
[free preview](https://1910815694620.gumroad.com/l/build-a-database-in-rust-preview), and
downloading it gets you one email when the full book is out.

felite is for learning. It is tested thoroughly, including against damaged files, but it is
not meant to replace SQLite.

## Where it stands

The repository grows with the book. So far it has the code for Chapters 1 to 15: Part I (storage) and Part II (SQL):

| Chapter | | Main files |
|---|---|---|
| 1 | A database is a file of pages | `src/page.rs`, `src/storage.rs`, `src/error.rs`, `src/pager.rs`, `src/dump.rs`, `examples/first_db.rs` |
| 2 | Rows as bytes | `src/varint.rs`, `src/record.rs` |
| 3 | Slotted pages | `src/slotted.rs` |
| 4 | A B+tree: search | `src/btree/node.rs`, `src/btree/search.rs` |
| 5 | Insert and split | `src/btree/insert.rs`, `src/btree/check.rs`, `examples/thousand_rows.rs` |
| 6 | Cursors and range scans | `src/btree/cursor.rs`, `examples/scan_speed.rs` |
| 7 | Delete, merge and the free list | `src/btree/delete.rs`, `src/pager.rs` |
| 8 | Rows larger than a page | `src/overflow.rs` |
| 9 | The buffer pool | `src/pager.rs` |
| 10 | A tokenizer | `src/sql/token.rs` |
| 11 | A parser | `src/sql/ast.rs`, `src/sql/parser.rs` |
| 12 | The catalog | `src/catalog.rs`, `src/db.rs` |
| 13 | Expressions and NULL | `src/bind.rs`, `src/eval.rs` |
| 14 | INSERT and SELECT | `src/exec.rs`, `src/query.rs`, `src/insert.rs`, `src/shell.rs`, `src/bin/felite.rs` |
| 15 | UPDATE and DELETE | `src/modify.rs` |
| 16 | What a crash does | `src/crash.rs`, `src/check.rs` |
| 17 | A write-ahead log | `src/wal.rs`, `src/pager.rs` |
| 18 | Recovery and checkpoints | `src/wal.rs`, `src/pager.rs`, `examples/crash_test.rs` |
| 19 | Transactions | `src/transaction.rs` |
| 20 | Sorting | `src/sort.rs`, `src/storage.rs`, `fixtures/flights.sql` |
| 21 | Aggregates | `src/aggregate.rs` |
| 22 | Joins | `src/join.rs` |
| 23 | Secondary indexes | `src/keys.rs`, `src/index.rs` |
| 24 | A query planner | `src/plan.rs` |
| 25 | The scoreboard | `src/csv.rs`, `src/slt.rs`, `examples/load.rs`, `examples/queries.rs` |

Paths are inside `felite/`. Still to come: the
write-ahead log and crash recovery, and a query planner. Until the book is finished, code
from earlier chapters can still change.

## Running it

You need Rust 1.99 or later. The repository pins the toolchain in `rust-toolchain.toml`, so
`rustup` installs the right one the first time you run `cargo`.

```text
$ cargo test                       # every example and challenge so far
$ cargo run --example first_db     # makes first.db, a three-page database (Chapter 1)
$ cargo xtask dump first.db 2      # a page of it as a hex dump
$ cargo xtask cells some.db 3      # the cells of a slotted page (Chapter 3)
$ cargo xtask check 4              # Chapter 4's examples, and each of its challenges
$ cargo xtask check 4.2            # one challenge, with the full output
$ cargo run --release -p xtask -- data   # the full flight data set, into data/ (Chapter 25)
$ cargo xtask slt                  # sqllogictest scripts against felite (Chapter 25)
$ cargo run --bin felite -- my.db   # a prompt: type SQL, end each statement with ;
```

## Following the book

The repository is a Cargo workspace with two crates. `felite/` is the library and `xtask/`
holds the tools above. Each chapter has two test files in `felite/tests/`:

- `book_chNN.rs` holds the examples the chapter prints. Every number the text quotes is an
  assertion there.
- `challenges_chNN.rs` checks the end-of-chapter challenges. The solutions are in
  `felite/src/challenges/chNN.rs`, so don't open that directory until you have tried them.

## Checking your own code

If you are writing felite yourself as you read, the same command that checks a chapter here
runs its tests against your crate. Clone this repository next to yours and give it the
directory that holds your `Cargo.toml`:

```text
$ cargo xtask check 7 --crate ../my-felite      # Chapter 7: its examples and every challenge
$ cargo xtask check 7.2 --crate ../my-felite    # Challenge 7.2 alone, with the full output
```

```text
Chapter 7
  Examples          passed (11 tests)
  Challenge 7.1     FAILED (1 of 1 test)
      ✗ predict_the_free_list
        assertion `left == right` failed   left: [5, 3, 0]  right: [1, 5, 3]
  Challenge 7.2     not started (no `delete_range`)
  Challenge 7.3     passed (1 test)
```

Each challenge is compiled on its own, so the ones you haven't written don't get in the way
of the ones you have. Nothing is written into your crate: the tests are copied to
`target/check` here and built against yours, along with the data files in `felite/fixtures`.
Your package can have any name. Set the `FELITE_CRATE` environment variable to leave out
`--crate`.

Two things to know. The tests reach everything by its name at the root of the crate, such as
`felite::delete_range`, so export each challenge's solution from your `lib.rs`. And they
assert exact numbers and exact error messages, since those are what the book prints. If you
word an error differently, that test will say so, and you can decide whether you mind.

A few tests check two challenges at once, because one challenge's answer is used to check
another's. Those run under both.

The `// @listing` comments mark the regions the book prints. They are how the book's build
finds its listings, and they have no effect on the code.

## A taste

```rust
use felite::{MemFile, Page, Pager, Value, decode, encode};

// A database in memory, with one page holding one row.
let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
let mut page = Page::new();
page.init_slotted(1);
let row = [Value::Integer(1432), Value::Text("SLC".to_string()), Value::Null];
page.insert_cell(0, &encode(&row)).unwrap();
let n = pager.allocate(page).unwrap();

let stored = pager.page(n).unwrap().cell(0).unwrap();
assert_eq!(decode(stored).unwrap(), row);
```

## Data and test files

`felite/fixtures/flights.sql` is a sample of the US Bureau of Transportation Statistics'
on-time performance data for January 2024, a work of the US government. `cargo xtask data`
downloads the full months from the Bureau and checks that the sample still matches.
`felite/fixtures/select1.test` is from sqllogictest, which the SQLite project wrote and placed
in the public domain. `felite/fixtures/airports.sqlite` was made with SQLite from the same
airports, for the last challenge of the book.

## Found a mistake?

Please open an issue, whether it is a bug in the code or a place where the book and the
repository disagree.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. The license covers the code in this repository.
The text of the book is not part of it.
