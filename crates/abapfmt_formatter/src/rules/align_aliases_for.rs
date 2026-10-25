//! ALIGN_ALIASES_FOR: aligns `ALIASES name FOR interface~name` declarations.
use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, end_x, is_comment};
use crate::rules::align_declarations::{for_each_section, is_comma_or_period, skip_command};
use crate::{Doc, RuleConfig};

const KEYWORD_ALIASES: usize = 0;
const IDENTIFIER1: usize = 1;
const KEYWORD_FOR: usize = 2;
const IDENTIFIER2: usize = 3;
const LINE_END_COMMENT: usize = 4;

fn is_alias_for(doc: &Doc, cmd: &Command, with_colon: bool) -> bool {
    if cmd.is_comment_only || !doc.syn.text(cmd.first).eq_ignore_ascii_case("ALIASES") {
        return false;
    }
    let mut t = cmd.first + 1;
    if with_colon && t <= cmd.last && doc.syn.types[t] == TokenType::Colon {
        t += 1;
    }
    t < cmd.last
        && doc.syn.types[t] == TokenType::Identifier
        && doc.syn.text(t + 1).eq_ignore_ascii_case("FOR")
}

fn contains_inner_comment_line(doc: &Doc, cmd: &Command) -> bool {
    let n = doc.syn.tokens.len();
    let mut at_start_or_end = true;
    for t in cmd.first..=cmd.last {
        let is_line =
            is_comment(doc, t) && doc.first_in_line(t) && (t + 1 >= n || doc.lines[t + 1] > 0);
        if is_line {
            if !at_start_or_end {
                return true;
            }
        } else {
            at_start_or_end = matches!(doc.syn.text(t), "." | "," | ":");
        }
    }
    false
}

pub fn align_aliases_for(doc: &mut Doc, config: &RuleConfig) {
    for_each_section(
        doc,
        config,
        true,
        |doc, cmd| is_alias_for(doc, cmd, true),
        |doc, cmd, _| is_alias_for(doc, cmd, false),
        execute_on,
    );
}

fn execute_on(doc: &mut Doc, cmds: &[Command], start_ci: usize, end_ci: usize, is_chain: bool) {
    let mut table = Table::new(5);
    let mut ci = start_ci;
    if contains_inner_comment_line(doc, &cmds[ci]) {
        return;
    }
    let mut start = cmds[ci].first;
    let n = doc.syn.tokens.len();
    let offset = if is_chain { start + 1 } else { start };
    let first_line_breaks = doc.lines[offset];
    let basic_indent = if is_chain {
        if first_line_breaks > 0 {
            doc.start_x(0, start) + 2
        } else {
            end_x(doc, start + 1) + 1
        }
    } else {
        doc.start_x(0, start)
    };
    loop {
        if is_chain {
            while start < n
                && start <= cmds[ci].last
                && is_comment(doc, start)
                && doc.first_in_line(start)
                && (start + 1 >= n || doc.lines[start + 1] > 0)
            {
                start += 1;
            }
            if start > cmds[ci].last {
                break;
            }
        } else {
            while ci < end_ci && skip_command(doc, &cmds[ci]) {
                ci += 1;
            }
            if ci == end_ci {
                break;
            }
            start = cmds[ci].first;
        }
        let line = table.add_line();
        if doc.syn.text(start).eq_ignore_ascii_case("ALIASES") {
            if is_chain {
                start += 1;
            } else {
                table.set_cell(line, KEYWORD_ALIASES, Cell::new(doc, start, start));
            }
            start += 1;
        }
        if start + 2 >= n {
            return;
        }
        let id1 = start;
        table.set_cell(line, IDENTIFIER1, Cell::new(doc, id1, id1));
        table.set_cell(line, KEYWORD_FOR, Cell::new(doc, id1 + 1, id1 + 1));
        table.set_cell(line, IDENTIFIER2, Cell::new(doc, id1 + 2, id1 + 2));
        start = id1 + 3;
        if start >= n || !is_comma_or_period(doc, start) {
            return;
        }
        if start + 1 < n
            && is_comment(doc, start + 1)
            && doc.lines[start + 1] == 0
            && start < cmds[ci].last
        {
            start += 1;
            table.set_cell(line, LINE_END_COMMENT, Cell::new(doc, start, start));
        }
        start += 1;
        if is_chain {
            if start > cmds[ci].last {
                break;
            }
        } else {
            ci += 1;
            if ci == end_ci {
                break;
            }
            if contains_inner_comment_line(doc, &cmds[ci]) {
                return;
            }
            start = cmds[ci].first;
        }
    }
    if !table.lines.is_empty() {
        table.align(doc, basic_indent, first_line_breaks, false, true);
    }
}
