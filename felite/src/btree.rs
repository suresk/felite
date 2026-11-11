//! The B+tree: every table and every index is one of these.

mod check;
mod cursor;
mod insert;
mod node;
pub(crate) mod search;

pub use check::TreeInfo;
pub use cursor::{Cursor, Entry};
pub use insert::MAX_CELL;
pub use node::{
    INTERIOR, LEAF, interior_cell, leaf_cell, parse_interior, parse_leaf,
};
pub use node::{row_id, row_key};
pub use search::Tree;
