//! DDL_EMPTY_LINES_* rules.
use super::ddl_util::*;
use crate::{ConfigExt, Doc, RuleConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
enum EmptyLine {
    KeepAsIs,
    Never,
    AtLeastOne,
    ExactlyOne,
}

fn empty_line(config: &RuleConfig, name: &str, default: &str) -> EmptyLine {
    match config.choice(name, default).as_str() {
        "NEVER" => EmptyLine::Never,
        "ALWAYS_AT_LEAST_ONE" => EmptyLine::AtLeastOne,
        "ALWAYS_EXACTLY_ONE" => EmptyLine::ExactlyOne,
        _ => EmptyLine::KeepAsIs,
    }
}

/// `Rule` for DDL_EMPTY_LINES_BETWEEN_SECTIONS.
pub fn between_sections(doc: &mut Doc, config: &RuleConfig) {
    let annos_define = empty_line(config, "BetweenEntityAnnosAndDefine", "ALWAYS_AT_LEAST_ONE");
    let params_select = empty_line(
        config,
        "BetweenParametersAndAsSelect",
        "ALWAYS_AT_LEAST_ONE",
    );
    let from_joins = empty_line(config, "BetweenSelectFromAndJoins", "ALWAYS_EXACTLY_ONE");
    let joins_assocs = empty_line(config, "BetweenJoinsAndAssociations", "ALWAYS_AT_LEAST_ONE");
    let before_list_start = empty_line(config, "BeforeSelectListStart", "ALWAYS_AT_LEAST_ONE");
    let after_list_start = empty_line(config, "AfterSelectListStart", "NEVER");
    let before_list_end = empty_line(config, "BeforeSelectListEnd", "NEVER");
    let after_list_end = empty_line(config, "AfterSelectListEnd", "ALWAYS_AT_LEAST_ONE");
    let max_empty = config.int("MaxConsecutiveEmptyLines", 2);

    if config.bool("RemoveAtDocumentEnd", true) {
        doc.trim_document_end();
    }

    let n = doc.syn.commands.len();
    for ci in 0..n {
        // multiple empty lines within the command
        let max_breaks = (max_empty + 1) as u32;
        let (first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
        for t in first..=last {
            if doc.lines[t] > max_breaks {
                doc.set_lines(t, max_breaks);
            }
        }

        if is_comment_line(doc, ci) {
            continue;
        }
        let Some(prev) = prev_non_comment(doc, ci) else {
            continue;
        };

        if starts_definition(doc, ci) {
            if is_annotation(doc, prev) {
                standardize(doc, ci, annos_define);
            }
        } else if closes_level(doc, ci)
            && prev_sibling(doc, ci).is_some_and(|s| {
                starts_entity_parameters(doc, s) || starts_select_list_before_from(doc, s)
            })
        {
            standardize(doc, ci, params_select);
        } else if starts_join(doc, ci) {
            if starts_from_clause(doc, prev) {
                standardize(doc, ci, from_joins);
            }
        } else if starts_association(doc, ci) {
            if starts_from_clause(doc, prev) {
                standardize(doc, ci, from_joins);
            } else if starts_join(doc, prev) {
                standardize(doc, ci, joins_assocs);
            }
        } else if starts_select_list_with_brace(doc, ci) {
            standardize(doc, ci, before_list_start);
            if let Some(fc) = first_child(doc, ci) {
                standardize(doc, fc, after_list_start);
            }
        } else if ends_select_list_with_brace(doc, ci) {
            standardize(doc, ci, before_list_end);
            if ci + 1 < n {
                standardize(doc, ci + 1, after_list_end);
            }
        }
    }
}

fn standardize(doc: &mut Doc, ci: usize, put: EmptyLine) {
    if put == EmptyLine::KeepAsIs {
        return;
    }
    let change = start_of_attached_comments(doc, ci);
    let first = doc.syn.commands[change].first;
    let lines = doc.lines[first];
    if lines == 0 {
        return;
    }
    let new = match put {
        EmptyLine::AtLeastOne => lines.max(2),
        EmptyLine::ExactlyOne => 2,
        _ => 1,
    };
    doc.set_lines(first, new);
}

// ---- DDL_EMPTY_LINES_WITHIN_SECTIONS ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Condense {
    Never,
    IfOnlyOneLiners,
    IfOnlyDetachedOneLiners,
    Always,
}

fn condense(config: &RuleConfig, name: &str) -> Condense {
    match config.choice(name, "IF_ONLY_DETACHED_ONE_LINERS").as_str() {
        "NEVER" => Condense::Never,
        "IF_ONLY_ONE_LINERS" => Condense::IfOnlyOneLiners,
        "ALWAYS" => Condense::Always,
        _ => Condense::IfOnlyDetachedOneLiners,
    }
}

#[derive(Clone, Copy)]
struct Section {
    first: usize,
    last: usize,
}

struct Opts {
    detach_key: bool,
    detach_assocs: bool,
}

pub fn within_sections(doc: &mut Doc, config: &RuleConfig) {
    let surround_params = config.int("SurroundParameterLineCountMin", 2);
    let surround_joins = config.int("SurroundJoinLineCountMin", 2);
    let surround_assocs = config.int("SurroundAssociationLineCountMin", 2);
    let surround_elements = config.int("SurroundElementLineCountMin", 2);
    let surround_clauses = config.int("SurroundClauseLineCountMin", 2);
    let opts = Opts {
        detach_key: config.bool("DetachKeyFields", true),
        detach_assocs: config.bool("DetachExposedAssociations", true),
    };
    let condense_params = condense(config, "CondenseParameters");
    let condense_joins = condense(config, "CondenseJoins");
    let condense_assocs = condense(config, "CondenseAssociations");
    let condense_elements = condense(config, "CondenseElements");
    let condense_clauses = match config.choice("CondenseClauses", "NEVER").as_str() {
        "NEVER" => Condense::Never,
        "IF_ONLY_ONE_LINERS" => Condense::IfOnlyOneLiners,
        "ALWAYS" => Condense::Always,
        _ => Condense::IfOnlyDetachedOneLiners,
    };

    let n = doc.syn.commands.len();
    for ci in 0..n {
        if is_comment_line(doc, ci) {
            continue;
        }
        let prev_sib = prev_non_comment_sibling(doc, ci);
        if starts_entity_parameters(doc, ci) {
            let mut lists = list_elements(doc, ci, false, &opts);
            if let Some(l) = lists.pop() {
                on_sections(doc, &l, surround_params, condense_params);
            }
        } else if starts_join(doc, ci) && !prev_sib.is_some_and(|p| starts_join(doc, p)) {
            let s = consecutive(doc, ci, starts_join);
            on_sections(doc, &s, surround_joins, condense_joins);
        } else if starts_association(doc, ci)
            && !prev_sib.is_some_and(|p| starts_association(doc, p))
        {
            let s = consecutive(doc, ci, starts_association);
            on_sections(doc, &s, surround_assocs, condense_assocs);
        } else if starts_select_list_with_brace(doc, ci) || starts_select_list_before_from(doc, ci)
        {
            for l in list_elements(doc, ci, true, &opts) {
                on_sections(doc, &l, surround_elements, condense_elements);
            }
        } else if ends_select_list_with_brace(doc, ci) {
            let start = next_non_comment_sibling(doc, ci);
            let s = match start {
                Some(s) => consecutive(doc, s, starts_clause),
                None => Vec::new(),
            };
            on_sections(doc, &s, surround_clauses, condense_clauses);
        }
    }
}

fn starts_clause(doc: &Doc, ci: usize) -> bool {
    let Some(f) = first_code(doc, ci) else {
        return false;
    };
    let last = doc.syn.commands[ci].last;
    is_any_kw(doc, f, &["WHERE", "HAVING"])
        || matches_on_siblings(doc, f, last, &[P::T("GROUP"), P::T("BY")])
}

fn consecutive(doc: &Doc, first: usize, pred: fn(&Doc, usize) -> bool) -> Vec<Section> {
    let mut out = Vec::new();
    let mut c = Some(first);
    while let Some(x) = c.filter(|x| pred(doc, *x)) {
        out.push(Section {
            first: start_of_attached_comments(doc, x),
            last: x,
        });
        c = next_non_comment_sibling(doc, x);
    }
    out
}

fn list_elements(doc: &mut Doc, parent: usize, is_select: bool, opts: &Opts) -> Vec<Vec<Section>> {
    let mut sections: Vec<Section> = Vec::new();
    let Some(mut command) = first_child(doc, parent) else {
        return Vec::new();
    };
    // ignore comments, because they may refer to multiple elements
    if is_comment_line(doc, command) {
        match next_non_comment_sibling(doc, command) {
            Some(c) => command = c,
            None => return Vec::new(),
        }
    }
    let mut start = Some(command);
    let mut cur = Some(command);
    while let Some(c) = cur {
        let next = next_non_comment_sibling(doc, c);
        let is_sep = last_code(doc, c).is_some_and(|t| matches!(doc.syn.text(t), "," | ";"));
        if next.is_none() || is_sep {
            if let Some(s) = start {
                sections.push(Section { first: s, last: c });
            }
            start = next;
        }
        cur = next;
    }
    if !is_select {
        return vec![sections];
    }

    let mut assoc_start = sections.len();
    if opts.detach_assocs {
        for i in (0..sections.len()).rev() {
            if main_element(doc, sections[i]).is_some_and(|m| exposes_association(doc, m)) {
                assoc_start = i;
            } else {
                break;
            }
        }
    }
    let (mut keys, mut non_keys, mut assocs) = (Vec::new(), Vec::new(), Vec::new());
    let mut in_keys = opts.detach_key;
    for (i, &s) in sections.iter().enumerate() {
        if in_keys {
            if main_element(doc, s)
                .and_then(|m| first_code(doc, m))
                .is_some_and(|t| is_kw(doc, t, "KEY"))
            {
                keys.push(s);
                continue;
            } else if i > 0 {
                add_empty_line_above(doc, s);
            }
            in_keys = false;
        }
        if i < assoc_start {
            non_keys.push(s);
            continue;
        } else if i > 0 && assocs.is_empty() {
            add_empty_line_above(doc, s);
        }
        assocs.push(s);
    }
    let mut out = Vec::new();
    for l in [keys, non_keys, assocs] {
        if !l.is_empty() {
            out.push(l);
        }
    }
    out
}

fn add_empty_line_above(doc: &mut Doc, s: Section) {
    let start = start_of_attached_comments(doc, s.first);
    let first = doc.syn.commands[start].first;
    let lines = doc.lines[first].max(2);
    doc.set_lines(first, lines);
}

fn main_element(doc: &Doc, s: Section) -> Option<usize> {
    (s.first..=s.last).find(|&c| !is_comment_line(doc, c) && !is_annotation(doc, c))
}

fn exposes_association(doc: &Doc, ci: usize) -> bool {
    let Some(f) = first_code(doc, ci) else {
        return false;
    };
    if doc.syn.types[f] != abapfmt_syntax::TokenType::Identifier {
        return false;
    }
    let text = doc.syn.text(f);
    if !text.starts_with('_')
        || !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '/')
    {
        return false;
    }
    let last = doc.syn.commands[ci].last;
    match next_code_sibling(doc, f, last) {
        None => true,
        Some(n) => doc.syn.text(n) == ",",
    }
}

fn line_count(doc: &Doc, s: Section) -> i64 {
    let first = doc.syn.commands[s.first].first;
    let last = doc.syn.commands[s.last].last;
    let sum: i64 = (first..=last).map(|t| i64::from(doc.lines[t])).sum();
    sum - i64::from(doc.lines[first]) + 1
}

fn on_sections(doc: &mut Doc, sections: &[Section], surround_min: i64, mode: Condense) {
    if sections.len() < 2 {
        return;
    }
    let mut condense_all = false;
    if matches!(
        mode,
        Condense::IfOnlyOneLiners | Condense::IfOnlyDetachedOneLiners
    ) {
        condense_all = true;
        for (k, s) in sections.iter().enumerate() {
            if line_count(doc, *s) > 1 {
                condense_all = false;
                break;
            }
            if mode == Condense::IfOnlyDetachedOneLiners
                && k > 0
                && doc.lines[doc.syn.commands[s.first].first] < 2
            {
                condense_all = false;
                break;
            }
        }
    }
    let mut a_lines = line_count(doc, sections[0]);
    for &b in &sections[1..] {
        let b_lines = line_count(doc, b);
        let start = start_of_attached_comments(doc, b.first);
        let first = doc.syn.commands[start].first;
        if condense_all || (mode == Condense::Always && a_lines == 1 && b_lines == 1) {
            doc.set_lines(first, 1);
        } else if a_lines >= surround_min || b_lines >= surround_min {
            let lines = doc.lines[first].max(2);
            doc.set_lines(first, lines);
        }
        a_lines = b_lines;
    }
}
