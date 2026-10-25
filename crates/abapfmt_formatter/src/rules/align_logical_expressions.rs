//! ALIGN_LOGICAL_EXPRESSIONS: aligns the Boolean operators and comparison operators of logical
//! expressions in IF, CHECK, WHILE, WHERE, ON etc. (port of `AlignLogicalExpressionsRule`,
//! `LogicalExpression` and `TreeAlign`).
use std::collections::HashSet;

use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, end_x, is_comment};
use crate::rules::align_declarations::{match_seq_last, next_sibling};
use crate::rules::align_methods_declaration::matches_phrase;
use crate::rules::align_with_second_word::is_abap_sql_operation;
use crate::rules::select_query::{next_code_sibling, sql_condition_end};
use crate::{ConfigExt, Doc, RuleConfig};

// ---- token helpers ----

fn is_pragma(doc: &Doc, t: usize) -> bool {
    doc.syn.types[t] == TokenType::Pragma
}

fn is_code(doc: &Doc, t: usize) -> bool {
    !is_comment(doc, t) && !is_pragma(doc, t)
}

fn kw(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.types[t] == TokenType::Keyword && doc.syn.text(t).eq_ignore_ascii_case(s)
}

fn any_kw(doc: &Doc, t: usize, set: &[&str]) -> bool {
    set.iter().any(|s| kw(doc, t, s))
}

fn text_is(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.text(t).eq_ignore_ascii_case(s)
}

fn prev_code(doc: &Doc, t: usize) -> Option<usize> {
    (0..t).rev().find(|j| is_code(doc, *j))
}

fn next_code(doc: &Doc, t: usize, last: usize) -> Option<usize> {
    (t + 1..=last).find(|j| is_code(doc, *j))
}

fn opens(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|c| c > t)
}

fn closes(doc: &Doc, t: usize) -> bool {
    doc.syn.bracket_match[t].is_some_and(|o| o < t)
}

/// `Token.getLastTokenOnSiblings(true, ASTERISK, phrases)`: searches the siblings from `t` on
/// for one of the `|`-separated phrases and returns the last token of the first match.
fn find_phrase(doc: &Doc, t: usize, last: usize, phrases: &str) -> Option<usize> {
    let mut cur = Some(t);
    while let Some(c) = cur {
        for phrase in phrases.split('|') {
            let words: Vec<&str> = phrase.split(' ').collect();
            if let Some(l) = match_seq_last(doc, c, last, &words) {
                return Some(l);
            }
        }
        cur = next_sibling(doc, c, last);
    }
    None
}

/// `Token.getLastTokenOfLogicalExpression`; `t` is the keyword that starts the expression.
pub(crate) fn last_token_of_logical_expression(
    doc: &Doc,
    cmd: &Command,
    t: usize,
) -> Option<usize> {
    let last = cmd.last;
    let first_code = doc.first_code_token(cmd)?;
    let parent = doc.syn.parent[t];
    let is_sql = is_abap_sql_operation(doc, cmd, first_code);
    let last_code = (cmd.first..=last).rev().find(|j| is_code(doc, *j));
    let last_non_comment = (cmd.first..=last).rev().find(|j| !is_comment(doc, *j));

    // 1. non-SQL cases on top level
    if t == first_code && any_kw(doc, first_code, &["IF", "ELSEIF", "CHECK", "WHILE"]) {
        return prev_code(doc, last_non_comment?);
    } else if kw(doc, t, "WHERE")
        && parent.is_none()
        && matches_phrase(doc, first_code, last, "LOOP AT|MODIFY|DELETE|FOR")
        && !is_sql
    {
        return match find_phrase(doc, t, last, "GROUP BY|USING KEY|TRANSPORTING|FROM") {
            None => prev_code(doc, last_code?),
            Some(e) if kw(doc, e, "BY") || kw(doc, e, "KEY") => prev_code(doc, prev_code(doc, e)?),
            Some(e) => prev_code(doc, e),
        };
    } else if kw(doc, t, "UNTIL") && parent.is_none() && kw(doc, first_code, "WAIT") {
        return match find_phrase(doc, t, last, "UP TO") {
            None => prev_code(doc, last_code?),
            Some(e) => prev_code(doc, prev_code(doc, e)?),
        };
    } else if kw(doc, first_code, "ASSERT") {
        if kw(doc, t, "CONDITION")
            || t == first_code && find_phrase(doc, t, last, "CONDITION").is_none()
        {
            return prev_code(doc, last_code?);
        }
        return None;
    }

    // 2. xsdbool( ... ) and boolc( ... )
    if (text_is(doc, t, "boolc(") || text_is(doc, t, "xsdbool(")) && opens(doc, t) {
        return prev_code(doc, doc.syn.bracket_match[t]?);
    }

    // 3. WHEN / UNTIL / WHILE / WHERE clauses inside constructor expressions
    let ctor = parent
        .and_then(|p| prev_code(doc, p))
        .filter(|c| doc.syn.types[*c] == TokenType::Keyword);
    if let Some(ctor) = ctor {
        if kw(doc, t, "WHEN") && kw(doc, ctor, "COND") {
            return find_phrase(doc, t, last, "THEN").and_then(|th| prev_code(doc, th));
        } else if any_kw(doc, t, &["UNTIL", "WHILE"])
            && any_kw(doc, ctor, &["REDUCE", "NEW", "VALUE"])
        {
            return find_phrase(doc, t, last, "LET|NEXT|FOR").and_then(|e| prev_code(doc, e));
        } else if kw(doc, t, "WHERE") && kw(doc, ctor, "FILTER") {
            return prev_code(doc, doc.syn.bracket_match[parent?]?);
        } else if kw(doc, t, "WHERE")
            && any_kw(doc, ctor, &["REDUCE", "NEW", "VALUE"])
            && let Some(n) = next_code_sibling(doc, t, last)
            && doc.syn.text(n) == "("
        {
            return doc.syn.bracket_match[n].filter(|c| *c > n);
        }
    }

    // 4. Boolean expressions at the right-hand side of (explicit or implicit) assignments
    if let Some(nc) = next_code(doc, t, last)
        && starts_rhs_of_assignment(doc, cmd, nc)
        && let Some(last_of_expr) = doc.syn.last_of_expression(nc, cmd.first, last)
    {
        let cx = Ctx {
            cmd_first: cmd.first,
            cmd_last: last,
            is_sql,
        };
        if let Some(exp) = LogExp::create(doc, &cx, None, nc, last_of_expr)
            && exp.contains_boolean_op_or_comparison()
        {
            return Some(last_of_expr);
        }
    }

    // 5. ABAP SQL conditions
    if is_sql {
        if kw(doc, t, "WHEN") {
            return find_phrase(doc, t, last, "THEN").and_then(|th| prev_code(doc, th));
        } else if any_kw(doc, t, &["WHERE", "HAVING", "ON"]) {
            return sql_condition_end(doc, t, last);
        }
    }
    None
}

/// `Token.startsRhsOfAssignment`
fn starts_rhs_of_assignment(doc: &Doc, cmd: &Command, t: usize) -> bool {
    let Some(prev) = (cmd.first..t).rev().find(|j| is_code(doc, *j)) else {
        return false;
    };
    if doc.syn.types[prev] == TokenType::AssignmentOp {
        return true;
    }
    let parent = doc.syn.parent[t];
    if parent != Some(prev) {
        return false;
    }
    let attached_next = |x: usize| x < cmd.last && doc.lines[x + 1] == 0 && doc.spaces[x + 1] == 0;
    let opens_call = doc.syn.types[prev] == TokenType::Identifier
        && opens(doc, prev)
        && doc.syn.text(prev).ends_with('(')
        && !attached_next(prev);
    if opens_call
        && !prev_code(doc, prev).is_some_and(|p| {
            doc.syn.types[p] == TokenType::Keyword
                && [
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
                ]
                .iter()
                .any(|k| text_is(doc, p, k))
        })
    {
        return !starts_lhs_of_assignment(doc, cmd, t);
    }
    if doc.syn.text(prev) != "(" {
        return false;
    }
    let Some(pp) = doc.syn.parent[prev] else {
        return false;
    };
    let is_ctor = (doc.syn.text(pp) == "#("
        || doc.syn.types[pp] == TokenType::Identifier
            && opens(doc, pp)
            && doc.syn.text(pp).ends_with('('))
        && !attached_next(pp)
        && prev_code(doc, pp).is_some_and(|p| {
            doc.syn.types[p] == TokenType::Keyword
                && [
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
                ]
                .iter()
                .any(|k| text_is(doc, p, k))
        });
    is_ctor && !starts_lhs_of_assignment(doc, cmd, t)
}

/// `Token.startsLhsOfAssignment`
fn starts_lhs_of_assignment(doc: &Doc, cmd: &Command, t: usize) -> bool {
    if doc.syn.types[t] != TokenType::Identifier || closes(doc, t) {
        return false;
    }
    let mut end = t;
    while let Some(c) = doc.syn.bracket_match[end].filter(|c| *c > end) {
        end = c;
    }
    next_code_sibling(doc, end, cmd.last)
        .is_some_and(|op| doc.syn.types[op] == TokenType::AssignmentOp)
}

// ---- logical expression ----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Binding {
    Unknown,
    Equiv,
    Or,
    And,
    Not,
    Comparison,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RelType {
    None,
    Comparison,
    PredicateExpression,
    PredicateFunction,
}

const KEYWORD_OF_BINDING: [&str; 5] = ["", "EQUIV", "OR", "AND", "NOT"];

struct LogExp {
    parent_binding: Option<Binding>,
    first: usize,
    last: usize,
    in_parens: bool,
    binding: Binding,
    keywords: Vec<usize>,
    inner: Vec<LogExp>,
    rel: RelType,
    supported: bool,
}

struct Ctx {
    cmd_first: usize,
    cmd_last: usize,
    is_sql: bool,
}

fn binding_of_token(doc: &Doc, t: usize, is_first: bool) -> Binding {
    if doc.syn.types[t] == TokenType::Keyword {
        if kw(doc, t, "EQUIV") {
            return Binding::Equiv;
        } else if kw(doc, t, "OR") {
            return Binding::Or;
        } else if kw(doc, t, "AND") {
            return Binding::And;
        } else if is_first && kw(doc, t, "NOT") {
            return Binding::Not;
        }
    }
    Binding::Unknown
}

fn is_between(doc: &Doc, t: usize) -> bool {
    doc.syn.types[t] == TokenType::ComparisonOp && text_is(doc, t, "BETWEEN")
}

impl LogExp {
    fn create(
        doc: &Doc,
        cx: &Ctx,
        parent_binding: Option<Binding>,
        first_: usize,
        last_: usize,
    ) -> Option<LogExp> {
        let first = if is_code(doc, first_) {
            first_
        } else {
            next_code(doc, first_, cx.cmd_last)?
        };
        let last = if is_code(doc, last_) {
            last_
        } else {
            prev_code(doc, last_)?
        };
        let end_token = next_code(doc, last, cx.cmd_last);
        let in_parens = opens(doc, first)
            && closes(doc, last)
            && doc.syn.text(first) == "("
            && doc.syn.text(last) == ")"
            && doc.syn.bracket_match[first] == Some(last);
        let mut e = LogExp {
            parent_binding,
            first,
            last,
            in_parens,
            binding: Binding::Comparison,
            keywords: Vec::new(),
            inner: Vec::new(),
            rel: RelType::None,
            supported: true,
        };
        let start = e.first_except_open(doc, cx)?;
        let end = if in_parens { Some(last) } else { end_token };

        // determine the binding level
        let mut min = Binding::Comparison;
        let mut token = Some(start);
        let mut is_first = true;
        let mut ignore_and = false;
        while let Some(t) = token.filter(|t| Some(*t) != end) {
            if is_between(doc, t) {
                ignore_and = true;
            } else if ignore_and && text_is(doc, t, "AND") {
                ignore_and = false;
            } else if doc.syn.types[t] == TokenType::Keyword {
                let b = binding_of_token(doc, t, is_first);
                if b != Binding::Unknown && b < min {
                    min = b;
                }
            }
            token = next_code_sibling(doc, t, cx.cmd_last);
            is_first = false;
        }
        e.binding = min;
        if min < Binding::Comparison {
            let keyword = KEYWORD_OF_BINDING[min as usize];
            let mut start_t = Some(start);
            ignore_and = false;
            let mut token = Some(start);
            while let Some(t) = token.filter(|t| Some(*t) != end) {
                if is_between(doc, t) {
                    ignore_and = true;
                } else if ignore_and && text_is(doc, t, "AND") {
                    ignore_and = false;
                } else if kw(doc, t, keyword) {
                    if Some(t) == start_t && min != Binding::Not {
                        return None;
                    }
                    if Some(t) != start_t {
                        let prev = prev_code(doc, t)?;
                        e.add_inner(doc, cx, start_t?, prev)?;
                    }
                    e.keywords.push(t);
                    start_t = next_code_sibling(doc, t, cx.cmd_last);
                }
                if min == Binding::Not {
                    break;
                }
                token = next_code_sibling(doc, t, cx.cmd_last);
            }
            if start_t != end {
                let prev = match end {
                    None => (cx.cmd_first..=cx.cmd_last)
                        .rev()
                        .find(|j| is_code(doc, *j))?,
                    Some(en) => prev_code(doc, en)?,
                };
                e.add_inner(doc, cx, start_t?, prev)?;
            }
            e.rel = RelType::None;
        } else {
            match e.find_comparison_or_predicate_keyword(doc, cx, start, end) {
                None => {
                    e.supported = false;
                    e.rel = RelType::None;
                }
                Some(k) => {
                    e.keywords.push(k);
                    e.rel = if kw(doc, k, "IS") {
                        RelType::PredicateExpression
                    } else if doc.syn.types[k] == TokenType::Identifier
                        || cx.is_sql && kw(doc, k, "EXISTS")
                    {
                        RelType::PredicateFunction
                    } else {
                        RelType::Comparison
                    };
                }
            }
        }
        Some(e)
    }

    /// `LogicalExpression.containsBooleanOpOrComparison`
    fn contains_boolean_op_or_comparison(&self) -> bool {
        if self.binding == Binding::Comparison {
            matches!(self.rel, RelType::Comparison | RelType::PredicateExpression)
        } else {
            true
        }
    }

    fn add_inner(&mut self, doc: &Doc, cx: &Ctx, start: usize, end: usize) -> Option<()> {
        let inner = LogExp::create(doc, cx, Some(self.binding), start, end)?;
        if !inner.supported {
            self.supported = false;
        }
        self.inner.push(inner);
        Some(())
    }

    fn first_except_open(&self, doc: &Doc, cx: &Ctx) -> Option<usize> {
        if self.in_parens {
            next_code(doc, self.first, cx.cmd_last)
        } else {
            Some(self.first)
        }
    }

    fn last_except_close(&self, doc: &Doc) -> Option<usize> {
        if self.in_parens {
            prev_code(doc, self.last)
        } else {
            Some(self.last)
        }
    }

    fn find_comparison_or_predicate_keyword(
        &self,
        doc: &Doc,
        cx: &Ctx,
        start: usize,
        end: Option<usize>,
    ) -> Option<usize> {
        let find = |pred: &dyn Fn(usize) -> bool| -> Option<usize> {
            let mut t = Some(start);
            while let Some(x) = t.filter(|x| Some(*x) != end) {
                if pred(x) {
                    return Some(x);
                }
                t = next_code_sibling(doc, x, cx.cmd_last);
            }
            None
        };
        if let Some(i) = find(&|x| text_is(doc, x, "IS")) {
            return Some(i);
        }
        if let Some(c) = find(&|x| {
            doc.syn.types[x] == TokenType::ComparisonOp || cx.is_sql && kw(doc, x, "LIKE")
        }) {
            return Some(c);
        }
        let is_ident = doc.syn.types[start] == TokenType::Identifier;
        if is_ident
            && [
                "line_exists(",
                "matches(",
                "contains(",
                "contains_any_of(",
                "contains_any_not_of(",
            ]
            .iter()
            .any(|f| text_is(doc, start, f))
            || cx.is_sql && kw(doc, start, "EXISTS")
        {
            return Some(start);
        }
        // predicative method calls
        if is_ident
            && doc.syn.text(start).ends_with('(')
            && let Some(ns) = next_code_sibling(doc, start, cx.cmd_last)
            && next_code(doc, ns, cx.cmd_last) == end
        {
            return Some(start);
        }
        // stand-alone Boolean identifiers
        if is_ident && next_code_sibling(doc, start, cx.cmd_last) == end {
            return Some(start);
        }
        None
    }

    fn to_tree_align(&self, doc: &Doc, cx: &Ctx, tree: &mut TreeAlign) -> Option<()> {
        let tok = |t: usize| Cell::new(doc, t, t);
        let term = |a: usize, b: usize| Cell::new(doc, a, b);
        match self.binding {
            Binding::Equiv | Binding::Or | Binding::And => {
                tree.add(
                    doc,
                    ColType::OpenBracketForBoolOp,
                    self.in_parens.then(|| tok(self.first)),
                );
                for (i, inner) in self.inner.iter().enumerate() {
                    tree.add(
                        doc,
                        ColType::BoolOperator,
                        (i > 0).then(|| tok(self.keywords[i - 1])),
                    );
                    inner.to_tree_align(doc, cx, tree)?;
                }
                tree.add(
                    doc,
                    ColType::CloseBracketForBoolOp,
                    self.in_parens.then(|| tok(self.last)),
                );
            }
            Binding::Not => {
                tree.add(
                    doc,
                    ColType::OpenBracketForNot,
                    self.in_parens.then(|| tok(self.first)),
                );
                tree.add(doc, ColType::NotKeyword, Some(tok(self.keywords[0])));
                self.inner[0].to_tree_align(doc, cx, tree)?;
                tree.add(
                    doc,
                    ColType::CloseBracketForNot,
                    self.in_parens.then(|| tok(self.last)),
                );
            }
            Binding::Comparison => {
                let has_parent_not = self.parent_binding == Some(Binding::Not);
                if !has_parent_not {
                    tree.add(doc, ColType::OpenBracketForNot, None);
                    tree.add(doc, ColType::NotKeyword, None);
                }
                tree.add(
                    doc,
                    ColType::OpenBracketForRel,
                    self.in_parens.then(|| tok(self.first)),
                );
                let start = self.first_except_open(doc, cx)?;
                let end = self.last_except_close(doc)?;
                if self.rel == RelType::PredicateFunction {
                    tree.add(doc, ColType::RelFunction, Some(term(start, end)));
                } else {
                    let k = self.keywords[0];
                    tree.add(
                        doc,
                        ColType::RelTerm1,
                        Some(term(start, prev_code(doc, k)?)),
                    );
                    tree.add(doc, ColType::RelOperator, Some(tok(k)));
                    tree.add(
                        doc,
                        ColType::RelTerm2,
                        Some(term(next_code(doc, k, cx.cmd_last)?, end)),
                    );
                }
                tree.add(
                    doc,
                    ColType::CloseBracketForRel,
                    self.in_parens.then(|| tok(self.last)),
                );
                if !has_parent_not {
                    tree.add(doc, ColType::CloseBracketForNot, None);
                }
            }
            Binding::Unknown => return None,
        }
        Some(())
    }
}

// ---- tree align ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColType {
    OpenBracketForBoolOp,
    BoolOperator,
    CloseBracketForBoolOp,
    OpenBracketForNot,
    NotKeyword,
    OpenBracketForRel,
    RelFunction,
    RelTerm1,
    RelOperator,
    RelTerm2,
    CloseBracketForRel,
    CloseBracketForNot,
}

fn col_closes_bracket(t: ColType) -> bool {
    matches!(
        t,
        ColType::CloseBracketForBoolOp | ColType::CloseBracketForNot | ColType::CloseBracketForRel
    )
}

fn col_opens_bracket(t: ColType) -> bool {
    matches!(
        t,
        ColType::OpenBracketForBoolOp | ColType::OpenBracketForNot | ColType::OpenBracketForRel
    )
}

struct Col {
    cells: Vec<Cell>,
    prev: Option<usize>,
    next: Option<usize>,
    brack_level: i32,
    ty: ColType,
    indent_for_next: i64,
    right_align: bool,
    do_not_align: bool,
    branch_count: usize,
    max_mono: i64,
    max_multi: i64,
}

impl Col {
    fn brack_level_for_next(&self, next_ty: ColType) -> i32 {
        self.brack_level + i32::from(col_opens_bracket(self.ty))
            - i32::from(col_closes_bracket(next_ty))
    }
}

struct TreeAlign {
    cols: Vec<Col>,
    cur: Option<usize>,
    first_token: usize,
    stop_cells: HashSet<usize>,
    in_oo_context: bool,
}

impl TreeAlign {
    fn new_col(&mut self, ty: ColType, prev: Option<usize>) -> usize {
        let idx = self.cols.len();
        let brack_level = prev.map_or(0, |p| self.cols[p].brack_level_for_next(ty));
        if let Some(p) = prev {
            self.cols[p].branch_count += 1;
            self.cols[p].next = Some(idx);
        }
        self.cols.push(Col {
            cells: Vec::new(),
            prev,
            next: None,
            brack_level,
            ty,
            indent_for_next: 0,
            right_align: false,
            do_not_align: false,
            branch_count: 0,
            max_mono: 0,
            max_multi: 0,
        });
        idx
    }

    fn add_cell(&mut self, doc: &Doc, col: usize, cell: Cell, at_front: bool) {
        let c = &mut self.cols[col];
        c.max_mono = c.max_mono.max(cell.mono_width(doc));
        c.max_multi = c.max_multi.max(cell.multi_width(doc));
        if at_front {
            c.cells.insert(0, cell);
        } else {
            c.cells.push(cell);
        }
    }

    fn add(&mut self, doc: &Doc, ty: ColType, cell: Option<Cell>) {
        // if the first token of the cell is in a new line, but the previous token is a
        // stand-alone AND / OR / EQUIV / "(" on its own line, continue right of that token
        let mut continues_line = cell.as_ref().is_some_and(|c| doc.lines[c.first] == 0);
        if let Some(c) = &cell
            && !continues_line
            && let Some(prev) = (0..c.first).rev().find(|j| !is_comment(doc, *j))
            && doc.first_in_line(prev)
            && (any_kw(doc, prev, &["AND", "OR", "EQUIV"]) || doc.syn.text(prev) == "(")
        {
            continues_line = true;
        }
        if cell.is_none()
            || continues_line
            || cell.as_ref().is_some_and(|c| c.first == self.first_token)
        {
            let next_col = self.cur.and_then(|c| self.cols[c].next);
            let next_has_content = next_col.is_some_and(|n| !self.cols[n].cells.is_empty());
            let new_has_content = cell.is_some();
            let mismatch = match next_col {
                None => true,
                Some(n) => {
                    self.cols[n].ty != ty
                        || col_opens_bracket(self.cols[n].ty) && next_has_content != new_has_content
                }
            };
            if mismatch {
                let c = self.new_col(ty, self.cur);
                self.cur = Some(c);
            } else {
                self.cur = next_col;
            }
        } else {
            // find the first column with matching bracket level and column type
            let brack_level = self
                .cur
                .map_or(0, |c| self.cols[c].brack_level_for_next(ty));
            let mut matched: Option<usize> = None;
            let mut partial: Option<usize> = None;
            let mut cur = self.cur;
            while let Some(c) = cur {
                let col = &self.cols[c];
                if col.brack_level == brack_level
                    && !(col.ty == ColType::BoolOperator && ty == ColType::OpenBracketForBoolOp)
                {
                    partial = Some(c);
                    if col.ty == ty {
                        matched = Some(c);
                    }
                }
                if col.ty == ColType::OpenBracketForBoolOp
                    && (col.brack_level < brack_level
                        || col.brack_level == brack_level
                            && ty != ColType::BoolOperator
                            && ty != ColType::OpenBracketForBoolOp)
                {
                    break;
                }
                cur = col.prev;
            }
            self.cur = cur;
            if let Some(m) = matched {
                self.cur = Some(m);
            } else {
                if let Some(p) = partial
                    && self.cols[p].prev.is_some()
                {
                    self.cur = self.cols[p].prev;
                }
                let c = self.new_col(ty, self.cur);
                self.cur = Some(c);
            }
        }
        if let Some(cell) = cell {
            let c = self.cur.unwrap();
            self.add_cell(doc, c, cell, false);
        }
    }

    fn is_first_non_empty_after_branch(&self, c: usize) -> bool {
        if self.cols[c].cells.is_empty() {
            return false;
        }
        let mut col = self.cols[c].prev;
        while let Some(x) = col {
            if self.cols[x].branch_count > 1 {
                return true;
            }
            if !self.cols[x].cells.is_empty() {
                return false;
            }
            col = self.cols[x].prev;
        }
        true
    }

    fn stop_aligning_line_of(&mut self, cell: &Cell) {
        self.stop_cells.insert(cell.last);
    }

    fn is_alignment_stopped_for_line_of(&self, doc: &Doc, cell: &Cell) -> bool {
        if doc.first_in_line(cell.first) || cell.first == 0 {
            return false;
        }
        self.stop_cells.contains(&(cell.first - 1))
    }

    #[allow(clippy::too_many_arguments)]
    fn align(
        &mut self,
        doc: &mut Doc,
        cmd: &Command,
        keyword: usize,
        style: &str,
        right_align_comparison_ops: bool,
        only_align_same_objects: bool,
        max_inner_spaces: i64,
        attach_parentheses: bool,
    ) {
        // the leftmost indent of all content to align
        let mut left_most = i64::MAX;
        for col in &self.cols {
            for cell in &col.cells {
                if doc.lines[cell.first] > 0 {
                    left_most = left_most.min(i64::from(doc.spaces[cell.first]));
                }
            }
        }
        // keyword is last in line (except for comments)?
        let keyword_last_in_line = (keyword + 1..=cmd.last)
            .find(|j| !(is_comment(doc, *j) && doc.lines[*j] == 0))
            .is_none_or(|j| doc.lines[j] > 0);
        let basic_indent = if keyword_last_in_line && doc.start_x(0, keyword) > left_most {
            let mut bi = left_most;
            if let Some(parent) = doc.syn.parent[keyword]
                && let Some(fc) = (parent + 1..=cmd.last).find(|j| !is_comment(doc, *j))
                && bi < doc.start_x(0, fc)
            {
                bi = doc.start_x(0, fc);
            }
            bi.max(i64::from(doc.spaces[cmd.first]) + 2)
        } else if style != "DO_NOT_ALIGN"
            && self.cols.len() >= 2
            && self.cols[0].ty == ColType::OpenBracketForBoolOp
            && self.cols[0].cells.is_empty()
            && self.cols[1].ty == ColType::BoolOperator
        {
            let cell = Cell::new(doc, keyword, keyword);
            self.add_cell(doc, 1, cell, true);
            self.cols[1].right_align = style == "RIGHT_ALIGN";
            doc.start_x(0, keyword)
        } else {
            end_x(doc, keyword) + 1
        };
        // left- or right-align comparison operators
        for col in &mut self.cols {
            if col.ty == ColType::RelOperator {
                col.right_align = right_align_comparison_ops;
            }
        }
        // do not align closing brackets and Boolean operators at line end
        for i in (0..self.cols.len()).rev() {
            if self.cols[i].next.is_none() {
                let mut column = Some(i);
                while let Some(c) = column.filter(|c| {
                    let col = &self.cols[*c];
                    col.branch_count <= 1
                        && (!self.is_first_non_empty_after_branch(*c)
                            || col.ty == ColType::BoolOperator)
                        && (col.cells.is_empty()
                            || col.cells.len() == 1
                            || col_closes_bracket(col.ty)
                            || col.ty == ColType::BoolOperator)
                }) {
                    self.cols[c].do_not_align = true;
                    column = self.cols[c].prev;
                }
            }
        }
        // optionally stop aligning if consecutive relational expressions work on different objects
        if only_align_same_objects {
            for ci in 0..self.cols.len() {
                if matches!(self.cols[ci].ty, ColType::RelTerm1 | ColType::RelFunction) {
                    let cells = self.cols[ci].cells.clone();
                    for i in 0..cells.len() {
                        let prev = (i > 0).then(|| &cells[i - 1]);
                        let next = cells.get(i + 1);
                        let same_prev = prev.is_some_and(|p| {
                            starts_with_same_object(doc, &cells[i], p, self.in_oo_context)
                        });
                        let same_next = next.is_some_and(|n| {
                            starts_with_same_object(doc, &cells[i], n, self.in_oo_context)
                        });
                        if !same_prev && !same_next {
                            self.stop_aligning_line_of(&cells[i]);
                        }
                    }
                }
            }
        }
        let mut after_attached_open = false;
        for ci in 0..self.cols.len() {
            let column_width = if self.cols[ci].cells.is_empty() {
                0
            } else {
                self.cols[ci].max_multi
            };
            let column_indent;
            if self.cols[ci].prev.is_none() {
                after_attached_open = false;
                column_indent = basic_indent;
            } else {
                column_indent = self.cols[self.cols[ci].prev.unwrap()].indent_for_next;
            }
            let mut column_indent = column_indent;
            let mut min_spaces_left = i64::from(!after_attached_open);
            if attach_parentheses
                && col_closes_bracket(self.cols[ci].ty)
                && column_width == 1
                && self.cols[ci].prev.is_some()
            {
                column_indent -= 1;
                min_spaces_left = 0;
            }
            let spaces_right = if column_width == 0 {
                0
            } else if attach_parentheses && col_opens_bracket(self.cols[ci].ty) && column_width == 1
            {
                after_attached_open = true;
                0
            } else {
                after_attached_open = false;
                1
            };
            self.cols[ci].indent_for_next = column_indent + column_width + spaces_right;
            // propagate stopped alignment from the previous columns
            let cells = self.cols[ci].cells.clone();
            for cell in &cells {
                if self.is_alignment_stopped_for_line_of(doc, cell) {
                    self.stop_aligning_line_of(cell);
                }
            }
            for cell in &cells {
                let current_indent = doc.start_x(0, cell.first);
                let mut new_indent = column_indent;
                if self.cols[ci].right_align {
                    new_indent += column_width - cell.multi_width(doc);
                }
                let first = cell.first;
                let mut spaces_left = i64::from(doc.spaces[first]) + (new_indent - current_indent);
                if column_indent > 0 {
                    spaces_left = spaces_left.max(min_spaces_left);
                }
                if doc.lines[first] == 0 {
                    if self.cols[ci].do_not_align
                        || (only_align_same_objects
                            && self.is_alignment_stopped_for_line_of(doc, cell))
                    {
                        spaces_left = min_spaces_left;
                    } else if spaces_left > max_inner_spaces
                        && Some(first) != next_code(doc, keyword, cmd.last)
                    {
                        // stop aligning if the maximum number of inner spaces would be exceeded
                        spaces_left = min_spaces_left;
                        self.stop_aligning_line_of(cell);
                    }
                }
                cell.set_whitespace(doc, doc.lines[first], spaces_left, true, true, None);
            }
        }
    }
}

fn read_var_name(text: &str, start: usize, in_oo: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut end = start;
    let mut first = true;
    while end < chars.len() {
        let c = chars[end];
        let ok = c.is_ascii_alphabetic()
            || matches!(c, '_' | '/' | '>')
            || !first && c.is_ascii_digit()
            || first && c == '<'
            || c == '%' && chars.get(end + 1) == Some(&'_')
            || !in_oo
                && (matches!(c, '%' | '$' | '*' | '?') || first && c == '&' || !first && c == '#');
        if !ok {
            break;
        }
        first = false;
        end += 1;
    }
    chars[start..end].iter().collect()
}

fn starts_with_same_object(doc: &Doc, a: &Cell, b: &Cell, in_oo: bool) -> bool {
    let ta = doc.token_text(a.first);
    let tb = doc.token_text(b.first);
    let sa = usize::from(ta.starts_with('!'));
    let sb = usize::from(tb.starts_with('!'));
    read_var_name(ta, sa, in_oo).eq_ignore_ascii_case(&read_var_name(tb, sb, in_oo))
}

// ---- the rule ----

pub fn align_logical_expressions(doc: &mut Doc, config: &RuleConfig) {
    let right_align_comparison_ops = config.bool("RightAlignComparisonOps", true);
    let only_align_same_objects = config.bool("OnlyAlignSameObjects", false);
    let max_inner_spaces = config.int("MaxInnerSpaces", 999);
    let style = |key: &str, default: &str| config.choice(key, default);
    let if_style = style("AlignIfWithBoolOps", "DO_NOT_ALIGN");
    let elseif_style = style("AlignElseIfWithBoolOps", "DO_NOT_ALIGN");
    let check_style = style("AlignCheckWithBoolOps", "DO_NOT_ALIGN");
    let while_style = style("AlignWhileWithBoolOps", "DO_NOT_ALIGN");
    let where_style = style("AlignWhereWithBoolOps", "DO_NOT_ALIGN");
    let until_style = style("AlignUntilWithBoolOps", "DO_NOT_ALIGN");
    let on_style = style("AlignSqlOnWithBoolOps", "LEFT_ALIGN");
    let sql_where_style = style("AlignSqlWhereWithBoolOps", "RIGHT_ALIGN");
    let having_style = style("AlignSqlHavingWithBoolOps", "RIGHT_ALIGN");
    let when_style = style("AlignSqlWhenWithBoolOps", "DO_NOT_ALIGN");

    for cmd in doc.commands().to_vec() {
        if cmd.is_comment_only || cmd.colon.is_some() {
            continue;
        }
        let Some(first) = doc.first_code_token(&cmd) else {
            continue;
        };
        let is_sql = is_abap_sql_operation(doc, &cmd, first);
        let mut token = Some(first);
        while let Some(t) = token {
            let end_last = last_token_of_logical_expression(doc, &cmd, t).filter(|l| *l >= t);
            let Some(end_last) = end_last else {
                token = next_code(doc, t, cmd.last);
                continue;
            };
            let end = next_code(doc, end_last, cmd.last);
            let style = if kw(doc, t, "IF") {
                &if_style
            } else if kw(doc, t, "ELSEIF") {
                &elseif_style
            } else if kw(doc, t, "CHECK") {
                &check_style
            } else if kw(doc, t, "WHILE") {
                &while_style
            } else if kw(doc, t, "WHERE") {
                if is_sql {
                    &sql_where_style
                } else {
                    &where_style
                }
            } else if kw(doc, t, "UNTIL") {
                &until_style
            } else if kw(doc, t, "ON") {
                &on_style
            } else if kw(doc, t, "HAVING") {
                &having_style
            } else if kw(doc, t, "WHEN") {
                &when_style
            } else {
                "DO_NOT_ALIGN"
            };
            execute_on(
                doc,
                &cmd,
                is_sql,
                t,
                end,
                style,
                right_align_comparison_ops,
                only_align_same_objects,
                max_inner_spaces,
            );
            token = if is_sql {
                next_code(doc, t, cmd.last)
            } else {
                end
            };
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_on(
    doc: &mut Doc,
    cmd: &Command,
    is_sql: bool,
    keyword: usize,
    end: Option<usize>,
    style: &str,
    right_align_comparison_ops: bool,
    only_align_same_objects: bool,
    max_inner_spaces: i64,
) {
    let Some(start) = next_code(doc, keyword, cmd.last) else {
        return;
    };
    let end_prev = end.map_or(cmd.last, |e| e - 1);
    if end_prev < start {
        return;
    }
    let cx = Ctx {
        cmd_first: cmd.first,
        cmd_last: cmd.last,
        is_sql,
    };
    let Some(exp) = LogExp::create(doc, &cx, None, start, end_prev) else {
        return;
    };
    if !exp.supported {
        return;
    }
    let mut tree = TreeAlign {
        cols: Vec::new(),
        cur: None,
        first_token: exp.first,
        stop_cells: HashSet::new(),
        in_oo_context: cmd.in_oo_context,
    };
    if exp.to_tree_align(doc, &cx, &mut tree).is_none() {
        return;
    }
    tree.align(
        doc,
        cmd,
        keyword,
        style,
        right_align_comparison_ops,
        only_align_same_objects,
        max_inner_spaces,
        false,
    );
}

/// DDL_ALIGN_LOGICAL_EXPRESSIONS: runs the same tree alignment on DDL conditions.
#[allow(clippy::too_many_arguments)]
pub fn align_ddl_expression(
    doc: &mut Doc,
    ci: usize,
    keyword: usize,
    last_in_expr: usize,
    style: &str,
    right_align_comparison_ops: bool,
    max_inner_spaces: i64,
    attach_parentheses: bool,
) {
    let cmd = doc.syn.commands[ci].clone();
    let Some(start) = next_code(doc, keyword, cmd.last) else {
        return;
    };
    let cx = Ctx {
        cmd_first: cmd.first,
        cmd_last: cmd.last,
        is_sql: true,
    };
    let Some(exp) = LogExp::create(doc, &cx, None, start, last_in_expr) else {
        return;
    };
    if !exp.supported {
        return;
    }
    let mut tree = TreeAlign {
        cols: Vec::new(),
        cur: None,
        first_token: exp.first,
        stop_cells: HashSet::new(),
        in_oo_context: false,
    };
    if exp.to_tree_align(doc, &cx, &mut tree).is_none() {
        return;
    }
    tree.align(
        doc,
        &cmd,
        keyword,
        style,
        right_align_comparison_ops,
        false,
        max_inner_spaces,
        attach_parentheses,
    );
}
