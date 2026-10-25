//! Port of abap-cleaner's `Command.distinguishOperators`: tells the assignment operator `=` from
//! the comparison operator `=` and the keyword `IN` / `BETWEEN` from their comparison variants.
use abapfmt_lexer::{Token, is_comparison_op};

use crate::{Command, TokenType};

const CONSTRUCTOR_OPS: &[&str] = &[
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
const BINARY_OPS: &[&str] = &[
    "+", "-", "*", "/", "DIV", "MOD", "**", "BIT-AND", "BIT-OR", "BIT-XOR", "&&", "&",
];

struct Cx<'a> {
    src: &'a str,
    tokens: &'a [Token],
    bm: &'a [Option<usize>],
    parent: &'a [Option<usize>],
    last: usize,
}

struct State<'a> {
    cx: Cx<'a>,
    types: &'a mut [TokenType],
}

impl State<'_> {
    fn text(&self, t: usize) -> &str {
        self.cx.tokens[t].text(self.cx.src)
    }

    fn text_is(&self, t: usize, s: &str) -> bool {
        self.text(t).eq_ignore_ascii_case(s)
    }

    fn is_kw(&self, t: usize, s: &str) -> bool {
        self.types[t] == TokenType::Keyword && self.text_is(t, s)
    }

    fn is_any_kw(&self, t: usize, set: &[&str]) -> bool {
        set.iter().any(|s| self.is_kw(t, s))
    }

    fn is_code(&self, t: usize) -> bool {
        !matches!(self.types[t], TokenType::Comment | TokenType::Pragma)
    }

    fn opens(&self, t: usize) -> bool {
        self.cx.bm[t].is_some_and(|c| c > t)
    }

    fn closes(&self, t: usize) -> bool {
        self.cx.bm[t].is_some_and(|o| o < t)
    }

    fn is_attached(&self, t: usize) -> bool {
        self.cx.tokens[t].lines_before == 0 && self.cx.tokens[t].spaces_before == 0
    }

    fn next_code_token(&self, t: usize) -> Option<usize> {
        (t + 1..=self.cx.last).find(|j| self.is_code(*j))
    }

    fn prev_code_token(&self, t: usize, first: usize) -> Option<usize> {
        (first..t).rev().find(|j| self.is_code(*j))
    }

    /// `Token.getNextSibling`: the closing token for a level opener.
    fn next_sibling(&self, t: usize) -> Option<usize> {
        let n = if self.opens(t) { self.cx.bm[t]? } else { t + 1 };
        (n <= self.cx.last).then_some(n)
    }

    /// `Token.getNextCodeSibling`: `None` after the last child of a level.
    fn next_code_sibling(&self, t: usize) -> Option<usize> {
        let mut cur = t;
        loop {
            let opens = self.opens(cur);
            let n = if opens { self.cx.bm[cur]? } else { cur + 1 };
            if n > self.cx.last || (!opens && self.closes(n)) {
                return None;
            }
            if self.is_code(n) {
                return Some(n);
            }
            cur = n;
        }
    }

    fn prev_code_sibling(&self, t: usize, first: usize) -> Option<usize> {
        if self.closes(t) {
            return self.cx.bm[t];
        }
        let p = self.prev_code_token(t, first)?;
        if self.cx.parent[t] == Some(p) {
            None
        } else {
            Some(p)
        }
    }

    fn next_sibling_while_level_opener(&self, mut t: usize) -> usize {
        while let Some(c) = self.cx.bm[t].filter(|c| *c > t) {
            t = c;
        }
        t
    }

    /// `matchesOnSiblings(true, "A B|C D", ...)` for a single pattern with alternatives.
    fn matches_phrase(&self, t: usize, phrases: &str) -> bool {
        phrases.split('|').any(|phrase| {
            let mut cur = Some(t);
            for (i, w) in phrase.split(' ').enumerate() {
                let Some(c) = cur else { return false };
                if !self.text_is(c, w) {
                    return false;
                }
                if i + 1 < phrase.split(' ').count() {
                    cur = self.next_code_sibling(c);
                }
            }
            true
        })
    }

    fn starts_constructor_expression(&self, t: usize, first: usize) -> bool {
        if self.text_is(t, "#(")
            || self.types[t] == TokenType::Identifier
                && self.opens(t)
                && self.text(t).ends_with('(')
        {
            if t + 1 > self.cx.last || self.is_attached(t + 1) {
                return false;
            }
            self.prev_code_token(t, first)
                .is_some_and(|p| self.is_any_kw(p, CONSTRUCTOR_OPS))
        } else {
            false
        }
    }

    fn may_start_lhs_of_assignment(&self, t: usize) -> bool {
        if self.types[t] != TokenType::Identifier || self.closes(t) {
            return false;
        }
        let end = self.next_sibling_while_level_opener(t);
        self.next_code_sibling(end)
            .is_some_and(|op| self.text(op) == "=")
    }

    // ---- expressions ----

    /// `Term.createArithmetic(first).lastToken`; `None` if the syntax is unexpected.
    fn arithmetic_last(&self, first: usize) -> Option<usize> {
        let ty = self.types[first];
        let mut token = first;
        if self.is_any_kw(first, CONSTRUCTOR_OPS) {
            token = self.next_code_token(token)?;
        } else if ty == TokenType::Keyword
            && (["DATA(", "FINAL(", "@DATA(", "@FINAL("]
                .iter()
                .any(|k| self.text_is(first, k))
                || self.text(first).to_ascii_uppercase().starts_with("TEXT-")
                || self.text_is(first, "CASE"))
        {
        } else if !matches!(
            ty,
            TokenType::Literal | TokenType::Identifier | TokenType::OtherOp
        ) && !(ty == TokenType::Keyword && self.may_be_misread_name(first))
        {
            return None;
        }
        if self.is_kw(token, "CASE") {
            let mut depth = 1;
            while depth > 0 {
                let Some(n) = self.next_code_sibling(token) else {
                    break;
                };
                token = n;
                if self.is_kw(token, "CASE") {
                    depth += 1;
                } else if self.is_kw(token, "END") {
                    depth -= 1;
                }
            }
        } else {
            token = self.next_sibling_while_level_opener(token);
        }
        while let Some(n) = self.next_code_token(token)
            && BINARY_OPS.iter().any(|b| self.text_is(n, b))
        {
            token = self.next_code_token(n)?;
            if self.is_any_kw(token, CONSTRUCTOR_OPS) {
                token = self.next_code_token(token)?;
            }
            token = self.next_sibling_while_level_opener(token);
        }
        Some(token)
    }

    /// A keyword that is followed by a comma, a closing bracket or the end of the command is
    /// most likely a name, as in `replace_regexpr( value = text )`.
    fn may_be_misread_name(&self, t: usize) -> bool {
        self.next_code_token(t)
            .is_none_or(|n| matches!(self.text(n), "," | "." | ")") || self.closes(n))
    }

    fn is_in_after_let(&self, t: usize, first: usize) -> bool {
        if !self.text_is(t, "IN") {
            return false;
        }
        let Some(parent) = self.cx.parent[t] else {
            return false;
        };
        if !self.starts_constructor_expression(parent, first) {
            return false;
        }
        let Some(close) = self.cx.bm[parent] else {
            return false;
        };
        let Some(mut cur) = (parent + 1..close).find(|j| self.is_code(*j)) else {
            return false;
        };
        // the first 'IN' sibling behind a sibling 'LET'
        let mut seen_let = false;
        loop {
            if self.text_is(cur, "LET") {
                seen_let = true;
            } else if seen_let && self.text_is(cur, "IN") {
                return cur == t;
            }
            match self.next_code_sibling(cur) {
                Some(n) => cur = n,
                None => return false,
            }
        }
    }

    fn last_of_relational(&self, first_tok: usize, first: usize) -> Option<usize> {
        let mut token = first_tok;
        if [
            "contains(",
            "contains_any_of(",
            "contains_any_not_of(",
            "matches(",
            "line_exists(",
        ]
        .iter()
        .any(|f| self.text_is(token, f))
        {
            return self.next_sibling(token);
        }
        token = self.arithmetic_last(token)?;
        let next = self.next_code_token(token)?;
        if self.is_kw(next, "IS") {
            token = self.next_code_sibling(next)?;
            if self.is_kw(token, "NOT") {
                token = self.next_code_sibling(token)?;
            }
            if self.is_any_kw(token, &["INITIAL", "BOUND", "ASSIGNED", "SUPPLIED"]) {
            } else if self.matches_phrase(token, "INSTANCE OF") {
                let of = self.next_code_sibling(token)?;
                token = self.next_code_sibling(of)?;
            } else {
                return None;
            }
        } else if (self.text(next) == "=" || is_comparison_op(self.text(next)))
            && !self.is_in_after_let(next, first)
        {
            let ternary = self.text_is(next, "BETWEEN");
            let t2 = self.arithmetic_last(self.next_code_sibling(next)?)?;
            if ternary {
                token = self.next_code_token(t2)?;
                if !self.is_kw(token, "AND") {
                    return None;
                }
                token = self.arithmetic_last(self.next_code_sibling(token)?)?;
            } else {
                token = t2;
            }
        }
        Some(token)
    }

    /// `Token.findLastTokenOfExpression`
    fn last_of_expression(&self, first_tok: usize, first: usize) -> Option<usize> {
        let mut token = first_tok;
        loop {
            while self.text(token).to_ascii_uppercase().ends_with("NOT") {
                token = self.next_code_sibling(token)?;
            }
            token = self.last_of_relational(token, first)?;
            let next = self.next_code_token(token)?;
            if !self.is_any_kw(next, &["AND", "OR", "EQUIV"]) {
                break;
            }
            token = self.next_code_sibling(next)?;
        }
        Some(token)
    }

    // ---- distinguishOperators ----

    #[allow(clippy::too_many_arguments)]
    fn distinguish_pre_check(
        &mut self,
        mut at_start: bool,
        start: usize,
        end: Option<usize>,
        pre_check: bool,
        outer: Option<usize>,
        first: usize,
    ) {
        if pre_check && !at_start {
            let code = if self.is_code(start) {
                Some(start)
            } else {
                self.next_code_sibling(start)
            };
            match code {
                Some(c)
                    if self.types[c] != TokenType::Keyword
                        && !self.may_start_lhs_of_assignment(c) =>
                {
                    at_start = true;
                }
                _ => {}
            }
        }
        self.distinguish(at_start, start, end, None, None, outer, first);
    }

    #[allow(clippy::too_many_arguments)]
    fn distinguish(
        &mut self,
        at_start: bool,
        start: usize,
        end: Option<usize>,
        start_cond: Option<&str>,
        end_cond: Option<&str>,
        outer: Option<usize>,
        first: usize,
    ) {
        let mut token = Some(start);
        let mut end_of_log_expr: Option<usize> = None;
        let mut end_of_rhs_expr: Option<usize> = None;
        let mut is_cmp_pos = at_start;
        let mut is_cmp_after_rhs = at_start;

        while token != end {
            let Some(t) = token else { break };
            if start_cond.is_some_and(|c| self.matches_phrase(t, c)) {
                is_cmp_pos = !at_start;
                if self.is_any_kw(t, &["UNTIL", "WHILE"]) {
                    end_of_log_expr = self
                        .next_code_sibling(t)
                        .and_then(|n| self.last_of_expression(n, first))
                        .and_then(|l| self.next_code_token(l));
                }
            } else if Some(t) == end_of_log_expr
                || end_cond.is_some_and(|c| self.matches_phrase(t, c))
            {
                is_cmp_pos = at_start;
                end_of_log_expr = None;
            } else if Some(t) == end_of_rhs_expr {
                is_cmp_pos = is_cmp_after_rhs;
                end_of_rhs_expr = None;
            }

            let ty = self.types[t];
            if self.text(t) == "="
                && matches!(ty, TokenType::AssignmentOp | TokenType::ComparisonOp)
            {
                let prev = self.prev_code_sibling(t, first);
                let prev_can_receive =
                    prev.is_some_and(|p| self.text(p) != ")" || self.is_attached(p));
                self.types[t] = if is_cmp_pos || !prev_can_receive {
                    TokenType::ComparisonOp
                } else {
                    TokenType::AssignmentOp
                };
                if let Some(next_code) = self.next_code_sibling(t).filter(|_| !is_cmp_pos) {
                    let after = self.next_sibling_while_level_opener(next_code);
                    let is_chain = self.cx.parent[next_code].is_none()
                        && self.types[next_code] == TokenType::Identifier
                        && self
                            .next_code_sibling(after)
                            .is_some_and(|n| self.text(n) == "=");
                    if is_chain {
                        end_of_rhs_expr = Some(after);
                    } else {
                        end_of_rhs_expr = self
                            .last_of_expression(next_code, first)
                            .and_then(|l| self.next_code_token(l));
                        if end_of_rhs_expr.is_some() {
                            is_cmp_after_rhs = is_cmp_pos;
                            is_cmp_pos = true;
                        }
                    }
                }
            } else if (self.text_is(t, "IN") || self.text_is(t, "BETWEEN"))
                && ty != TokenType::Identifier
            {
                self.types[t] = if is_cmp_pos {
                    TokenType::ComparisonOp
                } else {
                    TokenType::Keyword
                };
            }

            if self.opens(t) {
                let close = self.cx.bm[t];
                let ctor = self.prev_code_sibling(t, first);
                let next = t + 1;
                if self.text_is(t, "boolc(") || self.text_is(t, "xsdbool(") {
                    self.distinguish(true, next, close, None, None, None, first);
                } else if ctor.is_some_and(|c| self.is_kw(c, "COND")) {
                    self.distinguish(false, next, close, Some("WHEN"), Some("THEN"), ctor, first);
                } else if ctor.is_some_and(|c| self.is_any_kw(c, &["REDUCE", "NEW", "VALUE"])) {
                    self.distinguish(
                        false,
                        next,
                        close,
                        Some("UNTIL|WHILE"),
                        Some("LET|NEXT|FOR"),
                        ctor,
                        first,
                    );
                } else if ctor.is_some_and(|c| self.is_kw(c, "FILTER")) {
                    self.distinguish(false, next, close, Some("WHERE"), None, ctor, first);
                } else if ctor.is_some_and(|c| self.is_kw(c, "WHERE")) {
                    self.distinguish(true, next, close, None, None, outer, first);
                } else if ctor.is_some_and(|c| self.is_kw(c, "CORRESPONDING")) {
                    self.distinguish(false, next, close, None, None, ctor, first);
                } else if self.text(t) == "(" {
                    let in_corresponding = outer.is_some_and(|o| self.is_kw(o, "CORRESPONDING"));
                    let parent_is_paren =
                        self.cx.parent[t].is_some_and(|p| self.text(p) == "(") && !in_corresponding;
                    self.distinguish_pre_check(
                        is_cmp_pos || parent_is_paren,
                        next,
                        close,
                        true,
                        outer,
                        first,
                    );
                } else {
                    let pre_check = ctor.is_none() && self.text(t).ends_with('(');
                    self.distinguish_pre_check(false, next, close, pre_check, None, first);
                }
                token = self.next_sibling(t);
            } else {
                token = self.next_code_token(t);
            }
        }
    }
}

/// Re-types the `=`, `IN` and `BETWEEN` tokens of all commands.
pub(crate) fn distinguish_operators(
    src: &str,
    tokens: &[Token],
    commands: &[Command],
    bm: &[Option<usize>],
    parent: &[Option<usize>],
    types: &mut [TokenType],
) {
    for cmd in commands {
        if cmd.is_comment_only {
            continue;
        }
        let mut st = State {
            cx: Cx {
                src,
                tokens,
                bm,
                parent,
                last: cmd.last,
            },
            types: &mut *types,
        };
        let Some(first_code) = (cmd.first..=cmd.last).find(|j| st.is_code(*j)) else {
            continue;
        };
        let first = cmd.first;
        let fc_kw = st.types[first_code] == TokenType::Keyword;
        if fc_kw && st.is_any_kw(first_code, &["IF", "ELSEIF", "CHECK", "WHILE"]) {
            st.distinguish(true, first_code, None, None, None, None, first);
        } else if st.matches_phrase(first_code, "LOOP AT|MODIFY|DELETE|FOR") {
            st.distinguish(
                false,
                first_code,
                None,
                Some("WHERE"),
                Some("GROUP BY|USING KEY|TRANSPORTING|FROM"),
                None,
                first,
            );
        } else if fc_kw && st.is_any_kw(first_code, &["SELECT", "WITH"]) {
            st.distinguish(true, first_code, None, None, None, None, first);
        } else if fc_kw && st.is_kw(first_code, "UPDATE") {
            st.distinguish(
                false,
                first_code,
                None,
                Some("WHERE"),
                Some("%_HINTS|OPTIONS|CONNECTION"),
                None,
                first,
            );
        } else if fc_kw && st.is_kw(first_code, "WAIT") {
            st.distinguish(
                false,
                first_code,
                None,
                Some("UNTIL"),
                Some("UP TO"),
                None,
                first,
            );
        } else if fc_kw && st.is_kw(first_code, "ASSERT") {
            let has_condition = {
                let mut cur = Some(first_code);
                let mut found = false;
                while let Some(c) = cur {
                    if st.is_kw(c, "CONDITION") {
                        found = true;
                        break;
                    }
                    cur = st.next_code_sibling(c);
                }
                found
            };
            if has_condition {
                st.distinguish(
                    false,
                    first_code,
                    None,
                    Some("CONDITION"),
                    None,
                    None,
                    first,
                );
            } else {
                st.distinguish(true, first_code, None, None, None, None, first);
            }
        } else if fc_kw && st.is_kw(first_code, "TYPES") {
            st.distinguish(
                false,
                first_code,
                None,
                Some("ASSOCIATION"),
                None,
                None,
                first,
            );
        } else {
            st.distinguish(false, first, None, None, None, None, first);
        }
    }
}

impl crate::Syntax<'_> {
    /// `Token.findLastTokenOfExpression`: the last token of the (logical, relational or
    /// arithmetic) expression that starts with `first_tok` inside the command `cmd_first..=cmd_last`;
    /// `None` where abap-cleaner reports an unexpected syntax.
    pub fn last_of_expression(
        &self,
        first_tok: usize,
        cmd_first: usize,
        cmd_last: usize,
    ) -> Option<usize> {
        let mut types = self.types.clone();
        let st = State {
            cx: Cx {
                src: self.src,
                tokens: &self.tokens,
                bm: &self.bracket_match,
                parent: &self.parent,
                last: cmd_last,
            },
            types: &mut types,
        };
        st.last_of_expression(first_tok, cmd_first)
    }
}
