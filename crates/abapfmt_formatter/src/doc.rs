//! The mutable layout view rules work on.
//!
//! A [`Doc`] is the parsed source plus, per token, the number of line breaks and spaces before it
//! and an optional replacement text (for case/comment-sign changes). Rendering copies the original
//! whitespace of every token whose layout was not touched, so untouched code stays byte-identical.
use abapfmt_syntax::{Command, Options, Syntax, parse};

pub struct Doc<'a> {
    pub syn: Syntax<'a>,
    /// Line breaks before each token.
    pub lines: Vec<u32>,
    /// Spaces before each token (after the last line break).
    pub spaces: Vec<u32>,
    /// Replacement text for a token, if changed.
    pub text: Vec<Option<String>>,
    orig_lines: Vec<u32>,
    orig_spaces: Vec<u32>,
    eol: &'static str,
}

impl<'a> Doc<'a> {
    pub fn new(src: &'a str) -> Option<Doc<'a>> {
        let syn = parse(src, Options::default()).ok()?;
        let lines: Vec<u32> = syn.tokens.iter().map(|t| t.lines_before).collect();
        let spaces: Vec<u32> = syn.tokens.iter().map(|t| t.spaces_before).collect();
        let n = lines.len();
        Some(Doc {
            orig_lines: lines.clone(),
            orig_spaces: spaces.clone(),
            lines,
            spaces,
            text: vec![None; n],
            eol: if src.contains("\r\n") { "\r\n" } else { "\n" },
            syn,
        })
    }

    pub fn commands(&self) -> &[Command] {
        &self.syn.commands
    }

    pub fn token_text(&self, i: usize) -> &str {
        self.text[i].as_deref().unwrap_or_else(|| self.syn.text(i))
    }

    /// Set the line breaks before a token (a token that starts a line also keeps its spaces).
    pub fn set_lines(&mut self, i: usize, lines: u32) {
        self.lines[i] = lines;
    }

    pub fn set_spaces(&mut self, i: usize, spaces: u32) {
        self.spaces[i] = spaces;
    }

    /// The first token of a command that is neither a comment nor a pragma.
    pub fn first_code_token(&self, cmd: &Command) -> Option<usize> {
        use abapfmt_syntax::TokenType::{Comment, Pragma};
        (cmd.first..=cmd.last).find(|i| !matches!(self.syn.types[*i], Comment | Pragma))
    }

    /// Upper-cased text of the first code token.
    pub fn first_code_text(&self, cmd: &Command) -> Option<String> {
        self.first_code_token(cmd)
            .map(|i| self.syn.text(i).to_ascii_uppercase())
    }

    /// `TYPES`, `CONSTANTS`, `DATA`, `FIELD-SYMBOLS`, `CLASS-DATA`, `STATICS`, `RANGES`.
    pub fn is_declaration(&self, cmd: &Command) -> bool {
        self.first_code_text(cmd).is_some_and(|t| {
            matches!(
                t.as_str(),
                "TYPES"
                    | "CONSTANTS"
                    | "DATA"
                    | "FIELD-SYMBOLS"
                    | "CLASS-DATA"
                    | "STATICS"
                    | "RANGES"
            )
        })
    }

    /// `INCLUDE TYPE` / `INCLUDE STRUCTURE`.
    pub fn is_declaration_include(&self, cmd: &Command) -> bool {
        let Some(i) = self.first_code_token(cmd) else {
            return false;
        };
        self.syn.text(i).eq_ignore_ascii_case("INCLUDE")
            && self.next_code_token(i, cmd.last).is_some_and(|n| {
                let t = self.syn.text(n);
                t.eq_ignore_ascii_case("TYPE") || t.eq_ignore_ascii_case("STRUCTURE")
            })
    }

    /// The next token after `i` (up to `last`) that is neither a comment nor a pragma.
    pub fn next_code_token(&self, i: usize, last: usize) -> Option<usize> {
        use abapfmt_syntax::TokenType::{Comment, Pragma};
        (i + 1..=last).find(|j| !matches!(self.syn.types[*j], Comment | Pragma))
    }

    pub fn render(&self) -> String {
        let src = self.syn.src;
        let mut out = String::with_capacity(src.len());
        let mut prev_end = 0usize;
        for (i, t) in self.syn.tokens.iter().enumerate() {
            let gap = &src[prev_end..t.start as usize];
            if self.lines[i] == self.orig_lines[i] && self.spaces[i] == self.orig_spaces[i] {
                out.push_str(gap);
            } else {
                for _ in 0..self.lines[i] {
                    out.push_str(self.eol);
                }
                out.extend(std::iter::repeat_n(' ', self.spaces[i] as usize));
            }
            out.push_str(self.token_text(i));
            prev_end = t.end as usize;
        }
        out.push_str(&src[prev_end..]);
        out
    }
}
