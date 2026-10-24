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
