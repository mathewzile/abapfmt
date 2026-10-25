//! DDL_SPACES_* rules.
use abapfmt_syntax::TokenType;

use crate::{ConfigExt, Doc, RuleConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    KeepAsIs,
    Always,
    Never,
}

fn change(config: &RuleConfig, name: &str, default: &str) -> Change {
    match config.choice(name, default).as_str() {
        "ALWAYS" => Change::Always,
        "NEVER" => Change::Never,
        _ => Change::KeepAsIs,
    }
}

fn is_comment(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::Comment
}

fn is_annotation(doc: &Doc, first: usize) -> bool {
    doc.syn.text(first).starts_with('@')
}

fn is_comment_line_command(doc: &Doc, first: usize, last: usize) -> bool {
    is_comment(doc, first) && (first..=last).all(|i| is_comment(doc, i))
}

/// `Token.getPrevCodeToken` within the command.
fn prev_code(doc: &Doc, first: usize, i: usize) -> Option<usize> {
    (first..i).rev().find(|j| !is_comment(doc, *j))
}

/// The enclosing bracket token of `i`, as in `Token.getParent`.
fn parent(doc: &Doc, i: usize) -> Option<usize> {
    doc.syn.parent[i]
}

/// `Token.getPrevCodeSibling`: the previous code token on the same level (skipping over
/// bracketed groups, whose closing token stands for the group).
fn prev_code_sibling(doc: &Doc, first: usize, i: usize) -> Option<usize> {
    let mut cur = i;
    loop {
        if cur == first {
            return None;
        }
        let mut p = cur - 1;
        if doc.syn.parent[cur] == Some(p) {
            return None;
        }
        // the previous sibling of a token after a closed group is the group's closing token
        if is_comment(doc, p) {
            cur = p;
            continue;
        }
        if doc.syn.parent[p] != doc.syn.parent[cur] {
            // p is inside a group: step to the closing token's own level
            p = (first..cur)
                .rev()
                .find(|j| doc.syn.parent[*j] == doc.syn.parent[cur])?;
            if is_comment(doc, p) {
                cur = p;
                continue;
            }
        }
        return Some(p);
    }
}

fn may_be_followed_by_arithmetic_op(doc: &Doc, i: usize) -> bool {
    let t = doc.syn.text(i);
    if t == ")" || t == "]" {
        return true;
    }
    matches!(doc.syn.types[i], TokenType::Identifier | TokenType::Literal)
}

fn is_attached(doc: &Doc, i: usize) -> bool {
    doc.lines[i] == 0 && doc.spaces[i] == 0
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '/'
}

fn is_ddl_parameter_colon(doc: &Doc, first: usize, last: usize, i: usize) -> bool {
    if doc.syn.text(i) != ":" || i + 1 > last {
        return false;
    }
    let next = i + 1;
    if !(is_attached(doc, next) && doc.syn.types[next] == TokenType::Identifier) {
        return false;
    }
    let prev_char = if i == first {
        ' '
    } else {
        doc.syn.text(i - 1).chars().last().unwrap_or(' ')
    };
    !is_ident_char(prev_char) || !is_attached(doc, i)
}

// ---- DDL_SPACES_AROUND_SIGNS ----

pub fn around_signs(doc: &mut Doc, config: &RuleConfig) {
    let before_comment_sign = change(config, "SpaceBeforeCommentSign", "ALWAYS");
    let after_comment_sign = change(config, "SpaceAfterCommentSign", "ALWAYS");
    let before_colon = change(config, "SpaceBeforeColon", "KEEP_AS_IS");
    let after_colon = change(config, "SpaceAfterColon", "ALWAYS");
    let before_comma = change(config, "SpaceBeforeComma", "NEVER");
    let after_comma = change(config, "SpaceAfterComma", "ALWAYS");
    let after_comma_in_type = change(config, "SpaceAfterCommaInAbapType", "NEVER");
    let around_arith = change(config, "SpaceAroundArithmeticOps", "ALWAYS");

    let cmds = doc.commands().to_vec();
    for (ci, cmd) in cmds.iter().enumerate() {
        if is_annotation(doc, cmd.first) {
            continue;
        }
        for t in cmd.first..=cmd.last {
            // line-end comments
            let text = doc.token_text(t).to_string();
            if is_comment(doc, t) && (text.starts_with("//") || text.starts_with("--")) {
                if before_comment_sign != Change::KeepAsIs && doc.lines[t] == 0 && ci > 0 {
                    let spaces = if before_comment_sign == Change::Always {
                        doc.spaces[t].max(1)
                    } else {
                        0
                    };
                    doc.set_spaces(t, spaces);
                }
                let at_line_start = doc.lines[t] > 0 || ci == 0;
                if at_line_start && doc.spaces[t] == 0 && text.starts_with("//") {
                    // "//..." at position 0 is often commented-out code
                } else if after_comment_sign != Change::KeepAsIs && text.chars().count() > 2 {
                    let next_char = text.chars().nth(2).unwrap();
                    let first_char = text.chars().next().unwrap();
                    if after_comment_sign == Change::Always
                        && next_char != ' '
                        && next_char != first_char
                    {
                        doc.text[t] = Some(format!("{} {}", &text[..2], &text[2..]));
                    } else if after_comment_sign == Change::Never && next_char == ' ' {
                        doc.text[t] = Some(format!("{}{}", &text[..2], text[2..].trim_start()));
                    }
                }
            }

            if is_comment(doc, t) || doc.lines[t] > 0 {
                continue;
            }
            let ttext = doc.syn.text(t).to_string();

            if before_colon != Change::KeepAsIs && ttext == ":" {
                let in_path = parent(doc, t).is_some_and(|p| doc.syn.text(p).ends_with('['));
                if !in_path {
                    let spaces = if before_colon == Change::Always {
                        i64::from(doc.spaces[t]).max(1)
                    } else {
                        0
                    };
                    doc.set_spaces_adjusting_indent(cmd.last, t, spaces, false);
                }
            }
            if before_comma != Change::KeepAsIs && ttext == "," {
                let spaces = if before_comma == Change::Always {
                    i64::from(doc.spaces[t]).max(1)
                } else {
                    0
                };
                doc.set_spaces_adjusting_indent(cmd.last, t, spaces, false);
            }

            // previous token; for list elements, commas belong to the previous command
            let prev_token = if t > cmd.first {
                Some(t - 1)
            } else if ci > 0 {
                Some(cmds[ci - 1].last)
            } else {
                None
            };
            let next_token = (t < cmd.last).then_some(t + 1);
            let pc = prev_code(doc, cmd.first, t);
            if around_arith != Change::KeepAsIs
                && matches!(ttext.as_str(), "+" | "-" | "*" | "/")
                && pc.is_some_and(|p| may_be_followed_by_arithmetic_op(doc, p))
            {
                let child_of_assoc = parent(doc, t)
                    .and_then(|p| prev_code(doc, cmd.first, p))
                    .is_some_and(|p| {
                        let x = doc.syn.text(p);
                        x.eq_ignore_ascii_case("ASSOCIATION")
                            || x.eq_ignore_ascii_case("COMPOSITION")
                    });
                let prev_ends_dot = prev_token.is_some_and(|p| doc.syn.text(p).ends_with('.'));
                let in_path_bracket =
                    parent(doc, t).is_some_and(|p| doc.syn.text(p).ends_with('['));
                let star_in_parens = ttext == "*"
                    && prev_token.is_some_and(|p| doc.syn.text(p).ends_with('('))
                    && next_token.is_some_and(|n| doc.syn.text(n).starts_with(')'));
                if ttext == "*" && child_of_assoc {
                } else if ttext == "*" && prev_ends_dot {
                } else if ttext == "*" && in_path_bracket {
                } else if star_in_parens {
                } else if ttext == "/" && around_arith == Change::Never {
                } else {
                    let spaces = if around_arith == Change::Always {
                        i64::from(doc.spaces[t]).max(1)
                    } else {
                        0
                    };
                    doc.set_spaces_adjusting_indent(cmd.last, t, spaces, true);
                }
            }

            // space after colon, comma or arithmetic operator (the previous token)
            let Some(pt) = prev_token else { continue };
            let ptext = doc.syn.text(pt).to_string();
            let mut ct = Change::KeepAsIs;
            if ptext == ":" {
                ct = after_colon;
            } else if ptext == "," {
                let par = parent(doc, t);
                let par_prev = par.and_then(|p| prev_code(doc, cmd.first, p));
                ct = if par_prev.is_some_and(|p| doc.syn.text(p).starts_with("abap.")) {
                    after_comma_in_type
                } else {
                    after_comma
                };
            } else if matches!(ptext.as_str(), "+" | "-" | "*" | "/") {
                let ppc = if pt >= cmd.first {
                    prev_code(doc, cmd.first, pt)
                } else {
                    None
                };
                if ppc.is_some_and(|p| may_be_followed_by_arithmetic_op(doc, p)) {
                    ct = around_arith;
                }
            }
            if ct == Change::KeepAsIs {
                continue;
            }
            if ptext == "*" && (ttext == "]" || ttext == ":" || ttext == ")") {
                continue;
            }
            if pt >= cmd.first && is_ddl_parameter_colon(doc, cmd.first, cmd.last, pt) {
                continue;
            }
            if ptext == "/" && ct == Change::Never {
                continue;
            }
            let spaces = if ct == Change::Always { 1 } else { 0 };
            doc.set_spaces_adjusting_indent(cmd.last, t, spaces, true);
        }
    }
}

// ---- DDL_SPACES_AROUND_BRACKETS ----

pub fn around_brackets(doc: &mut Doc, config: &RuleConfig) {
    let card_around = change(config, "SpacesAroundCardBrackets", "ALWAYS");
    let card_inside = change(config, "SpacesInsideCardBrackets", "NEVER");
    let path_before = change(config, "SpacesBeforePathBrackets", "NEVER");
    let path_inside = change(config, "SpacesInsidePathBrackets", "NEVER");
    let func_before = change(config, "SpacesBeforeFuncParens", "NEVER");
    let func_inside = change(config, "SpacesInsideFuncParens", "NEVER");
    let type_before = change(config, "SpacesBeforeTypeParens", "NEVER");
    let type_inside = change(config, "SpacesInsideTypeParens", "NEVER");
    let arith_inside = change(config, "SpacesInsideArithParens", "NEVER");

    let cmds = doc.commands().to_vec();
    for cmd in &cmds {
        if is_annotation(doc, cmd.first) || is_comment_line_command(doc, cmd.first, cmd.last) {
            continue;
        }
        for t in cmd.first..=cmd.last {
            if is_comment(doc, t) {
                continue;
            }
            let text = doc.syn.text(t).to_string();
            let closing = text == "]" || text == ")";
            if text == "[" || text == "]" {
                let mut pc = prev_code_sibling(doc, cmd.first, t);
                if closing {
                    pc = pc.and_then(|p| prev_code_sibling(doc, cmd.first, p));
                }
                let is_assoc = pc.is_some_and(|p| {
                    let x = doc.syn.text(p);
                    doc.syn.types[p] == TokenType::Keyword
                        && (x.eq_ignore_ascii_case("ASSOCIATION")
                            || x.eq_ignore_ascii_case("COMPOSITION"))
                });
                if text == "[" {
                    let (b, a) = if is_assoc {
                        (card_around, card_inside)
                    } else {
                        (path_before, path_inside)
                    };
                    on_opening(doc, cmd.last, t, b, a);
                } else {
                    let (b, a) = if is_assoc {
                        (card_inside, card_around)
                    } else {
                        (path_inside, Change::KeepAsIs)
                    };
                    on_closing(doc, cmd.last, t, b, a);
                }
            } else if text == "(" || text == ")" {
                let mut pc = prev_code_sibling(doc, cmd.first, t);
                if closing {
                    pc = pc.and_then(|p| prev_code_sibling(doc, cmd.first, p));
                }
                let is_type = pc.is_some_and(|p| doc.syn.text(p).starts_with("abap."));
                let is_function = !is_type
                    && pc.is_some_and(|p| {
                        let x = doc.syn.text(p);
                        !x.is_empty()
                            && x.chars().enumerate().all(|(k, c)| {
                                c.is_ascii_alphabetic() || c == '_' || (k > 0 && c.is_ascii_digit())
                            })
                    });
                if text == "(" {
                    let b = if is_type {
                        type_before
                    } else if is_function {
                        func_before
                    } else {
                        Change::KeepAsIs
                    };
                    let a = if is_type {
                        type_inside
                    } else if is_function {
                        func_inside
                    } else {
                        arith_inside
                    };
                    on_opening(doc, cmd.last, t, b, a);
                } else {
                    let b = if is_type {
                        type_inside
                    } else if is_function {
                        func_inside
                    } else {
                        arith_inside
                    };
                    on_closing(doc, cmd.last, t, b, Change::KeepAsIs);
                }
            }
        }
    }
}

fn on_opening(doc: &mut Doc, cmd_last: usize, t: usize, before: Change, after: Change) {
    if before != Change::KeepAsIs && doc.lines[t] == 0 {
        let spaces = if before == Change::Always {
            i64::from(doc.spaces[t]).max(1)
        } else {
            0
        };
        doc.set_spaces_adjusting_indent(cmd_last, t, spaces, true);
    }
    if after != Change::KeepAsIs && t < cmd_last {
        let next = t + 1;
        // the first child of the opening bracket
        if doc.syn.parent[next] == Some(t) && doc.lines[next] == 0 && !is_comment(doc, next) {
            let spaces = if after == Change::Always {
                i64::from(doc.spaces[next]).max(1)
            } else {
                0
            };
            doc.set_spaces_adjusting_indent(cmd_last, next, spaces, true);
        }
    }
}

fn on_closing(doc: &mut Doc, cmd_last: usize, t: usize, before: Change, after: Change) {
    if before != Change::KeepAsIs && doc.lines[t] == 0 {
        let spaces = if before == Change::Always {
            i64::from(doc.spaces[t]).max(1)
        } else {
            0
        };
        doc.set_spaces_adjusting_indent(cmd_last, t, spaces, true);
    }
    if after != Change::KeepAsIs && t < cmd_last {
        let next = t + 1;
        if doc.lines[next] == 0 && !is_comment(doc, next) {
            let spaces = if after == Change::Always {
                i64::from(doc.spaces[next]).max(1)
            } else {
                0
            };
            doc.set_spaces_adjusting_indent(cmd_last, next, spaces, true);
        }
    }
}
