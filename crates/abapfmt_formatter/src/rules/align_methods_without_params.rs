//! ALIGN_METHODS_FOR_TESTING and ALIGN_METHODS_REDEFINITION (ports of
//! `AlignMethodsForTestingRule`, `AlignMethodsRedefinitionRule` and their common base class).
use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, is_comment, is_comment_line};
use crate::rules::align_aliases_for::contains_inner_comment_line;
use crate::rules::align_declarations::{
    find_seq_last, for_each_section, is_comma_or_period, is_kw, next_non_comment_sibling,
    skip_command,
};
use crate::{ConfigExt, Doc, RuleConfig};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    ForTesting,
    Redefinition,
}

const KEYWORD: usize = 0;
const METHOD_NAME: usize = 1;
// ForTesting: ABSTRACT, FOR_TESTING, RAISING, LINE_END_COMMENT
const ABSTRACT: usize = 2;
const FOR_TESTING: usize = 3;
const RAISING: usize = 4;
const FT_LINE_END_COMMENT: usize = 5;
// Redefinition: REDEFINITION, LINE_END_COMMENT
const REDEFINITION: usize = 2;
const RD_LINE_END_COMMENT: usize = 3;

#[derive(Clone, Copy, PartialEq)]
enum ChangeType {
    Always,
    KeepAsIs,
    Never,
}

pub fn align_methods_for_testing(doc: &mut Doc, config: &RuleConfig) {
    run(doc, config, Kind::ForTesting);
}

pub fn align_methods_redefinition(doc: &mut Doc, config: &RuleConfig) {
    run(doc, config, Kind::Redefinition);
}

/// `[ABSTRACT] FOR TESTING` directly at `t`.
fn at_for_testing(doc: &Doc, t: usize, last: usize) -> bool {
    let mut t = t;
    if t <= last && is_kw(doc, t, "ABSTRACT") {
        t += 1;
    }
    t < last && is_kw(doc, t, "FOR") && is_kw(doc, t + 1, "TESTING")
}

/// `ident [ABSTRACT] FOR TESTING` at `t`.
fn ident_for_testing(doc: &Doc, t: usize, last: usize) -> bool {
    t < last && doc.syn.types[t] == TokenType::Identifier && at_for_testing(doc, t + 1, last)
}

fn ident_redefinition(doc: &Doc, t: usize, last: usize) -> bool {
    t < last
        && doc.syn.types[t] == TokenType::Identifier
        && (is_kw(doc, t + 1, "REDEFINITION")
            || (t + 2 <= last && is_kw(doc, t + 1, "FINAL") && is_kw(doc, t + 2, "REDEFINITION")))
}

fn matches_first(doc: &Doc, cmd: &Command, kind: Kind) -> bool {
    if cmd.is_comment_only {
        return false;
    }
    let first = cmd.first;
    let last = cmd.last;
    let chain = cmd.colon == Some(first + 1);
    match kind {
        Kind::ForTesting => {
            if !is_kw(doc, first, "METHODS") {
                return false;
            }
            if chain {
                if find_seq_last(doc, first, last, &["FOR", "TESTING"]).is_none() {
                    return false;
                }
                let name = first + 2;
                ident_for_testing(doc, name, last)
                    || (name < last
                        && doc.syn.types[name] == TokenType::Identifier
                        && doc.syn.types[name + 1] == TokenType::Comma)
            } else {
                ident_for_testing(doc, first + 1, last)
            }
        }
        Kind::Redefinition => {
            if !is_kw(doc, first, "METHODS") {
                return false;
            }
            let name = if chain { first + 2 } else { first + 1 };
            ident_redefinition(doc, name, last)
        }
    }
}

fn matches_further(doc: &Doc, cmd: &Command, keyword: &str, kind: Kind) -> bool {
    if cmd.is_comment_only || !doc.syn.text(cmd.first).eq_ignore_ascii_case(keyword) {
        return false;
    }
    match kind {
        Kind::ForTesting => ident_for_testing(doc, cmd.first + 1, cmd.last),
        Kind::Redefinition => ident_redefinition(doc, cmd.first + 1, cmd.last),
    }
}

fn change_type(config: &RuleConfig, key: &str) -> ChangeType {
    match config.choice(key, "ALWAYS").as_str() {
        "NEVER" => ChangeType::Never,
        "KEEP_AS_IS" => ChangeType::KeepAsIs,
        _ => ChangeType::Always,
    }
}

fn run(doc: &mut Doc, config: &RuleConfig, kind: Kind) {
    let after_keyword = change_type(config, "ContinueAfterKeyword");
    let after_name = change_type(config, "ContinueAfterMethodName");
    for_each_section(
        doc,
        config,
        true,
        |doc, cmd| matches_first(doc, cmd, kind),
        |doc, cmd, kw| matches_further(doc, cmd, kw, kind),
        |doc, cmds, start, end, chain| {
            execute_on(
                doc,
                cmds,
                start,
                end,
                chain,
                kind,
                after_keyword,
                after_name,
            )
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn execute_on(
    doc: &mut Doc,
    cmds: &[Command],
    start: usize,
    end: usize,
    is_chain: bool,
    kind: Kind,
    after_keyword: ChangeType,
    after_name: ChangeType,
) {
    let ncols = if kind == Kind::ForTesting { 6 } else { 4 };
    let mut table = Table::new(ncols);
    let mut ci = start;
    if contains_inner_comment_line(doc, &cmds[ci]) {
        return;
    }
    let mut token = cmds[ci].first;
    let first_line_breaks = doc.lines[token];
    let basic_indent = doc.start_x(0, token);
    let n = doc.syn.tokens.len();
    loop {
        if is_chain {
            while token < n && token <= cmds[ci].last && is_comment_line(doc, token) {
                token += 1;
            }
            if token > cmds[ci].last {
                break;
            }
        } else {
            while ci < end && skip_command(doc, &cmds[ci]) {
                ci += 1;
            }
            if ci == end {
                break;
            }
            token = cmds[ci].first;
        }
        let Some(line_end) =
            build_table_line(doc, &mut table, token, cmds[ci].last, is_chain, kind)
        else {
            return;
        };
        token = line_end;
        if is_chain {
            token += 1;
            if token > cmds[ci].last {
                break;
            }
        } else {
            ci += 1;
            if ci == end {
                break;
            }
            if contains_inner_comment_line(doc, &cmds[ci]) {
                return;
            }
            token = cmds[ci].first;
        }
    }
    join_columns(doc, &mut table, is_chain, kind, after_keyword, after_name);
    if !table.lines.is_empty() {
        table.align(doc, basic_indent, first_line_breaks, false, true);
    }
}

fn build_table_line(
    doc: &Doc,
    table: &mut Table,
    mut token: usize,
    last: usize,
    is_chain: bool,
    kind: Kind,
) -> Option<usize> {
    let line = table.add_line();
    if is_kw(doc, token, "METHODS") && doc.syn.types[token] == TokenType::Keyword {
        if let Some(n) = next_non_comment_sibling(doc, token, last)
            && doc.syn.types[n] == TokenType::Colon
        {
            table.set_cell(line, KEYWORD, Cell::new(doc, token, n));
            token = n;
        } else {
            table.set_cell(line, KEYWORD, Cell::new(doc, token, token));
        }
        token += 1;
    }
    let name = token;
    table.set_cell(line, METHOD_NAME, Cell::new(doc, name, name));
    token = name + 1;
    match kind {
        Kind::ForTesting => {
            if at_for_testing(doc, token, last) {
                if is_kw(doc, token, "ABSTRACT") {
                    table.set_cell(line, ABSTRACT, Cell::new(doc, token, token));
                    token += 1;
                }
                table.set_cell(line, FOR_TESTING, Cell::new(doc, token, token + 1));
                token += 2;
                if token <= last && is_kw(doc, token, "RAISING") {
                    let period = find_seq_last(doc, token, last, &[",|."])?;
                    let raising_last = (token..period).rev().find(|j| !is_comment(doc, *j))?;
                    table.set_cell(line, RAISING, Cell::new(doc, token, raising_last));
                    token = period;
                }
            } else if !is_chain {
                table.lines.pop();
                return None;
            }
        }
        Kind::Redefinition => {
            let end = if is_kw(doc, token, "REDEFINITION") {
                token
            } else if token < last
                && is_kw(doc, token, "FINAL")
                && is_kw(doc, token + 1, "REDEFINITION")
            {
                token + 1
            } else {
                table.lines.pop();
                return None;
            };
            table.set_cell(line, REDEFINITION, Cell::new(doc, token, end));
            token = end + 1;
        }
    }
    while token < last && !is_comma_or_period(doc, token) {
        token += 1;
    }
    if token < last && is_comment(doc, token + 1) && doc.lines[token + 1] == 0 {
        token += 1;
        let col = if kind == Kind::ForTesting {
            FT_LINE_END_COMMENT
        } else {
            RD_LINE_END_COMMENT
        };
        table.set_cell(line, col, Cell::new(doc, token, token));
    }
    Some(token)
}

fn join_columns(
    doc: &mut Doc,
    table: &mut Table,
    is_chain: bool,
    kind: Kind,
    after_keyword: ChangeType,
    after_name: ChangeType,
) {
    if kind == Kind::ForTesting {
        for i in (RAISING..=FT_LINE_END_COMMENT).rev() {
            if table.column_is_empty(i) {
                continue;
            }
            let count = table.column_count(i);
            if count <= 1 || (count as f64) < (table.lines.len() as f64 * 0.2).floor() {
                table.join_into_previous_columns(doc, i, true);
            }
        }
    }
    for col in KEYWORD..=METHOD_NAME {
        if table.column_is_empty(col) {
            continue;
        }
        let continue_line = if col == KEYWORD {
            after_keyword
        } else {
            after_name
        };
        let mut force = continue_line != ChangeType::Always;
        for li in 0..table.lines.len() {
            if table.lines[li].cells[col].is_none() {
                continue;
            }
            let Some(next) = table.lines[li].cells[col + 1..].iter().flatten().next() else {
                continue;
            };
            let nt = next.first;
            match continue_line {
                ChangeType::Always => {
                    if doc.lines[nt] > 0 && !is_comment(doc, nt - 1) {
                        doc.lines[nt] = 0;
                        doc.spaces[nt] = 1;
                    }
                }
                ChangeType::KeepAsIs => {
                    if doc.lines[nt] == 0 {
                        force = false;
                    }
                }
                ChangeType::Never => {
                    if doc.lines[nt] == 0 {
                        let x = doc.start_x(0, nt);
                        doc.lines[nt] = 1;
                        doc.spaces[nt] = x.max(0) as u32;
                    }
                }
            }
        }
        if force {
            table.set_force_line_break_after(col, true, None);
            if col == METHOD_NAME && !is_chain {
                if table.column_force_line_break_after(KEYWORD) {
                    table.set_force_indent(2, Some(METHOD_NAME), 2);
                } else {
                    table.set_force_indent(2, Some(KEYWORD), 2);
                }
            }
        }
        if continue_line != ChangeType::Always {
            table.override_width_if_followed_by_line_breaks(doc, col);
        }
    }
}
