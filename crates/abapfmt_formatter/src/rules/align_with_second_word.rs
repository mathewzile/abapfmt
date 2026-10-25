//! ALIGN_WITH_SECOND_WORD: aligns the keywords of continuation lines with the second word,
//! e.g. `READ TABLE itab` / `WITH KEY ...`.
use abapfmt_syntax::{Command, TokenType};

use crate::rules::align_declarations::{
    is_kw, match_seq_last, matches_seq, next_non_comment, next_sibling, term_last,
};
use crate::{Doc, RuleConfig};

/// `Command.isAssignment(true, true)`
fn is_assignment(doc: &Doc, cmd: &Command) -> bool {
    let Some(first) = doc.first_code_token(cmd) else {
        return false;
    };
    let text = doc.syn.text(first).to_ascii_uppercase();
    let mut t = first;
    if text.starts_with("DATA(") || text.starts_with("FINAL(") || text.starts_with("@DATA(") {
        t = term_last(doc, first);
    } else if doc.syn.types[first] != TokenType::Identifier {
        return false;
    } else {
        t = term_last(doc, t);
    }
    let Some(mut next) = next_non_comment(doc, t, cmd.last) else {
        return false;
    };
    if doc.syn.types[next] == TokenType::Colon {
        let Some(n) = next_non_comment(doc, next, cmd.last) else {
            return false;
        };
        next = n;
    }
    doc.syn.types[next] == TokenType::AssignmentOp
}

pub(crate) fn is_abap_sql_operation(doc: &Doc, cmd: &Command, first: usize) -> bool {
    let last = cmd.last;
    let up = doc.syn.text(first).to_ascii_uppercase();
    match up.as_str() {
        "SELECT" | "WITH" => true,
        "OPEN" | "FETCH" | "CLOSE" => {
            matches_seq(doc, first, last, &["OPEN", "CURSOR"])
                || matches_seq(doc, first, last, &["FETCH", "NEXT", "CURSOR"])
                || matches_seq(doc, first, last, &["CLOSE", "CURSOR"])
        }
        "UPDATE" => true,
        "INSERT" => {
            let into = crate::rules::align_declarations::find_seq_last(doc, first, last, &["INTO"]);
            into.is_none() || next_non_comment(doc, first, last) == into
        }
        "DELETE" => {
            !matches_seq(
                doc,
                first,
                last,
                &["DELETE", "FROM", "MEMORY|DATABASE|SHARED"],
            ) && matches_seq(doc, first, last, &["DELETE", "FROM"])
        }
        _ => false,
    }
}

pub fn align_with_second_word(doc: &mut Doc, _config: &RuleConfig) {
    let cmds = doc.commands().to_vec();
    let mut in_method = !cmds
        .iter()
        .any(|c| c.is_method_function_form_or_event_block_start());
    for cmd in &cmds {
        if cmd.is_method_function_form_or_event_block_end() {
            in_method = false;
        }
        if cmd.is_method_function_form_or_event_block_start() {
            in_method = true;
        }
        let Some(first) = doc.first_code_token(cmd).filter(|_| !cmd.is_comment_only) else {
            continue;
        };
        let last = cmd.last;
        let opens_level = cmd.depth_after > cmd.level;
        let skip = !in_method
            || doc.is_declaration(cmd)
            || is_assignment(doc, cmd)
            || is_abap_sql_operation(doc, cmd, first)
            || cmd.in_class_definition
            || matches!(
                doc.first_code_text(cmd).as_deref(),
                Some(
                    "ALIASES"
                        | "INTERFACES"
                        | "CLASS-EVENTS"
                        | "CLASS-METHODS"
                        | "METHODS"
                        | "EVENTS"
                )
            )
            || (opens_level && !is_kw(doc, first, "LOOP"))
            || matches_seq(doc, first, last, &["CALL", "METHOD|FUNCTION|BADI"])
            || matches_seq(doc, first, last, &["CREATE", "OBJECT"])
            || matches_seq(
                doc,
                first,
                last,
                &["RECEIVE", "RESULTS", "FROM", "FUNCTION"],
            )
            || is_raise_with_exporting(doc, first, last)
            || next_sibling(doc, first, last) != Some(first + 1);
        let is_submit = is_kw(doc, first, "SUBMIT");
        let Some(second) = next_sibling(doc, first, last) else {
            continue;
        };
        if skip || doc.syn.types[first] != TokenType::Keyword || doc.lines[second] != 0 {
            continue;
        }
        let indent = doc.start_x(0, second);
        let mut token = Some(second);
        while let Some(t) = token {
            let and_or = ["AND", "OR", "EQUIV"].iter().any(|k| is_kw(doc, t, k));
            if doc.first_in_line(t)
                && i64::from(doc.spaces[t]) != indent
                && doc.syn.types[t] == TokenType::Keyword
                && (!and_or || is_submit && matches_seq(doc, t, last, &["AND", "RETURN"]))
            {
                let mut use_indent = indent;
                if is_submit && is_kw(doc, t, "WITH") {
                    use_indent = indent_of_previous_with(doc, t, indent);
                }
                let min = i64::from(doc.spaces[t]);
                doc.add_indent(last, t, use_indent - min, min, false);
            }
            token = next_sibling(doc, t, last);
        }
    }
}

fn is_raise_with_exporting(doc: &Doc, first: usize, last: usize) -> bool {
    let exporting = |from: Option<usize>| {
        from.is_some_and(|f| match_seq_last(doc, f, last, &["EXPORTING"]).is_some())
    };
    if matches_seq(doc, first, last, &["RAISE", "EXCEPTION|RESUMABLE"]) {
        // RAISE [RESUMABLE] EXCEPTION TYPE cx_class EXPORTING
        let mut t = next_non_comment(doc, first, last);
        if t.is_some_and(|x| is_kw(doc, x, "RESUMABLE")) {
            t = t.and_then(|x| next_non_comment(doc, x, last));
        }
        if let Some(e) = t.filter(|x| is_kw(doc, *x, "EXCEPTION"))
            && let Some(ty) = next_non_comment(doc, e, last).filter(|x| is_kw(doc, *x, "TYPE"))
            && let Some(id) = next_non_comment(doc, ty, last)
        {
            return exporting(next_non_comment(doc, id, last));
        }
    } else if matches_seq(doc, first, last, &["RAISE", "SHORTDUMP", "TYPE"]) {
        let t = next_non_comment(doc, first, last)
            .and_then(|x| next_non_comment(doc, x, last))
            .and_then(|x| next_non_comment(doc, x, last))
            .and_then(|x| next_non_comment(doc, x, last));
        return exporting(t);
    }
    false
}

fn indent_of_previous_with(doc: &Doc, cur: usize, fallback: i64) -> i64 {
    let mut use_indent = fallback;
    let mut t = cur;
    while t > 0 {
        t -= 1;
        if is_kw(doc, t, "WITH") {
            use_indent = doc.start_x(0, t);
        }
        if doc.first_in_line(t) {
            break;
        }
    }
    use_indent
}
