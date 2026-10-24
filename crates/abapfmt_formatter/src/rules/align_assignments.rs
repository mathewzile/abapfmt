//! ALIGN_ASSIGNMENTS: aligns consecutive assignments to components of the same structure.
use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, is_comment};
use crate::{ConfigExt, Doc, RuleConfig};

const IDENTIFIER: usize = 0;
const ASSIGNMENT_OP: usize = 1;
const REST_OF_COMMAND: usize = 2;

/// The last token of the identifier term (`ls_struc-(` ... `)` included).
fn identifier_last(doc: &Doc, first: usize) -> usize {
    let mut last = first;
    while let Some(close) = doc.syn.bracket_match[last].filter(|c| *c > last) {
        last = close;
    }
    last
}

/// `Command.isAssignment(false, false)` followed by `Token.getStructureVariable`.
fn structure_variable(doc: &Doc, cmd: &Command) -> Option<String> {
    let first = cmd.first;
    if doc.syn.types[first] != TokenType::Identifier {
        return None;
    }
    let last = identifier_last(doc, first);
    let mut next = last + 1;
    while next <= cmd.last && is_comment(doc, next) {
        next += 1;
    }
    if next <= cmd.last && doc.syn.types[next] == TokenType::Colon {
        next += 1;
    }
    while next <= cmd.last && is_comment(doc, next) {
        next += 1;
    }
    if next > cmd.last || doc.syn.types[next] != TokenType::AssignmentOp {
        return None;
    }
    let text = doc.syn.text(first);
    let pos = text.rfind('-').filter(|p| *p > 0)?;
    Some(text[..pos].to_string())
}

pub fn align_assignments(doc: &mut Doc, config: &RuleConfig) {
    let across_empty = config.bool("AlignAcrossEmptyLines", true);
    let across_comments = config.bool("AlignAcrossCommentLines", true);
    let cmds = doc.commands().to_vec();

    let mut last_var: Option<String> = None;
    let mut section_start = 0;
    let mut in_section = 0;
    for (ci, cmd) in cmds.iter().enumerate() {
        if across_comments && cmd.is_comment_only && (across_empty || doc.lines[cmd.first] <= 1) {
            continue;
        }
        let var = structure_variable(doc, cmd);
        if last_var != var || (!across_empty && doc.lines[cmd.first] > 1) {
            if last_var.is_some() && in_section > 1 {
                align_section(doc, &cmds[section_start..ci]);
            }
            last_var = var.clone();
            if var.is_some() {
                section_start = ci;
                in_section = 1;
            }
        } else if var.is_some() && cmd.first + 1 <= cmd.last && doc.lines[cmd.first + 1] == 0 {
            in_section += 1;
        }
    }
    if last_var.is_some() && in_section > 1 {
        align_section(doc, &cmds[section_start..]);
    }
}

fn align_section(doc: &mut Doc, cmds: &[Command]) {
    let mut table = Table::new(3);
    table.cols[ASSIGNMENT_OP].right_align = true;
    let first_line_breaks = doc.lines[cmds[0].first];
    let basic_indent = doc.start_x(0, cmds[0].first);
    for cmd in cmds {
        if cmd.is_comment_only {
            continue;
        }
        let line = table.add_line();
        let id_last = identifier_last(doc, cmd.first);
        table.set_cell(line, IDENTIFIER, Cell::new(doc, cmd.first, id_last));
        let mut op = id_last + 1;
        while is_comment(doc, op) {
            op += 1;
        }
        table.set_cell(line, ASSIGNMENT_OP, Cell::new(doc, op, op));
        let mut rest = op + 1;
        while is_comment(doc, rest) {
            rest += 1;
        }
        table.set_cell(line, REST_OF_COMMAND, Cell::new(doc, rest, cmd.last));
    }
    if !table.lines.is_empty() {
        table.align(doc, basic_indent, first_line_breaks, true, true);
    }
}
