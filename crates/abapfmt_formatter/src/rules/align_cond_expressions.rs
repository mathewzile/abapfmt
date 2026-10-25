//! ALIGN_COND_EXPRESSIONS: aligns the WHEN / THEN / ELSE parts of COND and SWITCH expressions.
use crate::align::{Cell, Table, is_comment};
use crate::rules::align_declarations::{is_kw, next_non_comment_sibling};
use crate::{ConfigExt, Doc, RuleConfig};

const LET_EXPR: usize = 0;
const OPERAND: usize = 1;
const WHEN: usize = 2;
const CONDITION: usize = 3;
const THEN: usize = 4;
const THEN_VALUE: usize = 5;
const ELSE: usize = 6;
const ELSE_VALUE: usize = 7;
const MAX_COLUMN_COUNT: usize = 8;

pub fn align_cond_expressions(doc: &mut Doc, config: &RuleConfig) {
    let max_line_length = config.int("MaxLineLength", 120);
    let one_liner = config.choice("OneLinerStyle", "KEEP");
    let simple = config.choice("SimpleStyle", "VERTICAL_LAYOUT");
    let tabular = config.choice("TabularStyle", "CREATE");
    let gap_after_else = config.bool("GapAfterElse", true);
    let then_on_when_line = config.bool("ThenOnWhenLine", true);
    let continue_after_else = config.bool("ContinueAfterElse", false);
    let value_indent = config.choice("ValueIndent", "ADD_2");
    let value_indent_add = match value_indent.as_str() {
        "ADD_2" => 2,
        "ADD_5" => 5,
        "ADD_7" => 7,
        _ => 0,
    };

    for cmd in doc.commands().to_vec() {
        if cmd.is_comment_only || cmd.in_class_definition {
            continue;
        }
        let mut t = cmd.first;
        while t <= cmd.last {
            if is_comment(doc, t) {
                t += 1;
                continue;
            }
            if (is_kw(doc, t, "COND") || is_kw(doc, t, "SWITCH"))
                && let Some(p) = next_non_comment_sibling(doc, t, cmd.last)
                && doc.syn.bracket_match[p].is_some_and(|c| c > p)
            {
                let o = Opts {
                    max_line_length,
                    one_liner: &one_liner,
                    simple: &simple,
                    tabular: &tabular,
                    gap_after_else,
                    then_on_when_line,
                    continue_after_else,
                    value_indent_add,
                };
                execute_on(doc, &o, p, is_kw(doc, t, "SWITCH"));
                t = p;
            }
            t += 1;
        }
    }
}

struct Opts<'a> {
    max_line_length: i64,
    one_liner: &'a str,
    simple: &'a str,
    tabular: &'a str,
    gap_after_else: bool,
    then_on_when_line: bool,
    continue_after_else: bool,
    value_indent_add: i64,
}

/// `readTermUntil`: the last sibling before one of the end keywords (or before the end).
fn read_term_until(
    doc: &Doc,
    start: usize,
    last: usize,
    include_end: bool,
    end_keywords: &[&str],
) -> usize {
    let mut last_token = start;
    let mut token = Some(start);
    while let Some(t) = token.filter(|t| !end_keywords.iter().any(|k| is_kw(doc, *t, k))) {
        last_token = t;
        token = next_non_comment_sibling(doc, t, last);
        if include_end && let Some(n) = token {
            last_token = n;
        }
    }
    last_token
}

fn execute_on(doc: &mut Doc, o: &Opts, parent: usize, is_switch: bool) {
    let close = doc.syn.bracket_match[parent].unwrap();
    let last = close - 1;
    let Some(first_child) = (parent + 1..close).find(|j| !is_comment(doc, *j)) else {
        return;
    };
    let mut table = Table::new(MAX_COLUMN_COUNT);
    let basic_indent = doc.start_x(0, first_child);
    let first_line_breaks = doc.lines[first_child];
    let mut line = table.add_line();
    let mut token = Some(first_child);

    // read initial LET ... IN
    if let Some(t) = token.filter(|t| is_kw(doc, *t, "LET")) {
        let l = read_term_until(doc, t, last, true, &["IN"]);
        table.set_cell(line, LET_EXPR, Cell::new(doc, t, l));
        token = next_non_comment_sibling(doc, l, last);
    }
    // for SWITCH expressions, read the operand
    if is_switch && let Some(t) = token {
        let l = read_term_until(doc, t, last, false, &["WHEN"]);
        table.set_cell(line, OPERAND, Cell::new(doc, t, l));
        token = next_non_comment_sibling(doc, l, last);
    }
    let mut when_count = 0;
    let mut else_token = None;
    let mut else_value = None;
    while let Some(mut t) = token {
        if is_kw(doc, t, "WHEN") {
            if when_count > 0 {
                line = table.add_line();
            }
            when_count += 1;
            table.set_cell(line, WHEN, Cell::new(doc, t, t));
            let Some(cs) = next_non_comment_sibling(doc, t, last) else {
                return;
            };
            let cl = read_term_until(doc, cs, last, false, &["THEN"]);
            table.set_cell(line, CONDITION, Cell::new(doc, cs, cl));
            let Some(then) = next_non_comment_sibling(doc, cl, last) else {
                return;
            };
            table.set_cell(line, THEN, Cell::new(doc, then, then));
            let Some(vs) = next_non_comment_sibling(doc, then, last) else {
                return;
            };
            let vl = read_term_until(doc, vs, last, false, &["WHEN", "ELSE"]);
            table.set_cell(line, THEN_VALUE, Cell::new(doc, vs, vl));
            t = vl;
        } else if is_kw(doc, t, "ELSE") {
            else_token = Some(t);
            table.set_cell(line, ELSE, Cell::new(doc, t, t));
            let Some(vs) = next_non_comment_sibling(doc, t, last) else {
                return;
            };
            let vl = read_term_until(doc, vs, last, false, &[]);
            table.set_cell(line, ELSE_VALUE, Cell::new(doc, vs, vl));
            else_value = Some((vs, vl));
            t = vl;
        }
        token = next_non_comment_sibling(doc, t, last);
    }

    // check whether a one-liner can and shall be created or kept
    if when_count == 1 {
        let fits = basic_indent + table.total_multi_line_width(doc) <= o.max_line_length;
        let align_as_one_liner = match o.one_liner {
            "CREATE" => fits && !(first_child..=close).any(|j| is_comment(doc, j)),
            "KEEP" => {
                let start = (parent + 1..close).find(|j| !is_comment(doc, *j)).unwrap();
                fits && !(start + 1..=close).any(|j| doc.lines[j] > 0)
            }
            _ => false,
        };
        if align_as_one_liner {
            table.align(doc, basic_indent, first_line_breaks, true, true);
            return;
        }
    }
    // for all NON-one-liners, force a line break after LET ... IN and the SWITCH operand,
    // and move the ELSE line below the WHEN lines
    if !table.column_is_empty(LET_EXPR) {
        table.set_force_line_break_after(LET_EXPR, false, None);
        table.set_force_indent(OPERAND, None, 0);
    }
    if !table.column_is_empty(OPERAND) {
        table.set_force_line_break_after(OPERAND, false, None);
        table.set_force_indent(WHEN, None, 0);
    }
    table.set_force_line_break_after(THEN_VALUE, false, None);
    table.set_force_indent(ELSE, None, 0);

    // simple layout
    if o.simple == "VERTICAL_LAYOUT" && when_count == 1 {
        table.set_force_line_break_after(CONDITION, false, None);
        table.set_force_indent(THEN, None, 0);
        table.remove_line_breaks_before(doc, ELSE_VALUE);
        table.align(doc, basic_indent, first_line_breaks, true, true);
        return;
    }
    // tabular layout
    if when_count > 1 {
        let tabular = match o.tabular {
            "CREATE" => basic_indent + table.total_multi_line_width(doc) <= o.max_line_length,
            "KEEP" => {
                !table.has_any_line_break_before(doc, THEN)
                    && !table.has_any_line_break_before(doc, THEN_VALUE)
            }
            _ => false,
        };
        if tabular {
            if o.gap_after_else
                && let (Some(et), Some((vs, vl))) = (else_token, else_value)
            {
                table.lines[line].cells[ELSE] = None;
                table.lines[line].cells[ELSE_VALUE] = None;
                line = table.add_line();
                table.set_cell(line, WHEN, Cell::new(doc, et, et));
                table.set_cell(line, THEN_VALUE, Cell::new(doc, vs, vl));
            }
            table.align(doc, basic_indent, first_line_breaks, true, true);
            return;
        }
    }
    // complex layout
    if o.then_on_when_line {
        table.remove_line_breaks_before(doc, THEN);
        table.join_into_previous_columns_ex(doc, THEN, true, true);
        table.set_force_line_break_after(CONDITION, false, None);
        table.set_force_indent(THEN_VALUE, None, o.value_indent_add);
    } else {
        table.set_force_line_break_after(CONDITION, false, None);
        table.set_force_indent(THEN, None, o.value_indent_add);
    }
    if o.continue_after_else {
        table.remove_line_breaks_before(doc, ELSE_VALUE);
        if o.value_indent_add == 7
            && let Some(cell) = table
                .lines
                .iter_mut()
                .rev()
                .find_map(|l| l.cells[ELSE_VALUE].as_mut())
        {
            cell.add_indent = 2;
        }
    } else {
        table.set_force_line_break_after(ELSE, false, None);
        table.set_force_indent(ELSE_VALUE, None, o.value_indent_add);
    }
    table.align(doc, basic_indent, first_line_breaks, true, true);
}
