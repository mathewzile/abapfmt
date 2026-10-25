//! ALIGN_METHODS_DECLARATION: aligns METHODS / CLASS-METHODS declarations (port of
//! `AlignMethodsDeclarationRule`).
use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, is_comment, is_comment_line};
use crate::rules::align_declarations::{
    find_seq_last, for_each_section, is_any_kw, is_comma_or_period, is_kw, matches_seq,
    next_non_comment_sibling, next_sibling, skip_command,
};
use crate::{ConfigExt, Doc, RuleConfig};

const KEYWORD: usize = 0;
const METHOD_NAME: usize = 1;
const ACCESS: usize = 2;
const PARAMETER_NAME: usize = 3;
const TYPE_OR_LIKE: usize = 4;
const DEFAULT_OR_OPTIONAL: usize = 5;
const MAX_COLUMN_COUNT: usize = 6;

const ACCESS_OR_EXCEPTION: &[&str] = &[
    "IMPORTING",
    "EXPORTING",
    "CHANGING",
    "RETURNING",
    "RAISING",
    "EXCEPTIONS",
];

#[derive(Clone, Copy, PartialEq)]
enum ChangeType {
    Always,
    KeepAsIs,
    Never,
}

fn change_type(config: &RuleConfig, key: &str, default: &str) -> ChangeType {
    match config.choice(key, default).as_str() {
        "ALWAYS" => ChangeType::Always,
        "NEVER" => ChangeType::Never,
        _ => ChangeType::KeepAsIs,
    }
}

struct Opts {
    continue_after_keyword: ChangeType,
    continue_after_method_name: ChangeType,
    continue_after_access: ChangeType,
    fill_ratio: f64,
    one_liners: String,
    align_consecutive: String,
    separate_with_empty_line: bool,
}

/// A phrase pattern such as `"FOR TESTING|FINAL REDEFINITION|REDEFINITION"`.
pub(crate) fn matches_phrase(doc: &Doc, t: usize, last: usize, phrases: &str) -> bool {
    phrases.split('|').any(|p| {
        let words: Vec<&str> = p.split(' ').collect();
        matches_seq(doc, t, last, &words)
    })
}

/// `Command.isRapHandlerMethodDef`
fn is_rap_handler_method_def(doc: &Doc, cmd: &Command) -> bool {
    let Some(first) = doc.first_code_token(cmd) else {
        return false;
    };
    let last = cmd.last;
    if !doc.syn.text(first).eq_ignore_ascii_case("METHODS") {
        return false;
    }
    let Some(name) = next_non_comment_sibling(doc, first, last) else {
        return false;
    };
    if doc.syn.types[name] != TokenType::Identifier {
        return false;
    }
    let Some(mut f) = next_non_comment_sibling(doc, name, last) else {
        return false;
    };
    if is_kw(doc, f, "FINAL") {
        let Some(n) = next_non_comment_sibling(doc, f, last) else {
            return false;
        };
        f = n;
    }
    if !is_kw(doc, f, "FOR") {
        return false;
    }
    if matches_seq(doc, f, last, &["FOR", "EVENT|TESTING"]) {
        return false;
    }
    matches_seq(
        doc,
        f,
        last,
        &[
            "FOR",
            "ENTITY|DETERMINE|GLOBAL|INSTANCE|AUTHORIZATION|FEATURES|LOCK|MODIFY|NUMBERING|PRECHECK|READ|VALIDATE",
        ],
    )
}

fn is_methods_cmd(doc: &Doc, cmd: &Command) -> bool {
    !cmd.is_comment_only
        && matches!(
            doc.first_code_text(cmd).as_deref(),
            Some("METHODS" | "CLASS-METHODS")
        )
}

pub fn align_methods_declaration(doc: &mut Doc, config: &RuleConfig) {
    let opts = Opts {
        continue_after_keyword: change_type(config, "ContinueAfterKeyword", "KEEP_AS_IS"),
        continue_after_method_name: change_type(config, "ContinueAfterMethodName", "KEEP_AS_IS"),
        continue_after_access: change_type(config, "ContinueAfterAccess", "ALWAYS"),
        fill_ratio: config.int("FillPercentageToJustifyOwnColumn", 40) as f64 / 100.0,
        one_liners: config.choice("HandleOneLiners", "KEEP_EXISTING"),
        align_consecutive: config.choice("AlignConsecutive", "ONE_LINERS"),
        separate_with_empty_line: config.bool("SeparateWithEmptyLine", true),
    };
    let passes = if opts.align_consecutive == "NEVER" {
        1
    } else {
        2
    };
    for pass in 0..passes {
        for_each_section(
            doc,
            config,
            false,
            |doc, cmd| is_methods_cmd(doc, cmd) && !is_rap_handler_method_def(doc, cmd),
            |doc, cmd, keyword| {
                pass > 0
                    && doc.first_code_text(cmd).as_deref() == Some(keyword)
                    && is_methods_cmd(doc, cmd)
                    && !is_rap_handler_method_def(doc, cmd)
            },
            |doc, cmds, start, end, _chain| execute_on(doc, &opts, cmds, start, end, pass),
        );
    }
}

fn cmd_index_of(cmds: &[Command], t: usize) -> usize {
    cmds.iter()
        .position(|c| (c.first..=c.last).contains(&t))
        .unwrap()
}

fn line_with_free_cell(table: &mut Table, column: usize) -> usize {
    let line = table.lines.len() - 1;
    if (column..MAX_COLUMN_COUNT).any(|c| table.lines[line].cells[c].is_some()) {
        return table.add_line();
    }
    line
}

fn execute_on(doc: &mut Doc, opts: &Opts, cmds: &[Command], start: usize, end: usize, pass: usize) {
    let multi = pass > 0;
    let mut table = Table::new(MAX_COLUMN_COUNT);
    for ci in start..end {
        let cmd = &cmds[ci];
        if skip_command(doc, cmd) || !is_methods_cmd(doc, cmd) {
            continue;
        }
        let last = cmd.last;
        if !multi {
            table = Table::new(MAX_COLUMN_COUNT);
        }
        // KEYWORD cell: [CLASS-]METHODS and, if applicable, the chain colon
        let mut token = doc.first_code_token(cmd).unwrap();
        let line = table.add_line();
        if let Some(n) = next_non_comment_sibling(doc, token, last)
            && doc.syn.types[n] == TokenType::Colon
        {
            table.set_cell(line, KEYWORD, Cell::new(doc, token, n));
            token = n;
        } else {
            table.set_cell(line, KEYWORD, Cell::new(doc, token, token));
        }
        // METHOD_NAME
        let Some(name) = next_non_comment_sibling(doc, token, last) else {
            continue;
        };
        let l = table.lines.len() - 1;
        table.set_cell(l, METHOD_NAME, Cell::new(doc, name, name));
        let mut cur = next_non_comment_sibling(doc, name, last);
        let mut is_event = false;
        while let Some(mut tok) = cur {
            if is_any_kw(doc, tok, ACCESS_OR_EXCEPTION) {
                tok = add_access_or_exception(doc, &mut table, tok, last, is_event);
            } else if doc.syn.types[tok] == TokenType::Keyword
                && matches_phrase(
                    doc,
                    tok,
                    last,
                    "FOR TESTING|FINAL REDEFINITION|REDEFINITION",
                )
            {
                tok = add_for_testing_or_redefinition(doc, &mut table, tok, last);
            } else if doc.syn.types[tok] == TokenType::Keyword
                && matches_phrase(
                    doc,
                    tok,
                    last,
                    "ABSTRACT|FINAL|DEFAULT IGNORE|DEFAULT FAIL|AMDP OPTION|FOR EVENT",
                )
            {
                if doc.lines[tok] > 0 {
                    let l = line_with_free_cell(&mut table, ACCESS);
                    table.set_cell(l, ACCESS, Cell::special(doc, tok, tok, 0, true));
                }
                if matches_seq(doc, tok, last, &["FOR", "EVENT"]) {
                    is_event = true;
                }
            } else if is_any_kw(doc, tok, &["TYPE", "LIKE"]) {
                add_parameter(doc, &mut table, tok, cmd.first);
            } else if is_any_kw(doc, tok, &["OPTIONAL", "DEFAULT"]) {
                add_optional_or_default(doc, &mut table, tok, cmd.first);
            } else if is_comma_or_period(doc, tok) {
                table = finish_table_if_full(doc, opts, cmds, tok, table, multi);
                let Some(n) = next_non_comment_sibling(doc, tok, last) else {
                    break;
                };
                is_event = false;
                let l = table.add_line();
                table.set_cell(l, METHOD_NAME, Cell::new(doc, n, n));
                tok = n;
            }
            cur = next_non_comment_sibling(doc, tok, last);
        }
    }
    if multi && !table.lines.is_empty() {
        exec_table(doc, opts, cmds, start, &mut table, multi);
    }
    // align inner quote-mark comment lines with the next code
    for cmd in &cmds[start..end] {
        for t in cmd.first..=cmd.last {
            if is_comment_line(doc, t)
                && doc.syn.text(t).starts_with('"')
                && let Some(next_code) =
                    (t + 1..doc.syn.tokens.len()).find(|j| !is_comment(doc, *j))
                && doc.spaces[t] != doc.spaces[next_code]
            {
                doc.spaces[t] = doc.spaces[next_code];
            }
        }
    }
}

fn add_access_or_exception(
    doc: &Doc,
    table: &mut Table,
    token: usize,
    last: usize,
    is_event: bool,
) -> usize {
    let mut line = line_with_free_cell(table, ACCESS);
    table.set_cell(line, ACCESS, Cell::new(doc, token, token));
    let mut token = token;
    if is_any_kw(doc, token, &["EXCEPTIONS", "RAISING"])
        || is_event && is_kw(doc, token, "IMPORTING")
    {
        loop {
            let Some(n) = next_non_comment_sibling(doc, token, last) else {
                break;
            };
            token = n;
            if table.lines[line].cells[PARAMETER_NAME].is_none() {
                // like PrettyPrinter, these names do not influence the PARAMETER_NAME column width
                table.set_cell(
                    line,
                    PARAMETER_NAME,
                    Cell::special(doc, token, token, 0, true),
                );
            }
            if doc.syn.bracket_match[token].is_some_and(|c| c > token)
                && let Some(n) = next_non_comment_sibling(doc, token, last)
            {
                token = n;
            }
            let Some(test) = next_non_comment_sibling(doc, token, last) else {
                break;
            };
            if doc.syn.types[test] != TokenType::Identifier {
                break;
            }
            if doc.lines[test] > 0 {
                line = table.add_line();
            }
        }
    }
    token
}

fn add_for_testing_or_redefinition(
    doc: &mut Doc,
    table: &mut Table,
    token: usize,
    last: usize,
) -> usize {
    let line = line_with_free_cell(table, ACCESS);
    table.set_cell(line, ACCESS, Cell::special(doc, token, token, 0, true));
    let mut token = token;
    if matches_seq(doc, token, last, &["FINAL", "REDEFINITION"]) {
        token = next_non_comment_sibling(doc, token, last).unwrap();
    } else if matches_seq(doc, token, last, &["FOR", "TESTING"]) {
        token = next_non_comment_sibling(doc, token, last).unwrap();
        // "RAISING" directly behind "FOR TESTING" stays there (as does the first exception class)
        if let Some(next) = next_sibling(doc, token, last)
            && is_kw(doc, next, "RAISING")
        {
            doc.lines[next] = 0;
            doc.spaces[next] = 1;
            token = next;
            if let Some(n2) = next_sibling(doc, token, last)
                && doc.syn.types[n2] == TokenType::Identifier
            {
                doc.lines[n2] = 0;
                doc.spaces[n2] = 1;
                token = n2;
            }
        }
    }
    token
}

fn add_parameter(doc: &Doc, table: &mut Table, type_or_like: usize, cmd_first: usize) {
    let line = line_with_free_cell(table, PARAMETER_NAME);
    let Some(name) = (cmd_first..type_or_like)
        .rev()
        .find(|j| !is_comment(doc, *j))
    else {
        return;
    };
    let cell = if doc.syn.text(name).starts_with(')')
        && let Some(open) = doc.syn.bracket_match[name].filter(|o| *o < name)
    {
        Cell::new(doc, open, name)
    } else {
        Cell::new(doc, name, name)
    };
    table.set_cell(line, PARAMETER_NAME, cell);
    table.set_cell(
        line,
        TYPE_OR_LIKE,
        Cell::new(doc, type_or_like, type_or_like),
    );
}

fn add_optional_or_default(doc: &Doc, table: &mut Table, token: usize, cmd_first: usize) {
    let line = table.lines.len() - 1;
    if let Some(old) = table.lines[line].cells[TYPE_OR_LIKE].clone()
        && let Some(prev) = (cmd_first..token).rev().find(|j| !is_comment(doc, *j))
    {
        table.set_cell(line, TYPE_OR_LIKE, Cell::new(doc, old.first, prev));
    }
    table.set_cell(line, DEFAULT_OR_OPTIONAL, Cell::new(doc, token, token));
}

fn finish_table_if_full(
    doc: &mut Doc,
    opts: &Opts,
    cmds: &[Command],
    token: usize,
    mut table: Table,
    multi: bool,
) -> Table {
    if multi {
        let name_line = table.line_index_of_last_non_empty_cell(METHOD_NAME);
        if is_suited_for_multi(doc, opts, &table, name_line, token) {
            return table;
        }
        // remove the last method declaration, as it is not suited for multi-alignment
        let keep = name_line.unwrap_or(0);
        table.lines.truncate(keep);
    }
    if !table.lines.is_empty() {
        let ci = cmd_index_of(cmds, token);
        exec_table(doc, opts, cmds, ci, &mut table, multi);
    }
    Table::new(MAX_COLUMN_COUNT)
}

fn is_suited_for_multi(
    doc: &Doc,
    opts: &Opts,
    table: &Table,
    name_line: Option<usize>,
    end_token: usize,
) -> bool {
    let Some(name_line) = name_line else {
        return false;
    };
    match opts.align_consecutive.as_str() {
        "ONE_LINERS" => {
            let mut start = table.lines[name_line].cells[METHOD_NAME]
                .as_ref()
                .unwrap()
                .first;
            if let Some(p) = (0..start).rev().find(|j| !is_comment(doc, *j))
                && (is_kw(doc, p, "METHODS") || is_kw(doc, p, "CLASS-METHODS"))
            {
                start = p;
            }
            // line breaks between start and the end of the declaration (a break after a comma is ignored)
            !(start + 1..end_token).any(|t| {
                doc.lines[t] > 0
                    && (0..t)
                        .rev()
                        .find(|j| !is_comment(doc, *j))
                        .is_none_or(|p| doc.syn.types[p] != TokenType::Comma)
            })
        }
        "ALL_TABULAR" => {
            for line in &table.lines[name_line..] {
                for col in [KEYWORD, METHOD_NAME, ACCESS] {
                    let Some(cell) = &line.cells[col] else {
                        continue;
                    };
                    let next = (cell.last + 1..doc.syn.tokens.len()).find(|j| !is_comment(doc, *j));
                    if let Some(n) = next
                        && doc.lines[n] > 0
                        && doc.syn.types[cell.last] != TokenType::Colon
                    {
                        return false;
                    }
                }
            }
            true
        }
        _ => false,
    }
}

fn exec_table(
    doc: &mut Doc,
    opts: &Opts,
    cmds: &[Command],
    ci: usize,
    table: &mut Table,
    one_liners_or_tabular: bool,
) {
    if table.lines.is_empty() {
        return;
    }
    let mut basic_indent = i64::from(doc.indent_of(ci));
    if table.column_is_empty(KEYWORD) {
        basic_indent += 2;
    }
    let first_token = table.lines[0].first_token().unwrap();
    let first_line_breaks = doc.lines[first_token];
    let mut partial_chains = false;
    if table.lines[0].cells[KEYWORD].is_none() {
        partial_chains = true;
    } else if let Some(lt) = table.lines.last().and_then(|l| l.last_token())
        && (lt..=cmds[cmd_index_of(cmds, lt)].last).any(|t| doc.syn.types[t] == TokenType::Colon)
    {
        partial_chains = true;
    }
    change_line_breaks(doc, opts, table, one_liners_or_tabular && !partial_chains);

    // join a sparsely filled DEFAULT / OPTIONAL column into the previous columns
    if !table.column_is_empty(DEFAULT_OR_OPTIONAL) {
        let n = table.column_count(DEFAULT_OR_OPTIONAL);
        if n <= 1 || (n as f64) < (table.lines.len() as f64 * opts.fill_ratio).floor() {
            table.join_into_previous_columns(doc, DEFAULT_OR_OPTIONAL, true);
        }
    }
    if one_liners_or_tabular {
        // methods without parameters do not contribute to the width of the method name column
        for line in &mut table.lines {
            if line.last_cell_index() == Some(METHOD_NAME) {
                line.cells[METHOD_NAME].as_mut().unwrap().override_width = Some(1);
            }
        }
    }
    let minimum = if one_liners_or_tabular { 2 } else { 1 };
    if table.lines.len() >= minimum {
        table.align(doc, basic_indent, first_line_breaks, false, true);
    }
    if opts.separate_with_empty_line {
        separate_with_empty_line(doc, cmds, table);
    }
}

fn change_line_breaks(doc: &mut Doc, opts: &Opts, table: &mut Table, always_continue: bool) {
    let potential_one_liner = table.lines.len() == 1 && !table.lines[0].contains_comments(doc);
    let current_one_liner = potential_one_liner && !table.lines[0].contains_inner_line_breaks(doc);
    for col in KEYWORD..=ACCESS {
        if table.column_is_empty(col) {
            continue;
        }
        let continue_line = if always_continue {
            if col == KEYWORD {
                ChangeType::KeepAsIs
            } else {
                ChangeType::Always
            }
        } else if potential_one_liner && opts.one_liners == "CREATE" {
            ChangeType::Always
        } else if current_one_liner && opts.one_liners == "KEEP_EXISTING" {
            ChangeType::KeepAsIs
        } else if col == KEYWORD {
            opts.continue_after_keyword
        } else if col == METHOD_NAME {
            opts.continue_after_method_name
        } else {
            opts.continue_after_access
        };
        let force = change_line_breaks_after_column(doc, table, col, continue_line);
        if force {
            table.set_force_line_break_after(col, true, None);
            if col == METHOD_NAME {
                if table.column_force_line_break_after(KEYWORD) {
                    table.set_force_indent(ACCESS, Some(METHOD_NAME), 2);
                } else {
                    table.set_force_indent(ACCESS, Some(KEYWORD), 2);
                }
            }
        }
        if continue_line != ChangeType::Always {
            table.override_width_if_followed_by_line_breaks(doc, col);
        }
    }
}

fn change_line_breaks_after_column(
    doc: &mut Doc,
    table: &Table,
    col: usize,
    continue_line: ChangeType,
) -> bool {
    let mut force = continue_line != ChangeType::Always;
    for line in &table.lines {
        if line.cells[col].is_none() {
            continue;
        }
        let Some(next) = line.cells[col + 1..].iter().flatten().next() else {
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
    force
}

/// `Token.getStartOfAttachedComments`
fn start_of_attached_comments(doc: &Doc, mut t: usize) -> usize {
    while doc.lines[t] == 1 && t > 0 && is_comment_line(doc, t - 1) {
        t -= 1;
    }
    t
}

fn separate_with_empty_line(doc: &mut Doc, cmds: &[Command], table: &Table) {
    for line in &table.lines {
        let Some(name_cell) = &line.cells[METHOD_NAME] else {
            continue;
        };
        let name = name_cell.first;
        let ci = cmd_index_of(cmds, name);
        let cmd = &cmds[ci];
        let end_token = find_seq_last(doc, name, cmd.last, &[",|."]);
        // one-liners, FOR TESTING and REDEFINITION are not separated
        let end = end_token.unwrap_or(cmd.last);
        let multi_line = (name + 1..end).any(|t| doc.lines[t] > 0);
        if !multi_line {
            continue;
        }
        let mut skip = false;
        let mut t = Some(name);
        while let Some(x) = t.filter(|x| *x != end) {
            if matches_phrase(doc, x, cmd.last, "FOR TESTING|REDEFINITION") {
                skip = true;
                break;
            }
            t = next_non_comment_sibling(doc, x, cmd.last);
        }
        if skip {
            continue;
        }

        // an empty line above the declaration (and above any comments attached to it)
        let line_first = line.first_token().unwrap();
        let change = if line_first == cmd.first {
            let s = start_of_attached_comments(doc, cmd.first);
            let sc = cmd_index_of(cmds, s);
            // not for the first command inside its block
            let first_in_block = sc == 0 || {
                let prev = &cmds[sc - 1];
                !prev.is_comment_only && prev.level < cmds[ci].level
            };
            (!first_in_block).then_some(s)
        } else {
            Some(start_of_attached_comments(doc, line_first))
        };
        if let Some(c) = change
            && doc.lines[c] <= 1
        {
            doc.lines[c] = 2;
        }
        // and below it
        let change = if let Some(e) = end_token.filter(|e| doc.syn.types[*e] == TokenType::Comma) {
            next_non_comment_sibling(doc, e, cmd.last).map(|n| start_of_attached_comments(doc, n))
        } else if ci + 1 < cmds.len() && cmds[ci + 1].level >= cmd.level {
            Some(cmds[ci + 1].first)
        } else {
            None
        };
        if let Some(c) = change
            && doc.lines[c] <= 1
        {
            doc.lines[c] = 2;
        }
    }
}
