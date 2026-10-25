//! Port of abap-cleaner's `SelectQuery` / `SelectClause`: splits ABAP SQL SELECT queries into
//! their clauses, for the ALIGN_SELECT_* rules.
use std::collections::HashSet;

use abapfmt_syntax::{Command, TokenType};

use crate::Doc;
use crate::align::is_comment;
use crate::rules::align_declarations::{
    matches_seq, next_non_comment, next_non_comment_sibling, next_sibling,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Clause {
    None,
    Union,
    Select,
    From,
    Fields,
    ForAllEntries,
    Where,
    GroupBy,
    Having,
    OrderBy,
    DbHints,
    Into,
    UpToOffset,
    AbapOptions,
}

/// A range of sibling tokens (`Term`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Term {
    pub first: usize,
    pub last: usize,
}

impl Term {
    pub fn is_on_single_line(&self, doc: &Doc) -> bool {
        (self.first + 1..=self.last).all(|t| doc.lines[t] == 0)
    }
}

#[derive(Debug)]
pub(crate) struct Query {
    pub start_query: Option<usize>,
    pub prev: Option<usize>,
    pub next: Option<usize>,
    pub start_token: usize,
    /// exclusive end (the token after the query), if any
    pub end_token: Option<usize>,
    pub clauses_split: bool,
    /// the clauses in the order in which they were found
    pub clauses: Vec<(Clause, Term)>,
}

fn kw(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.types[t] == TokenType::Keyword && doc.syn.text(t).eq_ignore_ascii_case(s)
}

fn any_kw(doc: &Doc, t: usize, set: &[&str]) -> bool {
    set.iter().any(|s| kw(doc, t, s))
}

fn opens_level(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t)
}

fn closes_level(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|o| o < t)
}

impl Query {
    pub fn clause(&self, c: Clause) -> Option<Term> {
        self.clauses
            .iter()
            .rev()
            .find(|(k, _)| *k == c)
            .map(|(_, t)| *t)
    }

    pub fn has_clause(&self, c: Clause) -> bool {
        self.clauses.iter().any(|(k, _)| *k == c)
    }

    pub fn select_token(&self) -> usize {
        self.clause(Clause::Select).unwrap().first
    }

    pub fn is_start_query(&self) -> bool {
        self.start_query.is_none()
    }

    pub fn is_main_query(&self, doc: &Doc) -> bool {
        self.is_start_query()
            && self
                .clause(Clause::Select)
                .is_some_and(|c| doc.syn.parent[c.first].is_none())
    }

    /// distinct clause types in the order they were found
    pub fn clause_types_in_order(&self) -> Vec<Clause> {
        self.clauses.iter().map(|(k, _)| *k).collect()
    }

    #[allow(dead_code)]
    pub fn has_select_list_in_select_clause(&self, doc: &Doc, last: usize) -> bool {
        let sel = self.clause(Clause::Select).unwrap();
        let mut t = Some(sel.first);
        while let Some(x) = t.filter(|x| any_kw(doc, *x, &["SELECT", "SINGLE", "FOR", "UPDATE"])) {
            t = next_non_comment_sibling(doc, x, last);
        }
        t != next_non_comment_sibling(doc, sel.last, last)
    }

    /// FROM contains at least one JOIN or CDS view parameters
    pub fn has_join_or_parameters(&self, doc: &Doc, last: usize) -> bool {
        let Some(from) = self.clause(Clause::From) else {
            return false;
        };
        let mut t = Some(from.first);
        while let Some(x) = t.filter(|x| *x <= from.last) {
            if kw(doc, x, "JOIN")
                || (doc.syn.text(x) == ")" && doc.syn.types[x] == TokenType::OtherOp)
            {
                return true;
            }
            t = next_sibling(doc, x, last);
        }
        false
    }

    /// the INTO clause has the form `INTO (dobj1, dobj2, ...)`
    pub fn has_into_list(&self, doc: &Doc, last: usize) -> bool {
        let Some(into) = self.clause(Clause::Into) else {
            return false;
        };
        let mut t = Some(into.first);
        while let Some(x) = t.filter(|x| *x <= into.last) {
            if doc.syn.text(x) == "(" && doc.syn.types[x] == TokenType::OtherOp {
                return true;
            }
            t = next_sibling(doc, x, last);
        }
        false
    }

    pub fn is_one_liner(&self, doc: &Doc) -> bool {
        if Some(self.start_token) == self.end_token {
            return true;
        }
        let end = self.end_token.unwrap_or(doc.syn.tokens.len());
        (self.start_token + 1..end).all(|t| doc.lines[t] == 0)
    }

    /// the length as a one-liner starting at the SELECT keyword, or -1
    pub fn length_on_one_line(&self, doc: &Doc) -> i64 {
        if self.clauses_split {
            return -1;
        }
        let end = self.end_token.unwrap_or(doc.syn.tokens.len());
        let mut sum = 0;
        for t in self.select_token()..end {
            if is_comment(doc, t) {
                return -1;
            }
            if t != self.start_token && !(doc.lines[t] == 0 && doc.spaces[t] == 0) {
                sum += 1;
            }
            sum += doc.token_text(t).chars().count() as i64;
            if sum > 255 {
                return -1;
            }
        }
        sum
    }

    /// condenses to one line, starting from the SELECT keyword
    pub fn condense(&self, doc: &mut Doc) {
        if self.clauses_split {
            return;
        }
        let end = self.end_token.unwrap_or(doc.syn.tokens.len());
        for t in self.select_token()..end {
            if t > 0 && is_comment(doc, t - 1) {
                break;
            }
            if t == self.start_token || (doc.lines[t] == 0 && doc.spaces[t] == 0) {
                continue;
            }
            doc.lines[t] = 0;
            doc.spaces[t] = 1;
        }
    }

    fn add_clause(&mut self, cur: Clause, start: Option<usize>, last: usize) {
        if cur != Clause::None
            && let Some(s) = start
        {
            self.clauses.push((cur, Term { first: s, last }));
        }
    }

    fn read(
        &mut self,
        doc: &Doc,
        cmd_last: usize,
        start: usize,
        opening: &mut HashSet<usize>,
    ) -> Option<usize> {
        if self.start_token == usize::MAX {
            self.start_token = start;
        } else {
            self.clauses_split = true;
        }
        let mut token = Some(start);
        let mut last_token = start;
        let mut cur_start: Option<usize> = None;
        let mut cur_clause = Clause::None;
        while let Some(mut t) = token {
            let mut next_clause = Clause::None;
            let ty = doc.syn.types[t];
            if any_kw(doc, t, &["UNION", "INTERSECT", "EXCEPT"]) {
                if !self.has_clause(Clause::Union) && !self.has_clause(Clause::Select) {
                    next_clause = Clause::Union;
                } else {
                    break;
                }
            } else if doc.syn.text(t) == "(" && cur_clause == Clause::Union {
                opening.insert(t);
                token = next_non_comment(doc, t, cmd_last);
                continue;
            } else if opens_level(doc, t) {
                while opens_level(doc, t) {
                    match next_sibling(doc, t, cmd_last) {
                        Some(n) => t = n,
                        None => break,
                    }
                }
            } else if closes_level(doc, t) || ty == TokenType::Period {
                break;
            } else if ty != TokenType::Keyword {
                // continue reading
            } else if !self.has_clause(Clause::Select) && kw(doc, t, "SELECT") {
                next_clause = Clause::Select;
            } else if !self.has_clause(Clause::From) && kw(doc, t, "FROM") {
                next_clause = Clause::From;
            } else if matches_seq(doc, t, cmd_last, &["CORRESPONDING", "FIELDS"]) {
                // skip this usage of 'FIELDS'
                t = next_non_comment_sibling(doc, t, cmd_last).unwrap_or(t);
            } else if !self.has_clause(Clause::Fields) && kw(doc, t, "FIELDS") {
                next_clause = Clause::Fields;
            } else if !self.has_clause(Clause::ForAllEntries)
                && matches_seq(doc, t, cmd_last, &["FOR", "ALL", "ENTRIES", "IN"])
            {
                next_clause = Clause::ForAllEntries;
            } else if !self.has_clause(Clause::Where) && kw(doc, t, "WHERE") {
                next_clause = Clause::Where;
            } else if !self.has_clause(Clause::GroupBy)
                && matches_seq(doc, t, cmd_last, &["GROUP", "BY"])
            {
                next_clause = Clause::GroupBy;
            } else if !self.has_clause(Clause::Having) && kw(doc, t, "HAVING") {
                next_clause = Clause::Having;
            } else if matches_seq(doc, t, cmd_last, &["ORDER", "BY"]) {
                if !self.has_clause(Clause::OrderBy) {
                    next_clause = Clause::OrderBy;
                } else {
                    break;
                }
            } else if !self.has_clause(Clause::DbHints)
                && doc.syn.text(t).eq_ignore_ascii_case("%_HINTS")
            {
                next_clause = Clause::DbHints;
            } else if any_kw(doc, t, &["INTO", "APPENDING"]) {
                if !self.has_clause(Clause::Into) && self.is_start_query() {
                    next_clause = Clause::Into;
                } else {
                    break;
                }
            } else if !self.has_clause(Clause::UpToOffset)
                && (matches_seq(doc, t, cmd_last, &["UP", "TO"]) || kw(doc, t, "OFFSET"))
            {
                next_clause = Clause::UpToOffset;
            } else if cur_clause == Clause::From
                && matches_seq(doc, t, cmd_last, &["WITH", "PRIVILEGED", "ACCESS"])
            {
                // skip 'WITH PRIVILEGED ACCESS' in the FROM clause
                t = next_non_comment_sibling(doc, t, cmd_last)
                    .and_then(|n| next_non_comment_sibling(doc, n, cmd_last))
                    .unwrap_or(t);
            } else if !self.has_clause(Clause::AbapOptions)
                && (matches_seq(doc, t, cmd_last, &["PRIVILEGED", "ACCESS"])
                    || matches_seq(doc, t, cmd_last, &["BYPASSING", "BUFFER"])
                    || kw(doc, t, "CONNECTION"))
            {
                if self.is_start_query() {
                    next_clause = Clause::AbapOptions;
                } else {
                    break;
                }
            }
            if next_clause != Clause::None {
                self.add_clause(cur_clause, cur_start, last_token);
                cur_clause = next_clause;
                cur_start = Some(t);
            }
            last_token = t;
            token = next_non_comment_sibling(doc, t, cmd_last);
        }
        // skip closing parentheses that were opened earlier
        if token.is_none() || token.is_some_and(|t| t != start && closes_level(doc, t)) {
            token = next_non_comment(doc, last_token, cmd_last);
            while let Some(t) = token.filter(|t| {
                doc.syn.text(*t) == ")"
                    && doc.syn.bracket_match[*t].is_some_and(|o| opening.contains(&o))
            }) {
                token = next_non_comment(doc, t, cmd_last);
            }
        }
        self.add_clause(cur_clause, cur_start, last_token);
        self.end_token = token;
        if token == Some(start) { None } else { token }
    }
}

/// `SelectQuery.createQueryChainsFrom`: returns the arena of queries and the indices of the
/// start queries of all chains.
pub(crate) fn create_query_chains(doc: &Doc, cmd: &Command) -> (Vec<Query>, Vec<usize>) {
    let mut arena: Vec<Query> = Vec::new();
    let mut chains = Vec::new();
    let mut select_tokens: HashSet<usize> = HashSet::new();
    let last = cmd.last;
    let Some(mut token) = doc.first_code_token(cmd) else {
        return (arena, chains);
    };
    loop {
        if kw(doc, token, "SELECT") && !select_tokens.contains(&token) {
            let mut start_query: Option<usize> = None;
            let mut prev_query: Option<usize> = None;
            let mut query_start = Some(token);
            let mut opening: HashSet<usize> = HashSet::new();
            while let Some(qs) = query_start {
                let mut q = Query {
                    start_query,
                    prev: prev_query,
                    next: None,
                    start_token: usize::MAX,
                    end_token: None,
                    clauses_split: false,
                    clauses: Vec::new(),
                };
                query_start = q.read(doc, last, qs, &mut opening);
                if q.has_clause(Clause::Select) {
                    select_tokens.insert(q.select_token());
                    let idx = arena.len();
                    arena.push(q);
                    if start_query.is_none() {
                        start_query = Some(idx);
                        chains.push(idx);
                    }
                    if let Some(p) = prev_query {
                        arena[p].next = Some(idx);
                    }
                    prev_query = Some(idx);
                }
                // final ORDER BY, INTO|APPENDING, and abap_options belong to the start query
                if let Some(qs) = query_start
                    && any_kw(
                        doc,
                        qs,
                        &[
                            "ORDER",
                            "INTO",
                            "APPENDING",
                            "PRIVILEGED",
                            "BYPASSING",
                            "CONNECTION",
                        ],
                    )
                    && let Some(sq) = start_query
                {
                    let mut sqv = std::mem::replace(
                        &mut arena[sq],
                        Query {
                            start_query: None,
                            prev: None,
                            next: None,
                            start_token: usize::MAX,
                            end_token: None,
                            clauses_split: false,
                            clauses: Vec::new(),
                        },
                    );
                    query_start = sqv.read(doc, last, qs, &mut opening);
                    arena[sq] = sqv;
                }
                if query_start.is_none_or(|t| doc.syn.types[t] == TokenType::Period) {
                    break;
                }
            }
        }
        // move inside parentheses etc. to find subqueries, too
        match next_non_comment(doc, token, last) {
            Some(n) => token = n,
            None => break,
        }
    }
    (arena, chains)
}

/// The next sibling token that is not a comment; `None` after the last child of a level
/// (unlike `next_non_comment_sibling`, which would return the parent's closing token).
pub(crate) fn next_code_sibling(doc: &Doc, t: usize, last: usize) -> Option<usize> {
    let mut cur = t;
    loop {
        let n = if opens_level(doc, cur) {
            doc.syn.bracket_match[cur]?
        } else {
            cur + 1
        };
        if n > last || (!opens_level(doc, cur) && closes_level(doc, n)) {
            return None;
        }
        if !is_comment(doc, n) {
            return Some(n);
        }
        cur = n;
    }
}

/// `Token.getLastTokenOfLogicalExpression` for ABAP SQL conditions: `t` is the WHERE / HAVING /
/// ON keyword; returns the last token of the condition.
pub(crate) fn sql_condition_end(doc: &Doc, t: usize, last: usize) -> Option<usize> {
    const INSIDE: &[&str] = &[
        "NOT", "AND", "OR", "IS", "BETWEEN", "IN", "LIKE", "ALL", "ANY", "SOME", "NULL", "INITIAL",
        "ESCAPE", "EXISTS", "CASE", "WHEN", "THEN", "ELSE", "END",
    ];
    const AGGREGATES: &[&str] = &[
        "AVG(",
        "MEDIAN(",
        "MAX(",
        "MIN(",
        "SUM(",
        "PRODUCT(",
        "STDDEV(",
        "VAR(",
        "CORR(",
        "CORR_SPEARMAN(",
        "STRING_AGG(",
        "COUNT(",
        "GROUPING(",
        "ALLOW_PRECISION_LOSS(",
    ];
    const CONSTRUCTORS: &[&str] = &[
        "NEW",
        "VALUE",
        "CONV",
        "CORRESPONDING",
        "CAST",
        "REF",
        "EXACT",
        "REDUCE",
        "FILTER",
        "COND",
        "SWITCH",
    ];
    const NEXT_CLAUSE: &[&str] = &[
        "UNION",
        "INTERSECT",
        "EXCEPT",
        "FIELDS",
        "FOR",
        "WHERE",
        "GROUP",
        "HAVING",
        "ORDER",
        "INTO",
        "APPENDING",
        "USING",
        "UP",
        "%_HINTS",
        "PRIVILEGED",
        "OFFSET",
        "BYPASSING",
        "CONNECTION",
        "OPTIONS",
        "INNER",
        "LEFT",
        "RIGHT",
        "JOIN",
        "CROSS",
        "ON",
        "CLIENT",
        "SOURCE",
        "CHILD",
        "PERIOD",
        "SIBLINGS",
        "DEPTH",
        "MULTIPLE",
        "ORPHANS",
        "CYCLES",
        "LOAD",
        "GENERATE",
        "DISTANCE",
        "MEASURES",
        "WITH",
    ];
    let text_in =
        |e: usize, set: &[&str]| set.iter().any(|s| doc.syn.text(e).eq_ignore_ascii_case(s));
    let mut end = next_code_sibling(doc, t, last);
    while let Some(e) = end.filter(|e| doc.syn.types[*e] != TokenType::Period) {
        if doc.syn.types[e] != TokenType::Keyword {
            // the condition can only end with a keyword, a period or a closing parenthesis
        } else if text_in(e, INSIDE) || text_in(e, AGGREGATES) || text_in(e, CONSTRUCTORS) {
        } else {
            break;
        }
        let Some(n) = next_code_sibling(doc, e, last) else {
            return Some(e);
        };
        end = Some(n);
    }
    let e = end?;
    if doc.syn.types[e] != TokenType::Period && !text_in(e, NEXT_CLAUSE) {
        return None;
    }
    (0..e).rev().find(|j| !is_comment(doc, *j))
}
