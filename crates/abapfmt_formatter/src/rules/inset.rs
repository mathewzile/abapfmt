//! INSET: moves all commands and comments to the correct indentation.
use abapfmt_syntax::Command;

use crate::{ConfigExt, Doc, RuleConfig};

fn is_quot_mark_comment_line(doc: &Doc, c: &Command) -> bool {
    c.is_comment_only && doc.syn.text(c.first).starts_with('"')
}

fn is_asterisk_comment_line(doc: &Doc, c: &Command) -> bool {
    c.is_comment_only && doc.syn.text(c.first).starts_with('*')
}

/// Align a comment block with a following ELSEIF/ELSE, WHEN or CATCH/CLEANUP?
fn align_mode(doc: &Doc, config: &RuleConfig, follower: &Command) -> String {
    let name = match doc.first_code_text(follower).as_deref() {
        Some("ELSEIF" | "ELSE") => "AlignWithFollowingElse",
        Some("WHEN") => "AlignWithFollowingWhen",
        Some("CATCH" | "CLEANUP") => "AlignWithFollowingCatch",
        _ => return "NEVER".into(),
    };
    config.choice(name, "IF_BLANK_LINE_ABOVE")
}

pub fn inset(doc: &mut Doc, config: &RuleConfig) {
    let on_class_definition = config.bool("ExecuteOnClassDefinitionSections", true);
    let any_alignment = [
        "AlignWithFollowingElse",
        "AlignWithFollowingWhen",
        "AlignWithFollowingCatch",
    ]
    .iter()
    .any(|n| config.choice(n, "IF_BLANK_LINE_ABOVE") != "NEVER");

    let cmds = doc.commands().to_vec();
    // a command that the preceding comment lines are aligned with
    let mut align: Option<usize> = None;
    for (ci, cmd) in cmds.iter().enumerate() {
        if align == Some(ci) {
            align = None;
        }
        if (!on_class_definition && cmd.in_class_definition) || is_asterisk_comment_line(doc, cmd) {
            continue;
        }

        if any_alignment && align.is_none() && is_quot_mark_comment_line(doc, cmd) {
            let mut follower = ci + 1;
            while follower < cmds.len()
                && is_quot_mark_comment_line(doc, &cmds[follower])
                && doc.lines[cmds[follower].first] == 1
            {
                follower += 1;
            }
            if follower < cmds.len() {
                let mode = align_mode(doc, config, &cmds[follower]);
                let skip =
                    mode == "NEVER" || (mode == "IF_BLANK_LINE_ABOVE" && doc.lines[cmd.first] < 2);
                align = (!skip).then_some(follower);
            }
        }

        let new_indent = i64::from(doc.indent_of(align.unwrap_or(ci)));
        // commands that follow on the same line as the previous command keep their position
        if doc.lines[cmd.first] == 0 && ci != 0 {
            continue;
        }
        let old = i64::from(doc.spaces[cmd.first]);
        if old != new_indent {
            doc.add_indent(cmd.last, cmd.first, new_indent - old, 0, false);
        }
    }
}
