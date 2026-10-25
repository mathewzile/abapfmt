//! ALIGN_FORM_DECLARATION: aligns the parameters of a FORM definition.
use abapfmt_syntax::TokenType;

use crate::align::{Cell, Table};
use crate::rules::align_declarations::{is_any_kw, is_kw, next_non_comment_sibling, term_last};
use crate::{ConfigExt, Doc, RuleConfig};

const FORM: usize = 0;
const FORM_NAME: usize = 1;
const PARAMETER_GROUP: usize = 2;
const PARAMETER_NAME: usize = 3;
const TYPE_OR_LIKE: usize = 4;

const PARAMETER_GROUPS: &[&str] = &["TABLES", "USING", "CHANGING", "RAISING"];

pub fn align_form_declaration(doc: &mut Doc, config: &RuleConfig) {
    let param_count_behind_name = config.int("ParamCountBehindFormName", 3);
    let continue_after_group = config.bool("ContinueAfterParamGroupKeyword", true);
    let align_types = config.bool("AlignTypes", true);
    let add_empty_line = config.bool("AddEmptyLine", true);
    let remove_empty_line = config.bool("RemoveEmptyLine", true);

    let cmds = doc.commands().to_vec();
    for (ci, cmd) in cmds.iter().enumerate() {
        if cmd.is_comment_only || doc.first_code_text(cmd).as_deref() != Some("FORM") {
            continue;
        }
        let last = cmd.last;
        let form = doc.first_code_token(cmd).unwrap();
        let mut table = Table::new(5);
        let mut line = table.add_line();
        table.set_cell(line, FORM, Cell::new(doc, form, form));
        let Some(name) = next_non_comment_sibling(doc, form, last) else {
            continue;
        };
        if doc.syn.types[name] != TokenType::Identifier {
            continue;
        }
        table.set_cell(line, FORM_NAME, Cell::new(doc, name, name));
        let mut in_raising = false;
        let mut token = next_non_comment_sibling(doc, name, last);
        let mut ok = true;
        while let Some(mut t) = token.filter(|t| doc.syn.types[*t] != TokenType::Period) {
            if is_any_kw(doc, t, PARAMETER_GROUPS) {
                in_raising = is_kw(doc, t, "RAISING");
                table.set_cell(line, PARAMETER_GROUP, Cell::new(doc, t, t));
                match next_non_comment_sibling(doc, t, last) {
                    Some(n) => t = n,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if doc.syn.types[t] == TokenType::Identifier
                && doc.syn.bracket_match[t].is_none_or(|c| c < t)
            {
                table.set_cell(line, PARAMETER_NAME, Cell::new(doc, t, t));
                token = next_non_comment_sibling(doc, t, last);
            } else if doc.syn.text(t).eq_ignore_ascii_case("VALUE(")
                || doc.syn.text(t).eq_ignore_ascii_case("RESUMABLE(")
            {
                let end = term_last(doc, t);
                table.set_cell(line, PARAMETER_NAME, Cell::new(doc, t, end));
                token = next_non_comment_sibling(doc, end, last);
            } else {
                ok = false;
                break;
            }
            if in_raising && let Some(c) = table.lines[line].cells[PARAMETER_NAME].as_mut() {
                c.override_width = Some(1);
            }
            let Some(mut t) = token else { break };
            if doc.syn.types[t] == TokenType::Period {
                break;
            }
            if !in_raising && is_any_kw(doc, t, &["TYPE", "LIKE", "STRUCTURE"]) {
                let type_start = t;
                if is_kw(doc, t, "STRUCTURE") {
                    match next_non_comment_sibling(doc, t, last) {
                        Some(n) => t = n,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                } else {
                    loop {
                        match next_non_comment_sibling(doc, t, last) {
                            Some(n) => t = n,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                        if doc.syn.types[t] == TokenType::Identifier || is_kw(doc, t, "TABLE") {
                            break;
                        }
                    }
                    if !ok {
                        break;
                    }
                }
                table.set_cell(line, TYPE_OR_LIKE, Cell::new(doc, type_start, t));
                token = next_non_comment_sibling(doc, t, last);
            }
            line = table.add_line();
        }
        if !ok {
            continue;
        }
        if table.lines.len() > 1 && table.lines[line].cells[PARAMETER_NAME].is_none() {
            table.lines.pop();
        }
        if table.lines.len() as i64 > param_count_behind_name {
            table.set_force_line_break_after(FORM_NAME, false, Some(2));
            table.set_force_indent(PARAMETER_GROUP, Some(FORM), 2);
        }
        if !continue_after_group {
            table.set_force_line_break_after(PARAMETER_GROUP, false, Some(2));
        }
        if !align_types {
            table.join_into_previous_columns(doc, TYPE_OR_LIKE, true);
        }
        let basic_indent = i64::from(doc.spaces[cmd.first]);
        let first_line_breaks = doc.lines[cmd.first];
        table.align(doc, basic_indent, first_line_breaks, false, true);

        // add or remove an empty line above the first command of the FORM implementation
        if ci + 1 < cmds.len() && !cmds[ci + 1].is_method_function_or_form_end() {
            let multi_line = (cmd.first + 1..=last).any(|t| doc.lines[t] > 0);
            let next = cmds[ci + 1].first;
            if multi_line && doc.lines[next] < 2 && add_empty_line {
                doc.lines[next] = 2;
            } else if !multi_line && doc.lines[next] >= 2 && remove_empty_line {
                doc.lines[next] = 1;
            }
        }
    }
}
