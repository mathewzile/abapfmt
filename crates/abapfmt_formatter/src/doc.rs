//! The mutable layout view rules work on.
//!
//! A [`Doc`] is the parsed source plus, per token, the number of line breaks and spaces before it
//! and an optional replacement text (for case/comment-sign changes). Rendering copies the original
//! whitespace of every token whose layout was not touched, so untouched code stays byte-identical.
use abapfmt_syntax::{Command, Options, Syntax, is_ddl_source, parse, parse_ddl};

pub struct Doc<'a> {
    pub syn: Syntax<'a>,
    /// Line breaks before each token.
    pub lines: Vec<u32>,
    /// Spaces before each token (after the last line break).
    pub spaces: Vec<u32>,
    /// Replacement text for a token, if changed.
    pub text: Vec<Option<String>>,
    /// Output order of the tokens; rules may move a period or comma in front of comments.
    order: Vec<usize>,
    orig_lines: Vec<u32>,
    orig_spaces: Vec<u32>,
    eol: &'static str,
    /// Indentation of the first command of the code (before any rule changed it).
    base_indent: u32,
    /// The code has an introductory statement (REPORT ...) or a class, so `base_indent` is not used.
    has_intro_or_class: bool,
}

impl<'a> Doc<'a> {
    pub fn new(src: &'a str) -> Option<Doc<'a>> {
        let syn = if is_ddl_source(src) {
            parse_ddl(src)?
        } else {
            parse(src, Options::default()).ok()?
        };
        let lines: Vec<u32> = syn.tokens.iter().map(|t| t.lines_before).collect();
        let spaces: Vec<u32> = syn.tokens.iter().map(|t| t.spaces_before).collect();
        let n = lines.len();
        let word = |c: &Command| {
            (c.first..=c.last)
                .find(|i| syn.types[*i] != abapfmt_syntax::TokenType::Comment)
                .map(|i| syn.text(i).to_ascii_uppercase())
                .unwrap_or_default()
        };
        let has_intro_or_class = syn.commands.iter().any(|c| {
            !c.is_comment_only
                && (c.is_class_start()
                    || matches!(
                        word(c).as_str(),
                        "CLASS-POOL"
                            | "FUNCTION-POOL"
                            | "INTERFACE-POOL"
                            | "PROGRAM"
                            | "REPORT"
                            | "TYPE-POOL"
                    ))
        });
        let base_indent = syn
            .commands
            .iter()
            .find(|c| !c.is_comment_only)
            .map(|c| {
                let spaces = syn.tokens[c.first].spaces_before;
                if c.is_method_start() {
                    spaces.max(2)
                } else {
                    spaces
                }
            })
            .or_else(|| {
                syn.commands
                    .iter()
                    .find(|c| c.is_comment_only && syn.text(c.first).starts_with('"'))
                    .map(|c| syn.tokens[c.first].spaces_before)
            })
            .unwrap_or(0);
        Some(Doc {
            base_indent,
            has_intro_or_class,
            orig_lines: lines.clone(),
            orig_spaces: spaces.clone(),
            lines,
            spaces,
            text: vec![None; n],
            order: (0..n).collect(),
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

    /// Token indices of `cmd` in current output order.
    pub fn order_of(&self, cmd: &Command) -> &[usize] {
        &self.order[cmd.first..=cmd.last]
    }

    /// Move token `t` so that it is output directly before token `target` (both in `cmd`).
    pub fn move_before(&mut self, cmd: &Command, t: usize, target: usize) {
        let range = cmd.first..=cmd.last;
        let from = self.order[range.clone()]
            .iter()
            .position(|x| *x == t)
            .unwrap()
            + cmd.first;
        self.order.remove(from);
        let to = self.order[cmd.first..cmd.last]
            .iter()
            .position(|x| *x == target)
            .unwrap()
            + cmd.first;
        self.order.insert(to, t);
    }

    /// Move token `t` so that it is output directly after token `anchor` (both in `cmd`).
    pub fn move_after(&mut self, cmd: &Command, t: usize, anchor: usize) {
        let from = self.order[cmd.first..=cmd.last]
            .iter()
            .position(|x| *x == t)
            .unwrap()
            + cmd.first;
        self.order.remove(from);
        let to = self.order[cmd.first..cmd.last]
            .iter()
            .position(|x| *x == anchor)
            .unwrap()
            + cmd.first;
        self.order.insert(to + 1, t);
    }

    /// Indentation of code that follows command `ci`: the spaces of the enclosing top-level
    /// command plus two per open block (abap-cleaner's `Command.getIndent()`).
    pub fn indent_after(&self, ci: usize) -> u32 {
        let cmds = self.commands();
        let base = cmds[..=ci]
            .iter()
            .rev()
            .find(|c| c.level == 0 && !c.is_comment_only)
            .map_or(0, |c| self.spaces[c.first]);
        base + 2 * cmds[ci].depth_after
    }

    /// Column where token `i` starts (`Token.getStartIndexInLine`); `first` is the first token
    /// of its command.
    pub fn start_x(&self, first: usize, i: usize) -> i64 {
        let len = |j: usize| self.token_text(j).chars().count() as i64;
        let mut k = i;
        while k > first && self.lines[k] == 0 {
            k -= 1;
        }
        let mut x = i64::from(self.spaces[k]);
        for j in k + 1..=i {
            x += len(j - 1) + i64::from(self.spaces[j]);
        }
        x
    }

    /// `Token.isFirstTokenInLine`: also true for the very first token of the source.
    pub fn first_in_line(&self, i: usize) -> bool {
        self.lines[i] > 0 || i == 0
    }

    /// `Command.addIndent`: adds `add` spaces to the first token of every line from token `from`
    /// to `cmd_last` whose indent is at least `min_spaces`. With `stop_at_lower_indent`, stops at
    /// the first line with a lower indent. Asterisk comment lines are not moved.
    pub fn add_indent(
        &mut self,
        cmd_last: usize,
        from: usize,
        add: i64,
        min_spaces: i64,
        stop_at_lower_indent: bool,
    ) {
        self.add_indent_range(from, cmd_last + 1, add, min_spaces, stop_at_lower_indent);
    }

    /// `Command.addIndent` for the tokens `from..end` (`end` excluded).
    pub fn add_indent_range(
        &mut self,
        from: usize,
        end: usize,
        add: i64,
        min_spaces: i64,
        stop_at_lower_indent: bool,
    ) {
        if add == 0 {
            return;
        }
        for t in from..end {
            let asterisk = self.syn.ddl.is_none()
                && self.syn.types[t] == abapfmt_syntax::TokenType::Comment
                && self.lines[t] > 0
                && self.syn.text(t).starts_with('*');
            if self.first_in_line(t) && !asterisk {
                if i64::from(self.spaces[t]) >= min_spaces {
                    self.spaces[t] = (i64::from(self.spaces[t]) + add).max(0) as u32;
                } else if stop_at_lower_indent {
                    break;
                }
            }
        }
    }

    /// `Token.setSpacesLeftAdjustingIndent`: sets the spaces before token `t` and shifts the
    /// following lines of its command (`cmd_last` is the command's last token) by the same amount.
    pub fn set_spaces_adjusting_indent(
        &mut self,
        cmd_last: usize,
        t: usize,
        new: i64,
        adjust_if_directly_below: bool,
    ) {
        let old = i64::from(self.spaces[t]);
        if new == old {
            return;
        }
        if t == cmd_last {
            self.spaces[t] = new as u32;
            return;
        }
        let min_spaces = if adjust_if_directly_below || self.lines[t + 1] > 0 {
            let x = self.start_x(0, t);
            if adjust_if_directly_below { x } else { x + 1 }
        } else {
            self.start_x(0, t + 1)
        };
        self.spaces[t] = new as u32;
        self.add_indent(cmd_last, t + 1, new - old, min_spaces, true);
    }

    /// The indentation command `ci` should have (`Command.getIndent`).
    pub fn indent_of(&self, ci: usize) -> u32 {
        use abapfmt_syntax::TokenType::{Colon, Comma};
        let cmds = self.commands();
        let c = &cmds[ci];
        if self.is_declaration_include(c) {
            // mimic the pretty printer: INCLUDE inside BEGIN OF ... END OF follows the previous element
            let prev = cmds[..ci]
                .iter()
                .rev()
                .find(|p| !p.is_comment_only && !self.is_declaration_include(p));
            if let Some(p) = prev
                && self.is_declaration(p)
                && let Some(sep) = (p.first..=p.last).rev().find(|j| {
                    matches!(self.syn.types[*j], Colon | Comma) && self.syn.parent[*j].is_none()
                })
                && let Some(start) = self.next_code_token(sep, p.last)
            {
                let mut x = self.start_x(p.first, start) as u32;
                let begins = self.syn.text(start).eq_ignore_ascii_case("BEGIN")
                    && self
                        .next_code_token(start, p.last)
                        .is_some_and(|n| self.syn.text(n).eq_ignore_ascii_case("OF"));
                if begins {
                    x += 2;
                }
                return x;
            }
        }
        let base = if c.root_no_base || self.has_intro_or_class {
            0
        } else {
            self.base_indent
        };
        c.indent_rel + base
    }

    pub fn render(&self) -> String {
        let src = self.syn.src;
        let mut out = String::with_capacity(src.len());
        for &i in &self.order {
            let t = &self.syn.tokens[i];
            let before = if i == 0 {
                0
            } else {
                self.syn.tokens[i - 1].end as usize
            };
            let gap = &src[before..t.start as usize];
            if self.lines[i] == self.orig_lines[i] && self.spaces[i] == self.orig_spaces[i] {
                out.push_str(gap);
            } else {
                for _ in 0..self.lines[i] {
                    out.push_str(self.eol);
                }
                out.extend(std::iter::repeat_n(' ', self.spaces[i] as usize));
            }
            out.push_str(self.token_text(i));
        }
        let tail = self.syn.tokens.last().map_or(0, |t| t.end as usize);
        out.push_str(&src[tail..]);
        out
    }
}
