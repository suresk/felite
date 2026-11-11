//! The B+tree: every table and every index is one of these.

mod node;
pub(crate) mod search;

pub use node::{
    INTERIOR, LEAF, interior_cell, leaf_cell, parse_interior, parse_leaf,
};
pub use node::{row_id, row_key};
pub use search::Tree;
