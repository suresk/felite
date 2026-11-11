mod btree;
mod challenges;
mod dump;
mod error;
mod page;
mod pager;
mod record;
mod slotted;
mod storage;
mod varint;

pub use btree::{
    INTERIOR, LEAF, Tree, interior_cell, leaf_cell, parse_interior, parse_leaf,
};
pub use btree::{MAX_CELL, TreeInfo, row_id, row_key};
pub use dump::hex_dump;
pub use error::{Error, Result};
pub use page::{PAGE_SIZE, Page, PageNo};
pub use pager::{MAGIC, Pager};
pub use record::{Value, decode, encode};
pub use slotted::{SLOT_SIZE, SLOTTED_HEADER};
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
