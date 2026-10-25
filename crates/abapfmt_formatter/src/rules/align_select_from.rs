//! ALIGN_SELECT_FROM: aligns the FROM clause of ABAP SQL queries with joins (port of
//! `AlignSelectFromRule`).
use crate::align::{Cell, Table, is_comment};
use crate::rules::align_declarations::{
    matches_seq, next_non_comment, next_non_comment_sibling, term_last,
};
use crate::rules::align_select_clauses::condense_term;
use crate::rules::align_with_second_word::is_abap_sql_operation;
use crate::rules::select_query::{
    Clause, Query, Term, create_query_chains, next_code_sibling, sql_condition_end,
};
use crate::{ConfigExt, Doc, RuleConfig};
use abapfmt_syntax::TokenType;

const JOIN: usize = 0;
const TABLE_NAME: usize = 1;
const AS: usize = 2;
const ON: usize = 3;
const MAX_COLUMN_COUNT: usize = 4;

struct Opts {
    first_table_pos: String,
    first_table_add: i64,
    join_indent: i64,
    further_table_pos: String,
    further_table_add: i64,
    on_position: String,
    align_as_across: bool,
    align_on_across: bool,
    client_position: String,
}

fn add_of(pos: &str) -> i64 {
    match pos {
        "BELOW_PLUS_2" | "PLUS_2" => 2,
        "PLUS_4" => 4,
        _ => 0,
    }
}

pub fn align_select_from(doc: &mut Doc, config: &RuleConfig) {
    let first = config.choice("FirstTableNamePosition", "CONTINUE");
    let further = config.choice("FurtherTableNamePositions", "BELOW_PLUS_2");
    let o = Opts {
        first_table_add: add_of(&first),
        first_table_pos: first,
        join_indent: add_of(&config.choice("SelectJoinIndent", "PLUS_2")),
        further_table_add: add_of(&further),
        further_table_pos: further,
        on_position: config.choice("SelectOnPosition", "CONTINUE"),
        align_as_across: config.bool("AlignAsAcrossJoins", false),
        align_on_across: config.bool("AlignOnAcrossJoins", false),
        client_position: config.choice("SelectClientPosition", "OWN_LINE_AFTER_JOINS"),
    };
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
                if !arena[i].is_one_liner(doc) {
                    execute_on(doc, &o, &arena[i], cmd.last);
                }
                q = arena[i].next;
            }
        }
    }
}

fn first_table_name_position(doc: &mut Doc, o: &Opts, from: usize, name_start: usize) {
    let (lines, spaces);
    if o.first_table_pos != "CONTINUE" {
        lines = 1;
        spaces = doc.start_x(0, from) + o.first_table_add;
    } else if is_comment(doc, name_start - 1) {
        lines = 1;
        spaces = crate::align::end_x(doc, from) + 1;
    } else {
        lines = 0;
        spaces = 1;
    }
    doc.lines[name_start] = lines;
    doc.spaces[name_start] = spaces.max(0) as u32;
}

fn calculate_add_indent(o: &Opts, doc: &Doc, table: &Table, line: usize, column: usize) -> i64 {
    let per_join = o.join_indent;
    let per_name = if o.further_table_pos == "CONTINUE" {
        0
    } else {
        o.further_table_add
    };
    let further_count = if line == table.lines.len() - 1 {
        table.lines.len() as i64 - 1
    } else {
        line as i64
    };
    let name_indent = further_count * (per_join + per_name);
    match column {
        JOIN => {
            if table.lines.len() == 1 {
                0
            } else {
                name_indent - per_name
            }
        }
        TABLE_NAME => {
            if o.further_table_pos == "CONTINUE" {
                0
            } else {
                name_indent
            }
        }
        _ => {
            if o.on_position == "CONTINUE" {
                return 0;
            }
            let add = match o.on_position.as_str() {
                "BELOW_JOIN_PLUS_2" => 2,
                "BELOW_JOIN_WORD_2" => table.lines[line].cells[JOIN]
                    .as_ref()
                    .map_or(0, |c| doc.token_text(c.first).chars().count() as i64 + 1),
                _ => 0,
            };
            name_indent + add
        }
    }
}

fn are_as_and_on_empty(table: &Table, line: usize) -> bool {
    table.lines[line].cells[AS].is_none() && table.lines[line].cells[ON].is_none()
}

fn prev_code(doc: &Doc, t: usize) -> usize {
    (0..t).rev().find(|j| !is_comment(doc, *j)).unwrap_or(0)
}

fn opens_level(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t)
}

fn enhance_table_name_up_to(doc: &Doc, o: &Opts, table: &mut Table, line: usize, token: usize) {
    if !are_as_and_on_empty(table, line) {
        return;
    }
    let cell = table.lines[line].cells[TABLE_NAME].clone().unwrap();
    let pc = prev_code(doc, token);
    if cell.first != token && cell.last != pc {
        let add = calculate_add_indent(o, doc, table, line, TABLE_NAME);
        table.set_cell(
            line,
            TABLE_NAME,
            Cell::special(doc, cell.first, pc, add, false),
        );
    }
}

fn enhance_table_name_with_parens(
    doc: &Doc,
    o: &Opts,
    table: &mut Table,
    line: usize,
    open: usize,
) {
    if !are_as_and_on_empty(table, line) {
        return;
    }
    let start = table.lines[line].cells[TABLE_NAME].as_ref().unwrap().first;
    let add = calculate_add_indent(o, doc, table, line, TABLE_NAME);
    table.set_cell(
        line,
        TABLE_NAME,
        Cell::special(doc, start, term_last(doc, open), add, true),
    );
}

fn has_children(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t + 1)
}

fn execute_on(doc: &mut Doc, o: &Opts, query: &Query, last: usize) {
    let Some(from) = query.clause(Clause::From) else {
        return;
    };
    let continue_after_join = o.further_table_pos == "CONTINUE";
    let from_token = from.first;
    let mut all: Vec<Table> = vec![Table::new(MAX_COLUMN_COUNT)];
    let mut level: Vec<usize> = vec![0];
    let mut ti = 0;

    let Some(name_start) = next_non_comment(doc, from_token, last) else {
        return;
    };
    first_table_name_position(doc, o, from_token, name_start);
    let mut line = all[ti].add_line();
    let first_name = Cell::new(doc, name_start, term_last(doc, name_start));
    all[ti].set_cell(line, TABLE_NAME, first_name);
    let mut token = Some(name_start);
    while let Some(mut t) = token {
        let opens_parens = doc.syn.text(t) == "(";
        let starts_join = ["LEFT", "RIGHT", "INNER", "CROSS", "JOIN"].iter().any(|k| {
            doc.syn.types[t] == TokenType::Keyword && doc.syn.text(t).eq_ignore_ascii_case(k)
        });
        let starts_as = doc.syn.types[t] == TokenType::Keyword
            && doc.syn.text(t).eq_ignore_ascii_case("AS")
            && next_code_sibling(doc, t, last)
                .is_some_and(|n| doc.syn.types[n] == TokenType::Identifier);
        let starts_on =
            doc.syn.types[t] == TokenType::Keyword && doc.syn.text(t).eq_ignore_ascii_case("ON");
        let starts_client = matches_seq(doc, t, last, &["CLIENT", "SPECIFIED"])
            || matches_seq(doc, t, last, &["USING", "CLIENT"])
            || matches_seq(doc, t, last, &["USING", "CLIENTS", "IN"])
            || matches_seq(doc, t, last, &["USING", "ALL", "CLIENTS"]);
        if opens_parens || starts_as || starts_join || starts_on || starts_client {
            enhance_table_name_up_to(doc, o, &mut all[ti], line, t);
        }
        if starts_join {
            let mut join_token = Some(t);
            while let Some(j) = join_token.filter(|j| {
                doc.syn.types[*j] == TokenType::Keyword
                    && !doc.syn.text(*j).eq_ignore_ascii_case("JOIN")
            }) {
                join_token = next_non_comment_sibling(doc, j, last);
            }
            if let Some(j) = join_token.filter(|j| doc.syn.text(*j).eq_ignore_ascii_case("JOIN")) {
                line = all[ti].add_line();
                let add = calculate_add_indent(o, doc, &all[ti], line, JOIN);
                let term = Term { first: t, last: j };
                condense_term(doc, term);
                if continue_after_join {
                    let c = Cell::special(doc, t, j, add, false);
                    all[ti].set_cell(line, JOIN, c);
                } else {
                    let ft = all[ti].lines[0].first_token().unwrap();
                    doc.lines[t] = doc.lines[t].max(1);
                    doc.spaces[t] = (doc.start_x(0, ft) + add).max(0) as u32;
                }
                t = j;
                let Some(next) = next_non_comment(doc, t, last) else {
                    break;
                };
                let add = calculate_add_indent(o, doc, &all[ti], line, TABLE_NAME);
                let c = Cell::special(doc, next, term_last(doc, next), add, false);
                all[ti].set_cell(line, TABLE_NAME, c);
                if continue_after_join && !is_comment(doc, next - 1) {
                    doc.lines[next] = 0;
                    doc.spaces[next] = 1;
                }
            }
        } else if starts_as {
            if all[ti].lines[line].cells[AS].is_some() {
                line = all[ti].add_line();
            }
            let n = next_non_comment_sibling(doc, t, last).unwrap();
            all[ti].set_cell(line, AS, Cell::new(doc, t, n));
        } else if starts_on {
            // find the (possibly previous) line which this ON clause belongs to
            let mut line_for_on = None;
            for i in (1..all[ti].lines.len()).rev() {
                if all[ti].lines[i].cells[ON].is_none() {
                    line_for_on = Some(i);
                    break;
                }
            }
            let line_for_on = line_for_on.unwrap_or_else(|| all[ti].add_line());
            if let Some(end) = sql_condition_end(doc, t, last) {
                let add = calculate_add_indent(o, doc, &all[ti], line_for_on, ON);
                let c = Cell::special(doc, t, end, add, false);
                all[ti].set_cell(line_for_on, ON, c);
                t = end;
            }
        } else if starts_client {
            if o.client_position != "KEEP_AS_IS" {
                let own_line = o.client_position == "OWN_LINE"
                    || o.client_position == "OWN_LINE_AFTER_JOINS" && all[ti].lines.len() > 1;
                if own_line {
                    let ft = all[ti].lines[0].first_token().unwrap();
                    doc.lines[t] = doc.lines[t].max(1);
                    doc.spaces[t] = doc.start_x(0, ft).max(0) as u32;
                } else if !is_comment(doc, t - 1) {
                    doc.lines[t] = 0;
                    doc.spaces[t] = 1;
                }
            }
            break;
        }
        if t == from.last {
            break;
        }
        // do NOT move into dynamic cases like 'FROM (from)'
        let move_inside = doc.syn.text(t) == "("
            && has_children(doc, t)
            && (doc.lines[t + 1] > 0 || doc.spaces[t + 1] > 0);
        if opens_level(doc, t) && !move_inside {
            token = Some(term_last(doc, t));
        } else if move_inside {
            // move into one or several parentheses, adding a new table on each level
            let mut cur = t;
            loop {
                enhance_table_name_with_parens(doc, o, &mut all[ti], line, cur);
                all.push(Table::new(MAX_COLUMN_COUNT));
                ti = all.len() - 1;
                level.push(ti);
                let Some(n) = next_non_comment(doc, cur, last) else {
                    return;
                };
                cur = n;
                line = all[ti].add_line();
                let c = Cell::new(doc, cur, term_last(doc, cur));
                all[ti].set_cell(line, TABLE_NAME, c);
                if !(doc.syn.text(cur) == "("
                    && has_children(doc, cur)
                    && (doc.lines[cur + 1] > 0 || doc.spaces[cur + 1] > 0))
                {
                    break;
                }
            }
            token = Some(cur);
        } else if next_code_sibling(doc, t, last).is_none() {
            // move out of a parenthesis, going back to the table of the previous level
            let Some(mut cur) = next_non_comment(doc, t, last) else {
                break;
            };
            loop {
                if level.len() > 1 {
                    level.pop();
                }
                ti = *level.last().unwrap();
                line = all[ti].lines.len() - 1;
                if cur == from.last {
                    break;
                }
                let Some(n) = next_non_comment(doc, cur, last) else {
                    break;
                };
                cur = n;
                if doc.syn.text(cur) != ")" {
                    break;
                }
            }
            if cur == from.last {
                break;
            }
            token = Some(cur);
        } else {
            token = next_code_sibling(doc, t, last);
        }
    }
    align_all_tables(doc, o, continue_after_join, &mut all);
}

fn align_all_tables(doc: &mut Doc, o: &Opts, continue_after_join: bool, all: &mut [Table]) {
    for table in all.iter_mut() {
        if continue_after_join {
            table.join_into_previous_columns_ex(doc, TABLE_NAME, true, true);
        }
        if !o.align_as_across || table.column_count(AS) < 2 {
            table.join_into_previous_columns_ex(doc, AS, true, true);
        }
        if o.on_position == "CONTINUE" {
            if !o.align_on_across {
                table.join_into_previous_columns_ex(doc, ON, true, true);
            }
        } else {
            table.set_force_line_break_after(AS, false, None);
            table.set_force_indent(ON, Some(JOIN), 0);
        }
        let Some(ft) = table.lines.first().and_then(|l| l.first_token()) else {
            continue;
        };
        let indent = doc.start_x(0, ft);
        let lb = doc.lines[ft];
        table.align(doc, indent, lb, true, false);
    }
}
