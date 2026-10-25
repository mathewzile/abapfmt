//! Lossless ABAP tokenizer, ported from abap-cleaner's `Tokenizer` (+ the preliminary typing in the
//! `Token` constructor).
//!
//! Tokens are spans into the source; nothing is rewritten. The whitespace before a token is
//! summarised as `lines_before` (line feeds) and `spaces_before` (spaces/tabs/NBSP after the last
//! line feed, each counting as one column, as in abap-cleaner). Trailing whitespace on a line is
//! dropped from that summary but stays reachable through the spans.
//!
//! Words are typed [`TokenKind::Word`] here; deciding KEYWORD vs IDENTIFIER is the job of the
//! grammar-based refiner. Only ABAP is handled; DDL/DCL/non-ABAP sections arrive with their rules.
#![forbid(unsafe_code)]

/// Preliminary token type (abap-cleaner's `TokenType`, with KEYWORD/IDENTIFIER not yet decided).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    Pragma,
    Literal,
    /// `=`, `?=`, `+=`, ... `=` may later turn out to be a comparison operator.
    AssignmentOp,
    ComparisonOp,
    /// A keyword or an identifier; decided by the refiner.
    Word,
    Period,
    Comma,
    Colon,
    /// `)`, `#(`, `+` and anything without a letter or underscore.
    OtherOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub lines_before: u32,
    pub spaces_before: u32,
    /// 1-based line of the *end* of the token (abap-cleaner's `sourceLineNum`).
    pub line: u32,
    pub kind: TokenKind,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start as usize..self.end as usize]
    }
}

#[derive(Debug)]
pub struct LexError {
    pub offset: usize,
    pub message: &'static str,
}

const NBSP: char = '\u{a0}';

fn is_space(c: char) -> bool {
    c == ' ' || c == '\t' || c == NBSP
}

fn is_line_sep(c: char) -> bool {
    c == '\r' || c == '\n'
}

/// Chars that end an ABAP word (`(` is included in the word).
fn is_abap_token_end(c: char) -> bool {
    is_space(c) || is_line_sep(c) || matches!(c, ',' | '.' | '\'' | '`' | '"' | ':' | '(' | ')')
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line: u32,
}

impl<'a> Lexer<'a> {
    fn char_at(&self, pos: usize) -> Option<char> {
        self.src.get(pos..).and_then(|s| s.chars().next())
    }

    fn next(&mut self) -> Result<Option<Token>, LexError> {
        let src = self.src;
        if self.pos >= src.len() {
            return Ok(None);
        }

        // whitespace
        let mut line_feeds = 0;
        let mut spaces;
        loop {
            while let Some(c) = self.char_at(self.pos).filter(|c| is_line_sep(*c)) {
                if c == '\n' {
                    line_feeds += 1;
                    self.line += 1;
                }
                self.pos += 1;
            }
            spaces = 0;
            while let Some(c) = self.char_at(self.pos).filter(|c| is_space(*c)) {
                spaces += 1;
                self.pos += c.len_utf8();
            }
            // spaces before a line break are trailing whitespace: ignore them
            if !self.char_at(self.pos).is_some_and(is_line_sep) {
                break;
            }
        }

        // a final line feed alone does not make a token
        if self.pos == src.len() && line_feeds == 1 && spaces == 0 {
            return Ok(None);
        }

        let cur = self.char_at(self.pos).unwrap_or(' ');
        let at_line_start = self.pos == 0 || (line_feeds > 0 && spaces == 0);

        let end = if self.pos == src.len() {
            self.pos
        } else if (cur == '*' && at_line_start) || cur == '"' {
            self.read_until_line_end()
        } else if cur == '\'' {
            self.read_literal('\'', true)
        } else if cur == '`' {
            self.read_literal('`', false)
        } else if cur == '|' || cur == '}' {
            self.read_string_template()
        } else if matches!(cur, '(' | '.' | ',' | ':') {
            self.pos + 1
        } else {
            self.read_word()?
        };

        let start = self.pos;
        self.pos = end;
        self.line += src[start..end].matches('\n').count() as u32;
        let text = &src[start..end];
        let kind = if at_line_start && text.starts_with('*') {
            TokenKind::Comment
        } else {
            infer_kind(text)
        };
        Ok(Some(Token {
            start: start as u32,
            end: end as u32,
            lines_before: line_feeds,
            spaces_before: spaces,
            line: self.line,
            kind,
        }))
    }

    fn read_until_line_end(&self) -> usize {
        self.src[self.pos..]
            .find(['\r', '\n'])
            .map_or(self.src.len(), |i| self.pos + i)
    }

    /// `'text'` and `` `text` ``, with doubled delimiters as escapes. A text field literal may be
    /// followed by a text symbol id, as in `'text'(001)`.
    fn read_literal(&self, delim: char, include_text_symbol_id: bool) -> usize {
        let src = self.src;
        let mut search = self.pos + 1;
        let literal_end = loop {
            match src[search..].find(delim) {
                None => break src.len(),
                Some(i) => {
                    let p = search + i;
                    if p + 1 == src.len() || src.as_bytes()[p + 1] != delim as u8 {
                        break p + 1;
                    }
                    search = p + 2;
                }
            }
        };
        if include_text_symbol_id
            && src.as_bytes().get(literal_end) == Some(&b'(')
            && let Some(close) = src[literal_end..].find(')')
        {
            return literal_end + close + 1;
        }
        literal_end
    }

    /// `|...|` up to the next `|` or `{` (and `}...|` / `}...{` continuing a template).
    fn read_string_template(&self) -> usize {
        const ESCAPES: [&str; 7] = ["\\|", "\\{", "\\}", "\\\\", "\\r", "\\n", "\\t"];
        let src = self.src;
        let mut i = self.pos + 1;
        while i < src.len() {
            if let Some(e) = ESCAPES.iter().find(|e| src[i..].starts_with(**e)) {
                i += e.len();
                continue;
            }
            let b = src.as_bytes()[i];
            if b == b'{' || b == b'|' {
                return i + 1;
            }
            i += 1;
        }
        src.len()
    }

    fn read_word(&self) -> Result<usize, LexError> {
        let src = self.src;
        let from = self.pos + self.char_at(self.pos).map_or(1, char::len_utf8);
        let Some(i) = src[from..].find(is_abap_token_end) else {
            return Ok(src.len());
        };
        let end = from + i;
        let c = src.as_bytes()[end];
        if c == b'(' && src.as_bytes().get(end + 1).is_some_and(u8::is_ascii_digit) {
            // "identifier(##)" as in "DATA lv_chars(20) TYPE c."
            return match src[end..].find(')') {
                Some(close) => Ok(end + close + 1),
                None => Err(LexError {
                    offset: end,
                    message: "closing parenthesis not found",
                }),
            };
        }
        Ok(if c == b'(' { end + 1 } else { end })
    }
}

fn infer_kind(text: &str) -> TokenKind {
    let first = text.chars().next();
    if text.starts_with('"') {
        TokenKind::Comment
    } else if text.starts_with("##") {
        TokenKind::Pragma
    } else if matches!(first, Some('\'' | '`' | '|' | '}'))
        || is_integer(text)
        || is_position_and_length(text)
    {
        TokenKind::Literal
    } else if is_assignment_op(text) {
        TokenKind::AssignmentOp
    } else if is_comparison_op(text) {
        TokenKind::ComparisonOp
    } else if text == ":" {
        TokenKind::Colon
    } else if text == "," {
        TokenKind::Comma
    } else if text == "." {
        TokenKind::Period
    } else if text.chars().any(|c| c.is_alphabetic() || c == '_') {
        TokenKind::Word
    } else {
        TokenKind::OtherOp
    }
}

/// `ABAP.isNumeric(text, false, false)`: optional sign, then digits only.
fn is_integer(text: &str) -> bool {
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `1(75)` as in `SELECTION-SCREEN PUSHBUTTON 1(75) ...`
fn is_position_and_length(text: &str) -> bool {
    let Some(inner) = text.strip_suffix(')') else {
        return false;
    };
    let Some((pos, len)) = inner.split_once('(') else {
        return false;
    };
    text.len() >= 4
        && !pos.is_empty()
        && pos.bytes().all(|b| b.is_ascii_digit())
        && len.bytes().all(|b| b.is_ascii_digit())
}

fn is_assignment_op(text: &str) -> bool {
    matches!(text, "=" | "?=" | "+=" | "-=" | "*=" | "/=" | "&&=")
}

pub fn is_comparison_op(text: &str) -> bool {
    let up = text.to_ascii_uppercase();
    matches!(
        up.as_str(),
        "<" | "<="
            | ">="
            | ">"
            | "<>"
            | "=<"
            | "=>"
            | "><"
            | "LT"
            | "LE"
            | "EQ"
            | "GE"
            | "GT"
            | "NE"
            | "IN"
            | "BETWEEN"
            | "CO"
            | "CN"
            | "CA"
            | "NA"
            | "CS"
            | "NS"
            | "CP"
            | "NP"
            | "BYTE-CO"
            | "BYTE-CN"
            | "BYTE-CA"
            | "BYTE-NA"
            | "BYTE-CS"
            | "BYTE-NS"
    )
}

/// Tokenize ABAP source. Never rewrites anything; token spans index into `src`.
pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer {
        src,
        pos: 0,
        line: 1,
    };
    let mut tokens = Vec::new();
    while let Some(t) = lexer.next()? {
        tokens.push(t);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(src: &str) -> Vec<(&str, TokenKind)> {
        lex(src)
            .unwrap()
            .iter()
            .map(|t| (t.text(src), t.kind))
            .collect()
    }

    #[test]
    fn simple_statement() {
        assert_eq!(
            kinds("DATA lv_chars(20) TYPE c."),
            [
                ("DATA", Word),
                ("lv_chars(20)", Word),
                ("TYPE", Word),
                ("c", Word),
                (".", Period)
            ]
        );
    }

    #[test]
    fn call_paren_stays_on_the_word() {
        assert_eq!(
            kinds("lo->m( 1 ) = x."),
            [
                ("lo->m(", Word),
                ("1", Literal),
                (")", OtherOp),
                ("=", AssignmentOp),
                ("x", Word),
                (".", Period)
            ]
        );
    }

    #[test]
    fn literals_and_templates() {
        assert_eq!(
            kinds("a = 'it''s'(001) && `x``y` && |a { b } \\| c|."),
            [
                ("a", Word),
                ("=", AssignmentOp),
                ("'it''s'(001)", Literal),
                ("&&", OtherOp),
                ("`x``y`", Literal),
                ("&&", OtherOp),
                ("|a {", Literal),
                ("b", Word),
                ("} \\| c|", Literal),
                (".", Period),
            ]
        );
    }

    #[test]
    fn comments_and_pragmas() {
        let src = "* line\nx = 1. \" tail\ny ##NEEDED.";
        assert_eq!(
            kinds(src),
            [
                ("* line", Comment),
                ("x", Word),
                ("=", AssignmentOp),
                ("1", Literal),
                (".", Period),
                ("\" tail", Comment),
                ("y", Word),
                ("##NEEDED", Pragma),
                (".", Period),
            ]
        );
    }

    #[test]
    fn star_is_a_comment_only_at_line_start() {
        assert_eq!(kinds("a = b * c.")[3], ("*", OtherOp));
        assert_eq!(kinds("  * x")[0].1, OtherOp);
    }

    #[test]
    fn whitespace_summary_is_lossless_about_layout() {
        let src = "IF a.\n\n    b = 1.   \n\tc.\r\nENDIF.\n";
        let toks = lex(src).unwrap();
        let by_text = |s: &str| toks.iter().find(|t| t.text(src) == s).unwrap();
        assert_eq!(
            (by_text("b").lines_before, by_text("b").spaces_before),
            (2, 4)
        );
        // a tab counts as one column
        assert_eq!(
            (by_text("c").lines_before, by_text("c").spaces_before),
            (1, 1)
        );
        assert_eq!(
            (
                by_text("ENDIF").lines_before,
                by_text("ENDIF").spaces_before
            ),
            (1, 0)
        );
        // the final line feed makes no token
        assert_eq!(toks.last().unwrap().text(src), ".");
        assert_eq!(toks.iter().filter(|t| t.text(src) == "ENDIF").count(), 1);
    }

    #[test]
    fn chain_and_operators() {
        assert_eq!(
            kinds("DATA: a TYPE i, b TYPE string."),
            [
                ("DATA", Word),
                (":", Colon),
                ("a", Word),
                ("TYPE", Word),
                ("i", Word),
                (",", Comma),
                ("b", Word),
                ("TYPE", Word),
                ("string", Word),
                (".", Period),
            ]
        );
        assert_eq!(kinds("IF a EQ b.")[2].1, ComparisonOp);
    }

    #[test]
    fn spans_cover_the_source_without_rewriting() {
        let src = "METHOD m.\n\tx = 'a\tb'.\u{a0}\u{a0}y = |t|.\nENDMETHOD.";
        let toks = lex(src).unwrap();
        let mut last = 0;
        for t in &toks {
            assert!(t.start as usize >= last);
            assert!(
                src[last..t.start as usize]
                    .chars()
                    .all(|c| c.is_whitespace())
            );
            last = t.end as usize;
        }
        assert_eq!(
            toks.iter()
                .find(|t| t.text(src).starts_with("'a"))
                .unwrap()
                .text(src),
            "'a\tb'"
        );
    }

    #[test]
    fn unclosed_length_spec_is_an_error() {
        assert!(lex("DATA x(20 TYPE c.").is_err());
    }
}
