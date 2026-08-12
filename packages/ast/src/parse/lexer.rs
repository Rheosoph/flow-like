//! FlowScript lexer: a context-free tokenizer.
//!
//! The lexer is intentionally "dumb" — it knows nothing about grammar context. All
//! disambiguation (e.g. object literal vs. raw-JSON default, `Field` vs. `Member` access)
//! is resolved by the parser. Tokens carry 1-based `line`/`col` for diagnostics.

use crate::parse::error::ParseError;

/// A lexical token kind.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Identifier or keyword (`const`, `for`, `onStart`, `aiGenerativeInvoke`, …).
    Ident(String),
    /// Double-quoted string literal (already unescaped).
    Str(String),
    /// Integer literal.
    Int(i64),
    /// Positive integer outside the signed literal range. FlowScript values remain `i64`, but
    /// metadata such as a cache TTL uses the full `u64` range.
    UInt(u64),
    /// Floating-point literal.
    Float(f64),
    /// `@` — decorator marker.
    At,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Dot,
    Assign,
    /// `?` — ternary then-marker.
    Question,
    /// `!` — logical-not prefix (negated `if`).
    Bang,
    /// A binary operator token (`==`, `!=`, `>`, `<=`, `+`, `*`, `&&`, …).
    Op(String),
    /// A `//`-style line comment (text after `//`, trimmed of one leading space).
    /// Anchor comments (`//@n:` etc.) are preserved verbatim including the leading `@`.
    Comment(String),
    /// End of input.
    Eof,
}

/// A token with source position.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
    /// Byte offset of the token start in the source string.
    pub byte: usize,
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    byte: usize,
}

/// Multi-character operators, longest first so the scanner is greedy.
const OPERATORS: &[&str] = &[
    "===", "!==", "==", "!=", ">=", "<=", "&&", "||", "**", ">", "<", "|", "+", "-", "*", "/", "%",
    "^",
];

/// Tokenize a FlowScript source string.
pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let mut lexer = Lexer {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
        byte: 0,
    };
    lexer.run()
}

impl Lexer {
    fn run(&mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_inline_ws();
            let Some(&c) = self.chars.get(self.pos) else {
                tokens.push(self.make(Tok::Eof));
                return Ok(tokens);
            };
            let token = match c {
                '\n' => {
                    self.advance();
                    continue;
                }
                '/' if self.peek(1) == Some('/') => self.line_comment(),
                '"' => self.string()?,
                '@' => self.single(Tok::At),
                '(' => self.single(Tok::LParen),
                ')' => self.single(Tok::RParen),
                '{' => self.single(Tok::LBrace),
                '}' => self.single(Tok::RBrace),
                '[' => self.single(Tok::LBracket),
                ']' => self.single(Tok::RBracket),
                ',' => self.single(Tok::Comma),
                ';' => self.single(Tok::Semi),
                ':' => self.single(Tok::Colon),
                '?' => self.single(Tok::Question),
                '.' if !self.peek_is_digit(1) => self.single(Tok::Dot),
                c if c.is_ascii_digit()
                    || (c == '-'
                        && self.peek_is_digit(1)
                        && signed_number_can_start_after(tokens.last())) =>
                {
                    self.number()?
                }
                c if is_ident_start(c) => self.ident(),
                _ => {
                    if let Some(op) = self.match_operator() {
                        op
                    } else {
                        return Err(self.err(format!("unexpected character `{c}`")));
                    }
                }
            };
            tokens.push(token);
        }
    }

    fn make(&self, tok: Tok) -> Token {
        Token {
            tok,
            line: self.line,
            col: self.col,
            byte: self.byte,
        }
    }

    fn single(&mut self, tok: Tok) -> Token {
        let token = self.make(tok);
        self.advance();
        token
    }

    fn advance(&mut self) {
        if let Some(&c) = self.chars.get(self.pos) {
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            self.byte += c.len_utf8();
            self.pos += 1;
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn peek_is_digit(&self, offset: usize) -> bool {
        self.peek(offset).is_some_and(|c| c.is_ascii_digit())
    }

    /// Skip spaces and tabs but not newlines (newlines are insignificant here yet still
    /// advance line tracking via `advance`).
    fn skip_inline_ws(&mut self) {
        while let Some(&c) = self.chars.get(self.pos) {
            if c == ' ' || c == '\t' || c == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn line_comment(&mut self) -> Token {
        let token_line = self.line;
        let token_col = self.col;
        let token_byte = self.byte;
        // consume `//`
        self.advance();
        self.advance();
        let mut text = String::new();
        let mut split_at_anchor = false;
        while let Some(&c) = self.chars.get(self.pos) {
            if c == '\n' {
                break;
            }
            // The renderer can put a trailing anchor on the same line as a label comment
            // (`{ // exec_out   //@n:id`). Stop before an embedded anchor (`//@n:` / `//@v:` /
            // `//@l:`) so it lexes as its own comment token — otherwise the anchor is swallowed
            // by the label and the node counts as deleted on reconcile. Non-anchor `//@x`
            // sequences (e.g. `//@todo`) stay part of the label text.
            if c == '/'
                && !text.is_empty()
                && self.chars.get(self.pos + 1) == Some(&'/')
                && self.chars.get(self.pos + 2) == Some(&'@')
                && matches!(self.chars.get(self.pos + 3).copied(), Some('n' | 'v' | 'l'))
                && self.chars.get(self.pos + 4) == Some(&':')
            {
                split_at_anchor = true;
                break;
            }
            text.push(c);
            self.advance();
        }
        // Drop a single leading space (renderer writes `// text` and `   //@n:id`).
        let trimmed = text.strip_prefix(' ').unwrap_or(&text);
        let trimmed = if split_at_anchor {
            trimmed.trim_end().to_string()
        } else {
            trimmed.to_string()
        };
        Token {
            tok: Tok::Comment(trimmed),
            line: token_line,
            col: token_col,
            byte: token_byte,
        }
    }

    fn string(&mut self) -> Result<Token, ParseError> {
        let token_line = self.line;
        let token_col = self.col;
        let token_byte = self.byte;
        self.advance(); // opening quote
        let mut value = String::new();
        loop {
            let Some(&c) = self.chars.get(self.pos) else {
                return Err(ParseError::new(
                    "unterminated string literal",
                    token_line,
                    token_col,
                ));
            };
            match c {
                '"' => {
                    self.advance();
                    break;
                }
                '\\' => {
                    self.advance();
                    let Some(&esc) = self.chars.get(self.pos) else {
                        return Err(ParseError::new(
                            "unterminated escape in string",
                            self.line,
                            self.col,
                        ));
                    };
                    // JSON's escape set plus `'`. `b`/`f` are required because a `Literal::Json`
                    // span is re-emitted verbatim after serde_json validates it — and the lexer
                    // runs over the whole file first, so without them a JSON default that would
                    // round-trip byte-exactly fails to lex. `'` and `/` denote characters that
                    // need no escape, so they normalize away, as `\uXXXX` already does.
                    // Unknown escapes stay a HARD ERROR: passing them through would silently turn
                    // a regex `"\d+"` into `"d+"`, which applies cleanly and fails at run time.
                    match esc {
                        '"' => value.push('"'),
                        '\'' => value.push('\''),
                        '\\' => value.push('\\'),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        'b' => value.push('\u{8}'),
                        'f' => value.push('\u{c}'),
                        '/' => value.push('/'),
                        'u' => {
                            let code = self.unicode_escape()?;
                            value.push(code);
                            continue;
                        }
                        other => return Err(self.err(format!("invalid escape `\\{other}`"))),
                    }
                    self.advance();
                }
                _ => {
                    value.push(c);
                    self.advance();
                }
            }
        }
        Ok(Token {
            tok: Tok::Str(value),
            line: token_line,
            col: token_col,
            byte: token_byte,
        })
    }

    /// Parse a `\uXXXX` escape (the backslash and `u` already consumed at `pos`).
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        // pos currently at `u`.
        self.advance();
        let mut hex = String::new();
        for _ in 0..4 {
            let Some(&c) = self.chars.get(self.pos) else {
                return Err(self.err("truncated unicode escape"));
            };
            hex.push(c);
            self.advance();
        }
        let code = u32::from_str_radix(&hex, 16)
            .map_err(|_| self.err(format!("invalid unicode escape `\\u{hex}`")))?;
        char::from_u32(code).ok_or_else(|| self.err(format!("invalid unicode scalar `\\u{hex}`")))
    }

    fn number(&mut self) -> Result<Token, ParseError> {
        let token_line = self.line;
        let token_col = self.col;
        let token_byte = self.byte;
        let start = self.pos;
        if self.chars.get(self.pos) == Some(&'-') {
            self.advance();
        }
        let mut is_float = false;
        while let Some(&c) = self.chars.get(self.pos) {
            if c.is_ascii_digit() {
                self.advance();
            } else if c == '.' && self.peek_is_digit(1) {
                is_float = true;
                self.advance();
            } else if (c == 'e' || c == 'E')
                && (self.peek_is_digit(1)
                    || matches!(self.peek(1), Some('+' | '-')) && self.peek_is_digit(2))
            {
                is_float = true;
                self.advance();
                if matches!(self.chars.get(self.pos), Some('+' | '-')) {
                    self.advance();
                }
            } else {
                break;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let tok = if is_float {
            Tok::Float(
                text.parse()
                    .map_err(|_| self.err(format!("invalid float `{text}`")))?,
            )
        } else {
            match text.parse::<i64>() {
                Ok(value) => Tok::Int(value),
                Err(_) if !text.starts_with('-') => Tok::UInt(
                    text.parse()
                        .map_err(|_| self.err(format!("invalid integer `{text}`")))?,
                ),
                Err(_) => return Err(self.err(format!("invalid integer `{text}`"))),
            }
        };
        Ok(Token {
            tok,
            line: token_line,
            col: token_col,
            byte: token_byte,
        })
    }

    fn ident(&mut self) -> Token {
        let token_line = self.line;
        let token_col = self.col;
        let token_byte = self.byte;
        let start = self.pos;
        while let Some(&c) = self.chars.get(self.pos) {
            if is_ident_continue(c) {
                self.advance();
            } else {
                break;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        Token {
            tok: Tok::Ident(text),
            line: token_line,
            col: token_col,
            byte: token_byte,
        }
    }

    fn match_operator(&mut self) -> Option<Token> {
        // `!` alone is the negation prefix; `!=`/`!==` are operators (handled below).
        for op in OPERATORS {
            if self.starts_with(op) {
                let token = self.make(Tok::Op((*op).to_string()));
                for _ in 0..op.chars().count() {
                    self.advance();
                }
                return Some(token);
            }
        }
        if self.chars.get(self.pos) == Some(&'!') {
            return Some(self.single(Tok::Bang));
        }
        if self.chars.get(self.pos) == Some(&'=') {
            return Some(self.single(Tok::Assign));
        }
        None
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.chars.get(self.pos + i) == Some(&c))
    }

    fn err(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(message, self.line, self.col)
    }
}

/// A leading `-` belongs to a numeric literal only where a new expression can start. After an
/// expression-ending token it is subtraction (`10-3`, `value - 1`). Keeping the sign on the
/// numeric token preserves the full `i64` literal range, including `i64::MIN`.
fn signed_number_can_start_after(previous: Option<&Token>) -> bool {
    match previous.map(|token| &token.tok) {
        None => true,
        Some(
            Tok::Str(_)
            | Tok::Int(_)
            | Tok::UInt(_)
            | Tok::Float(_)
            | Tok::RParen
            | Tok::RBracket
            | Tok::RBrace,
        ) => false,
        Some(Tok::Ident(name)) => name == "return",
        Some(_) => true,
    }
}

// Unicode-aware: `to_camel_case` keeps any alphanumeric char, so rendered identifiers
// (from user-named boards/variables/events) can carry non-ASCII letters.
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}
