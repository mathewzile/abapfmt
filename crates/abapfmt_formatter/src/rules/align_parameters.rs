//! ALIGN_PARAMETERS: aligns parameter assignments in method calls, as well as component
//! assignments in VALUE expressions, table expressions, table keys etc. (port of
//! `AlignParametersRule`).
use std::collections::HashSet;

use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, end_x, is_comment, max_end_index_in_any_line};
use crate::rules::align_declarations::matches_seq;
use crate::rules::align_with_second_word::is_abap_sql_operation;
use crate::{ConfigExt, Doc, RuleConfig};

const COLUMN_COUNT: usize = 9;
const LET_KEYWORD: usize = 0;
const LET_PARAMETER: usize = 1;
const LET_ASSIGNMENT_OP: usize = 2;
const LET_EXPRESSION: usize = 3;
const LET_IN_KEYWORD: usize = 4;
const KEYWORD: usize = 5;
const PARAMETER: usize = 6;
const ASSIGNMENT_OP: usize = 7;
const EXPRESSION: usize = 8;

const INDENT_STEP: i64 = 2;

const CONSTRUCTOR_OPS: &[&str] = &[
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

const SQL_FUNCTIONS: &[&str] = &[
    "CAST(",
    "ALLOW_PRECISION_LOSS(",
    "AVG(",
    "CORR(",
    "CORR_SPEARMAN(",
    "COUNT(",
    "GROUPING(",
    "MAX(",
    "MEDIAN(",
    "MIN(",
    "PRODUCT(",
    "STDDEV(",
    "STRING_AGG(",
    "SUM(",
    "VAR(",
    "ROW_NUMBER(",
    "RANK(",
    "DENSE_RANK(",
    "NTILE(",
    "LEAD(",
    "LAG(",
    "FIRST_VALUE(",
    "LAST_VALUE(",
    "abap_system_timezone(",
    "abap_user_timezone(",
    "abs(",
    "add_days(",
    "add_months(",
    "as_geo_json(",
    "bintohex(",
    "ceil(",
    "coalesce(",
    "concat(",
    "concat_with_space(",
    "currency_conversion(",
    "datn_add_days(",
    "datn_add_months(",
    "datn_days_between(",
    "dats_add_days(",
    "dats_add_months(",
    "dats_days_between(",
    "dats_from_datn(",
    "dats_is_valid(",
    "dats_tims_to_tstmp(",
    "dats_to_datn(",
    "dayname(",
    "days_between(",
    "div(",
    "division(",
    "extract_day(",
    "extract_hour(",
    "extract_minute(",
    "extract_month(",
    "extract_second(",
    "extract_year(",
    "floor(",
    "hextobin(",
    "initcap(",
    "instr(",
    "is_valid(",
    "left(",
    "length(",
    "like_regexpr(",
    "locate(",
    "locate_regexpr(",
    "locate_regexpr_after(",
    "lower(",
    "lpad(",
    "ltrim(",
    "mod(",
    "monthname(",
    "occurrences_regexpr(",
    "replace(",
    "replace_regexpr(",
    "right(",
    "round(",
    "rpad(",
    "rtrim(",
    "substring(",
    "substring_regexpr(",
    "tims_from_timn(",
    "tims_is_valid(",
    "tims_to_timn(",
    "to_blob(",
    "to_clob(",
    "tstmp_add_seconds(",
    "tstmp_current_utctimestamp(",
    "tstmp_is_valid(",
    "tstmp_seconds_between(",
    "tstmp_to_dats(",
    "tstmp_to_dst(",
    "tstmp_to_tims(",
    "tstmpl_from_utcl(",
    "tstmpl_to_utcl(",
    "unit_conversion(",
    "upper(",
    "utcl_add_seconds(",
    "utcl_current(",
    "utcl_seconds_between(",
    "uuid(",
    "weekday(",
];

const SQL_FUNCTIONS_WITH_ASSIGNMENTS: &[&str] = &[
    "like_regexpr(",
    "locate_regexpr(",
    "locate_regexpr_after(",
    "occurrences_regexpr(",
    "replace_regexpr(",
    "substring_regexpr(",
    "currency_conversion(",
    "unit_conversion(",
    "tstmp_seconds_between(",
    "tstmp_add_seconds(",
    "abap_system_timezone(",
    "abap_user_timezone(",
    "tstmp_to_dats(",
    "tstmp_to_tims(",
    "tstmp_to_dst(",
    "dats_tims_to_tstmp(",
    "tstmpl_to_utcl(",
    "tstmpl_from_utcl(",
    "dats_to_datn(",
    "dats_from_datn(",
    "tims_to_timn(",
    "tims_from_timn(",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Content {
    FunctionalCall,
    ProceduralCall,
    ImportOrExport,
    TableKey,
    GroupKey,
    ConstructorExpr,
    RowInValueOrNew,
}

struct Opts {
    max_line_length: i64,
    max_line_length_single: i64,
    max_params_procedural: i64,
    max_params_functional: i64,
    keywords_own_line_procedural: bool,
    keywords_own_line_functional: bool,
    align_assignments: bool,
    align_across_table_rows: bool,
    keep_components: String,
    keep_other: String,
    content_left: String,
}

struct TableStart {
    start_indent: i64,
    continue_on_same_line: bool,
    force_table_to_next_line: bool,
    early_indent: i64,
}

// ---- token helpers ----

fn is_code(doc: &Doc, t: usize) -> bool {
    !matches!(doc.syn.types[t], TokenType::Comment | TokenType::Pragma)
}

fn text_is(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.text(t).eq_ignore_ascii_case(s)
}

fn text_any(doc: &Doc, t: usize, set: &[&str]) -> bool {
    set.iter().any(|s| text_is(doc, t, s))
}

fn is_keyword(doc: &Doc, t: usize) -> bool {
    doc.syn.types[t] == TokenType::Keyword
}

fn kw(doc: &Doc, t: usize, s: &str) -> bool {
    is_keyword(doc, t) && text_is(doc, t, s)
}

fn any_kw(doc: &Doc, t: usize, set: &[&str]) -> bool {
    is_keyword(doc, t) && text_any(doc, t, set)
}

fn is_ident(doc: &Doc, t: usize) -> bool {
    doc.syn.types[t] == TokenType::Identifier
}

fn opens(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t)
}

/// `Token.closesLevel`: also true for tokens that close and open a level, like `)->method(`.
fn closes(doc: &Doc, t: usize) -> bool {
    let text = doc.syn.text(t);
    if doc.syn.types[t] == TokenType::Literal {
        text.starts_with('}')
    } else {
        text.starts_with([')', ']'])
    }
}

fn has_children(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t + 1)
}

fn is_attached(doc: &Doc, t: usize) -> bool {
    doc.lines[t] == 0 && doc.spaces[t] == 0
}

fn len(doc: &Doc, t: usize) -> i64 {
    doc.token_text(t).chars().count() as i64
}

fn next_tok(cmd: &Command, t: usize) -> Option<usize> {
    (t < cmd.last).then_some(t + 1)
}

fn prev_tok(cmd: &Command, t: usize) -> Option<usize> {
    (t > cmd.first).then(|| t - 1)
}

fn next_code_tok(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    (t + 1..=cmd.last).find(|j| is_code(doc, *j))
}

fn prev_code_tok(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    (cmd.first..t).rev().find(|j| is_code(doc, *j))
}

/// `Token.getNextSibling`: `None` after the last child of a level.
fn nsib(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    let o = opens(doc, t);
    let n = if o { doc.syn.bracket_match[t]? } else { t + 1 };
    (n <= cmd.last && (o || !closes(doc, n))).then_some(n)
}

fn next_code_sib(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    let mut c = nsib(doc, cmd, t)?;
    while !is_code(doc, c) {
        c = nsib(doc, cmd, c)?;
    }
    Some(c)
}

fn prev_sib(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    if closes(doc, t) && !opens(doc, t) {
        return doc.syn.bracket_match[t];
    }
    let p = prev_tok(cmd, t)?;
    (doc.syn.parent[t] != Some(p)).then_some(p)
}

fn prev_code_sib(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    let mut c = prev_sib(doc, cmd, t)?;
    while !is_code(doc, c) {
        c = prev_sib(doc, cmd, c)?;
    }
    Some(c)
}

/// The first sibling from `from` on (including `from`) whose text is one of `set`.
fn find_sibling(doc: &Doc, cmd: &Command, from: usize, set: &[&str]) -> Option<usize> {
    let mut c = Some(from);
    while let Some(x) = c {
        if text_any(doc, x, set) {
            return Some(x);
        }
        c = next_code_sib(doc, cmd, x);
    }
    None
}

fn is_comment_line(doc: &Doc, cmd: &Command, t: usize) -> bool {
    is_comment(doc, t) && doc.first_in_line(t) && (t >= cmd.last || doc.lines[t + 1] > 0)
}

fn is_asterisk_comment_line(doc: &Doc, cmd: &Command, t: usize) -> bool {
    is_comment_line(doc, cmd, t) && doc.syn.text(t).starts_with('*')
}

fn is_comment_after_code(doc: &Doc, cmd: &Command, t: usize) -> bool {
    is_comment(doc, t) && t > cmd.first && doc.lines[t] == 0
}

fn is_pseudo_comment_after_code(doc: &Doc, cmd: &Command, t: usize) -> bool {
    is_comment_after_code(doc, cmd, t)
        && doc
            .syn
            .text(t)
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("\"#EC"))
}

fn set_ws(doc: &mut Doc, t: usize, lines: u32, spaces: i64) -> bool {
    let spaces = spaces.max(0) as u32;
    if doc.lines[t] == lines && doc.spaces[t] == spaces {
        return false;
    }
    doc.lines[t] = lines;
    doc.spaces[t] = spaces;
    true
}

/// `Term.isFirstTokenAllowed`
fn first_token_allowed(doc: &Doc, t: usize) -> bool {
    match doc.syn.types[t] {
        TokenType::Identifier | TokenType::Literal | TokenType::OtherOp => true,
        TokenType::Keyword => {
            text_any(doc, t, CONSTRUCTOR_OPS)
                || text_any(doc, t, &["DATA(", "FINAL("])
                || doc
                    .syn
                    .text(t)
                    .get(..5)
                    .is_some_and(|p| p.eq_ignore_ascii_case("TEXT-"))
        }
        _ => false,
    }
}

/// `Term.createForExpression(first).lastToken`; `None` for an unexpected syntax.
fn expression_last(doc: &Doc, cmd: &Command, first: usize) -> Option<usize> {
    let l = doc.syn.last_of_expression(first, cmd.first, cmd.last)?;
    (!has_children(doc, l)).then_some(l)
}

// ---- structure helpers ----

/// `Token.opensGroupKey`
fn opens_group_key(doc: &Doc, cmd: &Command, t: usize) -> bool {
    if doc.syn.text(t) != "(" || !has_children(doc, t) || is_attached(doc, t + 1) {
        return false;
    }
    let Some(prev) = prev_code_sib(doc, cmd, t) else {
        return false;
    };
    let Some(prev_prev) = prev_code_sib(doc, cmd, prev) else {
        return false;
    };
    kw(doc, prev_prev, "GROUP")
        && kw(doc, prev, "BY")
        && doc
            .first_code_token(cmd)
            .is_some_and(|f| kw(doc, f, "LOOP"))
}

/// `Token.getEndOfParamsOrComponentsList`
fn end_of_params(doc: &Doc, cmd: &Command, t: usize) -> Option<usize> {
    if !opens(doc, t) || !has_children(doc, t) || doc.syn.types[t] == TokenType::Literal {
        return None;
    }
    let next = t + 1;
    if is_attached(doc, next)
        && (is_ident(doc, next)
            || doc.syn.types[next] == TokenType::Literal
            || matches!(doc.syn.text(next), "*" | "**"))
    {
        return None;
    }
    let is_sql = doc
        .first_code_token(cmd)
        .is_some_and(|f| is_abap_sql_operation(doc, cmd, f));
    let parent = doc.syn.parent[t];
    if text_any(doc, t, &["boolc(", "xsdbool("])
        || is_sql
            && (text_any(
                doc,
                t,
                &[
                    "OVER(",
                    "HIERARCHY(",
                    "HIERARCHY_SIBLINGS(",
                    "HIERARCHY_ANCESTORS(",
                    "HIERARCHY_DESCENDANTS(",
                    "HIERARCHY_ANCESTORS_AGGREGATE(",
                    "HIERARCHY_DESCENDANTS_AGGREGATE(",
                ],
            ) || doc.syn.text(t) == "@("
                || doc.syn.text(t).ends_with('['))
    {
        return None;
    }
    if is_sql && text_any(doc, t, SQL_FUNCTIONS) {
        return if text_any(doc, t, SQL_FUNCTIONS_WITH_ASSIGNMENTS) {
            next_code_sib(doc, cmd, t)
        } else {
            None
        };
    } else if doc.syn.text(t) == "(" {
        let prev = prev_code_sib(doc, cmd, t);
        if parent.is_none() && opens_group_key(doc, cmd, t) {
            return next_code_sib(doc, cmd, t);
        } else if is_sql && prev.is_some_and(|p| is_keyword(doc, p)) {
            return None;
        }
        let p = parent?;
        if doc.syn.text(p) == "(" || !doc.syn.text(p).ends_with('(') {
            return None;
        }
        let parent_prev = prev_code_tok(doc, cmd, p)?;
        if !any_kw(doc, parent_prev, &["VALUE", "NEW"]) {
            return None;
        }
        if let Some(prev) = prev
            && doc.syn.text(prev) != ")"
            && (matches!(
                doc.syn.types[prev],
                TokenType::AssignmentOp | TokenType::ComparisonOp | TokenType::OtherOp
            ) || any_kw(doc, prev, &["UNTIL", "WHILE", "WHERE"]))
        {
            return None;
        }
    } else {
        let prev = prev_code_tok(doc, cmd, t);
        if prev.is_some_and(|p| any_kw(doc, p, &["NEW", "VALUE"])) {
            // continue below
        } else if prev.is_some_and(|p| any_kw(doc, p, CONSTRUCTOR_OPS)) {
            return None;
        }
    }
    nsib(doc, cmd, t)
}

fn content_type(doc: &Doc, cmd: &Command, t: usize) -> Content {
    if doc.syn.text(t) == "(" {
        return if opens_group_key(doc, cmd, t) {
            Content::GroupKey
        } else {
            Content::RowInValueOrNew
        };
    }
    let prev = prev_code_sib(doc, cmd, t);
    if prev.is_some_and(|p| kw(doc, p, "METHOD")) {
        Content::ProceduralCall
    } else if prev.is_some_and(|p| any_kw(doc, p, &["NEW", "VALUE"])) {
        Content::ConstructorExpr
    } else {
        Content::FunctionalCall
    }
}

fn base_indent_for_raise(doc: &Doc, cmd: &Command, parent_token: usize) -> i64 {
    let mut base = doc.start_x(0, cmd.first);
    let mut token = next_code_sib(doc, cmd, cmd.first);
    while let Some(t) = token {
        if t == parent_token || (is_keyword(doc, t) && doc.lines[t] > 0) {
            break;
        }
        token = next_code_sib(doc, cmd, t);
    }
    if let Some(t) = token
        && is_keyword(doc, t)
        && doc.lines[t] > 0
    {
        base = i64::from(doc.spaces[t]) - INDENT_STEP;
    }
    base
}

fn base_indent(doc: &Doc, cmd: &Command, t: usize, ct: Content) -> i64 {
    if matches!(ct, Content::RowInValueOrNew | Content::GroupKey) {
        return doc.start_x(0, t);
    }
    let mut test = t;
    while !doc.first_in_line(test) && closes(doc, test) && test > cmd.first && opens(doc, test - 1)
    {
        test -= 1;
    }
    while !doc.first_in_line(test) && test > cmd.first && any_kw(doc, test - 1, &["CALL", "METHOD"])
    {
        test -= 1;
    }
    doc.start_x(0, test)
}

fn minimum_indent(doc: &Doc, cmd: &Command, t: usize, ct: Content) -> i64 {
    if matches!(ct, Content::RowInValueOrNew | Content::GroupKey) {
        return doc.start_x(0, t);
    }
    let mut test = t;
    while doc.lines[test] == 0 && test > cmd.first && Some(test - 1) != doc.syn.parent[t] {
        test -= 1;
    }
    // if the line starts with a very short variable ("exp = ..."), move right of the assignment operator
    if is_ident(doc, test) && len(doc, test) <= 4 && test < cmd.last {
        let op = test + 1;
        if doc.syn.types[op] == TokenType::AssignmentOp
            && doc.lines[op] == 0
            && doc.spaces[op] <= 2
            && op < cmd.last
        {
            let next = op + 1;
            if doc.lines[next] == 0 && doc.spaces[next] <= 1 {
                test = next;
            }
        }
    }
    doc.start_x(0, test)
}

fn first_component_of_table_key(doc: &Doc, cmd: &Command, first: usize) -> Option<usize> {
    // WITH [TABLE] KEY
    let mut c = Some(first);
    let mut key_token = None;
    while let Some(x) = c {
        if text_is(doc, x, "WITH") {
            let mut n = next_code_sib(doc, cmd, x);
            if let Some(t) = n
                && text_is(doc, t, "TABLE")
            {
                n = next_code_sib(doc, cmd, t);
            }
            if let Some(k) = n
                && text_is(doc, k, "KEY")
            {
                key_token = Some(k);
                break;
            }
        }
        c = next_code_sib(doc, cmd, x);
    }
    let key_token = key_token?;
    let mut identifier = next_code_sib(doc, cmd, key_token)?;
    if doc.syn.text(identifier) == "=" {
        return None;
    }
    let mut next = next_code_sib(doc, cmd, identifier)?;
    if doc.syn.text(next) != "=" {
        let mut n = next_code_sib(doc, cmd, next);
        if let Some(t) = n
            && kw(doc, t, "COMPONENTS")
        {
            n = next_code_sib(doc, cmd, t);
        }
        identifier = n?;
        next = next_code_sib(doc, cmd, identifier)?;
    }
    (doc.syn.text(next) == "=").then_some(identifier)
}

fn may_be_variable_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > 255 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_/-~%$*?#&".contains(c))
}

/// `Token.getBitsOfCommentedOutAssignment`
fn commented_out_assignment(doc: &Doc, cmd: &Command, t: usize) -> Option<[String; 4]> {
    if !is_asterisk_comment_line(doc, cmd, t) {
        return None;
    }
    let text = doc.token_text(t);
    let eq = text.find('=')?;
    let bits: Vec<&str> = text[..eq].split(' ').filter(|s| !s.is_empty()).collect();
    if bits.len() != 2 || !may_be_variable_name(bits[1]) {
        return None;
    }
    Some([
        bits[0].to_string(),
        bits[1].to_string(),
        "=".to_string(),
        text[eq + 1..].trim().to_string(),
    ])
}

fn align_asterisk_comment(
    doc: &mut Doc,
    cmd: &Command,
    t: usize,
    parameter_indent: i64,
    assignment_op_indent: i64,
) {
    let Some(bits) = commented_out_assignment(doc, cmd, t) else {
        return;
    };
    let mut s = bits[0].clone();
    s.push_str(&" ".repeat(parameter_indent.max(0) as usize));
    s.push_str(&bits[1]);
    let spaces = assignment_op_indent - parameter_indent - bits[1].chars().count() as i64;
    s.push_str(&" ".repeat(spaces.max(1) as usize));
    s.push_str(&bits[2]);
    if !bits[3].is_empty() {
        s.push(' ');
        s.push_str(&bits[3]);
    }
    if s != doc.token_text(t) {
        doc.text[t] = Some(s);
    }
}

// ---- the rule ----

pub fn align_parameters(doc: &mut Doc, config: &RuleConfig) {
    let o = Opts {
        max_line_length: config.int("MaxLineLength", 120),
        max_line_length_single: config.int("MaxLineLengthForSingleLine", 160),
        max_params_procedural: config.int("MaxParamCountBehindProceduralCall", 0),
        max_params_functional: config.int("MaxParamCountBehindFunctionalCall", 100),
        keywords_own_line_procedural: config.bool("PutProceduralCallKeywordsOnOwnLine", false),
        keywords_own_line_functional: config.bool("PutFunctionalCallKeywordsOnOwnLine", false),
        align_assignments: config.bool("AlignAssignments", true),
        align_across_table_rows: config.bool("AlignAcrossTableRows", true),
        keep_components: config.choice("KeepComponentsOnSingleLine", "IF_BELOW_MAX_LINE_LENGTH"),
        keep_other: config.choice("KeepOtherOneLiners", "NEVER"),
        content_left: config.choice("AllowContentLeftOfAssignOp", "TO_KEEP_MAX_LINE_LENGTH"),
    };
    for cmd in doc.commands().to_vec() {
        if cmd.is_comment_only || cmd.colon.is_some() {
            continue;
        }
        execute_on(doc, &o, &cmd);
    }
}

fn raise_matches(doc: &Doc, cmd: &Command, first: usize) -> bool {
    let n = |t: usize| next_code_sib(doc, cmd, t);
    if !text_is(doc, first, "RAISE") {
        return false;
    }
    let Some(mut t) = n(first) else {
        return false;
    };
    let has_exporting = |from: Option<usize>| {
        from.is_some_and(|f| find_sibling(doc, cmd, f, &["EXPORTING"]).is_some())
    };
    if text_is(doc, t, "EVENT") {
        return n(t).is_some_and(|id| {
            is_ident(doc, id) && n(id).is_some_and(|e| text_is(doc, e, "EXPORTING"))
        });
    }
    if text_is(doc, t, "RESUMABLE") {
        let Some(x) = n(t) else {
            return false;
        };
        t = x;
        if !text_is(doc, t, "EXCEPTION") {
            return false;
        }
    } else if !text_is(doc, t, "EXCEPTION") && !text_is(doc, t, "SHORTDUMP") {
        return false;
    }
    let Some(ty) = n(t) else {
        return false;
    };
    if !text_is(doc, ty, "TYPE") {
        return false;
    }
    let Some(id) = n(ty) else {
        return false;
    };
    is_ident(doc, id) && has_exporting(n(id))
}

fn execute_on(doc: &mut Doc, o: &Opts, cmd: &Command) {
    let Some(first) = doc.first_code_token(cmd) else {
        return;
    };
    let Some(period) = (cmd.first..=cmd.last).rev().find(|j| !is_comment(doc, *j)) else {
        return;
    };

    // CALL METHOD|FUNCTION|BADI without parentheses, CREATE OBJECT, RECEIVE RESULTS FROM FUNCTION
    if matches_seq(doc, first, cmd.last, &["CALL", "METHOD|FUNCTION|BADI"])
        || matches_seq(doc, first, cmd.last, &["CREATE", "OBJECT"])
        || matches_seq(
            doc,
            first,
            cmd.last,
            &["RECEIVE", "RESULTS", "FROM", "FUNCTION"],
        )
    {
        let first_keyword = find_sibling(
            doc,
            cmd,
            first,
            &[
                "EXPORTING",
                "IMPORTING",
                "TABLES",
                "CHANGING",
                "RECEIVING",
                "EXCEPTIONS",
            ],
        );
        if let Some(k) = first_keyword
            && doc.syn.parent[k].is_none()
            && let Some(parent_token) = prev_code_sib(doc, cmd, k)
        {
            let base = doc.start_x(0, cmd.first);
            align_params(
                doc,
                o,
                cmd,
                parent_token,
                period,
                base,
                base,
                Content::ProceduralCall,
            );
        }
    }

    // RAISE ... EXPORTING
    if raise_matches(doc, cmd, first)
        && let Some(k) = find_sibling(doc, cmd, first, &["EXPORTING"])
        && let Some(parent_token) = prev_code_sib(doc, cmd, k)
    {
        let base = base_indent_for_raise(doc, cmd, parent_token);
        align_params(
            doc,
            o,
            cmd,
            parent_token,
            period,
            base,
            base,
            Content::ProceduralCall,
        );
    }

    // IMPORT / EXPORT
    let is_import = text_is(doc, first, "IMPORT");
    if is_import || text_is(doc, first, "EXPORT") {
        let n = |t: usize| next_code_sib(doc, cmd, t);
        let (op_set, end_text): (&[&str], &str) = if is_import {
            (&["=", "TO"], "FROM")
        } else {
            (&["=", "FROM"], "TO")
        };
        if let Some(id) = n(first)
            && is_ident(doc, id)
            && let Some(op) = n(id)
            && text_any(doc, op, op_set)
            && n(op).is_some_and(|x| find_sibling(doc, cmd, x, &[end_text]).is_some())
            && let Some(end) = find_sibling(doc, cmd, first, &[end_text])
        {
            let base = base_indent_for_raise(doc, cmd, first);
            align_params(doc, o, cmd, first, end, base, base, Content::ImportOrExport);
        }
    }

    // READ|DELETE TABLE ... WITH [TABLE] KEY ...
    if matches_seq(doc, first, cmd.last, &["READ|DELETE", "TABLE"])
        && let Some(identifier) = first_component_of_table_key(doc, cmd, first)
        && let Some(end) = find_sibling(
            doc,
            cmd,
            identifier,
            &[
                "BINARY",
                "INTO",
                "ASSIGNING",
                "REFERENCE",
                "TRANSPORTING",
                ".",
            ],
        )
        && identifier != end
        && identifier > cmd.first
    {
        let base = if doc.lines[identifier] > 0 {
            doc.start_x(0, identifier)
        } else {
            end_x(doc, identifier - 1) + 1
        };
        align_params(
            doc,
            o,
            cmd,
            identifier - 1,
            end,
            base,
            base,
            Content::TableKey,
        );
    }

    // assignments in parentheses
    let pass_count = if o.content_left == "NEVER" { 1 } else { 2 };
    for _ in 0..pass_count {
        let before: Vec<(u32, u32)> = (cmd.first..=cmd.last)
            .map(|t| (doc.lines[t], doc.spaces[t]))
            .collect();
        let before_text: Vec<Option<String>> = doc.text[cmd.first..=cmd.last].to_vec();
        let mut rows_aligned_for_parent: HashSet<Option<usize>> = HashSet::new();
        let mut t = cmd.first;
        while t <= cmd.last {
            if let Some(end) = end_of_params(doc, cmd, t) {
                let ct = content_type(doc, cmd, t);
                let mut base = base_indent(doc, cmd, t, ct);
                let mut min = minimum_indent(doc, cmd, t, ct);
                match o.content_left.as_str() {
                    "ALWAYS" => base = min,
                    "NEVER" => min = base,
                    _ => {}
                }
                let parent = doc.syn.parent[t];
                if ct == Content::RowInValueOrNew && rows_aligned_for_parent.contains(&parent) {
                    // already aligned with the first row
                } else {
                    align_params(doc, o, cmd, t, end, base, min, ct);
                }
                if ct == Content::RowInValueOrNew && o.align_across_table_rows {
                    rows_aligned_for_parent.insert(parent);
                }
            }
            t += 1;
        }
        let changed = (cmd.first..=cmd.last)
            .any(|t| before[t - cmd.first] != (doc.lines[t], doc.spaces[t]))
            || doc.text[cmd.first..=cmd.last] != before_text[..];
        if !changed {
            break;
        }
    }
}

fn find_start_of_next_row(
    doc: &Doc,
    cmd: &Command,
    end_of_row: usize,
) -> Result<Option<usize>, ()> {
    let mut token = next_code_sib(doc, cmd, end_of_row);
    while let Some(t) = token {
        if doc.syn.text(t) == "(" {
            return Ok(Some(t));
        }
        if !is_ident(doc, t) {
            return Ok(None);
        }
        let Some(op) = next_code_sib(doc, cmd, t) else {
            return Ok(None);
        };
        if doc.syn.types[op] != TokenType::AssignmentOp {
            return Ok(None);
        }
        let Some(rhs) = next_code_sib(doc, cmd, op) else {
            return Ok(None);
        };
        let last = expression_last(doc, cmd, rhs).ok_or(())?;
        token = next_code_sib(doc, cmd, last);
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn align_params(
    doc: &mut Doc,
    o: &Opts,
    cmd: &Command,
    parent_token: usize,
    end_token: usize,
    base_indent: i64,
    minimum_indent: i64,
    ct: Content,
) {
    // ---- 1. prepare alignment ----
    let mut table = Table::new(COLUMN_COUNT);
    let mut parent_tokens: Vec<usize> = Vec::new();
    let mut other_line_starts: Vec<usize> = Vec::new();
    let mut cur_parent = parent_token;
    let mut cur_end = end_token;
    loop {
        parent_tokens.push(cur_parent);
        if build_align_table(
            doc,
            cmd,
            &mut table,
            cur_parent,
            cur_end,
            ct,
            &mut other_line_starts,
        )
        .is_none()
        {
            return;
        }
        set_forced_line_breaks(o, &mut table, ct);
        if ct != Content::RowInValueOrNew || !o.align_across_table_rows {
            break;
        }
        match find_start_of_next_row(doc, cmd, cur_end) {
            Err(()) => return,
            Ok(None) => break,
            Ok(Some(next_row)) => {
                let Some(close) = doc.syn.bracket_match[next_row].filter(|c| *c > next_row) else {
                    break;
                };
                cur_parent = next_row;
                cur_end = close;
            }
        }
    }

    if table.lines.is_empty() && other_line_starts.is_empty() {
        return;
    }

    if !o.align_assignments {
        for col in [LET_ASSIGNMENT_OP, LET_EXPRESSION, ASSIGNMENT_OP, EXPRESSION] {
            table.join_into_previous_columns(doc, col, true);
        }
    }

    let ts = determine_table_start(
        doc,
        o,
        cmd,
        parent_token,
        base_indent,
        minimum_indent,
        ct,
        &mut table,
        &other_line_starts,
    );

    let mut keep_on_single_line = false;
    let mut parent_token_of_first_line: Option<usize> = None;
    if ct == Content::RowInValueOrNew && o.align_across_table_rows {
        for &cp in &parent_tokens {
            let close = doc.syn.bracket_match[cp].filter(|c| *c > cp);
            if let Some(close) = close
                && determine_keep_on_single_line(doc, o, cp, close, ts.early_indent, true)
            {
                table.remove_all_lines_of_parent(doc, cp);
            } else if parent_token_of_first_line.is_none() {
                parent_token_of_first_line = Some(cp);
            }
        }
    } else {
        let for_tabular =
            ct == Content::RowInValueOrNew || is_table_of_single_components(doc, cmd, parent_token);
        keep_on_single_line = determine_keep_on_single_line(
            doc,
            o,
            parent_token,
            end_token,
            ts.early_indent,
            for_tabular,
        );
    }
    let parent_token_of_first_line = parent_token_of_first_line.unwrap_or(parent_token);

    // ---- 2. perform alignment changes ----
    let skip_align =
        keep_on_single_line && (table.lines.len() > 1 || ct == Content::RowInValueOrNew);
    if !table.lines.is_empty() && !skip_align {
        let first_token = table.first_token();
        let start_line_breaks = if ts.force_table_to_next_line {
            1
        } else if ts.continue_on_same_line
            && (first_token == Some(parent_token_of_first_line + 1)
                || first_token.is_some_and(|f| doc.lines[f] == 0)
                    && table.first_token_column_index() != Some(PARAMETER))
        {
            0
        } else {
            1
        };
        table.align(doc, ts.start_indent, start_line_breaks, true, true);
    }

    // prepare alignment of asterisk comment lines
    let parameter_indent = table.effective_indent(PARAMETER);
    let assignment_op_indent = table.effective_indent(ASSIGNMENT_OP);
    let expression_indent = table.effective_indent(EXPRESSION);
    let may_align_asterisk_comments = !table.lines.is_empty()
        && !table.column_is_empty(PARAMETER)
        && parameter_indent > 0
        && !table.column_is_empty(ASSIGNMENT_OP)
        && assignment_op_indent > 0
        && !table.column_is_empty(EXPRESSION)
        && expression_indent > 0;

    // align other lines: comment lines, inner parentheses, assignments in * comment lines
    let other_line_indent = if table.column_is_empty(LET_IN_KEYWORD) {
        ts.start_indent
    } else {
        parameter_indent
    };
    for &other in &other_line_starts {
        if doc.syn.text(other) == "(" {
            move_row_in_value_or_new_constructor(
                doc,
                cmd,
                other,
                other_line_indent,
                keep_on_single_line,
            );
        } else if may_align_asterisk_comments && is_asterisk_comment_line(doc, cmd, other) {
            align_asterisk_comment(doc, cmd, other, parameter_indent, assignment_op_indent);
        } else if !is_asterisk_comment_line(doc, cmd, other) {
            let cur_indent = doc.start_x(0, other);
            if cur_indent != other_line_indent {
                let mut add_indent_end = nsib(doc, cmd, other);
                if kw(doc, other, "WHERE")
                    && let Some(open) = add_indent_end
                    && doc.syn.text(open) == "("
                {
                    add_indent_end = doc.syn.bracket_match[open]
                        .filter(|c| *c > open)
                        .and_then(|close| nsib(doc, cmd, close));
                } else if doc.syn.text(other) == ")" {
                    let next = next_tok(cmd, other);
                    if let Some(n) = next
                        && doc.lines[n] == 0
                        && doc.syn.text(n) == ")"
                    {
                        // keep multiple closing parentheses where they are
                        continue;
                    }
                    add_indent_end = next;
                }
                doc.add_indent_range(
                    other,
                    add_indent_end.unwrap_or(cmd.last + 1),
                    other_line_indent - cur_indent,
                    cur_indent,
                    true,
                );
            }
            // adjust the line breaks of the first 'other line', e.g. moving it behind the parent token
            if Some(other) == next_tok(cmd, parent_token) {
                if ts.continue_on_same_line && doc.lines[other] > 0 {
                    set_ws(doc, other, 0, 1);
                } else if !ts.continue_on_same_line && doc.lines[other] == 0 {
                    set_ws(doc, other, 1, other_line_indent);
                }
            }
        }
    }
}

/// `AlignParametersRule.buildAlignTable`; `None` for an unexpected syntax.
fn build_align_table(
    doc: &Doc,
    cmd: &Command,
    table: &mut Table,
    parent_token: usize,
    end: usize,
    ct: Content,
    other_line_starts: &mut Vec<usize>,
) -> Option<()> {
    table.cols[ASSIGNMENT_OP].right_align = true;

    let Some(first) = next_tok(cmd, parent_token) else {
        return Some(());
    };
    let next_code = next_code_tok(doc, cmd, first);

    // a functional call with only an expression, but no actual parameter specified
    if ct == Content::FunctionalCall
        && first_token_allowed(doc, first)
        && !(is_ident(doc, first)
            && next_code.is_some_and(|n| doc.syn.types[n] == TokenType::AssignmentOp))
        && let Some(last) = expression_last(doc, cmd, first)
        && next_code_tok(doc, cmd, last) == Some(end)
    {
        let line = table.add_line();
        table.set_cell(line, EXPRESSION, Cell::new(doc, first, last));
        return Some(());
    }

    let is_receive_results = ct == Content::ProceduralCall
        && doc
            .first_code_token(cmd)
            .is_some_and(|f| kw(doc, f, "RECEIVE"));

    let mut token = Some(first);
    let mut is_in_let_expression = false;
    let mut continue_last_line = false;
    let mut table_ends_with_other_line = false;
    let mut is_in_exceptions = false;

    while let Some(tk) = token.filter(|t| *t != end && *t < cmd.last) {
        let next = tk + 1;
        let prev_sib = prev_code_sib(doc, cmd, tk);
        if (is_ident(doc, tk) || kw(doc, tk, "OTHERS"))
            && !prev_sib.is_some_and(|p| kw(doc, p, "FOR"))
            && (doc.syn.types[next] == TokenType::AssignmentOp
                || ct == Content::ImportOrExport && any_kw(doc, next, &["TO", "FROM"]))
            && next < cmd.last
            && !is_comment(doc, next + 1)
        {
            let parameter = tk;
            let mut keyword = prev_code_tok(doc, cmd, parameter);
            if let Some(k) = keyword {
                if matches!(ct, Content::GroupKey | Content::ImportOrExport)
                    || !is_keyword(doc, k)
                    || Some(k) != prev_code_sib(doc, cmd, parameter)
                {
                    keyword = None;
                } else if any_kw(
                    doc,
                    k,
                    &[
                        "EXPORTING",
                        "IMPORTING",
                        "TABLES",
                        "CHANGING",
                        "RECEIVING",
                        "EXCEPTIONS",
                    ],
                ) {
                    is_in_exceptions = kw(doc, k, "EXCEPTIONS");
                } else if kw(doc, k, "COMPONENTS") {
                    // keep keyword
                } else if kw(doc, k, "LET") {
                    is_in_let_expression = true;
                } else {
                    keyword = None;
                }
            }

            let assignment_op = next_code_sib(doc, cmd, parameter)?;
            let expr_start = assignment_op + 1;
            let mut expression: (usize, usize);
            if ct == Content::GroupKey
                && expr_start <= cmd.last
                && matches_seq(doc, expr_start, cmd.last, &["GROUP", "SIZE|INDEX"])
            {
                expression = (expr_start, next_code_sib(doc, cmd, expr_start)?);
            } else {
                let f = next_code_tok(doc, cmd, assignment_op)?;
                expression = (f, expression_last(doc, cmd, f)?);
            }
            if is_receive_results
                && is_in_exceptions
                && text_any(doc, parameter, &["system_failure", "communication_failure"])
                && let Some(n) = next_code_sib(doc, cmd, expression.1)
                && kw(doc, n, "MESSAGE")
                && let Some(message_identifier) = next_code_sib(doc, cmd, n)
            {
                expression = (assignment_op + 1, message_identifier);
            }

            let line = if continue_last_line {
                table.lines.len() - 1
            } else {
                table.add_line()
            };
            if let Some(k) = keyword {
                let col = if is_in_let_expression {
                    LET_KEYWORD
                } else {
                    KEYWORD
                };
                table.set_cell(line, col, Cell::new(doc, k, k));
            }
            let (pc, ac, ec) = if is_in_let_expression {
                (LET_PARAMETER, LET_ASSIGNMENT_OP, LET_EXPRESSION)
            } else {
                (PARAMETER, ASSIGNMENT_OP, EXPRESSION)
            };
            table.set_cell(line, pc, Cell::new(doc, parameter, parameter));
            table.set_cell(line, ac, Cell::new(doc, assignment_op, assignment_op));
            table.set_cell(line, ec, Cell::new(doc, expression.0, expression.1));
            continue_last_line = false;
            table_ends_with_other_line = false;
            if keyword.is_some() && other_line_starts.last().copied() == keyword {
                other_line_starts.pop();
            }
            token = next_tok(cmd, expression.1);
        } else if is_in_let_expression && kw(doc, tk, "IN") {
            is_in_let_expression = false;
            if doc.lines[tk] > 0 {
                // "IN" at line start is put to the keyword column of the following content
                let line = table.add_line();
                table.set_cell(line, LET_IN_KEYWORD, Cell::new(doc, tk, tk));
                table.add_width_to_end(doc, line, 1);
                continue_last_line = true;
                table_ends_with_other_line = false;
            }
            token = next_tok(cmd, tk);
        } else if ct == Content::RowInValueOrNew
            && (is_ident(doc, tk) || doc.syn.types[tk] == TokenType::Literal)
        {
            let last = expression_last(doc, cmd, tk)?;
            let line = if continue_last_line {
                table.lines.len() - 1
            } else {
                table.add_line()
            };
            table.set_cell(line, PARAMETER, Cell::special(doc, tk, last, 0, true));
            continue_last_line = false;
            table_ends_with_other_line = false;
            token = next_tok(cmd, last);
        } else {
            if Some(tk) == next_tok(cmd, parent_token) || doc.lines[tk] > 0 {
                if is_pseudo_comment_after_code(doc, cmd, tk) {
                    // avoid moving pseudo comments to the next line
                } else if is_comment_after_code(doc, cmd, tk)
                    && !matches!(
                        ct,
                        Content::ConstructorExpr
                            | Content::RowInValueOrNew
                            | Content::GroupKey
                            | Content::ImportOrExport
                    )
                {
                    // line-end comments after method calls usually do not refer to the parameters
                } else {
                    other_line_starts.push(tk);
                    table_ends_with_other_line = true;
                }
            } else if doc.syn.text(tk) == "("
                && doc.lines[tk] == 0
                && prev_sib.is_some_and(|p| kw(doc, p, "WHERE"))
            {
                // logical expression in parentheses after WHERE, not a table row
            } else if doc.syn.text(tk) == "("
                && doc.syn.parent[tk].is_some_and(|p| doc.syn.text(p) == "(")
            {
                // too deep nesting to be a row in a VALUE or NEW constructor
            } else if doc.syn.text(tk) == "(" {
                other_line_starts.push(tk);
                table_ends_with_other_line = true;
            }
            continue_last_line = false;
            token = nsib(doc, cmd, tk);
        }
    }

    if !table_ends_with_other_line {
        // consider the space needed for closing brackets and anything attached to them
        let mut add_width = 0;
        let mut tk = Some(end);
        while let Some(x) = tk {
            if doc.lines[x] != 0 || is_comment(doc, x) || !(closes(doc, x) || doc.spaces[x] == 0) {
                break;
            }
            add_width += i64::from(doc.spaces[x]) + len(doc, x);
            tk = next_tok(cmd, x);
        }
        if add_width > 0 && !table.lines.is_empty() {
            let last_line = table.lines.len() - 1;
            table.add_width_to_end(doc, last_line, add_width);
        }
    }
    Some(())
}

fn set_forced_line_breaks(o: &Opts, table: &mut Table, ct: Content) {
    if !table.column_is_empty(LET_KEYWORD) {
        table.set_force_line_break_after(LET_EXPRESSION, false, None);
        table.set_force_indent(LET_IN_KEYWORD, None, 0);
        return;
    }
    let max_param_count = match ct {
        Content::ProceduralCall => usize::from(o.max_params_procedural > 0),
        Content::FunctionalCall => usize::from(o.max_params_functional > 0),
        _ => 0,
    };
    let own_line = (ct == Content::ProceduralCall && o.keywords_own_line_procedural)
        || (ct == Content::FunctionalCall && o.keywords_own_line_functional);
    if !table.column_is_empty(KEYWORD) && own_line && table.lines.len() > max_param_count {
        table.set_force_line_break_after(KEYWORD, true, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn determine_table_start(
    doc: &Doc,
    o: &Opts,
    cmd: &Command,
    parent_token: usize,
    base_indent: i64,
    minimum_indent: i64,
    ct: Content,
    table: &mut Table,
    other_line_starts: &[usize],
) -> TableStart {
    let has_keywords = !table.column_is_empty(KEYWORD);
    let add_indent = if matches!(ct, Content::RowInValueOrNew | Content::GroupKey) || has_keywords {
        INDENT_STEP
    } else {
        2 * INDENT_STEP
    };
    let first_token_in_table = table.first_token();
    let mut continue_on_same_line = matches!(
        ct,
        Content::ConstructorExpr
            | Content::RowInValueOrNew
            | Content::GroupKey
            | Content::ImportOrExport
    ) || (first_token_in_table.is_some()
        && next_tok(cmd, parent_token) == first_token_in_table)
        || next_tok(cmd, parent_token).is_none_or(|n| doc.lines[n] == 0);

    let start_in_parentheses = end_x(doc, parent_token) + 1;
    let mut start_indent = start_in_parentheses;
    if let Some(ft) = first_token_in_table
        && ((!continue_on_same_line && ft > cmd.first && kw(doc, ft - 1, "WHERE"))
            || kw(doc, ft, "COMPONENTS"))
    {
        start_indent = doc.start_x(0, ft);
    }

    // the total width of the table, as well as the rows in a VALUE or NEW constructor
    let mut table_width = table.total_multi_line_width(doc);
    for &t in other_line_starts {
        if doc.syn.text(t) == "("
            && let Some(close) = doc.syn.bracket_match[t].filter(|c| *c > t)
        {
            let row_width = max_end_index_in_any_line(doc, t, close) - doc.start_x(0, t);
            let on_single_line = (t + 1..=close).all(|j| doc.lines[j] == 0);
            if !on_single_line || minimum_indent + row_width <= o.max_line_length_single {
                table_width = table_width.max(row_width);
            }
        }
    }

    let early_indent = (if base_indent + add_indent + table_width <= o.max_line_length {
        base_indent
    } else {
        minimum_indent
    }) + add_indent;

    let mut force_table_to_next_line = false;
    if start_indent + table_width > o.max_line_length {
        if start_in_parentheses + table_width <= o.max_line_length {
            force_table_to_next_line = true;
            start_indent = start_in_parentheses;
        } else {
            if early_indent < start_indent {
                continue_on_same_line = false;
                start_indent = early_indent;
            }
            if has_keywords && start_indent + table_width > o.max_line_length {
                table.set_force_line_break_after(KEYWORD, true, None);
            }
        }
    }

    let line_count = table.lines.len() as i64;
    let max_count = match ct {
        Content::ProceduralCall => Some(o.max_params_procedural),
        Content::FunctionalCall => Some(o.max_params_functional),
        _ => None,
    };
    if max_count.is_some_and(|m| line_count > m) {
        continue_on_same_line = false;
        start_indent = early_indent;
    }
    TableStart {
        start_indent,
        continue_on_same_line,
        force_table_to_next_line,
        early_indent,
    }
}

fn determine_keep_on_single_line(
    doc: &Doc,
    o: &Opts,
    parent_token: usize,
    end: usize,
    early_indent: i64,
    for_tabular_style: bool,
) -> bool {
    if (parent_token + 1..end).any(|t| doc.lines[t] > 0) {
        return false;
    }
    let keep = if for_tabular_style {
        &o.keep_components
    } else {
        &o.keep_other
    };
    match keep.as_str() {
        "ALWAYS" => true,
        "IF_BELOW_MAX_LINE_LENGTH" => {
            let max = if for_tabular_style {
                o.max_line_length_single
            } else {
                o.max_line_length
            };
            early_indent + (end_x(doc, end) - doc.start_x(0, parent_token)) <= max
        }
        _ => false,
    }
}

/// A table with single components like `VALUE #( ( 1 ) ( 2 ) )` or `VALUE #( ( lo_any ) )`.
fn is_table_of_single_components(doc: &Doc, cmd: &Command, parent_token: usize) -> bool {
    let mut test = next_tok(cmd, parent_token);
    while let Some(t) = test {
        if !matches!(doc.syn.text(t), "(" | ")") {
            return false;
        }
        if has_children(doc, t) {
            let first_child = (t + 1..).find(|j| is_code(doc, *j));
            if first_child.is_some_and(|f| next_code_sib(doc, cmd, f).is_some()) {
                return false;
            }
        }
        test = nsib(doc, cmd, t);
    }
    true
}

fn move_row_in_value_or_new_constructor(
    doc: &mut Doc,
    cmd: &Command,
    opening: usize,
    base_indent: i64,
    keep_on_single_line: bool,
) {
    let old_start = doc.start_x(0, opening);
    let parent = doc.syn.parent[opening];
    let changed;
    if doc.lines[opening] > 0 {
        if prev_tok(cmd, opening) == parent
            && parent.is_some_and(|p| end_x(doc, p) + 1 == base_indent)
        {
            changed = set_ws(doc, opening, 0, 1);
        } else if i64::from(doc.spaces[opening]) != base_indent {
            doc.spaces[opening] = base_indent.max(0) as u32;
            changed = true;
        } else {
            changed = false;
        }
    } else if opening > cmd.first {
        let spaces_left = base_indent - end_x(doc, opening - 1);
        changed = if spaces_left > 0 {
            set_ws(doc, opening, 0, spaces_left)
        } else if keep_on_single_line {
            set_ws(doc, opening, 0, 1)
        } else {
            set_ws(doc, opening, 1, base_indent)
        };
    } else {
        changed = false;
    }
    if changed && let Some(close) = doc.syn.bracket_match[opening].filter(|c| *c > opening) {
        let new_start = doc.start_x(0, opening);
        doc.add_indent_range(opening + 1, close, new_start - old_start, old_start, false);
    }
}
