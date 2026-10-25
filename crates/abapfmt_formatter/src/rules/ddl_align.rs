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

// ---- DDL_ALIGN_DATA_SOURCES ----

const KEYWORDS: usize = 0;
const TO_KEYWORDS: usize = 1;
const DATA_SOURCE: usize = 2;
const AS_ALIAS: usize = 3;
const ON_CONDITION: usize = 4;

struct DataSourceOpts {
    align_data_sources: bool,
    align_aliases: bool,
    align_on_conditions: bool,
    align_assocs_with_joins: bool,
    align_association_to: bool,
    consider_all_param_lines: bool,
}

pub fn data_sources(doc: &mut Doc, config: &RuleConfig) {
    let o = DataSourceOpts {
        align_data_sources: config.bool("AlignDataSources", true),
        align_aliases: config.bool("AlignAliases", true),
        align_on_conditions: config.bool("AlignOnConditions", true),
        align_assocs_with_joins: config.bool("AlignAssociationsWithJoins", false),
        align_association_to: config.bool("AlignAssociationTo", true),
        consider_all_param_lines: config.bool("ConsiderAllParamAssignLines", false),
    };
    for ci in 0..doc.syn.commands.len() {
        if cmd_parent(doc, ci).is_some() || is_comment_line(doc, ci) {
            continue;
        }
        let Some(source) = from_data_source(doc, ci) else {
            continue;
        };
        data_sources_of(doc, &o, ci, source);
    }
}

fn data_sources_of(doc: &mut Doc, o: &DataSourceOpts, ci: usize, source: usize) {
    // build and align the table of SELECT and JOIN commands
    let mut table = Table::new(5);
    build_data_source_line(doc, o, &mut table, ci, source, false);
    let mut cmd = next_non_comment_sibling(doc, ci);
    while let Some(c) = cmd {
        let Some(target) = join_target(doc, c) else {
            break;
        };
        build_data_source_line(doc, o, &mut table, c, target, true);
        cmd = next_non_comment(doc, c);
    }
    if !o.align_assocs_with_joins {
        align_data_source_table(doc, o, &mut table);
        table = Table::new(5);
    }
    // ... and of the ASSOCIATION commands (possibly together with the SELECT FROM and JOINs)
    while let Some(c) = cmd {
        let Some(target) = association_target(doc, c) else {
            break;
        };
        build_data_source_line(doc, o, &mut table, c, target, true);
        cmd = next_non_comment(doc, c);
    }
    // if the 'TO ...' column of all ASSOCIATIONs only consists of the 'to' keywords, join it
    if !table.column_is_empty(TO_KEYWORDS)
        && table
            .lines
            .iter()
            .filter_map(|l| l.cells[TO_KEYWORDS].as_ref())
            .map(|c| c.mono_width(doc))
            .max()
            == Some(2)
    {
        table.join_into_previous_columns(doc, TO_KEYWORDS, false);
    }
    align_data_source_table(doc, o, &mut table);
}

fn build_data_source_line(
    doc: &Doc,
    o: &DataSourceOpts,
    table: &mut Table,
    ci: usize,
    source: usize,
    has_on: bool,
) {
    let (cmd_first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
    let line = table.add_line();
    let mut first_in_line = source;
    while first_in_line > cmd_first && doc.lines[first_in_line] == 0 {
        first_in_line -= 1;
    }
    let to_kw = if is_kw(doc, first_in_line, "ASSOCIATION") {
        find_sibling(doc, first_in_line, last, "TO")
    } else {
        None
    };
    if first_in_line != source {
        let keywords_last = prev_code_sibling(doc, cmd_first, source).unwrap_or(first_in_line);
        if let Some(to) = to_kw.filter(|_| o.align_association_to) {
            let before_to = prev_code_sibling(doc, cmd_first, to).unwrap_or(first_in_line);
            table.set_cell(line, KEYWORDS, Cell::new(doc, first_in_line, before_to));
            table.set_cell(line, TO_KEYWORDS, Cell::new(doc, to, keywords_last));
        } else {
            table.set_cell(line, KEYWORDS, Cell::new(doc, first_in_line, keywords_last));
        }
    }

    // alias: the last "AS identifier" among the siblings
    let mut alias: Option<usize> = None;
    let mut t = Some(source);
    while let Some(x) = t {
        if doc.syn.text(x).eq_ignore_ascii_case("AS")
            && let Some(n) = next_code_sibling(doc, x, last)
            && doc.syn.types[n] == TokenType::Identifier
        {
            alias = Some(n);
        }
        t = next_code_sibling(doc, x, last);
    }
    let as_tok = alias.and_then(|a| prev_code_sibling(doc, cmd_first, a));
    let on = if has_on {
        find_sibling(doc, alias.unwrap_or(source), last, "ON")
    } else {
        None
    };
    let last_code_tok = last_code(doc, ci).unwrap_or(last);
    let source_last = if let Some(a) = as_tok {
        prev_code_sibling(doc, cmd_first, a)
    } else if let Some(on) = on {
        prev_code_sibling(doc, cmd_first, on)
    } else {
        Some(last_code_tok)
    }
    .unwrap_or(source);
    table.set_cell(line, DATA_SOURCE, Cell::new(doc, source, source_last));
    if let (Some(a), Some(al)) = (as_tok, alias) {
        table.set_cell(line, AS_ALIAS, Cell::new(doc, a, al));
    }
    if let Some(on) = on.filter(|o| doc.lines[*o] == 0) {
        table.set_cell(line, ON_CONDITION, Cell::new(doc, on, last_code_tok));
    }
}

fn align_data_source_table(doc: &mut Doc, o: &DataSourceOpts, table: &mut Table) {
    if table.column_is_empty(KEYWORDS) {
        return;
    }
    // line starts of the KEYWORDS column may deliberately differ in indent
    let mut min_indent = i64::MAX;
    for line in &table.lines {
        if let Some(c) = &line.cells[KEYWORDS] {
            min_indent = min_indent.min(i64::from(doc.spaces[c.first]));
        }
    }
    for line in &mut table.lines {
        if let Some(c) = &mut line.cells[KEYWORDS] {
            c.add_indent = i64::from(doc.spaces[c.first]) - min_indent;
        }
    }
    let mut column_with_sources = DATA_SOURCE;
    if !o.align_data_sources {
        table.join_into_previous_columns(doc, DATA_SOURCE, true);
        column_with_sources = KEYWORDS;
    }
    if !o.align_aliases {
        table.join_into_previous_columns(doc, AS_ALIAS, true);
    }
    if !o.align_on_conditions {
        table.join_into_previous_columns(doc, ON_CONDITION, true);
    }
    if !o.consider_all_param_lines {
        for li in 0..table.lines.len() {
            if let Some(c) = &mut table.lines[li].cells[column_with_sources]
                && (c.first + 1..=c.last).any(|t| doc.lines[t] > 0)
            {
                c.override_width = Some(end_x(doc, c.last) - doc.start_x(0, c.first));
            }
        }
    }
    table.align(doc, min_indent, 1, true, true);
}

// ---- DDL_ALIGN_FIELD_LISTS ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListLayout {
    MultiLine,
    OneLine,
    Derive,
    KeepAsIs,
}

fn list_layout(config: &RuleConfig, name: &str) -> ListLayout {
    match config.choice(name, "MULTI_LINE").as_str() {
        "ONE_LINE" => ListLayout::OneLine,
        "DERIVE" => ListLayout::Derive,
        "KEEP_AS_IS" => ListLayout::KeepAsIs,
        _ => ListLayout::MultiLine,
    }
}

struct FieldListOpts {
    max_line_length: i64,
    consider_dot_as_complex: bool,
}

pub fn field_lists(doc: &mut Doc, config: &RuleConfig) {
    let o = FieldListOpts {
        max_line_length: config.int("MaxLineLength", 120),
        consider_dot_as_complex: config.bool("ConsiderDotAsComplex", true),
    };
    let name_pos = config.choice("NameListPos", "LINE_START_PLUS_2");
    let name_layout = list_layout(config, "NameListLayout");
    let group_pos = config.choice("GroupByListPos", "CONTINUE");
    let complex_group = list_layout(config, "ComplexGroupByListLayout");
    let simple_group = list_layout(config, "SimpleGroupByListLayout");
    for ci in 0..doc.syn.commands.len() {
        if cmd_parent(doc, ci).is_some() || is_comment_line(doc, ci) {
            continue;
        }
        let Some(first) = first_code(doc, ci) else {
            continue;
        };
        let last = doc.syn.commands[ci].last;
        use P::{Ident, Opt, T};
        // name list (DDIC-based views only)
        if let Some(open) = last_of_sequence(
            doc,
            first,
            last,
            &[Opt("DEFINE"), Opt("ROOT"), T("VIEW"), Ident, T("(")],
        ) && open < last
            && doc.syn.parent[open + 1] == Some(open)
        {
            align_name_list(doc, &o, ci, open, &name_pos, name_layout);
            continue;
        }
        // GROUP BY clause
        let mut t = Some(first);
        while let Some(x) = t {
            if is_kw(doc, x, "GROUP")
                && let Some(by) = next_code_sibling(doc, x, last).filter(|b| is_kw(doc, *b, "BY"))
            {
                align_group_by(doc, &o, ci, x, by, &group_pos, complex_group, simple_group);
                break;
            }
            t = next_code_sibling(doc, x, last);
        }
    }
}

fn align_name_list(
    doc: &mut Doc,
    o: &FieldListOpts,
    ci: usize,
    open: usize,
    pos: &str,
    layout: ListLayout,
) {
    let (cmd_first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
    let Some(view) = prev_code_sibling(doc, cmd_first, open) else {
        return;
    };
    let Some(close) = next_code_sibling(doc, open, last) else {
        return;
    };
    let list_start = (open + 1..=last)
        .find(|t| !is_comment(doc, *t))
        .unwrap_or(open + 1);
    let Some(mut table) = build_comma_list(doc, list_start, close, last) else {
        return;
    };
    let (first_line_breaks, basic_indent);
    match pos {
        "CONTINUE" => {
            if is_comment(doc, open - 1) {
                let x = end_x(doc, view).max(0) as u32;
                set_ws(doc, open, 1, x);
            } else {
                set_ws(doc, open, 0, 0);
            }
            first_line_breaks = 0;
            basic_indent = end_x(doc, open) + 1;
            if is_comment(doc, close - 1) {
                let x = end_x(doc, view).max(0) as u32;
                set_ws(doc, close, 1, x);
            } else {
                set_ws(doc, close, 0, 1);
            }
        }
        "LINE_START_PLUS_2" => {
            set_ws(doc, open, 1, 0);
            first_line_breaks = 1;
            basic_indent = 2;
            set_ws(doc, close, 1, 0);
        }
        "LINE_START_PLUS_4" => {
            set_ws(doc, open, 1, 2);
            first_line_breaks = 1;
            basic_indent = 4;
            set_ws(doc, close, 1, 2);
        }
        "VIEW_NAME_PLUS_2" => {
            let x = start_index(doc, view);
            set_ws(doc, open, 1, x.max(0) as u32);
            first_line_breaks = 1;
            basic_indent = x + 2;
            set_ws(doc, close, 1, x.max(0) as u32);
        }
        "VIEW_NAME_PLUS_4" => {
            let x = start_index(doc, view) + 2;
            set_ws(doc, open, 1, x.max(0) as u32);
            first_line_breaks = 1;
            basic_indent = start_index(doc, view) + 4;
            set_ws(doc, close, 1, x.max(0) as u32);
        }
        _ => {
            first_line_breaks = doc.lines[list_start];
            basic_indent = start_index(doc, list_start);
        }
    }
    align_field_table(
        doc,
        o,
        &mut table,
        open,
        list_start,
        close,
        first_line_breaks,
        basic_indent,
        layout,
        layout,
    );
}

#[allow(clippy::too_many_arguments)]
fn align_group_by(
    doc: &mut Doc,
    o: &FieldListOpts,
    ci: usize,
    group: usize,
    by: usize,
    pos: &str,
    complex: ListLayout,
    simple: ListLayout,
) {
    let last = doc.syn.commands[ci].last;
    let Some(list_start) = next_code_sibling(doc, by, last) else {
        return;
    };
    let Some(last_clause) = last_code(doc, ci) else {
        return;
    };
    let Some(mut table) = build_comma_list(doc, list_start, last_clause, last) else {
        return;
    };
    let (first_line_breaks, basic_indent);
    match pos {
        "CONTINUE" => {
            first_line_breaks = 0;
            basic_indent = end_x(doc, by) + 1;
        }
        "LINE_START_PLUS_2" => {
            first_line_breaks = 1;
            basic_indent = first_token_in_line_spaces(doc, by) + 2;
        }
        "LINE_START_PLUS_4" => {
            first_line_breaks = 1;
            basic_indent = first_token_in_line_spaces(doc, by) + 4;
        }
        _ => {
            first_line_breaks = doc.lines[list_start];
            basic_indent = start_index(doc, list_start);
        }
    }
    align_field_table(
        doc,
        o,
        &mut table,
        group,
        list_start,
        last_clause,
        first_line_breaks,
        basic_indent,
        complex,
        simple,
    );
}

/// `buildTableFromCommaSepList`; the last term runs up to (and includes) `last_clause`.
fn build_comma_list(
    doc: &mut Doc,
    list_start: usize,
    last_clause: usize,
    cmd_last: usize,
) -> Option<Table> {
    let mut table = Table::new(1);
    let mut token = Some(list_start);
    loop {
        let first_in_term = token?;
        let mut last_in_term = first_in_term;
        while let Some(t) = token {
            if matches!(doc.syn.text(t), "," | ";") {
                break;
            }
            last_in_term = t;
            if t == last_clause {
                token = None;
                break;
            }
            token = next_code_sibling(doc, t, cmd_last);
        }
        let line = table.add_line();
        table.set_cell(line, 0, Cell::new(doc, first_in_term, last_in_term));
        let Some(t) = token else { break };
        let attached = doc.lines[t] == 0 && doc.spaces[t] == 0;
        if doc.syn.types[t] == TokenType::Comma && !attached && !is_comment(doc, t - 1) {
            set_ws(doc, t, 0, 0);
        }
        token = next_code_sibling(doc, t, cmd_last);
        if token.is_none() {
            break;
        }
    }
    Some(table)
}

#[allow(clippy::too_many_arguments)]
fn align_field_table(
    doc: &mut Doc,
    o: &FieldListOpts,
    table: &mut Table,
    first_clause: usize,
    list_start: usize,
    last_clause: usize,
    first_line_breaks: u32,
    basic_indent: i64,
    complex: ListLayout,
    simple: ListLayout,
) {
    if table.lines.is_empty() || (complex == ListLayout::KeepAsIs && simple == ListLayout::KeepAsIs)
    {
        return;
    }
    // determine the layout
    let mut layout = if complex == simple {
        complex
    } else {
        let (mut multi_token, mut has_dot) = (false, false);
        for l in &table.lines {
            if let Some(c) = &l.cells[0] {
                if c.first != c.last {
                    multi_token = true;
                } else if doc.syn.text(c.first).contains('.') {
                    has_dot = true;
                }
            }
        }
        if multi_token || (has_dot && o.consider_dot_as_complex) {
            complex
        } else {
            simple
        }
    };
    if layout == ListLayout::KeepAsIs {
        return;
    }
    if layout == ListLayout::Derive {
        let (mut breaks, mut cont) = (0, 0);
        for l in table.lines.iter().skip(1) {
            if l.first_token().is_some_and(|t| doc.lines[t] > 0) {
                breaks += 1;
            } else {
                cont += 1;
            }
        }
        layout = if breaks > cont {
            ListLayout::MultiLine
        } else {
            ListLayout::OneLine
        };
    }

    // the list starts directly behind the keyword(s), unless a comment is in the way
    if !is_comment(doc, list_start - 1) {
        set_ws(doc, list_start, 0, 1);
    }
    if layout == ListLayout::OneLine {
        super::ddl_position::condense_up_to(
            doc,
            first_clause,
            last_clause,
            o.max_line_length,
            basic_indent.max(0) as u32,
        );
    } else {
        table.align_ex(doc, basic_indent, first_line_breaks, true, true, true);
    }

    // align comments
    let token_before = (0..list_start).rev().find(|t| !is_comment(doc, *t));
    let cmd_first = doc.syn.commands[cmd_of(doc, list_start)].first;
    let cmd_last = doc.syn.commands[cmd_of(doc, list_start)].last;
    let mut token = list_start;
    while token > cmd_first && is_comment_line_tok(doc, token - 1) {
        token -= 1;
    }
    if is_comment_line_tok(doc, token)
        && doc.spaces[token] > 0
        && token > cmd_first
        && Some(token - 1) == token_before
        && end_x(doc, token - 1) + 1 == basic_indent
    {
        set_ws(doc, token, 0, 1);
        token += 1;
    }
    let mut t = Some(token);
    while let Some(x) = t {
        if is_comment_line_tok(doc, x) {
            indent_comment(doc, x, basic_indent);
        }
        if x == last_clause {
            break;
        }
        t = next_sibling_any(doc, x, cmd_last);
    }
}

fn is_comment_line_tok(doc: &Doc, t: usize) -> bool {
    is_comment(doc, t)
        && (doc.lines[t] > 0 || t == 0)
        && doc.lines.get(t + 1).is_none_or(|l| *l > 0)
}

fn indent_comment(doc: &mut Doc, t: usize, basic_indent: i64) {
    let text = doc.syn.text(t).to_string();
    if doc.spaces[t] > 0 || !text.starts_with("//") {
        let l = doc.lines[t];
        set_ws(doc, t, l, basic_indent.max(0) as u32);
    } else if text.chars().count() > 2 {
        let sign = text.chars().next().unwrap_or('/');
        let sign_end = text.find(|c| c != sign).unwrap_or(text.len());
        if sign_end >= text.len() {
            return;
        }
        let rest = &text[sign_end..];
        let text_pos = sign_end + (rest.len() - rest.trim_start_matches(' ').len());
        let new = format!(
            "{}{}{}",
            &text[..sign_end],
            " ".repeat(basic_indent.max(0) as usize),
            &text[text_pos..]
        );
        if new != text {
            doc.text[t] = Some(new);
        }
    }
}
