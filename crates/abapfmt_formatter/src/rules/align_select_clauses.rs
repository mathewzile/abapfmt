//! ALIGN_SELECT_CLAUSES: puts the clauses of ABAP SQL SELECT queries on separate lines (port of
//! `AlignSelectClausesRule`).
use abapfmt_syntax::TokenType;

use crate::align::{condense_up_to, sum_text_and_space_width};
use crate::rules::align_with_second_word::is_abap_sql_operation;
use crate::rules::select_query::{Clause, Query, Term, create_query_chains};
use crate::{ConfigExt, Doc, RuleConfig};

pub(crate) fn plus(config: &RuleConfig, key: &str, default: &str) -> i64 {
    match config.choice(key, default).as_str() {
        "PLUS_2" | "BELOW_PLUS_2" => 2,
        "PLUS_4" | "BELOW_PLUS_4" => 4,
        "PLUS_7" => 7,
        _ => 0,
    }
}

/// `Term.setTokenWhitespace`: sets the whitespace of `t` and moves the following lines of the
/// term accordingly.
pub(crate) fn set_term_token_whitespace(
    doc: &mut Doc,
    term: Term,
    t: usize,
    lines: u32,
    spaces: i64,
) {
    let spaces = spaces.max(0) as u32;
    if doc.lines[t] == lines && doc.spaces[t] == spaces {
        return;
    }
    let old = doc.start_x(0, t);
    doc.lines[t] = lines;
    doc.spaces[t] = spaces;
    if t == term.last {
        return;
    }
    let add = doc.start_x(0, t) - old;
    doc.add_indent(term.last, t + 1, add, old, false);
}

pub(crate) fn condense_term(doc: &mut Doc, term: Term) {
    let x = doc.start_x(0, term.first);
    condense_up_to(doc, term.first, term.last, 255, x, false);
}

struct Opts {
    max_line_length: i64,
    main_action: String,
    sub_action: String,
    max_list_before_from: i64,
    max_list_before_into: i64,
    new_line_for_from_with_joins: bool,
    clause_indent: i64,
    union_indent: i64,
    next_select_pos: String,
    next_select_add: i64,
    final_into_indent: i64,
}

pub fn align_select_clauses(doc: &mut Doc, config: &RuleConfig) {
    let o = Opts {
        max_line_length: config.int("MaxLineLength", 120),
        main_action: config.choice("MainQueryOneLinerAction", "KEEP_EXISTING"),
        sub_action: config.choice("SubQueryOneLinerAction", "KEEP_EXISTING"),
        max_list_before_from: config.int("MaxSelectListLengthBeforeFrom", 30),
        max_list_before_into: config.int("MaxSelectListLengthBeforeInto", 30),
        new_line_for_from_with_joins: config.bool("NewLineForFromWithJoins", true),
        clause_indent: plus(config, "SelectClauseIndent", "PLUS_2"),
        union_indent: plus(config, "SelectUnionIndent", "PLUS_0"),
        next_select_pos: config.choice("NextSelectPos", "BELOW_PLUS_0"),
        next_select_add: plus(config, "NextSelectPos", "BELOW_PLUS_0"),
        final_into_indent: plus(config, "SelectUnionIntoIndent", "PLUS_0"),
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
                execute_on(doc, &o, &arena, i, cmd.last);
                q = arena[i].next;
            }
        }
    }
}

fn execute_on(doc: &mut Doc, o: &Opts, arena: &[Query], qi: usize, cmd_last: usize) {
    let query = &arena[qi];
    let select_clause = query.clause(Clause::Select).unwrap();
    let select_token = select_clause.first;

    // if this is not the start query ...
    if let Some(union_clause) = query.clause(Clause::Union)
        && let Some(prev_i) = query.prev
    {
        let prev_query = &arena[prev_i];
        let mut ref_token = prev_query.select_token();
        // the UNION etc. may refer to multiple SELECTs in parentheses: base the indent on the
        // opening parenthesis in that case
        let prev_code = (0..union_clause.first)
            .rev()
            .find(|j| doc.syn.types[*j] != TokenType::Comment);
        if let Some(pc) = prev_code
            && doc.syn.text(pc) == ")"
        {
            let mut test = doc.syn.parent[ref_token];
            while let Some(p) = test {
                if doc.syn.bracket_match[p] == Some(pc) {
                    ref_token = p;
                    break;
                }
                test = doc.syn.parent[p];
            }
        }
        let prev_select_indent = doc.start_x(0, ref_token);
        let line_breaks = doc.lines[union_clause.first].max(1);
        let spaces_left = prev_select_indent + o.union_indent;
        set_term_token_whitespace(
            doc,
            union_clause,
            union_clause.first,
            line_breaks,
            spaces_left,
        );
        condense_term(doc, union_clause);
        // if SELECT is preceded by one or multiple "(", find the first "("
        let mut change_token = select_token;
        loop {
            if change_token == 0 {
                break;
            }
            let prev = change_token - 1;
            if doc.syn.text(prev) != "(" {
                break;
            }
            if change_token == select_token {
                set_term_token_whitespace(doc, select_clause, select_clause.first, 0, 1);
            } else {
                doc.lines[change_token] = 0;
                doc.spaces[change_token] = 1;
            }
            change_token = prev;
        }
        let union_is_single_word = union_clause.first == union_clause.last;
        let can_continue =
            change_token == 0 || doc.syn.types[change_token - 1] != TokenType::Comment;
        let pos = o.next_select_pos.as_str();
        let (line_breaks, spaces_left);
        let prev_select_x = doc.start_x(0, prev_query.select_token());
        if can_continue
            && (pos == "CONTINUE" || union_is_single_word && pos == "CONTINUE_OR_BELOW_SECOND")
        {
            line_breaks = 0;
            spaces_left = 1;
        } else if matches!(
            pos,
            "CONTINUE" | "CONTINUE_OR_BELOW_SECOND" | "BELOW_SECOND_WORD"
        ) {
            line_breaks = 1;
            spaces_left = prev_select_x
                + o.union_indent
                + doc.token_text(union_clause.first).chars().count() as i64
                + 1;
        } else {
            line_breaks = doc.lines[change_token].max(1);
            spaces_left = prev_select_x + o.union_indent + o.next_select_add;
        }
        if change_token == select_token {
            set_term_token_whitespace(
                doc,
                select_clause,
                select_clause.first,
                line_breaks,
                spaces_left,
            );
        } else {
            doc.lines[change_token] = line_breaks;
            doc.spaces[change_token] = spaces_left.max(0) as u32;
        }
    }

    // create or keep one-liners
    let action = if query.is_main_query(doc) {
        &o.main_action
    } else {
        &o.sub_action
    };
    if action != "ALWAYS_SPLIT" {
        let is_one_liner = query.is_one_liner(doc);
        if is_one_liner || action == "CREATE" {
            let length = query.length_on_one_line(doc);
            if length > 0 && doc.start_x(0, select_token) + length <= o.max_line_length {
                query.condense(doc);
                return;
            }
        }
    }

    // position other clauses
    let use_final_into_indent = query.is_main_query(doc) && query.next.is_some();
    for clause_type in query.clause_types_in_order() {
        if clause_type == Clause::Union || clause_type == Clause::Select {
            continue;
        }
        let select_clause = query.clause(Clause::Select).unwrap();
        let clause = query.clause(clause_type).unwrap();
        let is_simple = clause_type == Clause::From && !query.has_join_or_parameters(doc, cmd_last)
            || clause_type == Clause::Into && !query.has_into_list(doc, cmd_last);
        if is_simple
            && select_clause.is_on_single_line(doc)
            && clause.is_on_single_line(doc)
            && select_clause.last + 1 == clause.first
        {
            let select_width =
                sum_text_and_space_width(doc, select_clause.first, select_clause.last);
            let total = doc.start_x(0, select_clause.first)
                + select_width
                + 1
                + sum_text_and_space_width(doc, clause.first, clause.last);
            let max_list = if clause_type == Clause::From {
                o.max_list_before_from
            } else {
                o.max_list_before_into
            };
            if select_width <= max_list && total <= o.max_line_length {
                condense_term(doc, select_clause);
                condense_term(doc, clause);
                doc.lines[clause.first] = 0;
                doc.spaces[clause.first] = 1;
                continue;
            }
        } else if clause_type == Clause::From && doc.lines[clause.first] == 0 {
            // keep simple FROM behind SELECT
            if !o.new_line_for_from_with_joins {
                continue;
            } else if clause.first > 0
                && doc.syn.types[clause.first - 1] == TokenType::Keyword
                && doc
                    .syn
                    .text(clause.first - 1)
                    .eq_ignore_ascii_case("SELECT")
                && o.clause_indent == 7
            {
                continue;
            }
        }
        // determine indent for clause
        let line_breaks = doc.lines[clause.first].max(1);
        let mut spaces_left = doc.start_x(0, select_token);
        if use_final_into_indent
            && matches!(
                clause_type,
                Clause::Into | Clause::UpToOffset | Clause::AbapOptions
            )
        {
            spaces_left += o.final_into_indent;
        } else {
            spaces_left += o.clause_indent;
        }
        set_term_token_whitespace(doc, clause, clause.first, line_breaks, spaces_left);

        // continue after WHERE and INTO
        if clause.first < clause.last {
            let next = clause.first + 1;
            let is_code = doc.syn.types[next] != TokenType::Comment;
            if clause_type == Clause::Where && is_code && doc.lines[next] > 0 {
                set_term_token_whitespace(doc, clause, next, 0, 1);
            } else if clause_type == Clause::Into
                && is_code
                && (doc.lines[next] > 0 || doc.spaces[next] > 1)
            {
                set_term_token_whitespace(doc, clause, next, 0, 1);
            }
        }
    }
}
