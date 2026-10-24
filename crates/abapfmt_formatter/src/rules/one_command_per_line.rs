//! ONE_COMMAND_PER_LINE: if a line contains multiple commands, each is moved to its own line.
use abapfmt_syntax::{Command, TokenType};

use crate::{ConfigExt, Doc, RuleConfig};

/// The command that starts the line the command `ci` is on.
fn first_command_in_line(doc: &Doc, cmds: &[Command], mut ci: usize) -> usize {
    while ci > 0 && doc.lines[cmds[ci].first] == 0 {
        ci -= 1;
    }
    ci
}

/// Does the WHEN block consist of nothing but commands on the line of the WHEN?
fn is_one_liner_after_when(doc: &Doc, cmds: &[Command], when: usize) -> bool {
    let mut ci = when + 1;
    while ci < cmds.len() {
        let c = &cmds[ci];
        if doc.lines[c.first] > 0 {
            // another command on its own line inside the WHEN block: not a one-liner
            return c.level <= cmds[when].level;
        }
        if (c.first..=c.last).any(|t| doc.lines[t] > 0) {
            return false;
        }
        ci = (ci + 1..cmds.len())
            .find(|n| !cmds[*n].is_comment_only)
            .unwrap_or(cmds.len());
    }
    true
}

fn set_relative_indent(doc: &mut Doc, cmds: &[Command], prev: Option<usize>, ci: usize) {
    let cmd = &cmds[ci];
    let first = cmd.first;
    let mut new_indent = i64::from(doc.indent_of(ci));
    if let Some(p) = prev {
        let deviation = i64::from(doc.spaces[cmds[p].first]) - i64::from(doc.indent_of(p));
        new_indent = (new_indent + deviation).max(0);
    }
    if doc.lines[first] == 0 {
        // move the first token down by one line, keeping its horizontal position
        let old_start = doc.start_x(0, first);
        doc.lines[first] = 1;
        doc.spaces[first] = old_start as u32;
    }
    let add = new_indent - doc.start_x(0, first);
    doc.set_spaces_adjusting_indent(cmd.last, first, new_indent, true);

    // the commands that follow on the same line move along
    for next in &cmds[ci + 1..] {
        if doc.lines[next.first] > 0 {
            break;
        }
        if next.first < next.last {
            doc.add_indent(next.last, next.first + 1, add, 0, false);
        }
    }
}

pub fn one_command_per_line(doc: &mut Doc, config: &RuleConfig) {
    let keep_one_liners = config.bool("KeepOneLinersAfterWhen", true);
    let keep_multi_liners = config.bool("KeepMultiLinersAfterWhen", false);

    let cmds = doc.commands().to_vec();
    for (ci, cmd) in cmds.iter().enumerate() {
        let first_type = doc.syn.types[cmd.first];
        if ci == 0
            || doc.lines[cmd.first] > 0
            || cmd.is_comment_only
            // 'lv_x += 1..' is left to the empty command rule, 'DATA lv_x TYPE i. ##NEEDED' to the pragma rule
            || matches!(first_type, TokenType::Period | TokenType::Pragma)
        {
            continue;
        }
        // INCLUDE TYPE ty_s_other. TYPES:  (inside a BEGIN OF ... END OF block)
        if doc.first_code_text(&cmds[ci - 1]).as_deref() == Some("INCLUDE")
            && doc.syn.text(cmd.first).eq_ignore_ascii_case("TYPES")
            && cmd.first + 2 <= cmd.last
            && doc.syn.types[cmd.first + 1] == TokenType::Colon
            && doc.lines[cmd.first + 2] > 0
        {
            continue;
        }

        let line_start = first_command_in_line(doc, &cmds, ci);
        if doc.first_code_text(&cmds[line_start]).as_deref() == Some("WHEN") {
            let one_liner = is_one_liner_after_when(doc, &cmds, line_start);
            if (one_liner && keep_one_liners) || (!one_liner && keep_multi_liners) {
                continue;
            }
        }

        // adjust the indent of both the command at line start and the current command, using
        // the previous command as an anchor point, respectively
        let prev = (0..line_start)
            .rev()
            .find(|p| !cmds[*p].is_comment_only)
            .map(|p| first_command_in_line(doc, &cmds, p));
        set_relative_indent(doc, &cmds, prev, line_start);
        set_relative_indent(doc, &cmds, Some(line_start), ci);
    }
}
