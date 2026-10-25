//! ALIGN_PERFORM: aligns the parameters of obsolete `PERFORM` subroutine calls.
use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, condense_up_to};
use crate::rules::align_declarations::{
    find_seq_last, is_any_kw, is_comma_or_period, is_kw, matches_seq, next_non_comment_sibling,
    next_sibling,
};
use crate::{ConfigExt, Doc, RuleConfig};

const PERFORM_SUBR: usize = 0;
const PARAMETER_GROUP: usize = 1;
const PARAMETER_NAME: usize = 2;

const PARAMETER_GROUPS: &[&str] = &["TABLES", "USING", "CHANGING", "OF"];

fn opens_level(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t)
}

fn prev_code(doc: &Doc, t: usize, first: usize) -> Option<usize> {
    (first..t)
        .rev()
        .find(|j| doc.syn.types[*j] != TokenType::Comment)
}

/// The token after the IN PROGRAM / IF FOUND additions (where the parameters start).
fn params_start(doc: &Doc, first: usize, last: usize) -> Option<(usize, bool)> {
    let mut token = next_non_comment_sibling(doc, first, last)?;
    if opens_level(doc, token) {
        token = next_sibling(doc, token, last)?;
    }
    token = next_non_comment_sibling(doc, token, last)?;
    if is_kw(doc, token, "ON") {
        return None;
    }
    let mut has_additions = false;
    if matches_seq(doc, token, last, &["IN", "PROGRAM"]) {
        has_additions = true;
        token = next_non_comment_sibling(doc, token, last)?;
        token = next_non_comment_sibling(doc, token, last)?;
        if opens_level(doc, token) {
            token = next_sibling(doc, token, last)?;
            token = next_sibling(doc, token, last)?;
        } else if !is_comma_or_period(doc, token) {
            token = next_sibling(doc, token, last)?;
        }
    }
    if matches_seq(doc, token, last, &["IF", "FOUND"]) {
        has_additions = true;
        token = next_non_comment_sibling(doc, token, last)?;
        token = next_non_comment_sibling(doc, token, last)?;
    }
    Some((token, has_additions))
}

/// `PERFORM ... IF FOUND` written behind the parameters is moved before them. Returns true if
/// tokens were moved.
fn move_late_if_found(doc: &mut Doc, cmd: &Command, token: usize) -> bool {
    let last = cmd.last;
    let Some(period) = find_seq_last(doc, token, last, &["IF", "FOUND", "."]) else {
        return false;
    };
    let Some(found) = prev_code(doc, period, cmd.first) else {
        return false;
    };
    let Some(late_if) = prev_code(doc, found, cmd.first) else {
        return false;
    };
    if !is_kw(doc, late_if, "IF") || !is_kw(doc, found, "FOUND") || late_if < token {
        return false;
    }
    // the next token takes over the position of the removed term
    if doc.lines[late_if] > 0 && doc.lines[period] == 0 {
        doc.lines[period] = doc.lines[late_if];
        doc.spaces[period] = doc.spaces[late_if];
    }
    doc.move_before(cmd, late_if, token);
    doc.move_before(cmd, found, token);
    doc.lines[late_if] = 0;
    doc.spaces[late_if] = 1;
    doc.lines[found] = 0;
    doc.spaces[found] = 1;
    true
}

pub fn align_perform(doc: &mut Doc, config: &RuleConfig) {
    let max_line_length = config.int("MaxLineLength", 120);
    let param_count_after = config.int("ParamCountAfterPerform", 3);
    let break_after_additions = config.bool("BreakAfterAdditions", false);
    let align_with_form_name = config.bool("AlignWithFormName", false);
    let continue_after_group = config.bool("ContinueAfterParamGroupKeyword", true);
    let param_count_to_condense = config.int("ParamCountToCondense", 16);

    // moving IF FOUND changes the token order, which the command ranges do not follow: move first,
    // then run again on the parsed result
    let mut moved = false;
    for ci in 0..doc.commands().len() {
        let cmd = doc.commands()[ci].clone();
        if cmd.is_comment_only
            || doc.first_code_text(&cmd).as_deref() != Some("PERFORM")
            || cmd.colon.is_some()
        {
            continue;
        }
        let perform = doc.first_code_token(&cmd).unwrap();
        if let Some((token, _)) = params_start(doc, perform, cmd.last) {
            moved |= move_late_if_found(doc, &cmd, token);
        }
    }
    if moved {
        doc.rerun = true;
        return;
    }

    let mut ci = 0;
    while ci < doc.commands().len() {
        let cmd = doc.commands()[ci].clone();
        ci += 1;
        if cmd.is_comment_only
            || doc.first_code_text(&cmd).as_deref() != Some("PERFORM")
            || cmd.colon.is_some()
        {
            continue;
        }
        let perform = doc.first_code_token(&cmd).unwrap();
        let Some((token, has_additions)) = params_start(doc, perform, cmd.last) else {
            continue;
        };
        let last = cmd.last;
        let perform = doc.first_code_token(&cmd).unwrap();
        let Some(term_last) = prev_code(doc, token, perform) else {
            continue;
        };

        // build the align table
        let mut table = Table::new(3);
        let mut line = table.add_line();
        table.set_cell(line, PERFORM_SUBR, Cell::new(doc, perform, term_last));
        let mut t = next_non_comment_sibling(doc, term_last, last);
        while let Some(mut tok) = t.filter(|x| !is_comma_or_period(doc, *x)) {
            if is_any_kw(doc, tok, PARAMETER_GROUPS) {
                table.set_cell(line, PARAMETER_GROUP, Cell::new(doc, tok, tok));
                match next_non_comment_sibling(doc, tok, last) {
                    Some(n) => tok = n,
                    None => break,
                }
            }
            table.set_cell(line, PARAMETER_NAME, Cell::new(doc, tok, tok));
            line = table.add_line();
            t = next_non_comment_sibling(doc, tok, last);
        }
        if table.lines.len() > 1 && table.lines[line].cells[PARAMETER_NAME].is_none() {
            table.lines.pop();
        }

        condense_up_to(doc, perform, term_last, 255, doc.start_x(0, perform), false);
        if table.lines.len() == 1 && table.lines[0].cells[PARAMETER_NAME].is_none() {
            continue;
        }

        // forced line breaks and indents
        let of_group = table.lines[0].cells[PARAMETER_GROUP]
            .as_ref()
            .is_some_and(|c| is_kw(doc, c.first, "OF"));
        if !of_group {
            let break_after = table.lines.len() as i64 > param_count_after
                || has_additions && break_after_additions;
            if break_after {
                let offset = if align_with_form_name {
                    "PERFORM".len() as i64 + 1
                } else {
                    2
                };
                table.set_force_line_break_after(PERFORM_SUBR, false, Some(offset));
            }
            if !continue_after_group {
                table.set_force_line_break_after(PARAMETER_GROUP, false, Some(2));
            }
        }
        let condense = table.lines.len() as i64 >= param_count_to_condense;
        if condense {
            table.lines.retain(|l| l.cells[PARAMETER_GROUP].is_some());
        }
        let basic_indent = i64::from(doc.spaces[cmd.first]);
        let first_line_breaks = doc.lines[cmd.first];
        table.align(doc, basic_indent, first_line_breaks, false, true);

        if condense {
            for li in 0..table.lines.len() {
                let Some(first_param) = table.lines[li].cells[PARAMETER_NAME]
                    .as_ref()
                    .map(|c| c.first)
                else {
                    continue;
                };
                let last_param = if li + 1 < table.lines.len() {
                    let Some(g) = table.lines[li + 1].cells[PARAMETER_GROUP].as_ref() else {
                        continue;
                    };
                    prev_code(doc, g.first, cmd.first)
                } else {
                    (cmd.first..=last)
                        .rev()
                        .find(|j| doc.syn.types[*j] != TokenType::Comment)
                        .and_then(|p| prev_code(doc, p, cmd.first))
                };
                if let Some(lp) = last_param {
                    let x = doc.start_x(0, first_param);
                    condense_up_to(doc, first_param, lp, max_line_length, x, true);
                }
            }
        }
    }
}
