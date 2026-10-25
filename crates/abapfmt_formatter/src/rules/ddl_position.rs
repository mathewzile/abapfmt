//! DDL_POSITION_* rules (port of `RuleForDdlPosition` and the DdlPosition*Rule classes).
use super::ddl_util::*;
use crate::{ConfigExt, Doc, RuleConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lb {
    Always,
    KeepAsIs,
    Never,
}

fn line_break(config: &RuleConfig, name: &str, default: &str) -> Lb {
    match config.choice(name, default).as_str() {
        "ALWAYS" => Lb::Always,
        "NEVER" => Lb::Never,
        _ => Lb::KeepAsIs,
    }
}

/// `DdlLineBreakWithoutNever` mapped to `DdlLineBreak`.
fn line_break_without_never(config: &RuleConfig, name: &str) -> Lb {
    match config.choice(name, "ALWAYS").as_str() {
        "KEEP_AS_IS" => Lb::KeepAsIs,
        _ => Lb::Always,
    }
}

// ---- whitespace primitives (Token / Command methods of abap-cleaner) ----

fn set_ws(doc: &mut Doc, t: usize, lines: u32, spaces: u32) {
    doc.set_lines(t, lines);
    doc.set_spaces(t, spaces);
}

fn cmd_of(doc: &Doc, t: usize) -> usize {
    doc.syn.commands.partition_point(|c| c.last < t)
}

/// The token before `t` within its command.
fn prev_in_cmd(doc: &Doc, t: usize) -> Option<usize> {
    (t > doc.syn.commands[cmd_of(doc, t)].first).then(|| t - 1)
}

fn start_index(doc: &Doc, t: usize) -> i64 {
    doc.start_x(0, t)
}

fn is_comment_line_token(doc: &Doc, t: usize) -> bool {
    is_comment(doc, t)
        && (doc.lines[t] > 0 || t == 0)
        && doc.lines.get(t + 1).is_none_or(|l| *l > 0)
}

fn is_commented_out_annotation(doc: &Doc, t: usize) -> bool {
    let x = doc.syn.text(t);
    is_comment_line_token(doc, t)
        && (x.starts_with("//") || x.starts_with("--"))
        && x[2..].trim().starts_with('@')
}

fn starts_multi_line_comment(doc: &Doc, ci: usize) -> bool {
    is_comment_line(doc, ci) && {
        let x = doc.syn.text(doc.syn.commands[ci].first);
        x.starts_with("/*") && !x.contains("*/")
    }
}

fn ends_multi_line_comment(doc: &Doc, ci: usize) -> bool {
    is_comment_line(doc, ci) && doc.syn.text(doc.syn.commands[ci].first).contains("*/")
}

fn determine_line_breaks(change: bool, cur: u32, min: u32, max: u32) -> u32 {
    if change { cur.min(max).max(min) } else { cur }
}

/// `Token.setWhitespaceInclAttachedComments`.
fn set_ws_incl_attached(
    doc: &mut Doc,
    t: usize,
    mut min: u32,
    mut max: u32,
    new_spaces: u32,
    include_anno: bool,
) {
    let ci = cmd_of(doc, t);
    let first_of_cmd = doc.syn.commands[ci].first;
    let prev = prev_in_cmd(doc, t);
    if prev.is_none() && doc.lines[t] == 1 && ci > 0 && is_comment_line(doc, ci - 1) {
        // attached comment commands above
        let mut comment: Option<usize> = None;
        let mut test = ci;
        while doc.lines[doc.syn.commands[test].first] == 1 {
            if test == 0 {
                break;
            }
            let p = test - 1;
            if !is_comment_line(doc, p)
                || (!include_anno && is_commented_out_annotation(doc, doc.syn.commands[p].first))
            {
                break;
            }
            comment = Some(p);
            test = p;
        }
        let mut is_first = true;
        if let Some(mut c) = comment {
            if c == 0 {
                min = doc.lines[doc.syn.commands[c].first];
                max = min;
            }
            loop {
                let ctok = doc.syn.commands[c].first;
                let add = i64::from(new_spaces) - i64::from(doc.spaces[ctok]);
                let nl = determine_line_breaks(is_first, doc.lines[ctok], min, max);
                set_ws(doc, ctok, nl, new_spaces);
                if starts_multi_line_comment(doc, c) {
                    while !ends_multi_line_comment(doc, c) {
                        c += 1;
                        if c >= doc.syn.commands.len() || !is_comment_line(doc, c) {
                            break;
                        }
                        let tk = doc.syn.commands[c].first;
                        let sp = (i64::from(doc.spaces[tk]) + add).max(0) as u32;
                        set_ws(doc, tk, doc.lines[tk], sp);
                    }
                }
                c += 1;
                is_first = false;
                if c >= doc.syn.commands.len() || !is_comment_line(doc, c) {
                    break;
                }
            }
        }
        let nl = determine_line_breaks(is_first, doc.lines[t], min, max);
        set_ws(doc, t, nl, new_spaces);
    } else if let Some(p) = prev.filter(|p| doc.lines[t] == 1 && is_comment_line_token(doc, *p)) {
        let mut comment: Option<usize> = None;
        let mut test = t;
        while doc.lines[test] == 1 {
            let Some(p) = prev_in_cmd(doc, test) else {
                break;
            };
            if !is_comment_line_token(doc, p)
                || (!include_anno && is_commented_out_annotation(doc, p))
            {
                break;
            }
            comment = Some(p);
            test = p;
        }
        let _ = p;
        let mut is_first = true;
        if let Some(mut c) = comment {
            let last_of_cmd = doc.syn.commands[ci].last;
            loop {
                let add = i64::from(new_spaces) - i64::from(doc.spaces[c]);
                let nl = determine_line_breaks(is_first, doc.lines[c], min, max);
                set_ws(doc, c, nl, new_spaces);
                let text = doc.syn.text(c).to_string();
                if text.starts_with("/*") && !text.contains("*/") {
                    while !doc.syn.text(c).contains("*/") {
                        c += 1;
                        if c > last_of_cmd || !is_comment_line_token(doc, c) {
                            break;
                        }
                        let sp = (i64::from(doc.spaces[c]) + add).max(0) as u32;
                        set_ws(doc, c, doc.lines[c], sp);
                    }
                }
                c += 1;
                is_first = false;
                if c > last_of_cmd || !is_comment(doc, c) {
                    break;
                }
            }
        }
        let nl = determine_line_breaks(is_first, doc.lines[t], min, max);
        set_ws(doc, t, nl, new_spaces);
    } else {
        let nl = determine_line_breaks(true, doc.lines[t], min, max);
        set_ws(doc, t, nl, new_spaces);
    }
    let _ = first_of_cmd;
}

/// `RuleForDdlPosition.breakBefore`.
fn break_before(doc: &mut Doc, t: usize, lb: Lb, empty_line_if_breaking: bool, indent: i64) {
    match lb {
        Lb::Always => {}
        Lb::KeepAsIs => {
            if doc.lines[t] == 0 {
                return;
            }
        }
        Lb::Never => {
            let ci = cmd_of(doc, t);
            let prev =
                prev_in_cmd(doc, t).or_else(|| (ci > 0).then(|| doc.syn.commands[ci - 1].last));
            match prev {
                None => return,
                Some(p) if !is_comment(doc, p) => {
                    set_ws(doc, t, 0, 1);
                    return;
                }
                Some(_) => {}
            }
        }
    }
    let ci = cmd_of(doc, t);
    let mut min = if empty_line_if_breaking { 2 } else { 1 };
    if prev_in_cmd(doc, t).is_none() && ci == 0 {
        min = 0;
    }
    set_ws_incl_attached(doc, t, min, 2, indent.max(0) as u32, false);
}

/// `Token.condenseUpTo(last, MAX_LINE_LENGTH, indent, true)` with the indent of `start`'s line.
fn condense(doc: &mut Doc, start: usize, last: usize) {
    let mut line_start = start;
    while line_start > 0 && doc.lines[line_start] == 0 {
        line_start -= 1;
    }
    let indent = doc.spaces[line_start];
    condense_up_to(doc, start, last, 999, indent);
}

fn condense_up_to(doc: &mut Doc, start: usize, last: usize, max_len: i64, indent: u32) {
    if start == last {
        return;
    }
    let cmd_last = doc.syn.commands[cmd_of(doc, start)].last;
    let mut index_in_line = start_index(doc, start) + doc.token_text(start).chars().count() as i64;
    let mut last_movable: Option<usize> = None;
    let mut t = start + 1;
    while t <= cmd_last {
        let comma = doc.syn.types[t] == abapfmt_syntax::TokenType::Comma;
        let colon = doc.syn.types[t] == abapfmt_syntax::TokenType::Colon;
        let attached = doc.lines[t] == 0 && doc.spaces[t] == 0;
        let spaces_left = if attached || comma || colon { 0 } else { 1 };
        if spaces_left > 0 {
            last_movable = Some(t);
        }
        let text = doc.syn.text(t);
        let is_line_end_comment =
            is_comment(doc, t) && (text.starts_with("//") || text.starts_with("--"));
        let len = doc.token_text(t).chars().count() as i64;
        if is_line_end_comment {
            // keep comment lines
        } else if is_comment(doc, t - 1) {
            set_ws(doc, t, 1, indent);
            last_movable = None;
            index_in_line = start_index(doc, t) + len;
        } else if last_movable.is_some() && index_in_line + spaces_left + len > max_len {
            let lm = last_movable.unwrap();
            set_ws(doc, lm, 1, indent);
            if t != lm {
                set_ws(doc, t, 0, spaces_left as u32);
            }
            last_movable = None;
            index_in_line = start_index(doc, t) + len;
        } else {
            set_ws(doc, t, 0, spaces_left as u32);
            index_in_line += spaces_left + len;
        }
        if t == last {
            break;
        }
        t += 1;
    }
}

/// `RuleForDdlPosition.setNewIndent`: returns the last command that was moved.
fn set_new_indent(doc: &mut Doc, ci: usize, indent: i64) -> usize {
    let first = doc.syn.commands[ci].first;
    let mut add = indent - i64::from(doc.spaces[first]);
    if doc.lines[first] == 0 {
        let start = start_index(doc, first);
        add = indent - start;
        set_ws(doc, first, 1, start.max(0) as u32);
    }
    let multi = starts_multi_line_comment(doc, ci);
    let mut c = ci;
    loop {
        let (f, l) = (doc.syn.commands[c].first, doc.syn.commands[c].last);
        doc.add_indent_range(f, l + 1, add, 0, false);
        if !multi || ends_multi_line_comment(doc, c) {
            break;
        }
        c += 1;
        if c >= doc.syn.commands.len() {
            c -= 1;
            break;
        }
    }
    c
}

fn run_on_commands(doc: &mut Doc, mut f: impl FnMut(&mut Doc, usize)) {
    let n = doc.syn.commands.len();
    for ci in 0..n {
        if !is_comment_line(doc, ci) {
            f(doc, ci);
        }
    }
}

// ---- DDL_POSITION_BRACES ----

pub fn braces(doc: &mut Doc, config: &RuleConfig) {
    let open_lb = line_break(config, "BreakBeforeOpeningBrace", "ALWAYS");
    let open_indent = config.int("OpeningBraceIndent", 0);
    let close_lb = line_break(config, "BreakBeforeClosingBrace", "ALWAYS");
    let close_indent = config.int("ClosingBraceIndent", 0);
    let from_lb = line_break_without_never(config, "BreakBeforeFrom");
    let from_indent = config.int("FromIndent", 2);
    run_on_commands(doc, |doc, ci| {
        if cmd_parent(doc, ci).is_some() {
            return;
        }
        if opens_level(doc, ci)
            && let Some(b) = last_code(doc, ci).filter(|t| doc.syn.text(*t) == "{")
        {
            break_before(doc, b, open_lb, false, open_indent);
            return;
        }
        if closes_level(doc, ci)
            && let Some(b) = first_code(doc, ci)
        {
            if doc.syn.text(b) == "}" {
                break_before(doc, b, close_lb, false, close_indent);
            } else if is_kw(doc, b, "FROM") {
                break_before(doc, b, from_lb, false, from_indent);
            }
        }
    });
}

// ---- DDL_POSITION_CLAUSES ----

pub fn clauses(doc: &mut Doc, config: &RuleConfig) {
    let where_lb = line_break_without_never(config, "BreakBeforeWhereEtc");
    let where_indent = config.int("WhereEtcIndent", 2);
    let union_lb = line_break_without_never(config, "BreakBeforeUnionEtc");
    let union_indent = config.int("UnionEtcIndent", 0);
    run_on_commands(doc, |doc, ci| {
        if cmd_parent(doc, ci).is_some() {
            return;
        }
        let Some(token) = first_code(doc, ci) else {
            return;
        };
        let last = doc.syn.commands[ci].last;
        if is_any_kw(doc, token, &["WHERE", "HAVING"]) {
            clause(doc, ci, token, token, None, where_lb, where_indent, false);
        } else if matches_on_siblings(doc, token, last, &[P::T("GROUP"), P::T("BY")]) {
            let by = next_code_sibling(doc, token, last).unwrap_or(token);
            clause(doc, ci, token, by, None, where_lb, where_indent, false);
        } else if is_any_kw(doc, token, &["UNION", "INTERSECT", "EXCEPT"]) {
            let last_kw = if matches_on_siblings(doc, token, last, &[P::T("UNION"), P::T("ALL")]) {
                next_code_sibling(doc, token, last).unwrap_or(token)
            } else {
                token
            };
            let end = find_sibling(doc, token, last, "SELECT");
            let after_select_list = prev_code_sibling(doc, doc.syn.commands[ci].first, token)
                .is_none()
                && prev_non_comment(doc, ci)
                    .and_then(|p| first_code(doc, p))
                    .is_some_and(|t| doc.syn.text(t) == "}");
            clause(
                doc,
                ci,
                token,
                last_kw,
                end,
                union_lb,
                union_indent,
                !after_select_list,
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn clause(
    doc: &mut Doc,
    ci: usize,
    start: usize,
    last_keyword: usize,
    end: Option<usize>,
    lb: Lb,
    indent: i64,
    enforce_empty_line: bool,
) {
    let old = start_index(doc, last_keyword);
    break_before(doc, start, lb, enforce_empty_line, indent);
    if last_keyword != start {
        condense(doc, start, last_keyword);
    }
    let add = start_index(doc, last_keyword) - old;
    if add != 0 {
        let cmd_last = doc.syn.commands[ci].last;
        doc.add_indent_range(last_keyword + 1, end.unwrap_or(cmd_last + 1), add, 0, false);
    }
}

// ---- DDL_POSITION_DEFINE ----

pub fn define(doc: &mut Doc, config: &RuleConfig) {
    let define_lb = line_break_without_never(config, "BreakBeforeDefine");
    let define_indent = config.int("DefineIndent", 1);
    let name_lb = line_break(config, "BreakBeforeEntityName", "NEVER");
    let name_indent = config.int("EntityNameIndent", 2);
    let params_lb = line_break(config, "BreakBeforeWithParams", "ALWAYS");
    let params_indent = config.int("WithParamsIndent", 2);
    let param_indent = config.int("ParamsIndent", 4);
    run_on_commands(doc, |doc, ci| {
        if is_annotation(doc, ci) {
            return;
        }
        let Some(first) = first_code(doc, ci) else {
            return;
        };
        let last = doc.syn.commands[ci].last;
        if let Some(name) = entity_name_token(doc, ci) {
            let empty_line = ci > 0;
            break_before(doc, first, define_lb, empty_line, define_indent);
            if let Some(before_name) = (doc.syn.commands[ci].first..name)
                .rev()
                .find(|t| !is_comment(doc, *t))
            {
                condense(doc, first, before_name);
            }
            break_before(doc, name, name_lb, false, name_indent);
        }
        if let Some(params) = find_sibling_seq(doc, first, last, &["WITH", "PARAMETERS"]) {
            let with = prev_code_sibling(doc, doc.syn.commands[ci].first, params).unwrap_or(params);
            break_before(doc, with, params_lb, false, params_indent);
            condense(doc, with, params);
            let mut pc = first_child(doc, ci);
            while let Some(p) = pc {
                // comments at the end of the parameter list probably belong to the following command
                if is_comment_line(doc, p) && next_non_comment_sibling(doc, p).is_none() {
                    break;
                }
                let moved = set_new_indent(doc, p, param_indent);
                pc = next_sibling(doc, moved);
            }
        }
    });
}

// ---- DDL_POSITION_SELECT ----

pub fn select(doc: &mut Doc, config: &RuleConfig) {
    let as_select_lb = line_break(config, "BreakBeforeAsSelectFrom", "ALWAYS");
    let as_select_indent = config.int("AsSelectFromIndent", 2);
    let select_lb = line_break(config, "BreakBeforeSelectFrom", "ALWAYS");
    let select_indent = config.int("SelectFromIndent", 2);
    let proj_lb = line_break(config, "BreakBeforeAsProjectionOn", "ALWAYS");
    let proj_indent = config.int("AsProjectionOnIndent", 2);
    let source_lb = line_break(config, "BreakBeforeDataSource", "NEVER");
    let source_indent = config.int("DataSourceIndent", 4);
    run_on_commands(doc, |doc, ci| {
        let last = doc.syn.commands[ci].last;
        let first_code_tok = first_code(doc, ci);
        let mut token = first_code_tok;
        while let Some(t) = token {
            if as_select_lb != Lb::KeepAsIs
                && let Some(from) = last_of_sequence(
                    doc,
                    t,
                    last,
                    &[P::T("AS"), P::T("SELECT"), P::Opt("DISTINCT"), P::T("FROM")],
                )
            {
                let empty_line = closes_level(doc, ci) && Some(t) == first_code_tok;
                break_before(doc, t, as_select_lb, empty_line, as_select_indent);
                condense(doc, t, from);
                break_before_source(
                    doc,
                    next_code_sibling(doc, from, last),
                    source_lb,
                    source_indent,
                );
                break;
            }
            if proj_lb != Lb::KeepAsIs
                && let Some(on) =
                    last_of_sequence(doc, t, last, &[P::T("AS"), P::T("PROJECTION"), P::T("ON")])
            {
                let empty_line = closes_level(doc, ci) && Some(t) == first_code_tok;
                break_before(doc, t, proj_lb, empty_line, proj_indent);
                condense(doc, t, on);
                break_before_source(
                    doc,
                    next_code_sibling(doc, on, last),
                    source_lb,
                    source_indent,
                );
                break;
            }
            if select_lb != Lb::KeepAsIs
                && let Some(sel) = last_of_sequence(
                    doc,
                    t,
                    last,
                    &[
                        P::T("EXCEPT|INTERSECT|UNION"),
                        P::Opt("ALL"),
                        P::T("SELECT"),
                    ],
                )
            {
                if let Some(before) = prev_code_sibling(doc, doc.syn.commands[ci].first, sel) {
                    condense(doc, t, before);
                }
                break_before(doc, sel, select_lb, false, select_indent);
                if let Some(from2) = last_of_sequence(
                    doc,
                    sel,
                    last,
                    &[P::T("SELECT"), P::Opt("DISTINCT"), P::T("FROM")],
                ) {
                    condense(doc, sel, from2);
                    break_before_source(
                        doc,
                        next_code_sibling(doc, from2, last),
                        source_lb,
                        source_indent,
                    );
                }
                break;
            }
            token = next_code_sibling(doc, t, last);
        }
    });
}

fn break_before_source(doc: &mut Doc, source: Option<usize>, lb: Lb, indent: i64) {
    if lb != Lb::KeepAsIs
        && let Some(s) = source
    {
        break_before(doc, s, lb, false, indent);
    }
}

// ---- DDL_POSITION_JOIN / DDL_POSITION_ASSOCIATION ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum CondLb {
    Always,
    IfMultiLineFound,
    KeepAsIs,
    Never,
}

fn join_or_association_data_source(doc: &Doc, ci: usize) -> Option<usize> {
    let last = doc.syn.commands[ci].last;
    let mut t = first_code(doc, ci);
    while let Some(x) = t {
        if doc.syn.types[x] == abapfmt_syntax::TokenType::Keyword
            || matches!(doc.syn.text(x), "[" | "]")
        {
            t = next_code_sibling(doc, x, last);
        } else {
            break;
        }
    }
    t
}

fn contains_line_breaks_between(doc: &Doc, start: usize, end: usize) -> bool {
    (start + 1..end).any(|t| doc.lines[t] > 0)
}

pub fn join(doc: &mut Doc, config: &RuleConfig) {
    join_or_association(doc, config, true);
}

pub fn association(doc: &mut Doc, config: &RuleConfig) {
    join_or_association(doc, config, false);
}

fn join_or_association(doc: &mut Doc, config: &RuleConfig, is_join: bool) {
    let keywords_lb = line_break_without_never(config, "BreakBeforeKeywords");
    let keywords_indent = config.int("KeywordsIndent", 2);
    let source_lb = line_break(config, "BreakBeforeDataSource", "NEVER");
    let source_indent = config.int("DataSourceIndent", 2);
    let cond_lb = match config
        .choice("BreakBeforeCondition", "IF_MULTI_LINE_FOUND")
        .as_str()
    {
        "ALWAYS" => CondLb::Always,
        "KEEP_AS_IS" => CondLb::KeepAsIs,
        "NEVER" => CondLb::Never,
        _ => CondLb::IfMultiLineFound,
    };
    let cond_indent = config.int("ConditionIndent", 4);
    let filter_lb = if is_join {
        Lb::KeepAsIs
    } else {
        line_break_without_never(config, "BreakBeforeFilter")
    };
    let filter_indent = if is_join {
        -1
    } else {
        config.int("FilterIndent", 4)
    };

    // does the view contain a multi-line JOIN / ASSOCIATION condition, or data sources with parameters?
    let n = doc.syn.commands.len();
    let (mut multi_join, mut multi_assoc) = (false, false);
    for ci in 0..n {
        if cmd_parent(doc, ci).is_some() || is_comment_line(doc, ci) {
            continue;
        }
        let (j, a) = (starts_join(doc, ci), starts_association(doc, ci));
        if !(j || a) {
            continue;
        }
        let last = doc.syn.commands[ci].last;
        let Some(source) = join_or_association_data_source(doc, ci) else {
            continue;
        };
        let Some(on) = find_sibling(doc, source, last, "ON") else {
            continue;
        };
        let has_params =
            next_code_sibling(doc, source, last).is_some_and(|x| doc.syn.text(x) == "(");
        let end = last_token_of_ddl_logical_expression(doc, ci, on);
        let multi = has_params || end.is_some_and(|e| contains_line_breaks_between(doc, on, e));
        if multi {
            multi_join |= j;
            multi_assoc |= a;
        }
    }

    let (mut join_count, mut assoc_count) = (0, 0);
    for ci in 0..n {
        if cmd_parent(doc, ci).is_some() || is_comment_line(doc, ci) {
            continue;
        }
        let Some(first) = first_code(doc, ci) else {
            continue;
        };
        if doc.syn.text(first) == "{" {
            join_count = 0;
            assoc_count = 0;
            continue;
        }
        let applies = if is_join {
            starts_join(doc, ci)
        } else {
            starts_association(doc, ci)
        };
        if !applies {
            continue;
        }
        let is_first = if is_join {
            join_count == 0
        } else {
            assoc_count == 0
        };
        if is_join {
            join_count += 1;
        } else {
            assoc_count += 1;
        }
        let cond = match cond_lb {
            CondLb::Always => Lb::Always,
            CondLb::KeepAsIs => Lb::KeepAsIs,
            CondLb::Never => Lb::Never,
            CondLb::IfMultiLineFound => {
                if if is_join { multi_join } else { multi_assoc } {
                    Lb::Always
                } else {
                    Lb::Never
                }
            }
        };
        position_join_or_association(
            doc,
            ci,
            first,
            is_join,
            is_first,
            (keywords_lb, keywords_indent),
            (source_lb, source_indent),
            (cond, cond_indent),
            (filter_lb, filter_indent),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn position_join_or_association(
    doc: &mut Doc,
    ci: usize,
    first_code_tok: usize,
    is_join: bool,
    is_first: bool,
    keywords: (Lb, i64),
    source: (Lb, i64),
    cond: (Lb, i64),
    filter: (Lb, i64),
) {
    let Some(source_tok) = join_or_association_data_source(doc, ci) else {
        return;
    };
    let (cmd_first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
    let Some(last_keyword) = prev_code_sibling(doc, cmd_first, source_tok) else {
        return;
    };
    let on = find_sibling(doc, source_tok, last, "ON");
    let old_on = on.map_or(0, |o| start_index(doc, o));

    let (mut filter_first, mut filter_last, mut old_filter) = (None, None, 0);
    if !is_join && let Some(o) = on {
        filter_last = find_sibling_seq(doc, o, last, &["WITH", "DEFAULT", "FILTER"]);
        if let Some(fl) = filter_last {
            old_filter = start_index(doc, fl);
            filter_first = prev_code_sibling(doc, cmd_first, fl)
                .and_then(|p| prev_code_sibling(doc, cmd_first, p));
        }
    }

    break_before(doc, first_code_tok, keywords.0, is_first, keywords.1);
    condense(doc, first_code_tok, last_keyword);
    break_before(doc, source_tok, source.0, false, source.1);

    if let Some(o) = on {
        break_before(doc, o, cond.0, false, cond.1);
        let add = start_index(doc, o) - old_on;
        doc.add_indent_range(o + 1, filter_first.unwrap_or(last + 1), add, 0, false);
    }
    if let (Some(ff), Some(fl)) = (filter_first, filter_last) {
        break_before(doc, ff, filter.0, false, filter.1);
        condense(doc, ff, fl);
        let add = start_index(doc, fl) - old_filter;
        doc.add_indent_range(fl + 1, last + 1, add, 0, false);
    }
}
