mod btree;
mod catalog;
mod challenges;
mod db;
mod dump;
mod error;
mod overflow;
mod page;
mod pager;
mod record;
mod slotted;
mod sql;
mod storage;
mod varint;

pub use btree::{Cursor, Entry, MAX_CELL, TreeInfo, row_id, row_key};
pub use btree::{
    INTERIOR, LEAF, Tree, interior_cell, leaf_cell, parse_interior, parse_leaf,
};
pub use catalog::Table;
pub use db::{Database, Output};
pub use dump::hex_dump;
pub use error::{Error, Result};
pub use overflow::{OVERFLOW, SPILLED, load, release, store};
pub use page::{PAGE_SIZE, Page, PageNo};
pub use pager::{DEFAULT_FRAMES, FREE, MAGIC, Pager, PoolStats};
pub use record::{Value, decode, encode};
pub use slotted::{SLOT_SIZE, SLOTTED_HEADER};
pub use sql::{BinaryOp, ColumnDef, ColumnType, CreateTable, Expr, ExprKind, Insert};
pub use sql::{Keyword, Span, Symbol, Token, explain, tokenize};
pub use sql::{Select, SelectItem, Statement, UnaryOp, parse, parse_expr};
pub use storage::{MemFile, Storage};
pub use varint::{put_varint, unzigzag, varint, zigzag};

pub use challenges::ch01::{Counting, Counts, FailAfter, ReadOnly, open_checked};
pub use challenges::ch02::varint_strict;
pub use challenges::ch02::{
    column, decode_compact, encode_compact, key_f64, key_i64,
};
pub use challenges::ch03::{
    check_slotted, insert_cell_reusing, replace_cell, truncate_cells,
};
pub use challenges::ch04::{contains, first_key, last_key, path, separator};
pub use challenges::ch05::{bulk_load, count, put};
pub use challenges::ch06::{
    ReverseCursor, StableCursor, count_range, prefix_end, prefix_scan,
};
pub use challenges::ch07::{account, delete_range, free_pages, rebuild, tree_pages};
pub use challenges::ch08::{account_rows, load_range, update_row};
pub use challenges::ch09::{Shared, SharedPool, clock_misses, lru_misses};
pub use challenges::ch10::{hex_literal, scan_quoted, strip_block_comments, to_sql};
pub use challenges::ch11::{columns_used, depth, to_minimal_sql};
pub use challenges::ch12::{
    account_database, create_if_missing, rename_table, schema_dump,
};
