//! Front end for CDS DDL / DCL source (port of abap-cleaner's DDL tokenizer, `Command.canAddToDdl`
//! and `Command.finishBuildDdlOrDcl`). The result is a [`Syntax`] like for ABAP, plus
//! [`DdlInfo`] with the command hierarchy: select list elements, parameters and joins are
//! commands of their own, nested below the command that opens the `{ ... }` / `SELECT` list.
use std::collections::HashMap;

use abapfmt_lexer::{Token, TokenKind};

use crate::ddl_tables::{DCL_COLLOCATIONS, DDL_COLLOCATIONS};
use crate::{Command, Source, Syntax, TokenType};

/// Command hierarchy of a DDL / DCL source.
#[derive(Clone, Debug)]
pub struct DdlInfo {
    pub is_dcl: bool,
    /// Parent command of each command (the command that opens the level it is in).
    pub cmd_parent: Vec<Option<usize>>,
    /// The command opens a level (`{`, `SELECT`, `PARAMETERS`).
    pub cmd_opens: Vec<bool>,
    /// The command closes the level of its parent (`}`, `FROM`, `AS` after parameters).
    pub cmd_closes: Vec<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Brace,
    Select,
}

const LIST_ELEMENT_SEPARATORS: &[&str] = &[",", ";"];
const CLOSERS_AFTER_PARAMETER_LIST: &[&str] = &["as", "returns", "{"];

// ---- language detection ----

/// Does the source look like CDS DDL / DCL (abap-cleaner's `Language.preview`)?
pub fn is_ddl_source(src: &str) -> bool {
    let b = src.as_bytes();
    let mut pos = 0;
    let mut saw_comment = false;
    loop {
        while pos < b.len() && matches!(b[pos], b' ' | b'\n' | b'\r' | b'\t') {
            pos += 1;
        }
        if pos >= b.len() {
            return saw_comment;
        }
        if src[pos..].starts_with("//") {
            saw_comment = true;
            pos = src[pos..].find('\n').map_or(b.len(), |e| pos + e);
        } else if !saw_comment && (b[pos] == b'*' || b[pos] == b'"') {
            pos = src[pos..].find('\n').map_or(b.len(), |e| pos + e);
        } else {
            break;
        }
    }
    let rest = &src[pos..];
    if rest.starts_with(['@'])
        || rest.starts_with("//")
        || rest.starts_with("--")
        || rest.starts_with("/*")
    {
        return true;
    }
    let words: Vec<String> = rest
        .split_whitespace()
        .take(4)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let w = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    // [define] [root] ...
    let mut j = 0;
    if w(j) == "define" {
        j += 1;
    }
    if w(j) == "root" {
        j += 1;
    }
    let first = w(j);
    let second = w(j + 1);
    let plain = |a: &str, b: &str| w(0) == a && w(1) == b;
    matches!(first, "abstract" | "custom") && second == "entity"
        || first == "view"
        || (first == "table" && second == "function")
        || first == "hierarchy"
        || (w(0) == "define" && matches!(first, "transient" | "structure" | "table"))
        || (w(0) == "extend" && matches!(second, "abstract" | "custom" | "view"))
        || plain("annotate", "view")
        || plain("annotate", "entity")
        || matches!(first, "role" | "accesspolicy")
}

// ---- tokenizer ----

struct Tok {
    start: usize,
    end: usize,
    lines: u32,
    spaces: u32,
    line: u32,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | '/' | '$' | '#' | '@')
}

const NUMERIC_END: &str = " \u{a0}\r\n:',;(){}[]<>=+-*/";
const IDENT_END: &str = " \u{a0}\r\n:',;(){}[]<>=+-*";

fn tokenize(src: &str) -> Option<Vec<Tok>> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let at = |i: usize| chars.get(i).map(|c| c.1);
    let byte = |i: usize| chars.get(i).map_or(src.len(), |c| c.0);
    let starts = |i: usize, s: &str| src[byte(i)..].starts_with(s);
    let mut toks = Vec::new();
    let mut pos = 0;
    let mut line = 1u32;
    let mut in_ml_comment = false;
    loop {
        let mut lines = 0;
        let mut spaces;
        loop {
            while let Some(c) = at(pos).filter(|c| *c == '\r' || *c == '\n') {
                if c == '\n' {
                    lines += 1;
                    line += 1;
                }
                pos += 1;
            }
            spaces = 0;
            while at(pos).is_some_and(|c| c == ' ' || c == '\u{a0}' || c == '\t') {
                spaces += 1;
                pos += 1;
            }
            if !at(pos).is_some_and(|c| c == '\r' || c == '\n') {
                break;
            }
        }
        if pos >= n {
            return Some(toks);
        }
        let c = at(pos)?;
        let read_until = |from: usize, ends: &str, strs: &[&str]| -> usize {
            let mut i = from;
            // an initial `@<` is allowed for annotations after list elements
            if at(pos) == Some('@') && at(from) == Some('<') {
                i += 1;
            }
            while i < n {
                let ch = at(i).unwrap();
                if ends.contains(ch) || strs.iter().any(|s| starts(i, s)) {
                    break;
                }
                i += 1;
            }
            i
        };
        let end;
        if in_ml_comment || (c == '/' && starts(pos, "/*")) {
            // asterisk comment: up to `*/` or the line end
            let line_end = (pos..n)
                .find(|i| matches!(at(*i), Some('\r' | '\n')))
                .unwrap_or(n);
            let search = pos + if starts(pos, "/*") { 2 } else { 0 };
            let ce = (search..n).find(|i| starts(*i, "*/"))?;
            let ce = ce + 2;
            end = line_end.min(ce);
            in_ml_comment = ce > line_end;
        } else if starts(pos, "//") || starts(pos, "--") {
            end = read_until(pos + 1, "\r\n", &[]);
        } else if c == '\'' {
            let mut i = pos + 1;
            loop {
                match (i..n).find(|k| at(*k) == Some('\'')) {
                    None => {
                        end = n;
                        break;
                    }
                    Some(k) => {
                        if at(k + 1) == Some('\'') {
                            i = k + 2;
                        } else {
                            end = k + 1;
                            break;
                        }
                    }
                }
            }
        } else if ":,(){}[]".contains(c) {
            end = pos + 1;
        } else if "<>!=".contains(c) {
            let mut i = pos;
            while at(i).is_some_and(|x| "<>!=".contains(x)) {
                i += 1;
            }
            end = i;
        } else if c.is_ascii_digit() {
            let mut e = read_until(pos + 1, NUMERIC_END, &[]);
            if e - pos >= 3
                && at(pos + 1) == Some('.')
                && at(pos + 2) == Some('.')
                && "01n".contains(c)
            {
                e = pos + 1;
            }
            end = e;
        } else if c == '/' && !at(pos + 1).is_some_and(is_ident_start) {
            end = pos + 1;
        } else if "+-*".contains(c) {
            end = pos + 1;
        } else {
            let mut e = read_until(pos + 1, IDENT_END, &["//", "/*"]);
            if e - pos >= 3
                && at(pos + 1) == Some('.')
                && at(pos + 2) == Some('.')
                && "01n".contains(c)
            {
                e = pos + 1;
            }
            end = e;
        }
        toks.push(Tok {
            start: byte(pos),
            end: byte(end),
            lines,
            spaces,
            line,
        });
        pos = end;
    }
}

// ---- keywords ----

struct Colloc {
    parent: Option<String>,
    seq: Vec<String>,
}

struct Keywords {
    map: HashMap<String, Vec<usize>>,
    all: Vec<Colloc>,
}

fn keywords(list: &[&str]) -> Keywords {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    let mut all = Vec::new();
    for c in list {
        let c = c.to_ascii_lowercase();
        let (parent, rest) = match c.split_once(':') {
            Some((p, r)) => (Some(p.to_string()), r.to_string()),
            None => (None, c.clone()),
        };
        let seq: Vec<String> = rest
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        let idx = all.len();
        for k in &seq {
            if k != "(" {
                map.entry(k.clone()).or_default().push(idx);
            }
        }
        all.push(Colloc { parent, seq });
    }
    Keywords { map, all }
}

fn is_known_collocation(
    kws: &Keywords,
    seq: &[String],
    main_index: usize,
    parent_function: Option<&str>,
) -> bool {
    let main = seq[main_index].to_ascii_lowercase();
    let act: Vec<String> = seq.iter().map(|s| s.to_ascii_lowercase()).collect();
    let parent = parent_function.map(str::to_ascii_lowercase);
    let Some(list) = kws.map.get(&main) else {
        return false;
    };
    for &ci in list {
        let c = &kws.all[ci];
        let Some(mc) = c.seq.iter().position(|k| *k == main) else {
            continue;
        };
        if mc > main_index || c.seq.len() - mc > act.len() - main_index {
            continue;
        }
        if let Some(p) = &c.parent
            && parent.as_ref() != Some(p)
        {
            continue;
        }
        if (0..c.seq.len()).all(|k| c.seq[k] == act[k + main_index - mc]) {
            return true;
        }
    }
    false
}

fn is_comparison_op(text: &str) -> bool {
    matches!(text, "<" | "<=" | "=" | ">=" | ">" | "<>" | "!=")
}

fn is_numeric(text: &str) -> bool {
    let mut dots = 0;
    !text.is_empty()
        && text.chars().all(|c| {
            if c == '.' {
                dots += 1;
                dots == 1
            } else {
                c.is_ascii_digit()
            }
        })
        && text.chars().any(|c| c.is_ascii_digit())
}

// ---- builder ----

struct B<'a> {
    src: &'a str,
    toks: Vec<Tok>,
    types: Vec<TokenType>,
    opens: Vec<bool>,
    closes: Vec<bool>,
    parent: Vec<Option<usize>>,
    bm: Vec<Option<usize>>,
    kws: Keywords,
    // commands
    cmds: Vec<(usize, usize)>,
    cmd_parent: Vec<Option<usize>>,
    cmd_opens: Vec<Option<Kind>>,
    cmd_closes: Vec<Option<Kind>>,
}

impl B<'_> {
    fn tx(&self, i: usize) -> &str {
        &self.src[self.toks[i].start..self.toks[i].end]
    }

    fn is_kw(&self, i: usize, s: &str) -> bool {
        self.types[i] == TokenType::Keyword && self.tx(i).eq_ignore_ascii_case(s)
    }

    fn any_kw(&self, i: usize, set: &[&str]) -> bool {
        set.iter().any(|s| self.is_kw(i, s))
    }

    fn text_any(&self, i: usize, set: &[&str]) -> bool {
        set.iter().any(|s| self.tx(i).eq_ignore_ascii_case(s))
    }

    fn comment(&self, i: usize) -> bool {
        self.types[i] == TokenType::Comment
    }

    fn first_in_line(&self, i: usize) -> bool {
        i == 0 || self.toks[i].lines > 0
    }

    fn infer(&self, text: &str, dcl: bool) -> TokenType {
        if text.starts_with("//") || text.starts_with("--") || text.starts_with("/*") {
            TokenType::Comment
        } else if text.starts_with('\'') || is_numeric(text) {
            TokenType::Literal
        } else if is_comparison_op(text) {
            TokenType::ComparisonOp
        } else if !dcl && self.kws.map.contains_key(&text.to_ascii_lowercase()) {
            TokenType::Keyword
        } else if text == ":" {
            TokenType::Colon
        } else if text == "," {
            TokenType::Comma
        } else if text
            .chars()
            .any(|c| c.is_alphabetic() || matches!(c, '_' | '$' | '#' | '@'))
        {
            TokenType::Identifier
        } else {
            TokenType::OtherOp
        }
    }

    /// Last token that is not a comment in `first..=last`.
    fn last_non_comment(&self, first: usize, last: usize) -> Option<usize> {
        (first..=last).rev().find(|i| !self.comment(*i))
    }

    fn first_code(&self, first: usize, last: usize) -> Option<usize> {
        (first..=last).find(|i| !self.comment(*i))
    }

    fn cmd_last_code(&self, c: usize) -> Option<usize> {
        let (f, l) = self.cmds[c];
        self.last_non_comment(f, l)
    }

    fn cmd_last_code_is_kw(&self, c: usize, set: &[&str]) -> bool {
        self.cmd_last_code(c).is_some_and(|t| self.any_kw(t, set))
    }

    /// Next sibling that is not a comment within `..=last`; `None` after the last child.
    fn next_code_sibling(&self, t: usize, last: usize) -> Option<usize> {
        let mut cur = t;
        loop {
            let o = self.opens[cur] && self.bm[cur].is_some();
            if self.opens[cur] && !o {
                return None;
            }
            let n = if o { self.bm[cur]? } else { cur + 1 };
            if n > last || (!o && self.closes[n] && self.bm[n].is_some()) {
                return None;
            }
            if !self.comment(n) {
                return Some(n);
            }
            cur = n;
        }
    }

    fn prev_code_sibling(&self, t: usize, first: usize) -> Option<usize> {
        let mut cur = t;
        loop {
            if self.closes[cur] && self.bm[cur].is_some() {
                // the sibling before a closer is its opener
                let o = self.bm[cur]?;
                if !self.comment(o) {
                    return Some(o);
                }
                cur = o;
                continue;
            }
            if cur == first {
                return None;
            }
            let p = cur - 1;
            if self.parent[cur] == Some(p) {
                return None;
            }
            if !self.comment(p) {
                return Some(p);
            }
            cur = p;
        }
    }

    // ---- token tree ----

    fn add_token_to_tree(&mut self, t: usize, stack: &mut Vec<usize>) {
        let last = t - 1;
        if self.opens[last] && !self.closes[t] {
            stack.push(last);
            self.parent[t] = Some(last);
        } else if !self.opens[last] && self.closes[t] {
            if let Some(o) = stack.pop() {
                self.bm[o] = Some(t);
                self.bm[t] = Some(o);
                self.parent[t] = stack.last().copied();
            } else {
                self.closes[t] = false;
                self.parent[t] = None;
            }
        } else {
            if self.opens[last] && self.closes[t] {
                self.bm[last] = Some(t);
                self.bm[t] = Some(last);
            }
            self.parent[t] = stack.last().copied();
        }
    }

    // ---- command splitting ----

    fn annotation_colon(&self, first: usize, last: usize) -> Option<usize> {
        let mut t = self.first_code(first, last)?;
        if !self.tx(t).starts_with('@') {
            return None;
        }
        if self.tx(t) == "@" {
            t = (t + 1..=last).find(|i| !self.comment(*i))?;
        }
        loop {
            if self.tx(t) == ":" {
                return Some(t);
            }
            t = (t + 1..=last).find(|i| !self.comment(*i))?;
        }
    }

    fn can_add(&self, t: usize, cf: usize, prev: Option<usize>) -> bool {
        let last = t - 1;
        if self.comment(t) && self.toks[t].lines == 0 {
            return true;
        }
        // a comment line at command start cannot add more tokens
        if self.comment(cf) && (cf..=last).all(|i| self.comment(i)) {
            return false;
        }
        if self.tx(t) == "/*+[internal] {" {
            return false;
        }
        let first_code = self.first_code(cf, last);
        if first_code.is_some_and(|f| self.tx(f).starts_with('@')) {
            if self.text_any(t, LIST_ELEMENT_SEPARATORS) {
                return true;
            }
            let Some(last_code) = self.last_non_comment(cf, last) else {
                return true;
            };
            let Some(colon) = self.annotation_colon(cf, last) else {
                return self.tx(last_code) == "@"
                    || self.tx(last_code).ends_with('.')
                    || self.tx(t).starts_with('.')
                    || self.tx(t) == ":";
            };
            let mut value = self.next_code_sibling(colon, last);
            if let Some(v) = value
                && self.tx(v) == "#"
            {
                value = (v + 1..=last).find(|i| !self.comment(*i));
            }
            let value_end = value.and_then(|v| {
                if self.opens[v] {
                    self.next_code_sibling(v, last)
                } else {
                    Some(v)
                }
            });
            return value_end.is_none();
        }
        if self.parent[last].is_some() || self.tx(last) == "[" {
            return true;
        }

        let last_code = self.last_non_comment(cf, last);
        let prev_parent = prev.and_then(|p| self.cmd_parent[p]);
        let first_in_closers =
            first_code.is_some_and(|f| self.text_any(f, CLOSERS_AFTER_PARAMETER_LIST));
        let first_is_from = first_code.is_some_and(|f| self.tx(f).eq_ignore_ascii_case("FROM"));
        let at_top = match prev {
            None => true,
            Some(p) => {
                (prev_parent.is_none() && self.cmd_opens[p].is_none())
                    || prev_parent.is_some_and(|pp| {
                        (self.cmd_last_code_is_kw(pp, &["PARAMETERS"]) && first_in_closers)
                            || (self.cmd_last_code_is_kw(pp, &["SELECT", "DISTINCT"])
                                && first_is_from)
                    })
            }
        };

        let (mut in_brace, mut in_select_no_brace, mut in_param) = (false, false, false);
        if let Some(p) = prev {
            let opener = if self.cmd_opens[p].is_some() {
                Some(p)
            } else {
                prev_parent
            };
            if let Some(o) = opener
                && let Some(lc) = self.cmd_last_code(o)
            {
                in_brace = self.tx(lc) == "{";
                in_select_no_brace = self.any_kw(lc, &["SELECT", "DISTINCT"]);
                in_param = self.is_kw(lc, "PARAMETERS") && !first_in_closers;
            }
        }

        if at_top
            && let Some(lc) = last_code
            && self.is_kw(lc, "PARAMETERS")
            && let Some(lp) = (cf..lc).rev().find(|i| !self.comment(*i))
            && self.is_kw(lp, "WITH")
        {
            return false;
        }

        if at_top && !self.comment(t) && self.tx(t) == "{" {
            return false;
        } else if at_top && last_code.is_some_and(|lc| self.tx(lc) == "{") {
            return false;
        } else if at_top
            && last_code.is_some_and(|lc| self.text_any(lc, &["SELECT", "DISTINCT"]))
            && !self.any_kw(t, &["DISTINCT", "FROM"])
        {
            return false;
        }

        if in_brace || in_select_no_brace || in_param {
            if last_code.is_some_and(|lc| {
                self.text_any(lc, LIST_ELEMENT_SEPARATORS) && self.parent[lc].is_none()
            }) {
                return false;
            } else if self.tx(t).starts_with("@<") {
                return false;
            }
        }

        if in_brace && !self.comment(t) && self.tx(t) == "}" {
            return false;
        } else if in_select_no_brace && !self.comment(t) && self.is_kw(t, "FROM") {
            return false;
        } else if self.tx(cf) == "}" {
            return false;
        } else if in_param && self.text_any(t, CLOSERS_AFTER_PARAMETER_LIST) {
            return false;
        }

        if at_top && self.types[t] == TokenType::Keyword && first_code.is_some() {
            let fc = first_code.unwrap();
            if self.any_kw(fc, &["REDEFINE", "ASSOCIATION", "COMPOSITION"]) {
                // skip this section
            } else if self.is_kw(t, "TO")
                && last_code.is_some_and(|lc| self.any_kw(lc, &["REDIRECTED", "REFERENCE"]))
            {
                // skip this section
            } else if self.any_kw(t, &["INNER", "LEFT", "RIGHT", "CROSS"]) {
                return false;
            } else if self.is_kw(t, "JOIN")
                && !last_code
                    .is_some_and(|lc| self.any_kw(lc, &["INNER", "OUTER", "CROSS", "ONE", "MANY"]))
            {
                return false;
            } else if self.any_kw(t, &["EXACT", "MANY", "ONE", "TO"])
                && !last_code.is_some_and(|lc| {
                    self.any_kw(lc, &["INNER", "OUTER", "EXACT", "MANY", "ONE", "TO"])
                })
            {
                return false;
            }
            if self.is_kw(t, "REDEFINE") {
                return false;
            } else if self.is_kw(t, "ASSOCIATION")
                && !last_code.is_some_and(|lc| self.is_kw(lc, "REDEFINE"))
            {
                return false;
            } else if self.is_kw(t, "COMPOSITION")
                && !last_code.is_some_and(|lc| self.is_kw(lc, "TO") || self.tx(lc) == ":")
            {
                return false;
            }
        }

        if at_top && self.types[t] == TokenType::Keyword {
            if self.any_kw(t, &["WHERE", "GROUP", "HAVING"]) {
                return false;
            } else if self.any_kw(t, &["EXCEPT", "INTERSECT", "UNION"]) {
                return false;
            }
        }
        true
    }

    // ---- finishing a command ----

    fn tree_attach(&mut self, new: usize, prev: Option<usize>) {
        let Some(p) = prev else {
            self.cmd_parent.push(None);
            return;
        };
        let parent = if self.cmd_opens[p].is_some() {
            match self.cmd_closes[new] {
                None => Some(p),
                Some(k) if Some(k) == self.cmd_opens[p] => self.cmd_parent[p],
                Some(_) => {
                    self.cmd_closes[new] = None;
                    Some(p)
                }
            }
        } else if let Some(k) = self.cmd_closes[new] {
            match self.cmd_parent[p] {
                Some(pp) if self.cmd_opens[pp] == Some(k) => self.cmd_parent[pp],
                _ => {
                    self.cmd_closes[new] = None;
                    self.cmd_parent[p]
                }
            }
        } else {
            self.cmd_parent[p]
        };
        self.cmd_parent.push(parent);
    }

    fn finish(&mut self, first: usize, last: usize, dcl: bool) {
        let prev = self.cmds.len().checked_sub(1);
        let first_code = self.first_code(first, last);
        let last_code = self.last_non_comment(first, last);
        let mut opener = None;
        let mut closer = None;
        if let Some(lc) = last_code {
            if self.tx(lc) == "{" {
                opener = Some(Kind::Brace);
                self.opens[lc] = false;
            } else if self.any_kw(lc, &["SELECT", "DISTINCT"]) {
                opener = Some(Kind::Select);
            } else if self.is_kw(lc, "PARAMETERS") {
                opener = Some(Kind::Brace);
            }
        }
        let prev_parent = prev.and_then(|p| self.cmd_parent[p]);
        if let Some(pp) = prev_parent
            && self.cmd_last_code_is_kw(pp, &["PARAMETERS"])
        {
            if first_code.is_some_and(|f| self.text_any(f, CLOSERS_AFTER_PARAMETER_LIST)) {
                closer = Some(Kind::Brace);
            }
        } else if let Some(f) = first_code {
            if self.tx(f) == "}" {
                closer = Some(Kind::Brace);
                self.closes[f] = false;
            } else if self.is_kw(f, "FROM") {
                closer = Some(Kind::Select);
                self.closes[f] = false;
            }
        }
        self.cmd_closes.push(closer);
        self.cmd_opens.push(opener);
        self.cmds.push((first, last));
        let idx = self.cmds.len() - 1;
        // tree_attach pushes onto cmd_parent
        let (o, c) = (self.cmd_opens[idx], self.cmd_closes[idx]);
        let _ = (o, c);
        let prev_for_tree = prev;
        self.tree_attach_last(idx, prev_for_tree);

        self.correct_types(first, last, dcl);
    }

    fn tree_attach_last(&mut self, idx: usize, prev: Option<usize>) {
        self.tree_attach(idx, prev);
    }

    fn correct_types(&mut self, first: usize, last: usize, dcl: bool) {
        let is_annotation = self
            .first_code(first, last)
            .is_some_and(|f| self.tx(f).starts_with('@'));
        for t in first..=last {
            if self.types[t] != TokenType::Keyword {
                continue;
            }
            let prev_sib = self.prev_code_sibling(t, first);
            let next_sib = self.next_code_sibling(t, last);
            let text = |s: &Self, i: usize| s.tx(i).to_string();
            if is_annotation {
                self.types[t] = TokenType::Identifier;
            } else if self.parent[t].is_some() && self.text_any(t, &["VALUE", "WITH"]) {
                if next_sib.is_some_and(|n| self.tx(n) == "=>") {
                    self.types[t] = TokenType::Identifier;
                }
            } else if self.text_any(t, &["LEFT", "RIGHT"])
                && next_sib.is_some_and(|n| self.tx(n) == "(")
            {
                self.types[t] = TokenType::Identifier;
            } else if prev_sib.is_some_and(|p| self.tx(p).ends_with('.'))
                || next_sib.is_some_and(|n| self.tx(n).starts_with('.'))
            {
                self.types[t] = TokenType::Identifier;
            } else if (prev_sib.is_none()
                || prev_sib.is_some_and(|p| {
                    self.is_kw(p, "KEY") || self.text_any(p, LIST_ELEMENT_SEPARATORS)
                }))
                && next_sib.is_some_and(|n| self.is_kw(n, "AS"))
            {
                self.types[t] = TokenType::Identifier;
            } else if prev_sib.is_some_and(|p| self.is_kw(p, "AS"))
                && !self.text_any(t, &["SELECT", "PROJECTION", "PARENT"])
            {
                self.types[t] = TokenType::Identifier;
            }
            if self.types[t] == TokenType::Keyword {
                let mut first_kw = t;
                let mut k = self.prev_code_sibling(first_kw, first);
                while let Some(x) = k.filter(|x| self.types[*x] == TokenType::Keyword) {
                    first_kw = x;
                    k = self.prev_code_sibling(x, first);
                }
                let mut seq: Vec<String> = Vec::new();
                let mut index = 0;
                let mut k = Some(first_kw);
                while let Some(x) =
                    k.filter(|x| self.types[*x] == TokenType::Keyword || self.tx(*x) == "(")
                {
                    if x == t {
                        index = seq.len();
                    }
                    seq.push(text(self, x));
                    if self.tx(x) == "(" {
                        break;
                    }
                    k = self.next_code_sibling(x, last);
                }
                let parent_kw = self.parent[t]
                    .and_then(|p| self.prev_code_sibling(p, first))
                    .map(|x| text(self, x));
                let known = if dcl {
                    is_known_collocation(&self.dcl_kws(), &seq, index, parent_kw.as_deref())
                } else {
                    is_known_collocation(&self.kws, &seq, index, parent_kw.as_deref())
                };
                if !known {
                    self.types[t] = TokenType::Identifier;
                }
            }
        }
    }

    fn dcl_kws(&self) -> Keywords {
        keywords(DCL_COLLOCATIONS)
    }
}

/// Parse CDS DDL / DCL source; `None` if the source cannot be tokenized.
pub fn parse_ddl(src: &str) -> Option<Syntax<'_>> {
    let toks = tokenize(src)?;
    let n = toks.len();
    if n == 0 {
        return None;
    }
    // DCL: "[DEFINE] ROLE" or "[DEFINE] ACCESSPOLICY"
    let texts: Vec<&str> = toks.iter().map(|t| &src[t.start..t.end]).collect();
    let is_comment = |s: &str| s.starts_with("//") || s.starts_with("--") || s.starts_with("/*");
    let code: Vec<usize> = (0..n).filter(|i| !is_comment(texts[*i])).collect();
    let dcl = code.iter().enumerate().any(|(k, &i)| {
        let t = texts[i].to_ascii_lowercase();
        (t == "role" || t == "accesspolicy")
            && (k == 0
                || texts[code[k - 1]].eq_ignore_ascii_case("define")
                    && (k == 1 || {
                        let b = texts[code[k - 2]];
                        b == "}" || b == ")" || b == "]" || b.contains('\'') || is_annotation_end(b)
                    }))
    });
    let mut b = B {
        src,
        types: Vec::new(),
        opens: vec![false; n],
        closes: vec![false; n],
        parent: vec![None; n],
        bm: vec![None; n],
        kws: keywords(DDL_COLLOCATIONS),
        cmds: Vec::new(),
        cmd_parent: Vec::new(),
        cmd_opens: Vec::new(),
        cmd_closes: Vec::new(),
        toks,
    };
    let dcl_kws = keywords(DCL_COLLOCATIONS);
    for i in 0..n {
        let text = texts[i];
        let ty = if dcl {
            // DCL keywords are taken from the DCL collocation table
            if text.starts_with("//") || text.starts_with("--") || text.starts_with("/*") {
                TokenType::Comment
            } else if text.starts_with('\'') || is_numeric(text) {
                TokenType::Literal
            } else if is_comparison_op(text) {
                TokenType::ComparisonOp
            } else if dcl_kws.map.contains_key(&text.to_ascii_lowercase()) {
                TokenType::Keyword
            } else {
                b.infer(text, true)
            }
        } else {
            b.infer(text, false)
        };
        b.types.push(ty);
        if matches!(ty, TokenType::Keyword | TokenType::OtherOp) {
            let lower = text.to_ascii_lowercase();
            b.opens[i] = ["(", "[", "{", "case"].iter().any(|s| lower.ends_with(s));
            b.closes[i] = [")", "]", "}", "end"].iter().any(|s| lower.starts_with(s));
        }
    }

    let mut final_cmds: Vec<(usize, usize)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut cf = 0;
    // commands are finished in `b.cmds`; comment commands split out later are appended there, too
    let mut t = 1;
    while t <= n {
        let split = t == n || !b.can_add(t, cf, b.cmds.len().checked_sub(1));
        if split {
            let last = t - 1;
            b.finish(cf, last, dcl);
            // split out trailing comment lines
            let idx = b.cmds.len() - 1;
            let mut cut = last;
            let mut comments = Vec::new();
            while cut > cf && b.comment(cut) && b.first_in_line(cut) {
                comments.push(cut);
                cut -= 1;
            }
            if !comments.is_empty() {
                b.cmds[idx] = (cf, cut);
                let orig_opens = b.cmd_opens[idx].is_some();
                for &c in comments.iter().rev() {
                    b.cmds.push((c, c));
                    b.cmd_opens.push(None);
                    b.cmd_closes.push(None);
                    let parent = if orig_opens {
                        Some(idx)
                    } else {
                        b.cmd_parent[idx]
                    };
                    b.cmd_parent.push(parent);
                }
            }
            final_cmds.clear();
            if t == n {
                break;
            }
            cf = t;
            stack.clear();
        } else {
            b.add_token_to_tree(t, &mut stack);
        }
        t += 1;
    }

    let mut commands = Vec::new();
    for &(f, l) in &b.cmds {
        let is_comment_only = (f..=l).all(|i| b.comment(i));
        commands.push(Command::plain(f, l, is_comment_only));
    }
    let tokens: Vec<Token> = b
        .toks
        .iter()
        .enumerate()
        .map(|(i, t)| Token {
            start: t.start as u32,
            end: t.end as u32,
            lines_before: t.lines,
            spaces_before: t.spaces,
            line: t.line,
            kind: match b.types[i] {
                TokenType::Comment => TokenKind::Comment,
                TokenType::Literal => TokenKind::Literal,
                TokenType::ComparisonOp => TokenKind::ComparisonOp,
                TokenType::Colon => TokenKind::Colon,
                TokenType::Comma => TokenKind::Comma,
                TokenType::Keyword | TokenType::Identifier => TokenKind::Word,
                _ => TokenKind::OtherOp,
            },
        })
        .collect();
    let info = DdlInfo {
        is_dcl: dcl,
        cmd_parent: b.cmd_parent.clone(),
        cmd_opens: b.cmd_opens.iter().map(Option::is_some).collect(),
        cmd_closes: b.cmd_closes.iter().map(Option::is_some).collect(),
    };
    Some(Syntax {
        src,
        sources: vec![Source::Lexer; n],
        types: b.types,
        tokens,
        commands,
        bracket_match: b.bm,
        parent: b.parent,
        ddl: Some(info),
    })
}

fn is_annotation_end(_text: &str) -> bool {
    true
}
