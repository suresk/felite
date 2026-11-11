//! SQL as text: tokens, then syntax trees.

mod token;

pub use token::{Keyword, Span, Symbol, Token, explain, tokenize};
