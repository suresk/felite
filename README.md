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

The repository grows with the book. So far it has the code for Chapters 1 to 4:

| Chapter | | Main files |
|---|---|---|
| 1 | A database is a file of pages | `src/page.rs`, `src/storage.rs`, `src/error.rs`, `src/pager.rs`, `src/dump.rs`, `examples/first_db.rs` |
| 2 | Rows as bytes | `src/varint.rs`, `src/record.rs` |
| 3 | Slotted pages | `src/slotted.rs` |
| 4 | A B+tree: search | `src/btree/node.rs`, `src/btree/search.rs` |

Paths are inside `felite/`. Still to come: inserting into the tree and splitting pages,
cursors, deletion and a free list, a buffer pool, the SQL parser and executor, the
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
$ cargo xtask check 4              # one chapter's tests
```

## Following the book

The repository is a Cargo workspace with two crates. `felite/` is the library and `xtask/`
holds the tools above. Each chapter has two test files in `felite/tests/`:

- `book_chNN.rs` holds the examples the chapter prints. Every number the text quotes is an
  assertion there.
- `challenges_chNN.rs` checks the end-of-chapter challenges. The solutions are in
  `felite/src/challenges/chNN.rs`, so don't open that directory until you have tried them.

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

## Found a mistake?

Please open an issue, whether it is a bug in the code or a place where the book and the
repository disagree.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. The license covers the code in this repository.
The text of the book is not part of it.
