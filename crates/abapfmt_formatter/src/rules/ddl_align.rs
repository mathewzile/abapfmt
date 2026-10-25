//! DDL_ALIGN_* rules (port of `RuleForDdlAlignParameters` and the DdlAlign*Rule classes).
//!
//! Upstream reads a few settings from other rules (`SpaceBeforeColon`/`SpaceAfterColon` of
//! DDL_SPACES_AROUND_SIGNS, `SpacesInsideFuncParens` of DDL_SPACES_AROUND_BRACKETS and
//! `ParamsIndent` of DDL_POSITION_DEFINE); here their defaults are used.
use std::collections::HashSet;

use abapfmt_syntax::TokenType;

use super::ddl_util::*;
use crate::align::{Cell, Table, end_x};
use crate::{ConfigExt, Doc, RuleConfig};

const PARAMETER: usize = 0;
const ASSIGNMENT_OP: usize = 1;
const EXPRESSION: usize = 2;
const COLUMN_COUNT: usize = 3;

/// Settings that upstream reads from other rules; the pipeline passes them in the rule's config
/// under their upstream names (`ParamsIndent` of DDL_POSITION_DEFINE, `SpaceBeforeColon` and
/// `SpaceAfterColon` of DDL_SPACES_AROUND_SIGNS, `SpacesInsideFuncParens` of
/// DDL_SPACES_AROUND_BRACKETS).
#[derive(Clone, Copy)]
struct Shared {
    params_indent: i64,
    space_before_colon: Change,
    space_after_colon: Change,
    space_inside_parens: Change,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    KeepAsIs,
    Always,
    Never,
}

fn change(config: &RuleConfig, name: &str, default: &str) -> Change {
    match config.choice(name, default).as_str() {
        "ALWAYS" => Change::Always,
        "NEVER" => Change::Never,
        _ => Change::KeepAsIs,
    }
}

fn shared(config: &RuleConfig) -> Shared {
    Shared {
        params_indent: config.int("ParamsIndent", 4),
        space_before_colon: change(config, "SpaceBeforeColon", "KEEP_AS_IS"),
        space_after_colon: change(config, "SpaceAfterColon", "ALWAYS"),
        space_inside_parens: change(config, "SpacesInsideFuncParens", "NEVER"),
    }
}

fn set_ws(doc: &mut Doc, t: usize, lines: u32, spaces: u32) {
    doc.set_lines(t, lines);
    doc.set_spaces(t, spaces);
}

fn cmd_of(doc: &Doc, t: usize) -> usize {
    doc.syn.commands.partition_point(|c| c.last < t)
}

fn first_token_in_line_spaces(doc: &Doc, t: usize) -> i64 {
    let mut k = t;
    while k > 0 && doc.lines[k] == 0 {
        k -= 1;
    }
    i64::from(doc.spaces[k])
}

fn start_index(doc: &Doc, t: usize) -> i64 {
    doc.start_x(0, t)
}

fn assignment_spaces_before(sh: Shared, op: &str) -> i64 {
    if op == ":" {
        i64::from(sh.space_before_colon == Change::Always)
    } else {
        1
    }
}

fn assignment_spaces_after(sh: Shared, op: &str) -> i64 {
    if op == ":" {
        i64::from(sh.space_after_colon != Change::Never)
    } else {
        1
    }
}

fn is_attached(doc: &Doc, t: usize) -> bool {
    doc.lines[t] == 0 && doc.spaces[t] == 0
}

/// Is a space wanted inside the parentheses next to token `t`?
fn space_inside(doc: &Doc, sh: Shared, t: usize) -> bool {
    sh.space_inside_parens == Change::Always
        || (sh.space_inside_parens == Change::KeepAsIs && !is_attached(doc, t))
}

/// `RuleForDdlAlignParameters.joinColumns`.
fn join_columns(
    doc: &mut Doc,
    table: &mut Table,
    sh: Shared,
    align_ops: bool,
    align_exprs: bool,
    op: &str,
) {
    if !align_ops {
        table.join_into_previous_columns_with(
            doc,
            ASSIGNMENT_OP,
            true,
            assignment_spaces_before(sh, op),
            true,
        );
    }
    if !align_exprs {
        table.join_into_previous_columns_with(
            doc,
            EXPRESSION,
            true,
            assignment_spaces_after(sh, op),
            true,
        );
    }
}

/// `moveOpeningParenthesis`: move "(" behind the view / function name.
fn move_opening_parenthesis(doc: &mut Doc, open: usize) {
    if doc.lines[open] == 0 {
        return;
    }
    let prev = open - 1;
    let first_child = (open + 1 < doc.syn.tokens.len() && doc.syn.parent[open + 1] == Some(open))
        .then_some(open + 1);
    if is_comment(doc, prev) {
        // a comment in the way is not moved
        return;
    }
    if let Some(fc) = first_child
        && doc.lines[fc] == 0
    {
        let (l, s) = (doc.lines[open], doc.spaces[open]);
        set_ws(doc, fc, l, s);
    }
    set_ws(doc, open, 0, 0);
}

fn move_closing_parenthesis(doc: &mut Doc, sh: Shared, close: usize, fallback_indent: i64) {
    if is_comment(doc, close - 1) {
        let l = doc.lines[close];
        set_ws(doc, close, l, fallback_indent.max(0) as u32);
    } else {
        let spaces = u32::from(space_inside(doc, sh, close));
        set_ws(doc, close, 0, spaces);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NextPos {
    Continue,
    LineStart,
    LineStartPlus2,
    SourceName,
    SourceNamePlus2,
    KeepAsIs,
}

fn next_pos(config: &RuleConfig, name: &str) -> NextPos {
    match config.choice(name, "KEEP_AS_IS").as_str() {
        "CONTINUE" => NextPos::Continue,
        "LINE_START" => NextPos::LineStart,
        "LINE_START_PLUS_2" => NextPos::LineStartPlus2,
        "SOURCE_NAME" => NextPos::SourceName,
        "SOURCE_NAME_PLUS_2" => NextPos::SourceNamePlus2,
        _ => NextPos::KeepAsIs,
    }
}

fn move_token_after_closing_parens(doc: &mut Doc, open: usize, pos: NextPos) {
    let ci = cmd_of(doc, open);
    let (first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
    let Some(name) = prev_code_sibling(doc, first, open) else {
        return;
    };
    // the closing parenthesis is the next sibling (upstream also calls this for the "=>" token)
    let Some(close) = next_sibling_any(doc, open, last) else {
        return;
    };
    if close + 1 > last {
        return;
    }
    let next = close + 1;
    if is_kw(doc, next, "END") {
        return;
    }
    let mut lines = 1;
    let spaces = match pos {
        NextPos::Continue => {
            lines = 0;
            1
        }
        NextPos::LineStart => first_token_in_line_spaces(doc, open),
        NextPos::LineStartPlus2 => first_token_in_line_spaces(doc, open) + 2,
        NextPos::SourceName => start_index(doc, name),
        NextPos::SourceNamePlus2 => start_index(doc, name) + 2,
        NextPos::KeepAsIs => {
            lines = doc.lines[next].min(1);
            i64::from(doc.spaces[next])
        }
    };
    set_ws(doc, next, lines, spaces.max(0) as u32);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParamPos {
    Continue,
    LineStartPlus2,
    LineStartPlus4,
    NamePlus2,
    NamePlus4,
    KeepAsIs,
}

fn param_pos(config: &RuleConfig, name: &str) -> ParamPos {
    match config.choice(name, "KEEP_AS_IS").as_str() {
        "CONTINUE" => ParamPos::Continue,
        "LINE_START_PLUS_2" => ParamPos::LineStartPlus2,
        "LINE_START_PLUS_4" => ParamPos::LineStartPlus4,
        "SOURCE_NAME_PLUS_2" | "FUNCTION_NAME_PLUS_2" => ParamPos::NamePlus2,
        "SOURCE_NAME_PLUS_4" | "FUNCTION_NAME_PLUS_4" => ParamPos::NamePlus4,
        _ => ParamPos::KeepAsIs,
    }
}

/// `alignParameters`: aligns "param op expr" triples inside the parentheses opened by `open`.
fn align_parameters(
    doc: &mut Doc,
    sh: Shared,
    open: usize,
    op_text: &str,
    pos: ParamPos,
    align_ops: bool,
    align_params: bool,
) {
    let ci = cmd_of(doc, open);
    let (cmd_first, cmd_last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
    let Some(name) = prev_code_sibling(doc, cmd_first, open) else {
        return;
    };
    let Some(close) = doc.syn.bracket_match[open] else {
        return;
    };
    let first_inner = open + 1;
    if first_inner >= close || doc.syn.parent[first_inner] != Some(open) {
        return;
    }

    let mut first_line_breaks = 1;
    let indent;
    match pos {
        ParamPos::Continue => {
            first_line_breaks = 0;
            let mut i = end_x(doc, open);
            if space_inside(doc, sh, first_inner) {
                i += 1;
            }
            indent = i;
        }
        ParamPos::LineStartPlus2 => indent = first_token_in_line_spaces(doc, open) + 2,
        ParamPos::LineStartPlus4 => indent = first_token_in_line_spaces(doc, open) + 4,
        ParamPos::NamePlus2 => indent = start_index(doc, name) + 2,
        ParamPos::NamePlus4 => indent = start_index(doc, name) + 4,
        ParamPos::KeepAsIs => {
            first_line_breaks = doc.lines[first_inner].min(1);
            indent = start_index(doc, first_inner);
        }
    }

    // build the align table
    let mut table = Table::new(COLUMN_COUNT);
    let mut other_line_starts: Vec<usize> = Vec::new();
    let mut token = Some(first_inner);
    while let Some(t) = token {
        if t == close || t + 1 > cmd_last {
            break;
        }
        let next = t + 1;
        if doc.syn.types[t] == TokenType::Identifier
            && doc.syn.text(next) == op_text
            && next < cmd_last
            && !is_comment(doc, next + 1)
        {
            let op = next_code_sibling(doc, t, cmd_last).unwrap_or(next);
            let expr_start = op + 1;
            let expr_last = (expr_start..=cmd_last)
                .find(|j| {
                    doc.syn.parent[*j] == doc.syn.parent[expr_start] && doc.syn.text(*j) == ","
                })
                .or_else(|| (cmd_first..close).rev().find(|j| !is_comment(doc, *j)))
                .unwrap_or(expr_start);
            let line = table.add_line();
            table.set_cell(line, PARAMETER, Cell::new(doc, t, t));
            table.set_cell(line, ASSIGNMENT_OP, Cell::new(doc, op, op));
            table.set_cell(line, EXPRESSION, Cell::new(doc, expr_start, expr_last));
            token = Some(expr_last + 1);
        } else {
            if is_comment(doc, t) && doc.lines[t] > 0 {
                other_line_starts.push(t);
            }
            token = next_sibling_any(doc, t, cmd_last);
        }
    }
    if table.lines.is_empty() {
        return;
    }

    let fallback = indent.min(start_index(doc, open));
    move_closing_parenthesis(doc, sh, close, fallback);
    join_columns(doc, &mut table, sh, align_ops, align_params, op_text);
    table.align(doc, indent, first_line_breaks, true, true);

    // align comments between parameters
    let parameter_indent = table.effective_indent(PARAMETER);
    let op_indent = table.effective_indent(ASSIGNMENT_OP);
    let expr_indent = table.effective_indent(EXPRESSION);
    let may_align_commented =
        parameter_indent > 0 && (!align_ops || op_indent > 0) && (!align_params || expr_indent > 0);
    for start in other_line_starts {
        let is_comment_line_tok =
            doc.lines[start] > 0 && doc.lines.get(start + 1).is_none_or(|l| *l > 0);
        if may_align_commented
            && is_comment_line_tok
            && doc.spaces[start] == 0
            && doc.syn.text(start).starts_with("//")
        {
            align_commented_out_parameter(
                doc,
                sh,
                start,
                op_text,
                align_ops,
                align_params,
                parameter_indent,
                op_indent,
                expr_indent,
            );
        } else {
            let l = doc.lines[start];
            set_ws(doc, start, l, indent.max(0) as u32);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn align_commented_out_parameter(
    doc: &mut Doc,
    sh: Shared,
    comment: usize,
    op_text: &str,
    align_ops: bool,
    align_params: bool,
    parameter_indent: i64,
    op_indent: i64,
    expr_indent: i64,
) {
    let text = doc.syn.text(comment).to_string();
    let Some(op_pos) = text.find(op_text) else {
        return;
    };
    let op_end = op_pos + op_text.len();
    let bits: Vec<&str> = text[..op_pos]
        .split(' ')
        .filter(|s| !s.is_empty())
        .collect();
    if bits.len() != 2 || !is_allowed_parameter_name(bits[1]) {
        return;
    }
    let mut sb = String::new();
    sb.push_str(bits[0]);
    let start_len = sb.len();
    sb.push_str(&" ".repeat(parameter_indent.max(0) as usize));
    sb.push_str(bits[1]);
    let spaces_before_op = if align_ops {
        op_indent - (sb.len() - start_len) as i64
    } else {
        assignment_spaces_before(sh, op_text)
    };
    sb.push_str(&" ".repeat(spaces_before_op.max(0) as usize));
    sb.push_str(op_text);
    if op_end < text.len() {
        let spaces_before_expr = if align_params {
            if align_ops {
                1
            } else {
                expr_indent - (sb.len() - start_len) as i64
            }
        } else {
            assignment_spaces_after(sh, op_text)
        };
        sb.push_str(&" ".repeat(spaces_before_expr.max(0) as usize));
        sb.push_str(text[op_end..].trim_start());
    }
    if sb != text {
        doc.text[comment] = Some(sb);
    }
}

fn is_allowed_parameter_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .enumerate()
            .all(|(k, c)| c.is_ascii_alphabetic() || c == '_' || (k > 0 && c.is_ascii_digit()))
}

// ---- DDL_ALIGN_ENTITY_PARAMETERS ----

pub fn entity_parameters(doc: &mut Doc, config: &RuleConfig) {
    let sh = shared(config);
    let align_colons = config.bool("AlignColons", true);
    let align_types = config.bool("AlignTypes", true);
    let mut table = Table::new(COLUMN_COUNT);
    for ci in 0..doc.syn.commands.len() {
        if is_comment_line(doc, ci) || !is_parameters_element(doc, ci) {
            continue;
        }
        let Some(name) = first_code(doc, ci) else {
            return;
        };
        let last = doc.syn.commands[ci].last;
        let Some(colon) = next_code_sibling(doc, name, last).filter(|c| doc.syn.text(*c) == ":")
        else {
            return;
        };
        let Some(type_tok) = next_code_sibling(doc, colon, last) else {
            return;
        };
        let Some(last_tok) = last_code(doc, ci) else {
            return;
        };
        let line = table.add_line();
        table.set_cell(line, PARAMETER, Cell::new(doc, name, name));
        table.set_cell(line, ASSIGNMENT_OP, Cell::new(doc, colon, colon));
        table.set_cell(line, EXPRESSION, Cell::new(doc, type_tok, last_tok));
    }
    if table.lines.is_empty() {
        return;
    }
    join_columns(doc, &mut table, sh, align_colons, align_types, ":");
    table.align(doc, sh.params_indent, 1, true, true);
}

// ---- DDL_ALIGN_SOURCE_PARAMETERS ----

pub fn source_parameters(doc: &mut Doc, config: &RuleConfig) {
    let sh = shared(config);
    let pos = param_pos(config, "ParameterPos");
    let align_ops = config.bool("AlignAssignmentOps", true);
    let align_params = config.bool("AlignActualParams", true);
    let as_alias_pos = next_pos(config, "AsAliasPos");
    for ci in 0..doc.syn.commands.len() {
        if cmd_parent(doc, ci).is_some() || is_comment_line(doc, ci) {
            continue;
        }
        let (first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
        let mut token = Some(first);
        while let Some(t) = token {
            // "as select from view( ... )", "union all select from view( ... )", "left outer to many join view( ... )"
            let Some(open) = source_open_paren(doc, t, last) else {
                break;
            };
            move_opening_parenthesis(doc, open);
            align_parameters(doc, sh, open, ":", pos, align_ops, align_params);
            move_token_after_closing_parens(doc, open, as_alias_pos);
            token = next_sibling_any(doc, open, last);
        }
    }
}

/// `ASTERISK, "from|join|on", ANY_IDENTIFIER, "("` starting at `start`.
fn source_open_paren(doc: &Doc, start: usize, last: usize) -> Option<usize> {
    let mut t = Some(start);
    while let Some(x) = t {
        if let Some(src) = next_code_sibling(doc, x, last)
            && ["from", "join", "on"]
                .iter()
                .any(|k| doc.syn.text(x).eq_ignore_ascii_case(k))
            && doc.syn.types[src] == TokenType::Identifier
            && let Some(p) = next_code_sibling(doc, src, last)
            && doc.syn.text(p) == "("
        {
            return Some(p);
        }
        t = next_code_sibling(doc, x, last);
    }
    None
}

// ---- DDL_ALIGN_FUNCTION_PARAMETERS ----

pub fn function_parameters(doc: &mut Doc, config: &RuleConfig) {
    let sh = shared(config);
    let pos = param_pos(config, "ParameterPos");
    let align_ops = config.bool("AlignAssignmentOps", true);
    let align_params = config.bool("AlignActualParams", true);
    let mut aligned: HashSet<usize> = HashSet::new();
    for ci in 0..doc.syn.commands.len() {
        if is_comment_line(doc, ci) {
            continue;
        }
        let (first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
        for t in first..=last {
            if is_comment(doc, t) {
                continue;
            }
            let parent = doc.syn.parent[t];
            if doc.syn.text(t) == "=>"
                && let Some(p) = parent
                && doc.syn.text(p) == "("
                && !aligned.contains(&p)
            {
                move_opening_parenthesis(doc, p);
                align_parameters(doc, sh, p, "=>", pos, align_ops, align_params);
                move_token_after_closing_parens(doc, t, NextPos::KeepAsIs);
                aligned.insert(p);
            }
        }
    }
}
