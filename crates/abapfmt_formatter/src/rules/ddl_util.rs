//! Helpers shared by the DDL rules: the command hierarchy and abap-cleaner's `Command.startsDdl*`
//! predicates.
use abapfmt_syntax::TokenType;

use crate::Doc;

pub fn info<'d>(doc: &'d Doc) -> &'d abapfmt_syntax::DdlInfo {
    doc.syn.ddl.as_ref().expect("DDL document")
}

pub fn is_comment(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::Comment
}

/// `Command.isCommentLine`: the command consists of comments only.
pub fn is_comment_line(doc: &Doc, ci: usize) -> bool {
    let c = &doc.syn.commands[ci];
    c.is_comment_only
}

pub fn first_code(doc: &Doc, ci: usize) -> Option<usize> {
    let c = &doc.syn.commands[ci];
    (c.first..=c.last).find(|i| !is_comment(doc, *i))
}

pub fn last_code(doc: &Doc, ci: usize) -> Option<usize> {
    let c = &doc.syn.commands[ci];
    (c.first..=c.last).rev().find(|i| !is_comment(doc, *i))
}

pub fn is_kw(doc: &Doc, i: usize, text: &str) -> bool {
    doc.syn.types[i] == TokenType::Keyword && doc.syn.text(i).eq_ignore_ascii_case(text)
}

pub fn is_any_kw(doc: &Doc, i: usize, texts: &[&str]) -> bool {
    texts.iter().any(|t| is_kw(doc, i, t))
}

pub fn is_annotation(doc: &Doc, ci: usize) -> bool {
    first_code(doc, ci).is_some_and(|f| doc.syn.text(f).starts_with('@'))
}

pub fn opens_level(doc: &Doc, ci: usize) -> bool {
    info(doc).cmd_opens[ci]
}

pub fn closes_level(doc: &Doc, ci: usize) -> bool {
    info(doc).cmd_closes[ci]
}

pub fn cmd_parent(doc: &Doc, ci: usize) -> Option<usize> {
    info(doc).cmd_parent[ci]
}

pub fn prev_non_comment(doc: &Doc, ci: usize) -> Option<usize> {
    (0..ci).rev().find(|c| !is_comment_line(doc, *c))
}

/// `Command.getPrevSibling`: a command below the same parent; a closing command follows its
/// opener.
pub fn prev_sibling(doc: &Doc, ci: usize) -> Option<usize> {
    let p = cmd_parent(doc, ci);
    (0..ci).rev().find(|c| cmd_parent(doc, *c) == p)
}

pub fn next_sibling(doc: &Doc, ci: usize) -> Option<usize> {
    let p = cmd_parent(doc, ci);
    (ci + 1..doc.syn.commands.len()).find(|c| cmd_parent(doc, *c) == p)
}

pub fn prev_non_comment_sibling(doc: &Doc, ci: usize) -> Option<usize> {
    let mut c = prev_sibling(doc, ci);
    while let Some(x) = c {
        if !is_comment_line(doc, x) {
            return Some(x);
        }
        c = prev_sibling(doc, x);
    }
    None
}

pub fn next_non_comment_sibling(doc: &Doc, ci: usize) -> Option<usize> {
    let mut c = next_sibling(doc, ci);
    while let Some(x) = c {
        if !is_comment_line(doc, x) {
            return Some(x);
        }
        c = next_sibling(doc, x);
    }
    None
}

pub fn first_child(doc: &Doc, ci: usize) -> Option<usize> {
    if !opens_level(doc, ci) {
        return None;
    }
    (ci + 1..doc.syn.commands.len())
        .take(1)
        .find(|c| cmd_parent(doc, *c) == Some(ci))
}

/// `Command.getStartOfAttachedComments`: comment lines directly above the command.
pub fn start_of_attached_comments(doc: &Doc, ci: usize) -> usize {
    let mut c = ci;
    while doc.lines[doc.syn.commands[c].first] == 1 && c > 0 && is_comment_line(doc, c - 1) {
        c -= 1;
    }
    c
}

pub fn starts_entity_parameters(doc: &Doc, ci: usize) -> bool {
    opens_level(doc, ci) && last_code(doc, ci).is_some_and(|t| is_kw(doc, t, "PARAMETERS"))
}

pub fn starts_select_list_with_brace(doc: &Doc, ci: usize) -> bool {
    opens_level(doc, ci) && last_code(doc, ci).is_some_and(|t| doc.syn.text(t) == "{")
}

pub fn starts_select_list_before_from(doc: &Doc, ci: usize) -> bool {
    opens_level(doc, ci)
        && last_code(doc, ci).is_some_and(|t| is_any_kw(doc, t, &["SELECT", "DISTINCT"]))
}

pub fn ends_select_list_with_brace(doc: &Doc, ci: usize) -> bool {
    closes_level(doc, ci) && first_code(doc, ci).is_some_and(|t| doc.syn.text(t) == "}")
}

// ---- sequences of sibling tokens ----

/// One element of a sibling-token pattern (`TokenSearch`).
#[derive(Clone, Copy)]
pub enum P<'a> {
    /// A keyword or one of several alternatives (`"A|B"`), case-insensitive.
    T(&'a str),
    Opt(&'a str),
    Ident,
}

/// Next code token on the same level as `i`, up to `last`.
pub fn next_code_sibling(doc: &Doc, i: usize, last: usize) -> Option<usize> {
    let pi = doc.syn.parent[i];
    for j in i + 1..=last {
        let pj = doc.syn.parent[j];
        if pj == pi {
            if !is_comment(doc, j) {
                return Some(j);
            }
        } else if !pj.is_some_and(|p| p >= i) {
            return None;
        }
    }
    None
}

fn alt_match(doc: &Doc, i: usize, alts: &str) -> bool {
    alts.split('|')
        .any(|a| doc.syn.text(i).eq_ignore_ascii_case(a))
}

/// `Token.getLastTokenOnSiblings`: the last token of the pattern if it matches the code siblings
/// starting at `start`.
pub fn last_of_sequence(doc: &Doc, start: usize, last: usize, pats: &[P]) -> Option<usize> {
    let mut token = Some(start);
    let mut result = None;
    let mut k = 0;
    while k < pats.len() {
        let t = token?;
        let (matched, optional) = match pats[k] {
            P::T(a) => (alt_match(doc, t, a), false),
            P::Opt(a) => (alt_match(doc, t, a), true),
            P::Ident => (doc.syn.types[t] == TokenType::Identifier, false),
        };
        if matched {
            result = Some(t);
            token = next_code_sibling(doc, t, last);
            k += 1;
        } else if optional {
            k += 1;
        } else {
            return None;
        }
    }
    result
}

pub fn matches_on_siblings(doc: &Doc, start: usize, last: usize, pats: &[P]) -> bool {
    last_of_sequence(doc, start, last, pats).is_some()
}

pub fn token_starts_join(doc: &Doc, t: usize, last: usize) -> bool {
    is_any_kw(doc, t, &["INNER", "LEFT", "RIGHT", "CROSS", "JOIN"])
        || matches_on_siblings(doc, t, last, &[P::T("EXACT"), P::T("ONE"), P::T("TO")])
        || matches_on_siblings(doc, t, last, &[P::T("MANY|ONE"), P::T("TO")])
        || matches_on_siblings(doc, t, last, &[P::T("TO"), P::T("ONE|EXACT|MANY")])
}

pub fn token_starts_association(doc: &Doc, t: usize, last: usize) -> bool {
    is_any_kw(doc, t, &["ASSOCIATION", "COMPOSITION"])
        || matches_on_siblings(doc, t, last, &[P::T("REDEFINE"), P::T("ASSOCIATION")])
}

pub fn starts_join(doc: &Doc, ci: usize) -> bool {
    first_code(doc, ci).is_some_and(|f| token_starts_join(doc, f, doc.syn.commands[ci].last))
}

pub fn starts_association(doc: &Doc, ci: usize) -> bool {
    first_code(doc, ci).is_some_and(|f| token_starts_association(doc, f, doc.syn.commands[ci].last))
}

/// The data source after `FROM`, if the (top-level) command contains a FROM clause.
pub fn from_data_source(doc: &Doc, ci: usize) -> Option<usize> {
    if cmd_parent(doc, ci).is_some() {
        return None;
    }
    let c = &doc.syn.commands[ci];
    let first = c.first;
    let from = (first..=c.last).find(|i| doc.syn.parent[*i].is_none() && is_kw(doc, *i, "FROM"))?;
    next_code_sibling(doc, from, c.last)
}

pub fn starts_from_clause(doc: &Doc, ci: usize) -> bool {
    from_data_source(doc, ci).is_some()
}

pub fn starts_definition(doc: &Doc, ci: usize) -> bool {
    entity_name_token(doc, ci).is_some()
}

/// `Command.getDdlOrDclEntityNameToken`.
pub fn entity_name_token(doc: &Doc, ci: usize) -> Option<usize> {
    if cmd_parent(doc, ci).is_some() {
        return None;
    }
    let start = first_code(doc, ci)?;
    if doc.syn.types[start] != TokenType::Keyword {
        return None;
    }
    let last = doc.syn.commands[ci].last;
    use P::{Ident, Opt, T};
    let seqs: &[&[P]] = &[
        &[
            Opt("DEFINE"),
            Opt("ROOT"),
            T("ABSTRACT|CUSTOM"),
            T("ENTITY"),
            Ident,
        ],
        &[Opt("DEFINE"), Opt("ROOT"), T("VIEW"), Opt("ENTITY"), Ident],
        &[Opt("DEFINE"), T("TABLE"), T("FUNCTION"), Ident],
        &[Opt("DEFINE"), T("HIERARCHY"), Ident],
        &[T("DEFINE"), T("TRANSIENT"), T("VIEW"), Opt("ENTITY"), Ident],
        &[T("EXTEND"), T("ABSTRACT|CUSTOM"), T("VIEW"), Ident],
        &[T("EXTEND"), T("VIEW"), Opt("ENTITY"), Ident],
        &[T("ANNOTATE"), T("VIEW|ENTITY"), Ident],
        &[T("DEFINE"), T("STRUCTURE|TABLE"), Ident],
        &[Opt("DEFINE"), T("ROLE|ACCESSPOLICY"), Ident],
    ];
    seqs.iter()
        .find_map(|s| last_of_sequence(doc, start, last, s))
}
