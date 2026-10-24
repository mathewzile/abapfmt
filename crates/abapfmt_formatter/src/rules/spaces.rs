//! SPACES group rules.
use abapfmt_syntax::{Command, TokenType};

use crate::{ConfigExt, Doc, RuleConfig};

fn is_comment(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::Comment
}

/// A comment that is alone on its line (abap-cleaner's `Token.isCommentLine`).
fn is_comment_line(doc: &Doc, order: &[usize], pos: usize) -> bool {
    let i = order[pos];
    is_comment(doc, i)
        && (doc.lines[i] > 0 || order[0] == i && doc.syn.tokens[i].line == 1)
        && order.get(pos + 1).is_none_or(|n| doc.lines[*n] > 0)
}

/// SPACE_BEFORE_PERIOD: remove spaces before chain commas and the period, moving them up across
/// line-end comments.
pub fn space_before_period(doc: &mut Doc, config: &RuleConfig) {
    let on_comma = config.bool("ExecuteOnComma", true);
    let on_period = config.bool("ExecuteOnPeriod", true);
    let move_across = config.bool("MoveAcrossCommentLines", true);
    let on_class_def = config.bool("ExecuteOnClassDefinitionSections", true);
    if !on_comma && !on_period {
        return;
    }
    let cmds = doc.commands().to_vec();
    for (ci, cmd) in cmds.iter().enumerate() {
        if cmd.is_comment_only || (!on_class_def && cmd.in_class_definition) {
            continue;
        }
        let candidates: Vec<usize> = (cmd.first..=cmd.last)
            .filter(|&i| {
                doc.syn.parent[i].is_none()
                    && match doc.syn.types[i] {
                        TokenType::Comma => on_comma,
                        TokenType::Period => on_period,
                        _ => false,
                    }
            })
            .collect();
        for t in candidates {
            comma_or_period(doc, ci, cmd, t, move_across);
        }
    }
}

fn comma_or_period(doc: &mut Doc, ci: usize, cmd: &Command, t: usize, move_across: bool) {
    let order = doc.order_of(cmd).to_vec();
    let pos = order.iter().position(|x| *x == t).unwrap();
    let Some(prev_pos) = pos.checked_sub(1) else {
        return;
    };
    let prev = order[prev_pos];
    let next = order.get(pos + 1).copied();

    // after "/" as in `WRITE: / 'text', / .` keep one space
    let required = u32::from(doc.syn.text(prev) == "/");
    if doc.lines[t] == 0 {
        if doc.spaces[t] > required {
            doc.set_spaces(t, required);
        }
        return;
    }

    if is_comment_line(doc, &order, prev_pos) {
        if !move_across {
            return;
        }
        // old ABAP versions generate this comment when saving "FUNCTION any_function."; moving the
        // period would make them generate it again
        if doc.first_code_text(cmd).as_deref() == Some("FUNCTION")
            && doc.syn.text(prev).contains("add here the signature")
        {
            let first = doc.first_code_token(cmd).unwrap();
            let name = doc.next_code_token(first, cmd.last);
            let before_t = order[..pos]
                .iter()
                .rev()
                .find(|i| !matches!(doc.syn.types[**i], TokenType::Comment | TokenType::Pragma));
            if name.is_some() && name == before_t.copied() {
                return;
            }
        }
    }

    if doc.syn.types[t] == TokenType::Comma
        && let Some(n) = next
        && doc.lines[n] == 0
    {
        doc.lines[n] = doc.lines[t];
        doc.spaces[n] = doc.spaces[t];
    }

    if !is_comment(doc, prev) {
        doc.set_lines(t, 0);
        doc.set_spaces(t, required);
    } else if doc.syn.types[t] == TokenType::Period && next.is_some() {
        // do not merge comments before and after the period
    } else {
        let mut j = prev_pos;
        while j > 0 && is_comment(doc, order[j - 1]) {
            j -= 1;
        }
        let first_comment = order[j];
        let trailing_comment_lines = next.is_none() && is_comment_line(doc, &order, prev_pos);
        doc.move_before(cmd, t, first_comment);
        doc.set_lines(t, 0);
        doc.set_spaces(t, required);
        if trailing_comment_lines {
            // comment lines that now trail the period become commands of their own, which
            // abap-cleaner indents like the code that follows
            let indent = doc.indent_after(ci);
            let order = doc.order_of(cmd).to_vec();
            for pos in (0..order.len()).rev() {
                let i = order[pos];
                if !is_comment_line(doc, &order, pos) {
                    break;
                }
                if !doc.syn.text(i).starts_with('*') {
                    doc.set_spaces(i, indent);
                }
            }
        }
    }
}

/// SPACE_AROUND_COMMENT_SIGN: at least one space before and after the `"` comment sign.
pub fn space_around_comment_sign(doc: &mut Doc, config: &RuleConfig) {
    let before = config.bool("SpaceBeforeCommentSign", true);
    let after = config.bool("SpaceAfterCommentSign", true);
    for i in 0..doc.syn.tokens.len() {
        if doc.syn.types[i] != TokenType::Comment || !doc.syn.text(i).starts_with('"') {
            continue;
        }
        let first_in_line = doc.lines[i] > 0 || (doc.syn.tokens[i].line == 1 && i == 0);
        if before && !first_in_line && doc.spaces[i] == 0 {
            doc.set_spaces(i, 1);
        }
        // not for "# pseudo comments, "! ABAP Doc and "" commented-out comments
        if after
            && doc.syn.text(i)[1..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
        {
            let text = format!("\" {}", &doc.syn.text(i)[1..]);
            doc.text[i] = Some(text);
        }
    }
}

const SQL_LITERAL_TYPES: &[&str] = &[
    "INT1",
    "INT2",
    "INT4",
    "INT8",
    "DEC",
    "DECFLOAT16",
    "D16N",
    "DF16_RAW",
    "D16R",
    "DECFLOAT34",
    "D34N",
    "DF34_RAW",
    "D34R",
    "DF16_DEC",
    "D16D",
    "DF34_DEC",
    "D34D",
    "FLTP",
    "CHAR",
    "STRING",
    "SSTRING",
    "RAW",
    "RAWSTRING",
    "NUMC",
    "CLNT",
    "LANG",
    "DATS",
    "DATN",
    "TIMS",
    "TIMN",
    "UTCLONG",
    "CURR",
    "CUKY",
    "QUAN",
    "UNIT",
];

fn is_character_literal(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::Literal && doc.syn.text(i).starts_with(['\'', '`'])
}

fn is_attached(doc: &Doc, i: usize) -> bool {
    doc.lines[i] == 0 && doc.spaces[i] == 0
}

/// Put a space between token `t` and its predecessor, moving the rest of the command's lines along.
fn detach(doc: &mut Doc, cmd: &Command, t: usize) {
    let start = doc.start_x(cmd.first, t);
    doc.spaces[t] = 1;
    doc.add_indent(cmd.last, t, 1, start, true);
}

/// SPACES_IN_EMPTY_BRACKETS (upstream class `SpaceAroundTextLiteralRule`): spaces before and after
/// text field literals `'...'` and text string literals `` `...` ``.
pub fn space_around_text_literal(doc: &mut Doc, config: &RuleConfig) {
    let from_keywords = config.bool("SeparateFromKeywords", true);
    let from_operators = config.bool("SeparateFromOperators", true);
    let from_comments = config.bool("SeparateFromComments", true);
    let from_brackets = config.bool(
        "SeparateFromBrackets",
        config.bool("SeparateFromCharLiterals", true),
    );
    let from_bracket_pairs = config.bool(
        "SeparateFromBracketPairs",
        config.bool("SeparateCondensedCases", true),
    );

    for cmd in doc.commands().to_vec() {
        for t in cmd.first..=cmd.last {
            if !is_character_literal(doc, t) {
                continue;
            }
            if t > cmd.first && is_attached(doc, t) {
                let prev = t - 1;
                let prev_text = doc.syn.text(prev);
                let prev_type = doc.syn.types[prev];
                let prev_is_textual_op = prev_type == TokenType::ComparisonOp
                    && prev_text.starts_with(|c: char| c.is_alphabetic());
                let opener_close = doc.syn.bracket_match[prev].filter(|c| *c > prev);
                let detach_from_prev = if SQL_LITERAL_TYPES
                    .iter()
                    .any(|w| prev_text.eq_ignore_ascii_case(w))
                {
                    false
                } else if let Some(close) = opener_close {
                    // exclude ('...') where spaces could be a syntax error, as in CALL METHOD ('NAME')
                    // or ls_struc-('COMPONENT'); harmless cases such as CONV #('12.34') are optional
                    if close == t + 1 && is_attached(doc, close) {
                        let dynamic = matches!(prev_text, "(" | "[")
                            || ["->(", "=>(", "-("].iter().any(|s| prev_text.ends_with(s));
                        if dynamic || !from_bracket_pairs {
                            continue;
                        }
                    }
                    from_brackets
                } else if prev_text == "&&" {
                    from_operators
                } else if prev_type == TokenType::Keyword || prev_is_textual_op {
                    from_keywords
                } else {
                    matches!(
                        prev_type,
                        TokenType::AssignmentOp
                            | TokenType::OtherOp
                            | TokenType::Comma
                            | TokenType::Colon
                            | TokenType::ComparisonOp
                    ) && from_operators
                };
                if detach_from_prev {
                    detach(doc, &cmd, t);
                }
            }
            let next = t + 1;
            if next <= cmd.last && is_attached(doc, next) {
                let detach_from_next = if doc.syn.types[next] == TokenType::Comment {
                    from_comments
                } else {
                    doc.syn.bracket_match[next].is_some_and(|o| o < next)
                        && doc.syn.text(next).starts_with(')')
                        && from_brackets
                };
                if detach_from_next {
                    detach(doc, &cmd, next);
                }
            }
        }
    }
}
