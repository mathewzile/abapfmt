//! ALIGN_SELECT_LISTS: lays out the field lists of SELECT, FIELDS, GROUP BY, ORDER BY and
//! INTO (...) (port of `AlignSelectListsRule`).
use abapfmt_syntax::TokenType;

use crate::align::{Cell, Table, condense_up_to, end_x, is_comment, is_comment_line};
use crate::rules::align_declarations::{matches_seq, next_non_comment, next_sibling};
use crate::rules::align_select_clauses::set_term_token_whitespace;
use crate::rules::align_with_second_word::is_abap_sql_operation;
use crate::rules::select_query::{Clause, Query, Term, create_query_chains, next_code_sibling};
use crate::{ConfigExt, Doc, RuleConfig};

const FIELD: usize = 0;
const ADDITIONS: usize = 1;

const CONSTRUCTORS: &[&str] = &[
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
const BINARY_OPS: &[&str] = &[
    "+", "-", "*", "/", "DIV", "MOD", "**", "BIT-AND", "BIT-OR", "BIT-XOR", "&&", "&",
];
const WIN_FUNCTIONS: &[&str] = &[
    "AVG(",
    "CORR(",
    "CORR_SPEARMAN(",
    "COUNT(",
    "DENSE_RANK(",
    "FIRST_VALUE(",
    "LAG(",
    "LAST_VALUE(",
    "LEAD(",
    "MAX(",
    "MEDIAN(",
    "MIN(",
    "NTILE(",
    "RANK(",
    "ROW_NUMBER(",
    "STDDEV(",
    "SUM(",
    "VAR(",
];

struct Opts {
    max_line_length: i64,
    consider_tilde_as_complex: bool,
    align_as_in_select_list: bool,
    align_additions_in_order_by: bool,
}

fn opens_level(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t)
}

fn is_kw(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.types[t] == TokenType::Keyword && doc.syn.text(t).eq_ignore_ascii_case(s)
}

fn is_any_kw(doc: &Doc, t: usize, set: &[&str]) -> bool {
    set.iter().any(|s| is_kw(doc, t, s))
}

fn is_attached(doc: &Doc, t: usize) -> bool {
    doc.lines[t] == 0 && doc.spaces[t] == 0
}

fn prev_code(doc: &Doc, t: usize) -> usize {
    (0..t).rev().find(|j| !is_comment(doc, *j)).unwrap_or(0)
}

fn has_children(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t + 1)
}

fn first_code_child(doc: &Doc, t: usize) -> Option<usize> {
    let close = doc.syn.bracket_match[t].filter(|c| *c > t)?;
    (t + 1..close).find(|j| !is_comment(doc, *j))
}

pub fn align_select_lists(doc: &mut Doc, config: &RuleConfig) {
    let o = Opts {
        max_line_length: config.int("MaxLineLength", 120),
        consider_tilde_as_complex: config.bool("ConsiderTildeAsComplex", true),
        align_as_in_select_list: config.bool("AlignAsInSelectList", true),
        align_additions_in_order_by: config.bool("AlignAdditionsInOrderByList", false),
    };
    let complex_select = config.choice("ComplexSelectListLayout", "MULTI_LINE");
    let simple_select = config.choice("SimpleSelectListLayout", "DERIVE");
    let complex_group = config.choice("ComplexGroupByListLayout", "MULTI_LINE");
    let simple_group = config.choice("SimpleGroupByListLayout", "DERIVE");
    let complex_order = config.choice("ComplexOrderByListLayout", "MULTI_LINE");
    let simple_order = config.choice("SimpleOrderByListLayout", "DERIVE");
    let into_layout = config.choice("SelectIntoLayout", "DERIVE");

    for cmd in doc.commands().to_vec() {
        if cmd.is_comment_only {
            continue;
        }
        let Some(first) = doc.first_code_token(&cmd) else {
            continue;
        };
        if !is_abap_sql_operation(doc, &cmd, first) {
            continue;
        }
        let (arena, chains) = create_query_chains(doc, &cmd);
        for start in chains {
            let mut q = Some(start);
            while let Some(i) = q {
                let query = &arena[i];
                q = query.next;
                if query.is_one_liner(doc) {
                    continue;
                }
                let last = cmd.last;
                for (clause, complex, simple) in [
                    (Clause::Select, &complex_select, &simple_select),
                    (Clause::Fields, &complex_select, &simple_select),
                    (Clause::GroupBy, &complex_group, &simple_group),
                    (Clause::OrderBy, &complex_order, &simple_order),
                    (Clause::Into, &into_layout, &into_layout),
                ] {
                    execute_clause(doc, &o, query, clause, complex, simple, last);
                }
            }
        }
    }
}

fn opens_dynamic_syntax(doc: &Doc, t: usize, clause: Term) -> bool {
    if doc.syn.text(t) != "(" || !has_children(doc, t) {
        return false;
    }
    let next = t + 1;
    let next2 = t + 2;
    next2 < doc.syn.tokens.len()
        && is_attached(doc, next)
        && doc.syn.text(next2) == ")"
        && next2 == clause.last
}

fn execute_clause(
    doc: &mut Doc,
    o: &Opts,
    query: &Query,
    clause_type: Clause,
    complex_layout: &str,
    simple_layout: &str,
    last: usize,
) {
    if complex_layout == "KEEP_AS_IS" && simple_layout == "KEEP_AS_IS" {
        return;
    }
    let Some(clause) = query.clause(clause_type) else {
        return;
    };
    let start;
    match clause_type {
        Clause::Select if !query.has_select_list_in_select_clause(doc, last) => return,
        Clause::Select | Clause::Fields => {
            let Some(mut s) = next_code_sibling(doc, clause.first, last) else {
                return;
            };
            while is_any_kw(doc, s, &["DISTINCT", "SINGLE", "FOR", "UPDATE"]) {
                let Some(n) = next_code_sibling(doc, s, last) else {
                    return;
                };
                s = n;
            }
            if opens_dynamic_syntax(doc, s, clause) || doc.syn.text(s) == "*" && s == clause.last {
                return;
            }
            start = s;
        }
        Clause::GroupBy | Clause::OrderBy => {
            let Some(s) = next_code_sibling(doc, clause.first, last)
                .and_then(|n| next_code_sibling(doc, n, last))
            else {
                return;
            };
            if opens_dynamic_syntax(doc, s, clause) {
                return;
            }
            if clause_type == Clause::OrderBy && matches_seq(doc, s, last, &["PRIMARY", "KEY"]) {
                return;
            }
            start = s;
        }
        Clause::Into => {
            let Some(s) = next_code_sibling(doc, clause.first, last) else {
                return;
            };
            // skip everything except 'INTO (elem1, elem2,  ...)'
            if doc.syn.text(s) == "("
                && first_code_child(doc, s).is_some()
                && next_sibling(doc, s, last) == Some(clause.last)
            {
                start = first_code_child(doc, s).unwrap();
            } else {
                return;
            }
        }
        _ => return,
    }

    let has_comma = if clause_type == Clause::Into {
        let open = next_code_sibling(doc, clause.first, last).unwrap();
        first_code_child(doc, open).is_some_and(|c| {
            let mut t = Some(c);
            while let Some(x) = t {
                if doc.syn.types[x] == TokenType::Comma {
                    return true;
                }
                t = next_code_sibling(doc, x, last);
            }
            false
        })
    } else {
        let mut t = next_code_sibling(doc, clause.first, last);
        let mut found = false;
        while let Some(x) = t.filter(|x| *x <= clause.last) {
            if doc.syn.types[x] == TokenType::Comma {
                found = true;
                break;
            }
            t = next_sibling(doc, x, last);
        }
        found
    };

    let mut table = Table::new(2);
    if has_comma || clause_type == Clause::Fields {
        build_comma_sep_list(doc, &mut table, clause, start, last);
    } else {
        build_space_sep_list(doc, &mut table, clause_type, clause, start, last);
    }
    if table.lines.is_empty() {
        return;
    }
    let mut layout = determine_layout(doc, o, complex_layout, simple_layout, &table);
    if layout == "KEEP_AS_IS" {
        return;
    }
    if layout == "DERIVE" {
        // derive MULTI_LINE or ONE_LINE from the majority of fields in the current layout
        let mut line_break_count = 0;
        let mut continue_count = 0;
        for line in table.lines.iter().skip(1) {
            if doc.lines[line.first_token().unwrap()] > 0 {
                line_break_count += 1;
            } else {
                continue_count += 1;
            }
        }
        layout = if line_break_count > continue_count {
            "MULTI_LINE"
        } else {
            "ONE_LINE"
        }
        .to_string();
    }
    // the opening parenthesis of INTO (...) continues the line after INTO
    if clause_type == Clause::Into {
        let open = doc.syn.parent[start].unwrap();
        let close = doc.syn.bracket_match[open].unwrap();
        if !is_comment(doc, open - 1) {
            doc.lines[open] = 0;
            doc.spaces[open] = 1;
        }
        if is_attached(doc, start) {
            doc.spaces[start] = 1;
        }
        if is_attached(doc, close) {
            doc.spaces[close] = 1;
        }
    } else if !is_comment(doc, start - 1) {
        doc.lines[start] = 0;
        doc.spaces[start] = 1;
    }
    // layout the field list
    let basic_indent = end_x(doc, prev_code(doc, start)) + 1;
    if layout == "ONE_LINE" {
        condense_up_to(
            doc,
            clause.first,
            clause.last,
            o.max_line_length,
            basic_indent,
            true,
        );
    } else {
        // if FROM or INTO continue after a SELECT list that is split, move it to the next line
        if clause_type == Clause::Select && table.lines.len() > 1 {
            let order = query.clause_types_in_order();
            let pos = order.iter().position(|c| *c == clause_type);
            let next_of = |c: Option<Clause>| -> Option<Clause> {
                let i = order.iter().position(|x| Some(*x) == c)?;
                order.get(i + 1).copied()
            };
            let next1 = pos.and_then(|i| order.get(i + 1).copied());
            let next2 = next_of(next1);
            if let (Some(n1), Some(n2)) = (next1, next2)
                && matches!(n1, Clause::From | Clause::Into)
                && n2 != Clause::Union
            {
                let c1 = query.clause(n1).unwrap();
                let c2 = query.clause(n2).unwrap();
                if doc.lines[c1.first] == 0 && doc.lines[c2.first] > 0 {
                    let sp = i64::from(doc.spaces[c2.first]);
                    set_term_token_whitespace(doc, c1, c1.first, 1, sp);
                }
            }
        }
        let align_additions = if matches!(clause_type, Clause::Select | Clause::Fields) {
            o.align_as_in_select_list
        } else {
            o.align_additions_in_order_by
        };
        if !align_additions {
            table.join_into_previous_columns_ex(doc, ADDITIONS, true, true);
        }
        table.align(doc, basic_indent, 0, true, true);
    }

    // align comments
    let mut token = start;
    while token > 0 && is_comment_line(doc, token - 1) {
        token -= 1;
    }
    // move the first comment behind the keyword(s) that start the clause, if that fits the indent
    if is_comment_line(doc, token)
        && doc.syn.text(token).starts_with('"')
        && token > 0
        && doc.syn.types[token - 1] == TokenType::Keyword
        && end_x(doc, token - 1) + 1 == basic_indent
    {
        doc.lines[token] = 0;
        doc.spaces[token] = 1;
        token += 1;
    }
    // align all other comments of this clause
    let mut t = Some(token);
    while let Some(x) = t {
        if is_comment_line(doc, x) {
            indent_comment(doc, x, basic_indent);
        }
        if x == clause.last {
            break;
        }
        t = next_sibling(doc, x, last);
    }
}

fn indent_comment(doc: &mut Doc, t: usize, basic_indent: i64) {
    let text = doc.token_text(t).to_string();
    if text.starts_with('"') {
        doc.spaces[t] = basic_indent.max(0) as u32;
    } else if text.starts_with('*') && text.chars().count() > 1 {
        let chars: Vec<char> = text.chars().collect();
        let Some(sign_end) = chars.iter().position(|c| *c != '*') else {
            return;
        };
        let Some(rel) = chars[sign_end..].iter().position(|c| *c != ' ') else {
            return;
        };
        let text_pos = sign_end + rel;
        if text_pos == 0 {
            return;
        }
        let new_text: String = chars[..sign_end]
            .iter()
            .chain(std::iter::repeat_n(&' ', basic_indent.max(0) as usize))
            .chain(chars[text_pos..].iter())
            .collect();
        if new_text != text {
            doc.text[t] = Some(new_text);
        }
    }
}

fn determine_layout(doc: &Doc, o: &Opts, complex: &str, simple: &str, table: &Table) -> String {
    if complex == simple {
        return complex.to_string();
    }
    let mut has_tilde = false;
    let mut multi_token = false;
    let mut has_additions = false;
    for line in &table.lines {
        let cell = line.cells[FIELD].as_ref().unwrap();
        if cell.first != cell.last {
            multi_token = true;
        } else if doc.token_text(cell.first).contains('~') {
            has_tilde = true;
        }
        if line.cells[ADDITIONS].is_some() {
            has_additions = true;
        }
    }
    let is_complex = multi_token || has_additions || (has_tilde && o.consider_tilde_as_complex);
    if is_complex { complex } else { simple }.to_string()
}

fn build_comma_sep_list(doc: &mut Doc, table: &mut Table, clause: Term, start: usize, last: usize) {
    let mut token = Some(start);
    while let Some(first_in_term) = token {
        let mut last_in_term = first_in_term;
        let mut t = Some(first_in_term);
        while let Some(x) = t {
            if doc.syn.types[x] == TokenType::Comma
                || is_any_kw(doc, x, &["AS", "ASCENDING", "DESCENDING", "NULLS"])
            {
                break;
            }
            last_in_term = x;
            if x == clause.last {
                t = None;
                break;
            }
            t = next_code_sibling(doc, x, last);
        }
        token = t;
        let line = table.add_line();
        table.set_cell(line, FIELD, Cell::new(doc, first_in_term, last_in_term));
        let Some(mut x) = token else { break };
        // possible additions behind the field
        if doc.syn.types[x] != TokenType::Comma {
            let first_add = x;
            let mut last_add = x;
            let mut t = Some(x);
            while let Some(y) = t {
                if doc.syn.types[y] == TokenType::Comma {
                    break;
                }
                last_add = y;
                if y == clause.last {
                    t = None;
                    break;
                }
                t = next_code_sibling(doc, y, last);
            }
            table.set_cell(line, ADDITIONS, Cell::new(doc, first_add, last_add));
            let Some(n) = t else { break };
            x = n;
        }
        if doc.syn.types[x] == TokenType::Comma && !is_attached(doc, x) && doc.lines[x] == 0 {
            doc.spaces[x] = 0;
        }
        token = next_code_sibling(doc, x, last);
    }
}

/// `Term.createArithmetic(first).lastToken`
fn arithmetic_last(doc: &Doc, first: usize, last: usize) -> usize {
    let skip_levels = |mut t: usize| {
        while let Some(c) = doc.syn.bracket_match[t].filter(|c| *c > t) {
            t = c;
        }
        t
    };
    let mut token = first;
    if is_any_kw(doc, first, CONSTRUCTORS) {
        token = next_non_comment(doc, token, last).unwrap_or(token);
        token = skip_levels(token);
    } else if is_kw(doc, first, "CASE") {
        let mut depth = 1;
        while depth > 0 {
            let Some(n) = next_code_sibling(doc, token, last) else {
                break;
            };
            token = n;
            if is_kw(doc, token, "CASE") {
                depth += 1;
            } else if is_kw(doc, token, "END") {
                depth -= 1;
            }
        }
    } else {
        token = skip_levels(token);
    }
    while let Some(n) = next_non_comment(doc, token, last)
        && BINARY_OPS
            .iter()
            .any(|b| doc.syn.text(n).eq_ignore_ascii_case(b))
    {
        let Some(m) = next_non_comment(doc, n, last) else {
            break;
        };
        token = m;
        if is_any_kw(doc, token, CONSTRUCTORS) {
            token = next_non_comment(doc, token, last).unwrap_or(token);
        }
        token = skip_levels(token);
    }
    token
}

fn build_space_sep_list(
    doc: &mut Doc,
    table: &mut Table,
    clause_type: Clause,
    clause: Term,
    start: usize,
    last: usize,
) {
    let mut token = Some(start);
    while let Some(t) = token {
        // determine the next field
        let field_last;
        if clause_type == Clause::GroupBy && matches_seq(doc, t, last, &["GROUPING", "SETS|SETS("])
        {
            // GROUPING SETS ( ... )
            let mut x = next_sibling(doc, t, last).unwrap_or(t);
            while !opens_level(doc, x) && x < clause.last {
                x += 1;
            }
            field_last = doc.syn.bracket_match[x].filter(|c| *c > x).unwrap_or(x);
        } else {
            let mut fl = None;
            // window expressions: 'win_func OVER( ... )'
            if opens_level(doc, t)
                && WIN_FUNCTIONS
                    .iter()
                    .any(|w| doc.syn.text(t).eq_ignore_ascii_case(w))
                && let Some(over) =
                    next_code_sibling(doc, t, last).and_then(|n| next_code_sibling(doc, n, last))
                && doc.syn.text(over).eq_ignore_ascii_case("OVER(")
                && let Some(close) = next_code_sibling(doc, over, last)
            {
                fl = Some(close);
            }
            field_last = fl.unwrap_or_else(|| arithmetic_last(doc, t, last));
        }
        let line = table.add_line();
        table.set_cell(line, FIELD, Cell::new(doc, t, field_last));
        if field_last == clause.last {
            break;
        }
        // possible additions behind the field
        let Some(x) = next_code_sibling(doc, field_last, last) else {
            break;
        };
        let mut additions: Option<(usize, usize)> = None;
        if is_kw(doc, x, "AS") {
            let Some(n) = next_code_sibling(doc, x, last) else {
                break;
            };
            additions = Some((x, n));
        } else if is_any_kw(doc, x, &["ASCENDING", "DESCENDING", "NULLS"]) {
            let mut last_add = x;
            let mut cur = Some(x);
            while let Some(y) = cur.filter(|y| {
                is_any_kw(
                    doc,
                    *y,
                    &["ASCENDING", "DESCENDING", "NULLS", "FIRST", "LAST"],
                )
            }) {
                last_add = y;
                cur = next_code_sibling(doc, y, last);
            }
            additions = Some((x, last_add));
        }
        if let Some((a, b)) = additions {
            table.set_cell(line, ADDITIONS, Cell::new(doc, a, b));
            if b == clause.last {
                break;
            }
            token = next_code_sibling(doc, b, last);
        } else {
            token = Some(x);
        }
    }
}
