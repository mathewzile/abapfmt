//! ALIGN_CLEAR_FREE_AND_SORT: puts the operands of CLEAR:, FREE:, SORT ... BY and CATCH on
//! distinct lines or fills lines up to the maximum line length.
use abapfmt_syntax::TokenType;

use crate::align::{end_x, is_comment};
use crate::rules::align_declarations::{
    find_seq_last, is_kw, matches_seq, next_non_comment_sibling,
};
use crate::{ConfigExt, Doc, RuleConfig};

/// `Token.getMaxIndexInLine(endToken)`; `end` is exclusive.
fn max_index_in_line(doc: &Doc, token: usize, end: usize) -> i64 {
    let mut max = 0;
    for t in token..end {
        if t + 1 == end || doc.lines[t + 1] > 0 {
            max = max.max(end_x(doc, t));
        }
    }
    max
}

fn prev_code(doc: &Doc, t: usize, first: usize) -> Option<usize> {
    (first..t).rev().find(|j| !is_comment(doc, *j))
}

pub fn align_clear_free_and_sort(doc: &mut Doc, config: &RuleConfig) {
    let clear_mode = config.choice("DistinctLineClear", "ALWAYS");
    let free_mode = config.choice("DistinctLineFree", "ALWAYS");
    let sort_mode = config.choice("DistinctLineSort", "ALWAYS");
    let catch_mode = config.choice("DistinctLineCatch", "ALWAYS");
    let max_line_length = config.int("MaxLineLength", 120);

    for cmd in doc.commands().to_vec() {
        if cmd.is_comment_only {
            continue;
        }
        let Some(first) = doc.first_code_token(&cmd) else {
            continue;
        };
        let last = cmd.last;
        let kw = doc.syn.text(first).to_ascii_uppercase();
        let mut tokens: Vec<usize> = Vec::new();
        let distinct_line;
        match kw.as_str() {
            "CLEAR" | "FREE" => {
                let is_chain = cmd.colon.is_some()
                    && doc.next_code_token(first, last) == cmd.colon
                    && doc.syn.types[first] == TokenType::Keyword;
                let has_comma = (first..=last).any(|t| doc.syn.types[t] == TokenType::Comma);
                if !is_chain || !has_comma {
                    continue;
                }
                if kw == "CLEAR" {
                    if clear_mode == "KEEP_AS_IS" {
                        continue;
                    }
                    distinct_line = clear_mode == "ALWAYS"
                        || clear_mode == "ONLY_WITH_ADDITIONS"
                            && find_seq_last(doc, first, last, &["WITH"]).is_some();
                } else {
                    if free_mode == "KEEP_AS_IS" {
                        continue;
                    }
                    distinct_line = free_mode == "ALWAYS";
                }
                let mut t = next_non_comment_sibling(doc, first, last);
                while let Some(tok) = t {
                    let use_next =
                        matches!(doc.syn.types[tok], TokenType::Colon | TokenType::Comma);
                    t = next_non_comment_sibling(doc, tok, last);
                    if use_next && let Some(n) = t {
                        tokens.push(n);
                    }
                }
            }
            "SORT" => {
                if sort_mode == "KEEP_AS_IS" {
                    continue;
                }
                let mut addition_found = false;
                let mut t = find_seq_last(doc, first, last, &["BY"]);
                while let Some(tok) = t {
                    let text = doc.syn.text(tok);
                    if text.starts_with('(') || is_kw(doc, tok, "VALUE") {
                        // SORT itab BY (otab) / BY VALUE #( ... ) are skipped
                    } else if doc.syn.types[tok] == TokenType::Identifier
                        && !doc.syn.bracket_match[tok].is_some_and(|c| c > tok)
                    {
                        tokens.push(tok);
                        if !addition_found
                            && let Some(n) = next_non_comment_sibling(doc, tok, last)
                            && (is_kw(doc, n, "ASCENDING")
                                || is_kw(doc, n, "DESCENDING")
                                || matches_seq(doc, n, last, &["AS", "TEXT"]))
                        {
                            addition_found = true;
                        }
                    }
                    t = next_non_comment_sibling(doc, tok, last);
                    if let Some(n) = t
                        && doc.syn.types[n] == TokenType::Comma
                    {
                        t = find_seq_last(doc, n, last, &["BY"]);
                    }
                }
                distinct_line =
                    sort_mode == "ALWAYS" || sort_mode == "ONLY_WITH_ADDITIONS" && addition_found;
            }
            "CATCH" => {
                if catch_mode == "KEEP_AS_IS" {
                    continue;
                }
                distinct_line = catch_mode == "ALWAYS";
                let mut t = next_non_comment_sibling(doc, first, last);
                if t.is_some_and(|x| is_kw(doc, x, "BEFORE")) {
                    t = t.and_then(|x| next_non_comment_sibling(doc, x, last));
                }
                if t.is_some_and(|x| is_kw(doc, x, "UNWIND")) {
                    t = t.and_then(|x| next_non_comment_sibling(doc, x, last));
                }
                if t.is_none_or(|x| doc.syn.types[x] != TokenType::Identifier) {
                    continue;
                }
                while let Some(tok) = t.filter(|x| {
                    doc.syn.types[*x] == TokenType::Identifier || is_kw(doc, *x, "INTO")
                }) {
                    if distinct_line && is_kw(doc, tok, "INTO") && !is_comment(doc, tok - 1) {
                        doc.lines[tok] = 0;
                        doc.spaces[tok] = 1;
                        break;
                    }
                    tokens.push(tok);
                    if is_kw(doc, tok, "INTO") {
                        break;
                    }
                    t = next_non_comment_sibling(doc, tok, last);
                }
            }
            _ => continue,
        }
        if tokens.len() < 2 {
            continue;
        }

        let mut indent = 0i64;
        for i in 0..tokens.len() {
            let token = tokens[i];
            let end_token = tokens.get(i + 1).copied();
            let end_excl = end_token.unwrap_or(last + 1);
            let prev_code_tok = prev_code(doc, token, cmd.first);
            let is_first_section =
                i == 0 || kw == "SORT" && prev_code_tok.is_some_and(|p| is_kw(doc, p, "BY"));
            if is_first_section {
                indent = if doc.lines[token] == 0 {
                    end_x(doc, token - 1) + 1
                } else {
                    doc.start_x(0, token)
                };
            }
            let first_after_break = (token + 1..end_excl).find(|t| doc.lines[*t] > 0);
            let old_start = doc.start_x(0, token);
            let width = max_index_in_line(doc, token, end_excl) - old_start;
            let mut move_by = 0;
            if is_first_section {
                // only condense multiple spaces, e.g. in "CLEAR:   lv_any_value, ..."
                if doc.lines[token] == 0 && doc.spaces[token] > 1 {
                    move_by = indent - old_start;
                    doc.spaces[token] = 1;
                }
            } else if distinct_line
                || is_comment(doc, token - 1)
                || end_x(doc, token - 1) + 1 + width > max_line_length
            {
                move_by = indent - old_start;
                doc.lines[token] = 1;
                doc.spaces[token] = indent.max(0) as u32;
            } else {
                move_by = end_x(doc, token - 1) + 1 - old_start;
                doc.lines[token] = 0;
                doc.spaces[token] = 1;
            }
            if move_by != 0
                && let Some(from) = first_after_break
            {
                doc.add_indent(end_excl - 1, from, move_by, old_start, true);
            }
        }
    }
}
