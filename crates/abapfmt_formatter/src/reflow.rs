//! `lineWidth` reflow: after all rules have run, statements with lines longer than the configured
//! width get line breaks at token boundaries (gofmt style: break where the nesting is shallowest,
//! keep what fits). Lines that already fit are never touched, so the pass is idempotent.
//!
//! Only whitespace changes. Chains (`DATA: a, b.`), comments and DDL/DCL sources are left alone.
use abapfmt_syntax::TokenType;

use crate::Doc;
use crate::align::end_x;

/// Number of brackets that enclose token `t`.
fn depth(doc: &Doc, t: usize) -> usize {
    let mut d = 0;
    let mut p = doc.syn.parent[t];
    while let Some(x) = p {
        d += 1;
        p = doc.syn.parent[x];
    }
    d
}

fn is_comment(doc: &Doc, t: usize) -> bool {
    doc.syn.types[t] == TokenType::Comment
}

/// Operators after which a line break reads badly.
fn is_operator(text: &str) -> bool {
    matches!(
        text,
        "=" | "+"
            | "-"
            | "*"
            | "/"
            | "&"
            | "&&"
            | "<"
            | ">"
            | "<="
            | ">="
            | "<>"
            | "+="
            | "-="
            | "*="
            | "/="
            | "&&="
            | "?="
            | "**"
            | "->"
            | "=>"
            | "-->"
            | "|"
    )
}

/// Reflows the ABAP source `src`; returns it unchanged if there is nothing to do.
pub fn reflow(src: &str, width: usize, tabular: usize) -> String {
    let Some(mut doc) = Doc::new(src) else {
        return src.to_string();
    };
    if doc.syn.ddl.is_some() {
        return src.to_string();
    }
    let (width, tabular) = (width as i64, tabular as i64);
    let mut changed = false;
    for ci in 0..doc.syn.commands.len() {
        let cmd = doc.syn.commands[ci].clone();
        if cmd.is_comment_only || cmd.colon.is_some() {
            continue;
        }
        // every break shortens a line, so the number of tokens bounds the iterations
        for _ in cmd.first..=cmd.last {
            if !break_first_overlong_line(&mut doc, cmd.first, cmd.last, width, tabular) {
                break;
            }
            changed = true;
        }
    }
    if changed {
        doc.render()
    } else {
        src.to_string()
    }
}

/// Breaks the first line of the command that is too long; false if there is none (or it cannot
/// be broken).
fn break_first_overlong_line(
    doc: &mut Doc,
    first: usize,
    last: usize,
    width: i64,
    tabular: i64,
) -> bool {
    let mut ls = first;
    while ls <= last {
        // the line is [ls, le]
        let mut le = ls;
        while le < last && doc.lines[le + 1] == 0 {
            le += 1;
        }
        // a trailing comment may be longer than the line; only the code counts
        let mut code_end = le;
        while code_end > ls && is_comment(doc, code_end) {
            code_end -= 1;
        }
        // a row of a table in tabular style may be longer
        let limit = if doc.syn.text(ls) == "(" {
            width.max(tabular)
        } else {
            width
        };
        if end_x(doc, code_end) > limit && break_line(doc, first, ls, code_end, limit) {
            return true;
        }
        ls = le + 1;
    }
    false
}

fn break_line(doc: &mut Doc, first: usize, ls: usize, le: usize, width: i64) -> bool {
    let line_indent = doc.spaces[ls];
    let start_depth = depth(doc, ls);
    type Score = (usize, u8, u8, i64);
    let mut best: Option<(Score, usize, u32)> = None;
    for t in ls + 1..=le {
        if doc.spaces[t] == 0 || is_comment(doc, t) || is_comment(doc, t - 1) {
            continue;
        }
        let (text, prev) = (doc.syn.text(t), doc.syn.text(t - 1));
        // nothing before the break may exceed the width, and punctuation stays with its token
        if end_x(doc, t - 1) > width
            || matches!(text, "." | "," | ")" | "]" | "}")
            || is_operator(text)
            || prev.ends_with(['(', '['])
        {
            continue;
        }
        let d = depth(doc, t);
        let mut indent = line_indent + if ls == first { 2 } else { 0 };
        if d > start_depth {
            indent += 2;
        }
        let x = doc.start_x(first, t);
        if x <= i64::from(indent) {
            continue;
        }
        let score = (
            d,
            u8::from(is_operator(prev)),
            u8::from(doc.syn.types[t] != TokenType::Keyword),
            -x,
        );
        if best.as_ref().is_none_or(|(s, _, _)| score < *s) {
            best = Some((score, t, indent));
        }
    }
    let Some((_, t, indent)) = best else {
        return false;
    };
    doc.set_lines(t, 1);
    doc.set_spaces(t, indent);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_are_untouched() {
        let src = "DATA lv_a TYPE i.\n";
        assert_eq!(reflow(src, 120, 160), src);
    }

    #[test]
    fn long_statements_break_before_keywords() {
        let src =
            "SELECT carrid, connid FROM spfli INTO TABLE @DATA(lt_flights) WHERE carrid = 'LH'.\n";
        let out = reflow(src, 40, 40);
        assert!(out.lines().all(|l| l.len() <= 40), "{out}");
        assert!(out.lines().count() > 1);
        assert_eq!(reflow(&out, 40, 40), out, "idempotent");
    }

    #[test]
    fn breaking_keeps_the_tokens() {
        let src = "lv_result = zcl_helper=>get_value( iv_first = 1 iv_second = lv_other iv_third = 3 ).\n";
        let out = reflow(src, 50, 50);
        assert_eq!(
            out.split_whitespace().collect::<Vec<_>>(),
            src.split_whitespace().collect::<Vec<_>>()
        );
    }
}
