//! NEEDLESS_SPACES: removes multiple spaces where no alignment intention can be identified.
//!
//! Port of abap-cleaner's `NeedlessSpacesRule`. The sections that are better handled by the
//! dedicated alignment rules (parameter lists, logical expressions, COND/SWITCH, declarations ...)
//! are skipped; of the expression structure, only what that needs is recognised here (see
//! [`last_of_logical_expression`]).
use std::collections::{BTreeMap, HashMap, HashSet};

use abapfmt_syntax::{Command, TokenType};

use crate::{ConfigExt, Doc, RuleConfig};

struct TokenPos {
    tok: usize,
    prev: Option<usize>,
    start_x: i64,
    line: u32,
    cmd: usize,
    cmd_last: usize,
}

/// Tokens by 2 * X position (+1 for special alignment).
type ByX = BTreeMap<i64, Vec<TokenPos>>;
/// Line number -> (first token of that line, its command).
type FirstOfLine = HashMap<u32, (usize, usize)>;

fn len(doc: &Doc, i: usize) -> i64 {
    doc.token_text(i).chars().count() as i64
}

fn is_comment(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::Comment
}

fn is_comment_after_code(doc: &Doc, cmd_first: usize, i: usize) -> bool {
    is_comment(doc, i) && i > cmd_first && doc.lines[i] == 0
}

fn text_is(doc: &Doc, i: usize, any: &[&str]) -> bool {
    let t = doc.syn.text(i);
    any.iter().any(|a| t.eq_ignore_ascii_case(a))
}

fn prev_code(doc: &Doc, first: usize, i: usize) -> Option<usize> {
    (first..i).rev().find(|j| !is_comment(doc, *j))
}

fn next_code(doc: &Doc, i: usize, last: usize) -> Option<usize> {
    (i + 1..=last).find(|j| !is_comment(doc, *j))
}

fn closer(doc: &Doc, t: usize) -> Option<usize> {
    doc.syn.bracket_match[t].filter(|c| *c > t)
}

/// The code token before `t` on the same level (`None` if `t` is the first one inside a bracket).
fn prev_sibling(doc: &Doc, first: usize, t: usize) -> Option<usize> {
    prev_code(doc, first, t).filter(|p| Some(*p) != doc.syn.parent[t])
}

fn kw(doc: &Doc, cmd: &Command) -> String {
    doc.first_code_text(cmd).unwrap_or_default()
}

fn is_non_abap(doc: &Doc, cmd: &Command) -> bool {
    doc.syn.types[cmd.first] == TokenType::NonAbap
}

enum Pat {
    Word(&'static str),
    Opt(&'static str),
    AnyIdentifier,
}

/// Do the code tokens starting at the command's first code token match `pats` one after another?
fn matches_seq(doc: &Doc, cmd: &Command, pats: &[Pat]) -> bool {
    let Some(mut i) = doc.first_code_token(cmd) else {
        return false;
    };
    for pat in pats {
        let ok = match pat {
            Pat::Word(w) => text_is(doc, i, &w.split('|').collect::<Vec<_>>()),
            Pat::Opt(w) => {
                if text_is(doc, i, &[w]) {
                    true
                } else {
                    continue;
                }
            }
            Pat::AnyIdentifier => doc.syn.types[i] == TokenType::Identifier,
        };
        if !ok {
            return false;
        }
        match next_code(doc, i, cmd.last) {
            Some(n) => i = n,
            None => return true,
        }
    }
    true
}

fn skip_whole_command(doc: &Doc, cmd: &Command) -> bool {
    if is_non_abap(doc, cmd) {
        return true;
    }
    if doc.first_code_token(cmd).is_none() {
        return false;
    }
    let first = kw(doc, cmd);
    if matches!(
        first.as_str(),
        "SELECT" | "UPDATE" | "METHODS" | "CLASS-METHODS" | "ENDEXEC"
    ) {
        return true;
    }
    if doc.is_declaration(cmd) || doc.is_declaration_include(cmd) {
        return true;
    }
    use Pat::*;
    matches_seq(doc, cmd, &[Word("CALL"), Word("METHOD|FUNCTION|BADI")])
        || matches_seq(doc, cmd, &[Word("CREATE"), Word("OBJECT")])
        || matches_seq(
            doc,
            cmd,
            &[
                Word("RAISE"),
                Opt("RESUMABLE"),
                Word("EXCEPTION"),
                Word("TYPE"),
                AnyIdentifier,
                Word("EXPORTING"),
            ],
        )
        || matches_seq(
            doc,
            cmd,
            &[
                Word("RAISE"),
                Word("SHORTDUMP"),
                Word("TYPE"),
                AnyIdentifier,
                Word("EXPORTING"),
            ],
        )
        || matches_seq(doc, cmd, &[Word("READ|DELETE"), Word("TABLE")])
}

const CONSTRUCTOR_OPERATORS: &[&str] = &[
    "NEW",
    "VALUE",
    "CONV",
    "CORRESPONDING",
    "CAST",
    "REF",
    "EXACT",
    "REDUCE",
    "FILTER",
    "COND",
    "SWITCH",
];

/// If `t` opens a list of parameters or components, the closing token.
fn end_of_params(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    let close = closer(doc, t)?;
    if doc.syn.types[t] == TokenType::Literal || close == t + 1 {
        return None;
    }
    let next = t + 1;
    let attached = doc.lines[next] == 0 && doc.spaces[next] == 0;
    if attached
        && (matches!(
            doc.syn.types[next],
            TokenType::Identifier | TokenType::Literal
        ) || text_is(doc, next, &["*", "**"]))
    {
        // DATA(lv_variable), lv_any(5), ULINE AT /10(**)
        return None;
    }
    let text = doc.syn.text(t);
    if text.eq_ignore_ascii_case("boolc(") || text.eq_ignore_ascii_case("xsdbool(") {
        return None;
    }
    if text == "(" {
        let prev = prev_sibling(doc, cmd.first, t);
        let Some(parent) = doc.syn.parent[t] else {
            // LOOP AT ... GROUP BY ( key1 = dobj1 ... ): only here may a stand-alone "(" on the top level
            // start a component list
            let group_key = !attached
                && kw(doc, cmd) == "LOOP"
                && prev.is_some_and(|p| {
                    text_is(doc, p, &["BY"])
                        && prev_sibling(doc, cmd.first, p)
                            .is_some_and(|g| text_is(doc, g, &["GROUP"]))
                });
            return group_key.then_some(close);
        };
        let parent_text = doc.syn.text(parent);
        if parent_text == "(" || !parent_text.ends_with('(') {
            return None;
        }
        if !prev_code(doc, cmd.first, parent).is_some_and(|p| text_is(doc, p, &["VALUE", "NEW"])) {
            return None;
        }
        // "VALUE #( param = a * ( b + c ) )": the "(" is part of an arithmetic expression
        if let Some(p) = prev
            && doc.syn.text(p) != ")"
            && (matches!(
                doc.syn.types[p],
                TokenType::AssignmentOp | TokenType::ComparisonOp | TokenType::OtherOp
            ) || text_is(doc, p, &["UNTIL", "WHILE", "WHERE"]))
        {
            return None;
        }
    } else {
        let prev = prev_code(doc, cmd.first, t);
        if prev.is_some_and(|p| text_is(doc, p, &["NEW", "VALUE"])) {
            // continue
        } else if prev.is_some_and(|p| text_is(doc, p, CONSTRUCTOR_OPERATORS)) {
            return None;
        }
    }
    Some(close)
}

fn last_code_token(doc: &Doc, cmd: &Command) -> Option<usize> {
    (cmd.first..=cmd.last).rev().find(|j| !is_comment(doc, *j))
}

/// If `t` starts a logical expression, its last code token. Covers IF/ELSEIF/CHECK/WHILE,
/// WHERE of LOOP/MODIFY/DELETE/FOR, ASSERT, xsdbool/boolc, and WHEN/UNTIL/WHILE/WHERE inside
/// constructor expressions. Not covered: boolean expressions on the right-hand side of an
/// assignment and ABAP SQL conditions.
fn last_of_logical_expression(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    let first = doc.first_code_token(cmd)?;
    let first_kw = kw(doc, cmd);
    let before_period = || last_code_token(doc, cmd).and_then(|l| prev_code(doc, cmd.first, l));

    if t == first && matches!(first_kw.as_str(), "IF" | "ELSEIF" | "CHECK" | "WHILE") {
        return before_period();
    }
    let top_level = doc.syn.parent[t].is_none();
    if top_level
        && text_is(doc, t, &["WHERE"])
        && (matches_seq(doc, cmd, &[Pat::Word("LOOP"), Pat::Word("AT")])
            || matches!(first_kw.as_str(), "MODIFY" | "DELETE" | "FOR"))
    {
        let end = (t + 1..=cmd.last)
            .filter(|j| doc.syn.parent[*j].is_none())
            .find_map(|j| {
                let next_is =
                    |w| next_code(doc, j, cmd.last).is_some_and(|n| text_is(doc, n, &[w]));
                if text_is(doc, j, &["GROUP"]) && next_is("BY")
                    || text_is(doc, j, &["USING"]) && next_is("KEY")
                {
                    Some((j, true))
                } else if text_is(doc, j, &["TRANSPORTING", "FROM"]) {
                    Some((j, false))
                } else {
                    None
                }
            });
        return match end {
            None => before_period(),
            Some((j, _)) => prev_code(doc, cmd.first, j),
        };
    }
    if first_kw == "ASSERT" {
        if text_is(doc, t, &["CONDITION"])
            || (t == first
                && !(first + 1..=cmd.last)
                    .any(|j| doc.syn.parent[j].is_none() && text_is(doc, j, &["CONDITION"])))
        {
            return before_period();
        }
        return None;
    }

    // xsdbool( ... ) and boolc( ... )
    if text_is(doc, t, &["boolc(", "xsdbool("]) {
        return closer(doc, t).and_then(|c| prev_code(doc, cmd.first, c));
    }

    // WHEN / UNTIL / WHILE / WHERE inside constructor expressions
    let parent = doc.syn.parent[t]?;
    let ctor = prev_sibling(doc, cmd.first, parent)?;
    if doc.syn.types[ctor] != TokenType::Keyword {
        return None;
    }
    let sibling_after = |words: &[&str]| {
        (t + 1..=cmd.last).find(|j| doc.syn.parent[*j] == Some(parent) && text_is(doc, *j, words))
    };
    if text_is(doc, t, &["WHEN"]) && text_is(doc, ctor, &["COND"]) {
        sibling_after(&["THEN"]).and_then(|j| prev_code(doc, cmd.first, j))
    } else if text_is(doc, t, &["UNTIL", "WHILE"])
        && text_is(doc, ctor, &["REDUCE", "NEW", "VALUE"])
    {
        sibling_after(&["LET", "NEXT", "FOR"]).and_then(|j| prev_code(doc, cmd.first, j))
    } else if text_is(doc, t, &["WHERE"]) && text_is(doc, ctor, &["FILTER"]) {
        closer(doc, parent).and_then(|c| prev_code(doc, cmd.first, c))
    } else {
        None
    }
}

fn skip_until_token(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    // scope of AlignParametersRule
    if let Some(end) = end_of_params(doc, cmd, t) {
        return Some(end);
    }
    // scope of AlignLogicalExpressionsRule
    if let Some(end) = last_of_logical_expression(doc, cmd, t) {
        return next_code(doc, end, cmd.last);
    }
    let close = closer(doc, t);
    // scope of AlignCondExpressionsRule
    if close.is_some()
        && prev_sibling(doc, cmd.first, t).is_some_and(|p| text_is(doc, p, &["COND", "SWITCH"]))
    {
        return close;
    }
    // empty parentheses and brackets are handled in a dedicated section
    close.filter(|c| *c == t + 1)
}

fn remove_spaces_from_empty_brackets(doc: &mut Doc, cmds: &[Command]) {
    for cmd in cmds {
        for t in cmd.first..=cmd.last {
            if closer(doc, t) == Some(t + 1)
                && doc.syn.types[t] != TokenType::Literal
                && doc.lines[t + 1] == 0
                && doc.spaces[t + 1] > 1
            {
                let start = doc.start_x(cmd.first, t + 1);
                let add = 1 - i64::from(doc.spaces[t + 1]);
                doc.spaces[t + 1] = 1;
                doc.add_indent(cmd.last, t + 1, add, start, true);
            }
        }
    }
}

fn is_numeric(s: &str) -> bool {
    let body = s
        .strip_prefix(['-', '+'])
        .or_else(|| s.strip_suffix(['-', '+']))
        .unwrap_or(s);
    !body.is_empty()
        && body.chars().all(|c| c.is_ascii_digit() || c == '.')
        && body.matches('.').count() <= 1
}

fn is_string_literal(text: &str) -> bool {
    text.starts_with(['\'', '`', '|', '}'])
}

fn is_integer_literal(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::Literal && !is_string_literal(doc.syn.text(i))
}

fn is_float_literal(doc: &Doc, i: usize) -> bool {
    let t = doc.syn.text(i);
    doc.syn.types[i] == TokenType::Literal
        && t.len() >= 2
        && t.starts_with('\'')
        && t.ends_with('\'')
        && is_numeric(&t[1..t.len() - 1])
}

fn numeric_alignment_index(num: &str) -> i64 {
    let chars: Vec<char> = num.chars().collect();
    if let Some(dot) = chars.iter().position(|c| *c == '.') {
        return dot as i64 - 1;
    }
    (0..chars.len())
        .rev()
        .find(|i| chars[*i] != '-' && chars[*i] != '\'')
        .unwrap_or(chars.len()) as i64
}

fn group_tokens_by_x(doc: &Doc, cmds: &[Command], cmd_offset: usize) -> (ByX, FirstOfLine) {
    let mut by_x = ByX::new();
    let mut first_of_line = HashMap::new();
    let mut line = 0;
    let mut command_number = 0;
    for (ci, cmd) in cmds.iter().enumerate() {
        if cmd.is_comment_only {
            continue;
        }
        let mut skip_until: Option<usize> = None;
        let mut x = 0i64;
        for t in cmd.first..=cmd.last {
            if doc.lines[t] > 0 {
                x = i64::from(doc.spaces[t]);
                line += 1;
                first_of_line.insert(line, (t, cmd_offset + ci));
            } else {
                x += i64::from(doc.spaces[t]);
            }
            if let Some(end) = skip_until {
                if t == end {
                    skip_until = skip_until_token(doc, cmd, t);
                }
                x += len(doc, t);
                continue;
            }
            skip_until = skip_until_token(doc, cmd, t);

            let pos = |x| TokenPos {
                tok: t,
                prev: (t > cmd.first).then(|| t - 1),
                start_x: x,
                line,
                cmd: command_number,
                cmd_last: cmd.last,
            };
            by_x.entry(2 * x).or_default().push(pos(x));
            let ty = doc.syn.types[t];
            if matches!(ty, TokenType::AssignmentOp | TokenType::ComparisonOp) {
                let special = x + len(doc, t) - 1;
                by_x.entry(2 * special + 1).or_default().push(pos(x));
            } else if is_integer_literal(doc, t) || is_float_literal(doc, t) {
                let special = x + numeric_alignment_index(doc.syn.text(t));
                by_x.entry(2 * special + 1).or_default().push(pos(x));
            }
            x += len(doc, t);
        }
        command_number += 1;
    }
    (by_x, first_of_line)
}

fn types_match(a: TokenType, b: TokenType) -> bool {
    use TokenType::*;
    if matches!(a, Colon | Comma | Period) || matches!(b, Colon | Comma | Period) {
        return false;
    }
    for t in [AssignmentOp, ComparisonOp, Comment, Pragma] {
        if a == t || b == t {
            return a == b;
        }
    }
    true
}

fn breaks_sequence(
    doc: &Doc,
    cmds: &[Command],
    first: &TokenPos,
    prev: &TokenPos,
    test: &TokenPos,
    x: i64,
    first_of_line: &FirstOfLine,
) -> bool {
    let is_line_end_comment =
        is_comment(doc, test.tok) && test.prev.is_some() && doc.lines[test.tok] == 0;
    if is_line_end_comment {
        // do not consider text in skipped lines
    } else if test.cmd != prev.cmd && test.line > prev.line + 1 {
        for line in prev.line + 1..test.line {
            let Some(&(ft, ci)) = first_of_line.get(&line) else {
                continue;
            };
            let cmd = cmds
                .iter()
                .find(|c| c.first <= ft && ft <= c.last)
                .unwrap_or(&cmds[ci.min(cmds.len() - 1)]);
            let k = kw(doc, cmd);
            if i64::from(doc.spaces[ft]) < x && !matches!(k.as_str(), "ELSEIF" | "ELSE" | "WHEN") {
                return true;
            }
        }
    }
    let both_after_assignment = first
        .prev
        .is_some_and(|p| doc.syn.types[p] == TokenType::AssignmentOp)
        && test
            .prev
            .is_some_and(|p| doc.syn.types[p] == TokenType::AssignmentOp);
    !both_after_assignment && !types_match(doc.syn.types[first.tok], doc.syn.types[test.tok])
}

#[allow(clippy::too_many_arguments)]
fn process_sequence(
    doc: &mut Doc,
    list: &[TokenPos],
    start: usize,
    end: usize,
    special: bool,
    processed: &mut HashSet<usize>,
    cmd_first_of: &dyn Fn(usize) -> usize,
    process_line_end_comments: bool,
) {
    let mut spare = i64::MAX;
    let mut first_in_line_only = true;
    let mut max_prev_end = 0;
    if special {
        let mut all_same_start = true;
        for tp in &list[start..end] {
            if !doc.first_in_line(tp.tok) {
                let p = tp.tok - 1;
                max_prev_end = max_prev_end.max(doc.start_x(cmd_first_of(tp.tok), p) + len(doc, p));
            }
            if tp.start_x != list[start].start_x {
                all_same_start = false;
            }
        }
        // left-aligned tokens are processed again at the lower X position
        if all_same_start {
            return;
        }
        for tp in &list[start..end] {
            processed.insert(tp.tok);
        }
    }
    for tp in &list[start..end] {
        let t = tp.tok;
        let here = if special {
            doc.start_x(cmd_first_of(t), t) - max_prev_end - 1
        } else {
            i64::from(doc.spaces[t]) - 1
        };
        spare = spare.min(here);
        if !doc.first_in_line(t) {
            first_in_line_only = false;
        }
    }
    if first_in_line_only || spare <= 0 {
        return;
    }
    for tp in &list[start..end] {
        let t = tp.tok;
        let cmd_first = cmd_first_of(t);
        if !process_line_end_comments && is_comment_after_code(doc, cmd_first, t) {
            continue;
        }
        let new = i64::from(doc.spaces[t]) - spare;
        doc.set_spaces_adjusting_indent(tp.cmd_last, t, new, false);
    }
}

fn execute_on_groups(
    doc: &mut Doc,
    cmds: &[Command],
    by_x: &ByX,
    first_of_line: &FirstOfLine,
    process_line_end_comments: bool,
) {
    let cmd_first_of = |t: usize| {
        cmds.iter()
            .find(|c| c.first <= t && t <= c.last)
            .map_or(0, |c| c.first)
    };
    let mut processed = HashSet::new();
    for (&key, list) in by_x.iter().rev() {
        let special = key % 2 != 0;
        let real_x = key.div_euclid(2);
        let mut start = 0;
        while start < list.len() {
            let first = &list[start];
            let mut prev = first;
            let mut end = start;
            let mut skip = false;
            loop {
                if processed.contains(&prev.tok) {
                    skip = true;
                }
                end += 1;
                if end == list.len() {
                    break;
                }
                let test = &list[end];
                if breaks_sequence(doc, cmds, first, prev, test, real_x, first_of_line) {
                    break;
                }
                prev = test;
            }
            if !skip {
                process_sequence(
                    doc,
                    list,
                    start,
                    end,
                    special,
                    &mut processed,
                    &cmd_first_of,
                    process_line_end_comments,
                );
            }
            start = end;
        }
    }
}

pub fn needless_spaces(doc: &mut Doc, config: &RuleConfig) {
    let across_empty = config.bool("SearchAcrossEmptyLines", true);
    let across_comments = config.bool("SearchAcrossCommentLines", true);
    let line_end_comments = config.bool("ProcessLineEndComments", false);
    let empty_brackets = config.bool("ProcessEmptyBrackets", true);

    let cmds = doc.commands().to_vec();
    let indent = |c: &Command| i64::from(c.level) * 2;
    let mut start = 0;
    while start < cmds.len() {
        let mut contains = false;
        let mut skip = false;
        let mut c = start;
        loop {
            if skip_whole_command(doc, &cmds[c]) {
                skip = true;
                break;
            }
            contains = true;
            let prev = c;
            c += 1;
            if c == cmds.len() {
                break;
            }
            let (pk, ck) = (kw(doc, &cmds[prev]), kw(doc, &cmds[c]));
            if indent(&cmds[c]) != indent(&cmds[prev]) {
                // do not break inside IF ... ENDIF and CASE ... ENDCASE: there may be intentional
                // alignment across ELSEIF / ELSE / WHEN lines
                let continues = (matches!(ck.as_str(), "ELSEIF" | "ELSE") && pk != "IF")
                    || (matches!(pk.as_str(), "ELSEIF" | "ELSE") && ck != "ENDIF")
                    || (ck == "WHEN" && pk != "CASE")
                    || (pk == "WHEN" && ck != "ENDCASE");
                if !continues {
                    break;
                }
            }
            if !across_empty && doc.lines[cmds[c].first] > 1 {
                break;
            }
            if !across_comments && cmds[c].is_comment_only {
                break;
            }
            if is_non_abap(doc, &cmds[c]) {
                break;
            }
        }
        if contains {
            let section = &cmds[start..c];
            if empty_brackets {
                remove_spaces_from_empty_brackets(doc, section);
            }
            let (by_x, first_of_line) = group_tokens_by_x(doc, section, start);
            execute_on_groups(doc, &cmds, &by_x, &first_of_line, line_end_comments);
        }
        start = if skip { c + 1 } else { c };
    }
}
