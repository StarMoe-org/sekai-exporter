//! A parser for the Lua of MySekai scripts.
//!
//! The scripts are data to the forms that read them: a form walks the syntax tree of a script and
//! takes the calls and the control flow it knows.

use std::fmt;

use serde::Serialize;

/// A number as a script writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    pub(crate) fn value(self) -> f64 {
        match self {
            Self::Int(value) => value as f64,
            Self::Float(value) => value,
        }
    }
}

/// A sequence of statements.
pub(crate) type Block = Vec<Stat>;

/// A call. A method call has the method as its callee.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Call {
    pub(crate) callee: Expr,
    pub(crate) arguments: Vec<Expr>,
}

impl Call {
    /// The name of a call to a plain name.
    pub(crate) fn name(&self) -> Option<&str> {
        match &self.callee {
            Expr::Name(name) => Some(name),
            _ => None,
        }
    }
}

/// An entry of a table constructor.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TableItem {
    Named(String, Expr),
    Keyed(Expr, Expr),
    Positional(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Not,
    Negate,
    Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Or,
    And,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    NotEqual,
    Equal,
    Concat,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Nil,
    Bool(bool),
    Number(Number),
    Str(String),
    Vararg,
    Name(String),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Call>),
    Function(Block),
    Table(Vec<TableItem>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Stat {
    Call(Call),
    Local(Vec<String>, Vec<Expr>),
    Assign(Vec<Expr>, Vec<Expr>),
    If(Vec<(Expr, Block)>, Option<Block>),
    While(Expr, Block),
    Repeat(Block, Expr),
    For(Vec<Expr>, Block),
    Function(Expr, Block),
    Do(Block),
    Return(Vec<Expr>),
    Break,
}

/// A script that is not Lua.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LuaError {
    line: u32,
    message: String,
}

impl fmt::Display for LuaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// The statements of `source`. A byte order mark at its start is ignored.
pub(crate) fn parse(source: &str) -> Result<Block, LuaError> {
    let tokens = Lexer::new(source.strip_prefix('\u{feff}').unwrap_or(source)).tokens()?;
    Parser { tokens, at: 0 }.chunk()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Number(Number),
    Str(String),
    /// A keyword or a punctuation mark.
    Mark(&'static str),
    End,
}

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The punctuation marks, each before the marks it starts with.
const PUNCTUATION: &[&str] = &[
    "...", "..", "==", "~=", "<=", ">=", "+", "-", "*", "/", "%", "^", "#", "<", ">", "=", "(",
    ")", "{", "}", "[", "]", ";", ":", ",", ".",
];

struct Lexer<'a> {
    source: &'a str,
    at: usize,
    line: u32,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            at: 0,
            line: 1,
        }
    }

    /// The tokens of the source with their lines, ending with [`Token::End`].
    fn tokens(mut self) -> Result<Vec<(Token, u32)>, LuaError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_blanks()?;
            let line = self.line;
            let token = self.token()?;
            let end = token == Token::End;
            tokens.push((token, line));
            if end {
                return Ok(tokens);
            }
        }
    }

    fn rest(&self) -> &'a str {
        &self.source[self.at..]
    }

    fn byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.at).copied()
    }

    fn error(&self, message: impl Into<String>) -> LuaError {
        LuaError {
            line: self.line,
            message: message.into(),
        }
    }

    /// Moves past the bytes that `keep` accepts.
    fn skip_while(&mut self, keep: impl Fn(u8) -> bool) {
        while self.byte().is_some_and(&keep) {
            self.at += 1;
        }
    }

    /// Moves `length` bytes on, counting the line ends passed.
    fn advance(&mut self, length: usize) {
        let passed = &self.rest()[..length];
        self.line += passed.matches('\n').count() as u32;
        self.at += length;
    }

    /// Moves past white space and comments.
    fn skip_blanks(&mut self) -> Result<(), LuaError> {
        loop {
            let rest = self.rest();
            let blank = rest.len() - rest.trim_start().len();
            self.advance(blank);
            if !self.rest().starts_with("--") {
                return Ok(());
            }
            self.at += 2;
            match self.long_bracket() {
                Some(level) => {
                    self.long_text(level)?;
                }
                None => self.skip_while(|byte| byte != b'\n'),
            }
        }
    }

    /// The level of the long bracket that opens here.
    fn long_bracket(&self) -> Option<usize> {
        let rest = self.rest().strip_prefix('[')?;
        let level = rest.len() - rest.trim_start_matches('=').len();
        rest[level..].starts_with('[').then_some(level)
    }

    /// The text of the long bracket of `level` that opens here.
    fn long_text(&mut self, level: usize) -> Result<&'a str, LuaError> {
        self.at += level + 2;
        let close = format!("]{}]", "=".repeat(level));
        let length = self
            .rest()
            .find(&close)
            .ok_or_else(|| self.error("a long bracket is not closed"))?;
        let body = &self.rest()[..length];
        self.advance(length + close.len());
        Ok(body
            .strip_prefix("\r\n")
            .or(body.strip_prefix('\n'))
            .unwrap_or(body))
    }

    fn token(&mut self) -> Result<Token, LuaError> {
        let Some(byte) = self.byte() else {
            return Ok(Token::End);
        };
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => Ok(self.word()),
            b'0'..=b'9' => self.number(),
            b'.' if self
                .rest()
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_digit) =>
            {
                self.number()
            }
            b'"' | b'\'' => self.quoted(byte),
            b'[' => match self.long_bracket() {
                Some(level) => self
                    .long_text(level)
                    .map(|text| Token::Str(text.to_owned())),
                None => self.punctuation(),
            },
            _ => self.punctuation(),
        }
    }

    fn word(&mut self) -> Token {
        let start = self.at;
        self.skip_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        let word = &self.source[start..self.at];
        match KEYWORDS.iter().find(|keyword| **keyword == word) {
            Some(keyword) => Token::Mark(keyword),
            None => Token::Name(word.to_owned()),
        }
    }

    fn punctuation(&mut self) -> Result<Token, LuaError> {
        let rest = self.rest();
        let mark = PUNCTUATION
            .iter()
            .find(|mark| rest.starts_with(**mark))
            .ok_or_else(|| self.error("an unknown symbol"))?;
        self.at += mark.len();
        Ok(Token::Mark(mark))
    }

    fn number(&mut self) -> Result<Token, LuaError> {
        let start = self.at;
        let hexadecimal = self.rest().starts_with("0x") || self.rest().starts_with("0X");
        let number = if hexadecimal {
            self.at += 2;
            self.skip_while(|byte| byte.is_ascii_hexdigit());
            i64::from_str_radix(&self.source[start + 2..self.at], 16)
                .ok()
                .map(Number::Int)
        } else {
            self.decimal()
        };
        if self
            .byte()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(self.error("a malformed number"));
        }
        number
            .map(Token::Number)
            .ok_or_else(|| self.error("a malformed number"))
    }

    fn decimal(&mut self) -> Option<Number> {
        let start = self.at;
        self.skip_while(|byte| byte.is_ascii_digit());
        let fraction = self.byte() == Some(b'.');
        if fraction {
            self.at += 1;
            self.skip_while(|byte| byte.is_ascii_digit());
        }
        let exponent = matches!(self.byte(), Some(b'e' | b'E'));
        if exponent {
            self.at += 1;
            if matches!(self.byte(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            self.skip_while(|byte| byte.is_ascii_digit());
        }
        let text = &self.source[start..self.at];
        if fraction || exponent {
            text.parse().ok().map(Number::Float)
        } else {
            text.parse().ok().map(Number::Int)
        }
    }

    fn quoted(&mut self, quote: u8) -> Result<Token, LuaError> {
        self.at += 1;
        let mut bytes = Vec::new();
        loop {
            let byte = self
                .byte()
                .ok_or_else(|| self.error("a string is not closed"))?;
            self.at += 1;
            match byte {
                b'\\' => self.escape(&mut bytes)?,
                b'\n' => return Err(self.error("a string is not closed")),
                _ if byte == quote => break,
                _ => bytes.push(byte),
            }
        }
        String::from_utf8(bytes)
            .map(Token::Str)
            .map_err(|_| self.error("a string is not UTF-8"))
    }

    /// Reads what follows a backslash in a string.
    fn escape(&mut self, out: &mut Vec<u8>) -> Result<(), LuaError> {
        let byte = self
            .byte()
            .ok_or_else(|| self.error("a string is not closed"))?;
        self.at += 1;
        let value = match byte {
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 11,
            b'\\' | b'"' | b'\'' => byte,
            b'\n' => {
                self.line += 1;
                byte
            }
            b'z' => {
                let rest = self.rest();
                self.advance(rest.len() - rest.trim_start().len());
                return Ok(());
            }
            b'x' => self.escape_code(16, 2, 2)?,
            b'0'..=b'9' => {
                self.at -= 1;
                self.escape_code(10, 1, 3)?
            }
            _ => return Err(self.error("an unknown escape in a string")),
        };
        out.push(value);
        Ok(())
    }

    /// The byte written with `min` to `max` digits of `radix`.
    fn escape_code(&mut self, radix: u32, min: usize, max: usize) -> Result<u8, LuaError> {
        let rest = self.rest().as_bytes();
        let length = rest
            .iter()
            .take(max)
            .take_while(|byte| char::from(**byte).is_digit(radix))
            .count();
        let code = (length >= min)
            .then(|| u8::from_str_radix(&self.rest()[..length], radix).ok())
            .flatten()
            .ok_or_else(|| self.error("a malformed escape in a string"))?;
        self.at += length;
        Ok(code)
    }
}

/// The operators of binary expressions with their left and right priorities.
const BINARY: &[(&str, BinaryOp, u8, u8)] = &[
    ("or", BinaryOp::Or, 1, 1),
    ("and", BinaryOp::And, 2, 2),
    ("<", BinaryOp::Less, 3, 3),
    (">", BinaryOp::Greater, 3, 3),
    ("<=", BinaryOp::LessEqual, 3, 3),
    (">=", BinaryOp::GreaterEqual, 3, 3),
    ("~=", BinaryOp::NotEqual, 3, 3),
    ("==", BinaryOp::Equal, 3, 3),
    ("..", BinaryOp::Concat, 5, 4),
    ("+", BinaryOp::Add, 6, 6),
    ("-", BinaryOp::Subtract, 6, 6),
    ("*", BinaryOp::Multiply, 7, 7),
    ("/", BinaryOp::Divide, 7, 7),
    ("%", BinaryOp::Modulo, 7, 7),
    ("^", BinaryOp::Power, 10, 9),
];

const UNARY_PRIORITY: u8 = 8;

/// The marks that end a block.
const BLOCK_END: &[&str] = &["end", "else", "elseif", "until"];

struct Parser {
    tokens: Vec<(Token, u32)>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at].0
    }

    fn lookahead(&self) -> &Token {
        self.tokens
            .get(self.at + 1)
            .map_or(&Token::End, |token| &token.0)
    }

    fn mark(&self) -> Option<&'static str> {
        match self.peek() {
            Token::Mark(mark) => Some(mark),
            _ => None,
        }
    }

    fn bump(&mut self) -> Token {
        let token = self.tokens[self.at].0.clone();
        self.at = (self.at + 1).min(self.tokens.len() - 1);
        token
    }

    fn accept(&mut self, mark: &str) -> bool {
        let found = self.mark() == Some(mark);
        if found {
            self.bump();
        }
        found
    }

    fn error(&self, message: impl Into<String>) -> LuaError {
        LuaError {
            line: self.tokens[self.at].1,
            message: message.into(),
        }
    }

    fn expect(&mut self, mark: &str) -> Result<(), LuaError> {
        if self.accept(mark) {
            Ok(())
        } else {
            Err(self.error(format!("expected {mark}, found {:?}", self.peek())))
        }
    }

    fn name(&mut self) -> Result<String, LuaError> {
        match self.bump() {
            Token::Name(name) => Ok(name),
            found => Err(self.error(format!("expected a name, found {found:?}"))),
        }
    }

    fn chunk(mut self) -> Result<Block, LuaError> {
        let block = self.block()?;
        match self.peek() {
            Token::End => Ok(block),
            found => Err(self.error(format!("unexpected {found:?}"))),
        }
    }

    fn at_block_end(&self) -> bool {
        *self.peek() == Token::End || self.mark().is_some_and(|mark| BLOCK_END.contains(&mark))
    }

    fn block(&mut self) -> Result<Block, LuaError> {
        let mut block = Vec::new();
        while !self.at_block_end() {
            if self.accept(";") {
                continue;
            }
            let statement = self.statement()?;
            let last = matches!(statement, Stat::Return(_));
            block.push(statement);
            if last {
                self.accept(";");
                break;
            }
        }
        Ok(block)
    }

    fn block_to_end(&mut self) -> Result<Block, LuaError> {
        let block = self.block()?;
        self.expect("end")?;
        Ok(block)
    }

    fn statement(&mut self) -> Result<Stat, LuaError> {
        let Some(mark) = self.mark() else {
            return self.expression_statement();
        };
        match mark {
            "if" => self.if_statement(),
            "while" => self.while_statement(),
            "for" => self.for_statement(),
            "repeat" => self.repeat_statement(),
            "function" => self.function_statement(),
            "local" => self.local_statement(),
            "return" => self.return_statement(),
            "do" => {
                self.bump();
                self.block_to_end().map(Stat::Do)
            }
            "break" => {
                self.bump();
                Ok(Stat::Break)
            }
            _ => self.expression_statement(),
        }
    }

    fn if_statement(&mut self) -> Result<Stat, LuaError> {
        let mut arms = Vec::new();
        loop {
            self.bump();
            let condition = self.expression()?;
            self.expect("then")?;
            arms.push((condition, self.block()?));
            if self.mark() != Some("elseif") {
                break;
            }
        }
        let otherwise = if self.accept("else") {
            Some(self.block()?)
        } else {
            None
        };
        self.expect("end")?;
        Ok(Stat::If(arms, otherwise))
    }

    fn while_statement(&mut self) -> Result<Stat, LuaError> {
        self.bump();
        let condition = self.expression()?;
        self.expect("do")?;
        Ok(Stat::While(condition, self.block_to_end()?))
    }

    fn repeat_statement(&mut self) -> Result<Stat, LuaError> {
        self.bump();
        let body = self.block()?;
        self.expect("until")?;
        Ok(Stat::Repeat(body, self.expression()?))
    }

    /// Reads a numeric or generic `for`; its loop variables are not kept.
    fn for_statement(&mut self) -> Result<Stat, LuaError> {
        self.bump();
        self.name()?;
        while self.accept(",") {
            self.name()?;
        }
        if !self.accept("=") {
            self.expect("in")?;
        }
        let head = self.expression_list()?;
        self.expect("do")?;
        Ok(Stat::For(head, self.block_to_end()?))
    }

    fn function_statement(&mut self) -> Result<Stat, LuaError> {
        self.bump();
        let mut name = Expr::Name(self.name()?);
        while self.mark() == Some(".") || self.mark() == Some(":") {
            self.bump();
            name = Expr::Field(Box::new(name), self.name()?);
        }
        Ok(Stat::Function(name, self.function_body()?))
    }

    fn local_statement(&mut self) -> Result<Stat, LuaError> {
        self.bump();
        if self.accept("function") {
            let name = Expr::Name(self.name()?);
            return Ok(Stat::Function(name, self.function_body()?));
        }
        let mut names = vec![self.name()?];
        while self.accept(",") {
            names.push(self.name()?);
        }
        let values = if self.accept("=") {
            self.expression_list()?
        } else {
            Vec::new()
        };
        Ok(Stat::Local(names, values))
    }

    fn return_statement(&mut self) -> Result<Stat, LuaError> {
        self.bump();
        if self.at_block_end() || self.mark() == Some(";") {
            return Ok(Stat::Return(Vec::new()));
        }
        self.expression_list().map(Stat::Return)
    }

    /// Reads a call, or an assignment to the expression that starts it.
    fn expression_statement(&mut self) -> Result<Stat, LuaError> {
        let first = self.suffixed()?;
        if self.mark() != Some("=") && self.mark() != Some(",") {
            return match first {
                Expr::Call(call) => Ok(Stat::Call(*call)),
                _ => Err(self.error("a statement is neither a call nor an assignment")),
            };
        }
        let mut targets = vec![first];
        while self.accept(",") {
            targets.push(self.suffixed()?);
        }
        self.expect("=")?;
        Ok(Stat::Assign(targets, self.expression_list()?))
    }

    fn expression_list(&mut self) -> Result<Vec<Expr>, LuaError> {
        let mut list = vec![self.expression()?];
        while self.accept(",") {
            list.push(self.expression()?);
        }
        Ok(list)
    }

    fn expression(&mut self) -> Result<Expr, LuaError> {
        self.binary(0)
    }

    /// Reads an expression of operators whose left priority is above `limit`.
    fn binary(&mut self, limit: u8) -> Result<Expr, LuaError> {
        let mut left = self.unary()?;
        while let Some(&(_, op, _, right)) = BINARY
            .iter()
            .find(|(mark, _, priority, _)| self.mark() == Some(*mark) && *priority > limit)
        {
            self.bump();
            left = Expr::Binary(op, Box::new(left), Box::new(self.binary(right)?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, LuaError> {
        let op = match self.mark() {
            Some("not") => UnaryOp::Not,
            Some("-") => UnaryOp::Negate,
            Some("#") => UnaryOp::Length,
            _ => return self.simple(),
        };
        self.bump();
        let operand = self.binary(UNARY_PRIORITY)?;
        Ok(match (op, operand) {
            (UnaryOp::Negate, Expr::Number(Number::Int(value))) => {
                Expr::Number(Number::Int(-value))
            }
            (UnaryOp::Negate, Expr::Number(Number::Float(value))) => {
                Expr::Number(Number::Float(-value))
            }
            (op, operand) => Expr::Unary(op, Box::new(operand)),
        })
    }

    fn simple(&mut self) -> Result<Expr, LuaError> {
        let literal = match self.peek() {
            Token::Number(number) => Expr::Number(*number),
            Token::Str(text) => Expr::Str(text.clone()),
            Token::Mark("nil") => Expr::Nil,
            Token::Mark("true") => Expr::Bool(true),
            Token::Mark("false") => Expr::Bool(false),
            Token::Mark("...") => Expr::Vararg,
            Token::Mark("{") => return self.table(),
            Token::Mark("function") => {
                self.bump();
                return self.function_body().map(Expr::Function);
            }
            _ => return self.suffixed(),
        };
        self.bump();
        Ok(literal)
    }

    /// Reads a name or a parenthesised expression and the fields and calls that follow it.
    fn suffixed(&mut self) -> Result<Expr, LuaError> {
        let mut expr = if self.accept("(") {
            let inner = self.expression()?;
            self.expect(")")?;
            inner
        } else {
            Expr::Name(self.name()?)
        };
        loop {
            match self.peek() {
                Token::Mark(".") => {
                    self.bump();
                    expr = Expr::Field(Box::new(expr), self.name()?);
                }
                Token::Mark("[") => {
                    self.bump();
                    let key = self.expression()?;
                    self.expect("]")?;
                    expr = Expr::Index(Box::new(expr), Box::new(key));
                }
                Token::Mark(":") => {
                    self.bump();
                    let callee = Expr::Field(Box::new(expr), self.name()?);
                    expr = self.call(callee)?;
                }
                Token::Mark("(" | "{") | Token::Str(_) => expr = self.call(expr)?,
                _ => return Ok(expr),
            }
        }
    }

    fn call(&mut self, callee: Expr) -> Result<Expr, LuaError> {
        let arguments = self.arguments()?;
        Ok(Expr::Call(Box::new(Call { callee, arguments })))
    }

    fn arguments(&mut self) -> Result<Vec<Expr>, LuaError> {
        match self.peek() {
            Token::Str(text) => {
                let argument = Expr::Str(text.clone());
                self.bump();
                Ok(vec![argument])
            }
            Token::Mark("{") => Ok(vec![self.table()?]),
            _ => {
                self.expect("(")?;
                if self.accept(")") {
                    return Ok(Vec::new());
                }
                let arguments = self.expression_list()?;
                self.expect(")")?;
                Ok(arguments)
            }
        }
    }

    /// Reads the parameters and body of a function; the parameters are not kept.
    fn function_body(&mut self) -> Result<Block, LuaError> {
        self.expect("(")?;
        while !self.accept(")") {
            if !self.accept("...") {
                self.name()?;
            }
            if self.mark() != Some(")") {
                self.expect(",")?;
            }
        }
        self.block_to_end()
    }

    fn table(&mut self) -> Result<Expr, LuaError> {
        self.expect("{")?;
        let mut items = Vec::new();
        while !self.accept("}") {
            items.push(self.table_item()?);
            if !self.accept(",") && !self.accept(";") {
                self.expect("}")?;
                break;
            }
        }
        Ok(Expr::Table(items))
    }

    fn table_item(&mut self) -> Result<TableItem, LuaError> {
        if self.accept("[") {
            let key = self.expression()?;
            self.expect("]")?;
            self.expect("=")?;
            return Ok(TableItem::Keyed(key, self.expression()?));
        }
        if matches!(self.peek(), Token::Name(_)) && *self.lookahead() == Token::Mark("=") {
            let name = self.name()?;
            self.bump();
            return Ok(TableItem::Named(name, self.expression()?));
        }
        self.expression().map(TableItem::Positional)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(value: i64) -> Expr {
        Expr::Number(Number::Int(value))
    }

    #[test]
    fn a_call_keeps_its_arguments() {
        let block =
            parse("change_npc_eye(1, EyePresets.normal, nil, -2, 0.5, \"a\", true)").unwrap();
        let [Stat::Call(call)] = block.as_slice() else {
            panic!("{block:?}");
        };
        assert_eq!(call.name(), Some("change_npc_eye"));
        assert_eq!(call.arguments[0], number(1));
        assert_eq!(
            call.arguments[1],
            Expr::Field(Box::new(Expr::Name("EyePresets".into())), "normal".into())
        );
        assert_eq!(call.arguments[2], Expr::Nil);
        assert_eq!(call.arguments[3], number(-2));
        assert_eq!(call.arguments[4], Expr::Number(Number::Float(0.5)));
        assert_eq!(call.arguments[5], Expr::Str("a".into()));
        assert_eq!(call.arguments[6], Expr::Bool(true));
    }

    #[test]
    fn comments_and_a_byte_order_mark_are_not_code() {
        let block =
            parse("\u{feff}-- wait(1)\n--[[ wait(2)\n]] --[==[ ]] wait(3) ]==]\nwait(4)").unwrap();
        let [Stat::Call(call)] = block.as_slice() else {
            panic!("{block:?}");
        };
        assert_eq!(call.arguments, [number(4)]);
    }

    #[test]
    fn operators_bind_by_priority() {
        let block = parse("x = a or b and c < d .. e + f * -g ^ h").unwrap();
        let [Stat::Assign(_, values)] = block.as_slice() else {
            panic!("{block:?}");
        };
        let Expr::Binary(BinaryOp::Or, _, right) = &values[0] else {
            panic!("{values:?}");
        };
        let Expr::Binary(BinaryOp::And, _, right) = right.as_ref() else {
            panic!("{right:?}");
        };
        let Expr::Binary(BinaryOp::Less, _, right) = right.as_ref() else {
            panic!("{right:?}");
        };
        let Expr::Binary(BinaryOp::Concat, _, right) = right.as_ref() else {
            panic!("{right:?}");
        };
        let Expr::Binary(BinaryOp::Add, _, right) = right.as_ref() else {
            panic!("{right:?}");
        };
        let Expr::Binary(BinaryOp::Multiply, _, right) = right.as_ref() else {
            panic!("{right:?}");
        };
        let Expr::Unary(UnaryOp::Negate, power) = right.as_ref() else {
            panic!("{right:?}");
        };
        assert!(matches!(
            power.as_ref(),
            Expr::Binary(BinaryOp::Power, _, _)
        ));
    }

    #[test]
    fn strings_decode_their_escapes() {
        let block = parse(
            r#"x = "a\n\65\x42\"" y = [[
long]] z = 'c'"#,
        )
        .unwrap();
        let values: Vec<_> = block
            .iter()
            .map(|statement| match statement {
                Stat::Assign(_, values) => values[0].clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(values[0], Expr::Str("a\nAB\"".into()));
        assert_eq!(values[1], Expr::Str("long".into()));
        assert_eq!(values[2], Expr::Str("c".into()));
    }

    #[test]
    fn a_table_holds_its_three_kinds_of_item() {
        let block = parse("T = { a = 1, [2] = 3, 4; }").unwrap();
        let [Stat::Assign(_, values)] = block.as_slice() else {
            panic!("{block:?}");
        };
        assert_eq!(
            values[0],
            Expr::Table(vec![
                TableItem::Named("a".into(), number(1)),
                TableItem::Keyed(number(2), number(3)),
                TableItem::Positional(number(4)),
            ])
        );
    }

    #[test]
    fn a_method_call_and_calls_without_parentheses_are_calls() {
        let block = parse("a:b(1)\nf \"x\"\ng { 1 }").unwrap();
        let names: Vec<Option<&str>> = block
            .iter()
            .map(|statement| match statement {
                Stat::Call(call) => call.name(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(names, [None, Some("f"), Some("g")]);
    }

    #[test]
    fn what_is_not_lua_is_an_error_with_its_line() {
        for source in [
            "a = 1\nb +",
            "if x then",
            "a.b",
            "x = \"open",
            "goto done",
            "--[[ open",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
        assert_eq!(
            parse("a = 1\nb +")
                .unwrap_err()
                .to_string()
                .split(':')
                .next(),
            Some("line 2")
        );
    }
}
