//! From tokens to a syntax tree.

use crate::error::{Error, Result};
use crate::record::Value;
use crate::sql::ast::{BinaryOp, ColumnDef, ColumnType, CreateTable, Expr, ExprKind};
use crate::sql::ast::{Insert, Select, SelectItem, Statement, UnaryOp};
use crate::sql::token::{Keyword, Span, Symbol, Token, tokenize};

// @listing parser
/// The tokens of one statement, and how far into them we have read.
struct Parser {
    tokens: Vec<(Token, Span)>,
    at: usize,
    /// How deep the expression being parsed has become. Section 11.8.
    depth: usize,
}

impl Parser {
    /// The token we are looking at. The list ends with `End`, and we never move
    /// past it, so there always is one.
    fn peek(&self) -> &Token {
        &self.tokens[self.at].0
    }

    fn span(&self) -> Span {
        self.tokens[self.at].1
    }

    /// Returns the token we are looking at and moves to the next.
    fn advance(&mut self) -> Token {
        let token = self.tokens[self.at].0.clone();
        if token != Token::End {
            self.at += 1;
        }
        token
    }
}
// @end

// @listing describe
/// A token as an error message should name it.
fn describe(token: &Token) -> String {
    match token {
        Token::Keyword(keyword) => format!("`{}`", keyword.spelling()),
        Token::Ident(name) => format!("the name `{name}`"),
        Token::Integer(value) => format!("the number {value}"),
        Token::Real(value) => format!("the number {value:?}"),
        Token::Text(text) => format!("the string '{text}'"),
        Token::Symbol(symbol) => format!("`{}`", symbol.spelling()),
        Token::End => "the end of the statement".to_string(),
    }
}

/// An error saying what was wanted at `span` and which token was there.
fn wrong<T>(wanted: &str, found: &Token, span: Span) -> Result<T> {
    Err(Error::Syntax(format!("expected {wanted}, found {}", describe(found)), span))
}
// @end

impl Parser {
    // @listing expect
    /// An error saying what was wanted where we are now.
    fn expected<T>(&self, wanted: &str) -> Result<T> {
        wrong(wanted, self.peek(), self.span())
    }

    /// If the next token is this keyword, moves past it and says so.
    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        let found = *self.peek() == Token::Keyword(keyword);
        if found {
            self.advance();
        }
        found
    }

    fn eat_symbol(&mut self, symbol: Symbol) -> bool {
        let found = *self.peek() == Token::Symbol(symbol);
        if found {
            self.advance();
        }
        found
    }

    /// Moves past this keyword, which must be next.
    fn expect_keyword(&mut self, keyword: Keyword) -> Result<()> {
        if self.eat_keyword(keyword) {
            return Ok(());
        }
        self.expected(&format!("`{}`", keyword.spelling()))
    }

    fn expect_symbol(&mut self, symbol: Symbol) -> Result<()> {
        if self.eat_symbol(symbol) {
            return Ok(());
        }
        self.expected(&format!("`{}`", symbol.spelling()))
    }

    /// Moves past a name, which must be next, and returns it.
    fn name(&mut self, what: &str) -> Result<String> {
        match self.peek() {
            Token::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => self.expected(what),
        }
    }
    // @end

    // @listing statement
    fn statement(&mut self) -> Result<Statement> {
        let statement = match self.peek() {
            Token::Keyword(Keyword::Create) => {
                Statement::CreateTable(self.create_table()?)
            }
            Token::Keyword(Keyword::Insert) => Statement::Insert(self.insert()?),
            Token::Keyword(Keyword::Select) => Statement::Select(self.select()?),
            // @hide
            Token::Keyword(Keyword::Drop) => Statement::DropTable(self.drop_table()?),
            // @show
            _ => return self.expected("a statement"),
        };
        self.eat_symbol(Symbol::Semicolon);
        match self.peek() {
            Token::End => Ok(statement),
            _ => self.expected("the end of the statement"),
        }
    }
    // @end

    // @listing parse-create
    fn create_table(&mut self) -> Result<CreateTable> {
        self.expect_keyword(Keyword::Create)?;
        self.expect_keyword(Keyword::Table)?;
        let name = self.name("a table name")?;
        self.expect_symbol(Symbol::LeftParen)?;
        let mut columns = vec![self.column_def()?];
        while self.eat_symbol(Symbol::Comma) {
            columns.push(self.column_def()?);
        }
        self.expect_symbol(Symbol::RightParen)?;
        Ok(CreateTable { name, columns })
    }

    fn column_def(&mut self) -> Result<ColumnDef> {
        let name = self.name("a column name")?;
        let span = self.span();
        let kind = match self.advance() {
            Token::Keyword(Keyword::Integer) => ColumnType::Integer,
            Token::Keyword(Keyword::Real) => ColumnType::Real,
            Token::Keyword(Keyword::Text) => ColumnType::Text,
            other => {
                return wrong(
                    "a column type (`INTEGER`, `REAL` or `TEXT`)",
                    &other,
                    span,
                );
            }
        };
        let mut column =
            ColumnDef { name, kind, primary_key: false, not_null: false };
        loop {
            if self.eat_keyword(Keyword::Primary) {
                self.expect_keyword(Keyword::Key)?;
                column.primary_key = true;
            } else if self.eat_keyword(Keyword::Not) {
                self.expect_keyword(Keyword::Null)?;
                column.not_null = true;
            } else {
                return Ok(column);
            }
        }
    }
    // @end

    // @listing parse-drop
    fn drop_table(&mut self) -> Result<String> {
        self.expect_keyword(Keyword::Drop)?;
        self.expect_keyword(Keyword::Table)?;
        self.name("a table name")
    }
    // @end

    // @listing parse-insert
    fn insert(&mut self) -> Result<Insert> {
        self.expect_keyword(Keyword::Insert)?;
        self.expect_keyword(Keyword::Into)?;
        let table = self.name("a table name")?;
        let mut columns = Vec::new();
        if self.eat_symbol(Symbol::LeftParen) {
            columns.push(self.name("a column name")?);
            while self.eat_symbol(Symbol::Comma) {
                columns.push(self.name("a column name")?);
            }
            self.expect_symbol(Symbol::RightParen)?;
        }
        self.expect_keyword(Keyword::Values)?;
        self.expect_symbol(Symbol::LeftParen)?;
        let mut values = vec![self.expr(0)?];
        while self.eat_symbol(Symbol::Comma) {
            values.push(self.expr(0)?);
        }
        self.expect_symbol(Symbol::RightParen)?;
        Ok(Insert { table, columns, values })
    }
    // @end

    // @listing parse-select
    fn select(&mut self) -> Result<Select> {
        self.expect_keyword(Keyword::Select)?;
        let mut items = vec![self.select_item()?];
        while self.eat_symbol(Symbol::Comma) {
            items.push(self.select_item()?);
        }
        self.expect_keyword(Keyword::From)?;
        let from = self.name("a table name")?;
        let filter =
            if self.eat_keyword(Keyword::Where) { Some(self.expr(0)?) } else { None };
        Ok(Select { items, from, filter })
    }

    fn select_item(&mut self) -> Result<SelectItem> {
        if self.eat_symbol(Symbol::Star) {
            return Ok(SelectItem::Star);
        }
        let expr = self.expr(0)?;
        let alias = if self.eat_keyword(Keyword::As) {
            Some(self.name("a name")?)
        } else {
            None
        };
        Ok(SelectItem::Expr { expr, alias })
    }
    // @end
}

// @listing binding-power
/// How tightly each binary operator holds its operands. A larger number binds
/// tighter.
fn binding_power(token: &Token) -> Option<(BinaryOp, u8)> {
    use BinaryOp::*;
    Some(match token {
        Token::Keyword(Keyword::Or) => (Or, 1),
        Token::Keyword(Keyword::And) => (And, 2),
        Token::Symbol(Symbol::Equal) => (Equal, 4),
        Token::Symbol(Symbol::NotEqual) => (NotEqual, 4),
        Token::Symbol(Symbol::Less) => (Less, 4),
        Token::Symbol(Symbol::LessEqual) => (LessEqual, 4),
        Token::Symbol(Symbol::Greater) => (Greater, 4),
        Token::Symbol(Symbol::GreaterEqual) => (GreaterEqual, 4),
        Token::Symbol(Symbol::Plus) => (Add, 5),
        Token::Symbol(Symbol::Minus) => (Subtract, 5),
        Token::Symbol(Symbol::Star) => (Multiply, 6),
        Token::Symbol(Symbol::Slash) => (Divide, 6),
        Token::Symbol(Symbol::Percent) => (Remainder, 6),
        _ => return None,
    })
}

/// The binding powers of the operators that are not in the table above.
const NOT_POWER: u8 = 3;
const IS_POWER: u8 = 4;
const NEGATE_POWER: u8 = 7;
// @end

// @listing max-depth
/// The deepest tree the parser will build for one expression.
const MAX_DEPTH: usize = 100;
// @end

impl Parser {
    // @listing parse-expr
    /// Parses an expression made of operators that bind more tightly than `floor`.
    fn expr(&mut self, floor: u8) -> Result<Expr> {
        let depth = self.depth;
        let result = self.climb(floor);
        self.depth = depth;
        result
    }

    fn climb(&mut self, floor: u8) -> Result<Expr> {
        self.deeper()?;
        let mut left = self.prefix()?;
        loop {
            let span = left.span;
            if *self.peek() == Token::Keyword(Keyword::Is) && IS_POWER > floor {
                self.advance();
                let negated = self.eat_keyword(Keyword::Not);
                self.expect_keyword(Keyword::Null)?;
                let kind = ExprKind::IsNull { operand: Box::new(left), negated };
                left = Expr { kind, span };
                continue;
            }
            let Some((op, power)) = binding_power(self.peek()) else {
                return Ok(left);
            };
            if power <= floor {
                return Ok(left);
            }
            self.advance();
            self.deeper()?;
            let right = self.expr(power)?;
            let kind =
                ExprKind::Binary { op, left: Box::new(left), right: Box::new(right) };
            left = Expr { kind, span };
        }
    }
    // @end

    // @listing deeper
    /// Counts one more level of the expression, and refuses if there are too many.
    fn deeper(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let message = format!("an expression more than {MAX_DEPTH} levels deep");
            return Err(Error::Syntax(message, self.span()));
        }
        Ok(())
    }
    // @end

    // @listing parse-prefix
    /// Parses what an expression can start with: a literal, a column, something
    /// in parentheses, or a prefix operator and its operand.
    fn prefix(&mut self) -> Result<Expr> {
        let span = self.span();
        let kind = match self.advance() {
            Token::Integer(value) => ExprKind::Literal(Value::Integer(value)),
            Token::Real(value) => ExprKind::Literal(Value::Real(value)),
            Token::Text(text) => ExprKind::Literal(Value::Text(text)),
            Token::Keyword(Keyword::Null) => ExprKind::Literal(Value::Null),
            Token::Ident(first) => {
                if self.eat_symbol(Symbol::Dot) {
                    let name = self.name("a column name")?;
                    ExprKind::Column { table: Some(first), name }
                } else {
                    ExprKind::Column { table: None, name: first }
                }
            }
            Token::Symbol(Symbol::LeftParen) => {
                let inner = self.expr(0)?;
                self.expect_symbol(Symbol::RightParen)?;
                return Ok(inner);
            }
            Token::Symbol(Symbol::Minus) => {
                let operand = Box::new(self.expr(NEGATE_POWER)?);
                ExprKind::Unary { op: UnaryOp::Negate, operand }
            }
            Token::Keyword(Keyword::Not) => {
                let operand = Box::new(self.expr(NOT_POWER)?);
                ExprKind::Unary { op: UnaryOp::Not, operand }
            }
            other => return wrong("an expression", &other, span),
        };
        Ok(Expr { kind, span })
    }
    // @end
}

// @listing parse
/// Parses one statement.
pub fn parse(source: &str) -> Result<Statement> {
    let mut parser = Parser { tokens: tokenize(source)?, at: 0, depth: 0 };
    parser.statement()
}

/// Parses text that is a single expression and nothing else.
pub fn parse_expr(source: &str) -> Result<Expr> {
    let mut parser = Parser { tokens: tokenize(source)?, at: 0, depth: 0 };
    let expr = parser.expr(0)?;
    match parser.peek() {
        Token::End => Ok(expr),
        _ => parser.expected("the end of the expression"),
    }
}
// @end
