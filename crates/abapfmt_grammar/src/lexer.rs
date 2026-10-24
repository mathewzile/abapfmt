//! Port of abaplint's lexer (`1_lexer/lexer.ts`, MIT), producing the whitespace-flavoured token
//! kinds that the generated grammar matches with `tok()`. This is *not* the formatter's lexer; it
//! only exists to feed the grammar, and its tokens are spans into the same source.
#![allow(clippy::byte_char_slices, clippy::if_same_then_else)]
use crate::Tk;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexClass {
    Comment,
    Pragma,
    /// `.` or `,`
    Punctuation,
    Normal,
}

#[derive(Clone, Copy, Debug)]
pub struct GToken {
    pub start: u32,
    pub end: u32,
    pub kind: Tk,
    pub class: LexClass,
}

impl GToken {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start as usize..self.end as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Ping,
    Str,
    Template,
    Comment,
    Pragma,
}

const EOF: i32 = -1;
const NL: i32 = b'\n' as i32;

fn c(ch: u8) -> i32 {
    ch as i32
}

fn is_split(ch: i32) -> bool {
    [
        b' ', b':', b'.', b',', b'-', b'+', b'(', b')', b'[', b']', b'\\', b'\t', b'\n',
    ]
    .iter()
    .any(|x| c(*x) == ch)
}

fn is_buf(ch: i32) -> bool {
    [b'.', b',', b':', b'(', b')', b'[', b']', b'+', b'@']
        .iter()
        .any(|x| c(*x) == ch)
}

fn after_literal(ch: i32) -> bool {
    ch == EOF
        || [
            b' ', b'\t', b'\n', b'.', b',', b':', b'"', b'(', b')', b'[', b']',
        ]
        .iter()
        .any(|x| c(*x) == ch)
}

struct Lex<'a> {
    raw: &'a [u8],
    src: &'a str,
    /// stream offset, starts at -1 like abaplint's `LexerStream`
    offset: isize,
    buf_start: usize,
    buf_end: usize,
    buf_empty: bool,
    mode: Mode,
    out: Vec<GToken>,
}

impl Lex<'_> {
    fn at(&self, o: isize) -> i32 {
        if o < 0 || o as usize >= self.raw.len() {
            EOF
        } else {
            self.raw[o as usize] as i32
        }
    }
    fn current(&self) -> i32 {
        if self.offset < 0 {
            NL
        } else {
            self.at(self.offset)
        }
    }
    fn ahead(&self) -> i32 {
        self.at(self.offset + 1)
    }
    fn aahead(&self) -> i32 {
        self.at(self.offset + 2)
    }
    fn prev(&self) -> i32 {
        self.at(self.offset - 1)
    }
    fn prev_prev(&self) -> i32 {
        self.at(self.offset - 2)
    }

    fn buf_add(&mut self) {
        if self.buf_empty {
            self.buf_start = self.offset.max(0) as usize;
            self.buf_empty = false;
        }
        self.buf_end = (self.offset + 1) as usize;
    }
    fn buf_len(&self) -> usize {
        self.buf_end - self.buf_start
    }
    fn buf_clear(&mut self) {
        self.buf_start = 0;
        self.buf_end = 0;
        self.buf_empty = true;
    }
    fn count_is_even(&self, ch: u8) -> bool {
        let end = self.buf_end.min(self.raw.len());
        self.raw[self.buf_start.min(end)..end]
            .iter()
            .filter(|b| **b == ch)
            .count()
            % 2
            == 0
    }

    fn add(&mut self) {
        let end = self.buf_end.min(self.raw.len());
        let start = self.buf_start.min(end);
        let slice = &self.src[start..end];
        let trimmed_start = start + (slice.len() - slice.trim_start().len());
        let trimmed_end = trimmed_start + slice.trim().len();
        let s = &self.src[trimmed_start..trimmed_end];
        if !s.is_empty() {
            let white_before = {
                let before = self.offset - s.len() as isize;
                before >= 0
                    && matches!(self.at(before), x if x == c(b' ') || x == NL || x == c(b'\t') || x == c(b':'))
            };
            let next = self.ahead();
            let white_after = next == c(b' ')
                || next == NL
                || next == c(b'\t')
                || next == c(b':')
                || next == c(b',')
                || next == c(b'.')
                || next == EOF
                || next == c(b'"');
            let (kind, class) = self.classify(s, white_before, white_after);
            self.out.push(GToken {
                start: trimmed_start as u32,
                end: trimmed_end as u32,
                kind,
                class,
            });
        }
        self.buf_clear();
    }

    fn classify(&self, s: &str, wb: bool, wa: bool) -> (Tk, LexClass) {
        let flavored = |base: &str| {
            let name = format!(
                "{}{base}{}",
                if wb { "W" } else { "" },
                if wa { "W" } else { "" }
            );
            Tk::from_name(&name).unwrap_or(Tk::Other)
        };
        let plain = |name: &str| Tk::from_name(name).unwrap_or(Tk::Other);
        match self.mode {
            Mode::Comment => return (Tk::Other, LexClass::Comment),
            Mode::Ping | Mode::Str => return (Tk::Other, LexClass::Normal),
            Mode::Template => {
                let (f, l) = (s.as_bytes()[0], s.as_bytes()[s.len() - 1]);
                let k = if f == b'|' && l == b'|' {
                    plain("StringTemplate")
                } else if f == b'|' && l == b'{' && wa {
                    plain("StringTemplateBegin")
                } else if f == b'}' && l == b'|' && wb {
                    plain("StringTemplateEnd")
                } else if f == b'}' && l == b'{' && wa && wb {
                    plain("StringTemplateMiddle")
                } else {
                    Tk::Other
                };
                return (k, LexClass::Normal);
            }
            _ => {}
        }
        if s.len() > 2 && s.starts_with("##") {
            return (Tk::Other, LexClass::Pragma);
        }
        match s {
            "." | "," => return (Tk::Other, LexClass::Punctuation),
            "[" => return (flavored("BracketLeft"), LexClass::Normal),
            "(" => return (flavored("ParenLeft"), LexClass::Normal),
            "]" => return (flavored("BracketRight"), LexClass::Normal),
            ")" => return (flavored("ParenRight"), LexClass::Normal),
            "-" => return (flavored("Dash"), LexClass::Normal),
            "+" => return (flavored("Plus"), LexClass::Normal),
            "@" => return (flavored("At"), LexClass::Normal),
            "->" => return (flavored("InstanceArrow"), LexClass::Normal),
            "=>" => return (flavored("StaticArrow"), LexClass::Normal),
            _ => {}
        }
        if self.mode == Mode::Normal && s.starts_with('\\') {
            let adj = isize::from(self.ahead() == EOF);
            let prev_off = self.offset - s.len() as isize - adj;
            let pc = self.at(prev_off);
            let white_before_backslash =
                pc == c(b' ') || pc == NL || pc == c(b'\t') || pc == c(b':');
            if !white_before_backslash {
                return (plain("AssociationName"), LexClass::Normal);
            }
        }
        (Tk::Other, LexClass::Normal)
    }

    fn run(&mut self) {
        loop {
            let current = self.current();
            self.buf_add();
            let ahead = self.ahead();
            let aahead = self.aahead();
            match self.mode {
                Mode::Normal => {
                    if is_split(ahead) {
                        self.add();
                    } else if ahead == c(b'\'') {
                        self.add();
                        self.mode = Mode::Str;
                    } else if ahead == c(b'|') || ahead == c(b'}') {
                        self.add();
                        self.mode = Mode::Template;
                    } else if ahead == c(b'`') {
                        self.add();
                        self.mode = Mode::Ping;
                    } else if ahead == c(b'#') && aahead == c(b'#') {
                        self.add();
                        self.mode = Mode::Pragma;
                    } else if ahead == c(b'"') || (ahead == c(b'*') && current == NL) {
                        self.add();
                        self.mode = Mode::Comment;
                    } else if ahead == c(b'@') && self.buf_is_blank() {
                        self.add();
                    } else if (ahead == c(b'-') || ahead == c(b'=')) && aahead == c(b'>') {
                        self.add();
                    } else if current == c(b'>')
                        && ahead != c(b' ')
                        && (self.prev() == c(b'-') || self.prev() == c(b'='))
                    {
                        self.add();
                    } else if self.buf_len() == 1
                        && (is_buf(current) || (current == c(b'-') && ahead != c(b'>')))
                    {
                        self.add();
                    }
                }
                Mode::Pragma
                    if [b',', b':', b'.', b' ', b'\n']
                        .iter()
                        .any(|x| c(*x) == ahead) =>
                {
                    self.add();
                    self.mode = Mode::Normal;
                }
                Mode::Pragma => {}
                Mode::Ping
                    if self.buf_len() > 1
                        && current == c(b'`')
                        && !(ahead == c(b'`') && aahead == c(b'`'))
                        && ahead != c(b'`')
                        && self.count_is_even(b'`') =>
                {
                    if after_literal(ahead) {
                        self.add();
                    }
                    self.mode = if ahead == c(b'"') {
                        Mode::Comment
                    } else {
                        Mode::Normal
                    };
                }
                Mode::Template
                    if self.buf_len() > 1
                        && (current == c(b'|') || current == c(b'{'))
                        && (self.prev() != c(b'\\')
                            || (self.prev_prev() == c(b'\\') && self.prev() == c(b'\\'))) =>
                {
                    if current == c(b'{') || after_literal(ahead) {
                        self.add();
                    }
                    self.mode = Mode::Normal;
                }
                Mode::Template if ahead == c(b'}') && current != c(b'\\') => self.add(),
                Mode::Str
                    if current == c(b'\'')
                        && self.buf_len() > 1
                        && !(ahead == c(b'\'') && aahead == c(b'\''))
                        && ahead != c(b'\'')
                        && self.count_is_even(b'\'') =>
                {
                    self.add();
                    self.mode = if ahead == c(b'"') {
                        Mode::Comment
                    } else {
                        Mode::Normal
                    };
                }
                _ => {
                    if ahead == NL && self.mode != Mode::Template {
                        self.add();
                        self.mode = Mode::Normal;
                    } else if self.mode == Mode::Template && current == NL {
                        self.add();
                    }
                }
            }
            // advance
            if self.offset as usize == self.raw.len() && self.offset >= 0 {
                break;
            }
            self.offset += 1;
        }
        self.add();
    }

    fn buf_is_blank(&self) -> bool {
        let end = self.buf_end.min(self.raw.len());
        self.src[self.buf_start.min(end)..end].trim().is_empty()
    }
}

/// Lex `src` the way abaplint does. `\r` must not be present (strip or normalise before calling);
/// offsets are byte offsets into `src`.
pub fn lex(src: &str) -> Vec<GToken> {
    let mut l = Lex {
        raw: src.as_bytes(),
        src,
        offset: -1,
        buf_start: 0,
        buf_end: 0,
        buf_empty: true,
        mode: Mode::Normal,
        out: Vec::new(),
    };
    l.run();
    l.out
}
