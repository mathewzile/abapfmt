//! ALIGN_ABAP_DOC: aligns the `|` of `@parameter` / `@raising` / `@exception` ABAP Doc lines.
use abapfmt_syntax::Command;

use crate::{ConfigExt, Doc, RuleConfig};

const SIGN: &str = "\"!";

fn is_abap_doc_line(doc: &Doc, cmd: &Command) -> bool {
    cmd.is_comment_only && doc.token_text(cmd.first).starts_with(SIGN)
}

/// The keyword and the name in front of the `|`, if this is a parameter documentation line.
fn parameter_doc_parts(doc: &Doc, cmd: &Command) -> Option<(String, String, usize)> {
    if !is_abap_doc_line(doc, cmd) {
        return None;
    }
    let text = doc.token_text(cmd.first);
    let pipe = text.find('|')?;
    if pipe == text.len() - 1 {
        return None;
    }
    let parts: Vec<&str> = text[SIGN.len()..pipe]
        .split(' ')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 {
        return None;
    }
    if !["@parameter", "@raising", "@exception"]
        .iter()
        .any(|k| parts[0].eq_ignore_ascii_case(k))
    {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string(), pipe))
}

pub fn align_abap_doc(doc: &mut Doc, config: &RuleConfig) {
    let across_empty = config.bool("AlignAcrossEmptyLines", true);
    let across_non_empty = config.bool("AlignAcrossNonEmptyLines", true);
    let cmds = doc.commands().to_vec();
    let mut ci = 0;
    while ci < cmds.len() {
        if parameter_doc_parts(doc, &cmds[ci]).is_none() {
            ci += 1;
            continue;
        }
        let mut line_count = 1;
        let mut end = ci;
        loop {
            end += 1;
            if end >= cmds.len() || !is_abap_doc_line(doc, &cmds[end]) {
                break;
            }
            if parameter_doc_parts(doc, &cmds[end]).is_some() {
                line_count += 1;
                continue;
            }
            let empty = doc.token_text(cmds[end].first) == SIGN;
            if if empty {
                !across_empty
            } else {
                !across_non_empty
            } {
                break;
            }
        }
        // only align if the whole parameter interface was covered (i.e. what follows is the declaration)
        let followed_by_comment = end < cmds.len()
            && cmds[end].is_comment_only
            && doc.token_text(cmds[end].first).starts_with('"');
        if line_count > 1 && !followed_by_comment {
            align_section(doc, &cmds[ci..end]);
        }
        ci = end;
    }
}

fn align_section(doc: &mut Doc, cmds: &[Command]) {
    let mut max_keyword = 0;
    let mut max_name = 0;
    for cmd in cmds {
        if let Some((k, n, _)) = parameter_doc_parts(doc, cmd) {
            max_keyword = max_keyword.max(k.chars().count());
            max_name = max_name.max(n.chars().count());
        }
    }
    for cmd in cmds {
        let Some((mut keyword, mut name, pipe)) = parameter_doc_parts(doc, cmd) else {
            continue;
        };
        let text = doc.token_text(cmd.first).to_string();
        let docu = text[pipe + 1..].trim().to_string();
        keyword.push_str(&" ".repeat(max_keyword + 1 - keyword.chars().count()));
        name.push_str(&" ".repeat(max_name + 1 - name.chars().count()));
        let changed = format!("{SIGN} {keyword}{name}| {docu}");
        if changed != text {
            doc.text[cmd.first] = Some(changed);
        }
    }
}
