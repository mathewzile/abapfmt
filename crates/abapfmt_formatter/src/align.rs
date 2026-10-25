//! Port of abap-cleaner's `AlignTable` / `AlignColumn` / `AlignLine` / `AlignCell`, shared by the
//! ALIGN_* rules. A cell is a range of tokens (a "term"); all positions are read live from the
//! `Doc`, so the order of operations matters just like upstream.
use abapfmt_syntax::TokenType;

use crate::Doc;

#[derive(Clone, Debug)]
pub struct Cell {
    pub first: usize,
    pub last: usize,
    pub add_indent: i64,
    pub override_width: Option<i64>,
    old_start: i64,
}

impl Cell {
    pub fn new(doc: &Doc, first: usize, last: usize) -> Cell {
        Cell {
            first,
            last,
            add_indent: 0,
            override_width: None,
            old_start: doc.start_x(0, first),
        }
    }

    /// `AlignCell*.createSpecial`
    pub fn special(doc: &Doc, first: usize, last: usize, add_indent: i64, width1: bool) -> Cell {
        let mut c = Cell::new(doc, first, last);
        c.add_indent = add_indent;
        c.override_width = width1.then_some(1);
        c
    }

    fn has_inner_comment(&self, doc: &Doc) -> bool {
        (self.first..self.last).any(|t| is_comment(doc, t))
    }

    pub(crate) fn mono_width(&self, doc: &Doc) -> i64 {
        self.add_indent
            + self
                .override_width
                .unwrap_or_else(|| sum_text_and_space_width(doc, self.first, self.last))
    }

    pub(crate) fn multi_width(&self, doc: &Doc) -> i64 {
        self.add_indent
            + self.override_width.unwrap_or_else(|| {
                max_end_index_in_any_line(doc, self.first, self.last) - doc.start_x(0, self.first)
            })
    }

    fn actual_mono_width(&self, doc: &Doc) -> i64 {
        self.add_indent + max_end_index_in_any_line(doc, self.first, self.last)
            - doc.start_x(0, self.first)
    }

    fn actual_multi_width(&self, doc: &Doc) -> i64 {
        self.add_indent + end_x(doc, self.last) - doc.start_x(0, self.first)
    }

    fn contains(&self, t: usize) -> bool {
        (self.first..=self.last).contains(&t)
    }

    /// `AlignCell.setWhitespace`; `overlength` is the token that got an emergency line break
    /// and the indent it must be aligned to.
    pub(crate) fn set_whitespace(
        &self,
        doc: &mut Doc,
        lines: u32,
        spaces: i64,
        keep_multiline: bool,
        condense_inner: bool,
        overlength: Option<(usize, i64)>,
    ) {
        doc.lines[self.first] = lines;
        doc.spaces[self.first] = (self.add_indent + spaces).max(0) as u32;
        if keep_multiline {
            let add = doc.start_x(0, self.first) - self.old_start;
            add_indent_to_term(doc, self.first, self.last, add);
            return;
        }
        for t in self.first + 1..=self.last {
            if doc.lines[t] == 0 && !condense_inner {
                continue;
            }
            if doc.lines[t] > 0 && is_comment(doc, t - 1) {
                continue;
            }
            if let Some((lb, sp)) = overlength
                && doc.lines[t] > 0
                && t == lb
            {
                doc.spaces[t] = sp.max(0) as u32;
                continue;
            }
            if (is_comment(doc, t) && doc.lines[t] > 0) || is_comment(doc, t - 1) {
                continue;
            }
            // a token attached to the previous one stays attached, as in "DATA lv_text(20) TYPE c."
            let attached = doc.lines[t] == 0 && doc.spaces[t] == 0;
            doc.lines[t] = 0;
            doc.spaces[t] = u32::from(!attached);
        }
    }
}

pub fn is_comment(doc: &Doc, t: usize) -> bool {
    doc.syn.types[t] == TokenType::Comment
}

fn len(doc: &Doc, t: usize) -> i64 {
    doc.token_text(t).chars().count() as i64
}

pub fn end_x(doc: &Doc, t: usize) -> i64 {
    doc.start_x(0, t) + len(doc, t)
}

/// `Term.getSumTextAndSpaceWidth(true)`
pub fn sum_text_and_space_width(doc: &Doc, first: usize, last: usize) -> i64 {
    let mut result = len(doc, first);
    for t in first + 1..=last {
        if doc.lines[t] > 0 || doc.spaces[t] > 0 {
            result += 1;
        }
        result += len(doc, t);
    }
    result
}

/// `Term.getMaxEndIndexInAnyLine(false)`
pub fn max_end_index_in_any_line(doc: &Doc, first: usize, last: usize) -> i64 {
    if first == last {
        return end_x(doc, first);
    }
    let n = doc.syn.tokens.len();
    let mut result = 0;
    for t in first..=last {
        if is_comment(doc, t) {
            continue;
        }
        let next = t + 1;
        let last_in_line = next >= n || doc.lines[next] > 0 || is_comment(doc, next);
        if t == last || last_in_line {
            result = result.max(end_x(doc, t));
        }
    }
    result
}

/// `Term.addIndent`: shifts all lines after the first one.
fn add_indent_to_term(doc: &mut Doc, first: usize, last: usize, add: i64) {
    if add == 0 {
        return;
    }
    for t in first + 1..=last {
        let asterisk = is_comment(doc, t) && doc.lines[t] > 0 && doc.syn.text(t).starts_with('*');
        if doc.lines[t] > 0 && !asterisk {
            doc.spaces[t] = (i64::from(doc.spaces[t]) + add).max(0) as u32;
        }
    }
}

#[derive(Default, Clone)]
pub struct Column {
    pub right_align: bool,
    force_line_break_after: bool,
    override_width_with_1: bool,
    minimum_width: i64,
    force_indent_base: Option<usize>,
    force_indent_offset: i64,
    effective_indent: i64,
}

#[derive(Clone, Default)]
pub struct Line {
    pub cells: Vec<Option<Cell>>,
    /// token before which an emergency line break may go, and the tokens to align it with
    pub overlength: Option<(usize, usize, Option<usize>)>,
}

impl Line {
    pub fn first_cell(&self) -> Option<&Cell> {
        self.cells.iter().flatten().next()
    }
    pub fn last_cell(&self) -> Option<&Cell> {
        self.cells.iter().flatten().last()
    }
    pub fn first_token(&self) -> Option<usize> {
        self.first_cell().map(|c| c.first)
    }
    pub fn last_token(&self) -> Option<usize> {
        self.last_cell().map(|c| c.last)
    }
    pub fn last_cell_index(&self) -> Option<usize> {
        self.cells.iter().rposition(Option::is_some)
    }
    pub fn contains_comments(&self, doc: &Doc) -> bool {
        match (self.first_token(), self.last_token()) {
            (Some(a), Some(b)) => (a..=b).any(|t| is_comment(doc, t)),
            _ => false,
        }
    }
    pub fn contains_inner_line_breaks(&self, doc: &Doc) -> bool {
        match (self.first_token(), self.last_token()) {
            (Some(a), Some(b)) => (a + 1..=b).any(|t| doc.lines[t] > 0),
            _ => false,
        }
    }
}

pub struct Table {
    pub cols: Vec<Column>,
    pub lines: Vec<Line>,
    max_line_length: i64,
}

impl Table {
    pub fn new(ncols: usize) -> Table {
        Table {
            cols: vec![
                Column {
                    force_indent_offset: -1,
                    ..Column::default()
                };
                ncols
            ],
            lines: Vec::new(),
            max_line_length: 0,
        }
    }

    pub fn add_line(&mut self) -> usize {
        self.lines.push(Line {
            cells: vec![None; self.cols.len()],
            overlength: None,
        });
        self.lines.len() - 1
    }

    pub fn set_cell(&mut self, line: usize, col: usize, cell: Cell) {
        self.lines[line].cells[col] = Some(cell);
    }

    pub fn column_count(&self, col: usize) -> usize {
        self.lines.iter().filter(|l| l.cells[col].is_some()).count()
    }

    pub fn column_is_empty(&self, col: usize) -> bool {
        self.column_count(col) == 0
    }

    /// `AlignColumn.setForceLineBreakAfter(overrideWidthWith1, offsetForNextColumn)`
    pub fn set_force_line_break_after(
        &mut self,
        col: usize,
        width1: bool,
        next_offset: Option<i64>,
    ) {
        self.cols[col].force_line_break_after = true;
        self.cols[col].override_width_with_1 = width1;
        if let Some(off) = next_offset {
            self.set_force_indent(col + 1, Some(col), off);
        }
    }

    /// `AlignColumn.setForceIndent(baseColumn, offset)`; `None` as base means the basic indent.
    pub fn set_force_indent(&mut self, col: usize, base: Option<usize>, offset: i64) {
        self.cols[col].force_indent_base = base;
        self.cols[col].force_indent_offset = offset;
    }

    /// `AlignTable.overrideWidthIfColumnIsFollowedByLineBreaks`
    pub fn override_width_if_followed_by_line_breaks(&mut self, doc: &Doc, col: usize) {
        let mut followed = true;
        if !self.cols[col].force_line_break_after {
            for line in &self.lines {
                if line.cells[col].is_none() {
                    continue;
                }
                if let Some(next) = line.cells[col + 1..].iter().flatten().next()
                    && doc.lines[next.first] == 0
                {
                    followed = false;
                    break;
                }
            }
        }
        if followed {
            for line in &mut self.lines {
                if let Some(c) = line.cells[col].as_mut() {
                    c.override_width = Some(1);
                }
            }
        }
    }

    /// `AlignColumn.getEffectiveIndent`
    pub fn effective_indent(&self, col: usize) -> i64 {
        self.cols[col].effective_indent
    }

    pub fn first_token(&self) -> Option<usize> {
        self.lines.first().and_then(Line::first_token)
    }

    /// `AlignTable.getFirstTokenColumnIndex`
    pub fn first_token_column_index(&self) -> Option<usize> {
        self.lines
            .first()
            .and_then(|l| l.cells.iter().position(Option::is_some))
    }

    /// `AlignTable.removeAllLinesOfParent`
    pub fn remove_all_lines_of_parent(&mut self, doc: &Doc, parent: usize) {
        self.lines.retain(|l| {
            l.first_token()
                .is_some_and(|t| doc.syn.parent[t] != Some(parent))
        });
    }

    /// `AlignLine.addWidthToEnd`
    pub fn add_width_to_end(&mut self, doc: &Doc, line: usize, add: i64) {
        let Some(ci) = self.lines[line].last_cell_index() else {
            return;
        };
        let cell = self.lines[line].cells[ci].as_mut().unwrap();
        if cell.override_width.is_some() {
            return;
        }
        let start = doc.start_x(0, cell.first);
        let last_line_width = end_x(doc, cell.last) - start;
        let max_line_width = max_end_index_in_any_line(doc, cell.first, cell.last) - start;
        if last_line_width + add > max_line_width {
            cell.override_width = Some(last_line_width + add);
        }
    }

    pub fn column_force_line_break_after(&self, col: usize) -> bool {
        self.cols[col].force_line_break_after
    }

    pub fn line_index_of_last_non_empty_cell(&self, col: usize) -> Option<usize> {
        self.lines.iter().rposition(|l| l.cells[col].is_some())
    }

    pub fn set_minimum_width(&mut self, col: usize, w: i64) {
        self.cols[col].minimum_width = w;
    }

    pub fn set_max_line_length(&mut self, n: i64) {
        self.max_line_length = n;
    }

    pub fn remove_line_at(&mut self, i: usize) {
        self.lines.remove(i);
    }

    /// `AlignColumn.isPreviousColumnFixedWidth`
    pub fn is_previous_column_fixed_width(&self, doc: &Doc, col: usize) -> bool {
        if col == 0 {
            return false;
        }
        let (mut min, mut max) = (-1i64, -1i64);
        for line in &self.lines {
            if line.cells[col].is_none() {
                continue;
            }
            let Some(prev) = &line.cells[col - 1] else {
                return false;
            };
            let w = prev.multi_width(doc);
            min = if min < 0 { w } else { min.min(w) };
            max = if max < 0 { w } else { max.max(w) };
        }
        min >= 0 && min == max
    }

    /// `AlignColumn.joinIntoPreviousColumns(condense, 1, false)`
    pub fn join_into_previous_columns(&mut self, doc: &mut Doc, col: usize, condense: bool) {
        self.join_into_previous_columns_ex(doc, col, condense, false);
    }

    /// `AlignColumn.joinIntoPreviousColumns(condense, 1, removeLineBreaksBetweenCells)`
    pub fn join_into_previous_columns_ex(
        &mut self,
        doc: &mut Doc,
        col: usize,
        condense: bool,
        remove_line_breaks: bool,
    ) {
        assert!(col > 0);
        for li in 0..self.lines.len() {
            let Some(cell) = self.lines[li].cells[col].take() else {
                continue;
            };
            for i in (0..col).rev() {
                if let Some(dest) = self.lines[li].cells[i].clone() {
                    if condense && (remove_line_breaks || !doc.first_in_line(cell.first)) {
                        let prev = cell.first - 1;
                        if !is_comment(doc, prev) {
                            cell.set_whitespace(doc, 0, 1, true, false, None);
                        } else {
                            let prev_code = (0..prev)
                                .rev()
                                .find(|j| !is_comment(doc, *j))
                                .unwrap_or(prev);
                            let x = end_x(doc, prev_code) + 1;
                            cell.set_whitespace(doc, 1, x, true, false, None);
                        }
                    }
                    // the joined cell keeps the special settings of the destination cell
                    let mut joined = Cell::new(doc, dest.first, cell.last);
                    joined.add_indent = dest.add_indent;
                    joined.override_width = (dest.override_width == Some(1)).then_some(1);
                    self.lines[li].cells[i] = Some(joined);
                    break;
                } else if i == 0 || !self.column_is_empty(i) {
                    self.lines[li].cells[i] = Some(cell);
                    break;
                }
            }
        }
    }

    /// `AlignColumn.hasAnyLineBreakBefore`
    pub fn has_any_line_break_before(&self, doc: &Doc, col: usize) -> bool {
        self.lines
            .iter()
            .filter_map(|l| l.cells[col].as_ref())
            .any(|c| doc.lines[c.first] > 0)
    }

    /// `AlignColumn.removeLineBreaksBefore`
    pub fn remove_line_breaks_before(&mut self, doc: &mut Doc, col: usize) {
        for line in &self.lines {
            let Some(cell) = &line.cells[col] else {
                continue;
            };
            let first = cell.first;
            if doc.lines[first] == 0 || (first > 0 && is_comment(doc, first - 1)) {
                continue;
            }
            doc.lines[first] = 0;
            doc.spaces[first] = 1;
        }
    }

    /// `AlignTable.getTotalMultiLineWidth`
    pub fn total_multi_line_width(&self, doc: &Doc) -> i64 {
        let mut total = 0;
        let mut result = 0;
        for col in 0..self.cols.len() {
            result += self.width_with_space(doc, col, true);
            if self.cols[col].force_line_break_after {
                total = total.max(result);
                result = 0;
            }
        }
        (total.max(result) - 1).max(0)
    }

    /// `AlignColumn.removeFromTable`
    pub fn remove_from_table(&mut self, doc: &mut Doc, col: usize, condense: bool) {
        assert!(col > 0);
        for li in 0..self.lines.len() {
            let Some(cell) = self.lines[li].cells[col].take() else {
                continue;
            };
            if condense && !doc.first_in_line(cell.first) {
                cell.set_whitespace(doc, 0, 1, true, false, None);
            }
        }
    }

    fn width_with_space(&self, doc: &Doc, col: usize, multi: bool) -> i64 {
        if self.column_is_empty(col) {
            return 0;
        }
        let c = &self.cols[col];
        let max = self
            .lines
            .iter()
            .filter_map(|l| l.cells[col].as_ref())
            .map(|cell| {
                if c.override_width_with_1 {
                    1
                } else if multi {
                    cell.multi_width(doc)
                } else {
                    cell.mono_width(doc)
                }
            })
            .max()
            .unwrap_or(0);
        max.max(c.minimum_width) + 1
    }

    fn can_align_to_mono_line(&self, doc: &Doc) -> bool {
        for line in &self.lines {
            let mut comment_before = false;
            for cell in line.cells.iter().flatten() {
                if cell.has_inner_comment(doc) || comment_before {
                    return false;
                }
                if is_comment(doc, cell.last) {
                    comment_before = true;
                }
            }
        }
        true
    }

    /// `AlignTable.align(basicIndent, firstLineBreaks, keepMultiline, condenseInnerSpaces, false)`
    pub fn align(
        &mut self,
        doc: &mut Doc,
        basic_indent: i64,
        first_line_breaks: u32,
        keep_multiline: bool,
        condense_inner: bool,
    ) {
        let keep_multiline = keep_multiline || !self.can_align_to_mono_line(doc);
        for c in &mut self.cols {
            c.effective_indent = 0;
        }
        let starts_with_first_token = self
            .lines
            .first()
            .and_then(|l| l.first_token())
            .is_some_and(|t| t == 0);
        let min_spaces_left = 1;
        let widths: Vec<i64> = (0..self.cols.len())
            .map(|c| self.width_with_space(doc, c, keep_multiline))
            .collect();

        for li in 0..self.lines.len() {
            let is_first_line = li == 0;
            let mut line_breaks = if is_first_line { first_line_breaks } else { 1 };
            let mut spaces_left = if line_breaks == 0 && !starts_with_first_token {
                min_spaces_left
            } else {
                basic_indent
            };
            if let Some(ft) = self.lines[li].first_token()
                && ft > 0
            {
                let prev_end = end_x(doc, ft - 1);
                if prev_end + 1 == basic_indent {
                    line_breaks = 0;
                    spaces_left = 1;
                }
            }

            let mut column_indent = basic_indent;
            #[allow(clippy::needless_range_loop)]
            for ci in 0..self.cols.len() {
                if self.cols[ci].force_indent_offset >= 0 {
                    let base = self.cols[ci]
                        .force_indent_base
                        .map_or(basic_indent, |b| self.cols[b].effective_indent);
                    column_indent = base + self.cols[ci].force_indent_offset;
                    spaces_left = column_indent;
                }
                self.cols[ci].effective_indent = column_indent;
                if self.column_is_empty(ci) {
                    if self.cols[ci].force_line_break_after {
                        line_breaks = 1;
                    }
                    continue;
                }
                let column_width = widths[ci];
                let Some(cell) = self.lines[li].cells[ci].clone() else {
                    spaces_left += column_width;
                    column_indent += column_width;
                    continue;
                };
                let prev = cell.first.checked_sub(1);

                if line_breaks == 0 {
                    if prev.is_some_and(|p| is_comment(doc, p)) {
                        // prevent code from being appended to a comment
                        line_breaks = 1;
                        spaces_left = column_indent;
                    } else if cell.first != 0 {
                        spaces_left = spaces_left.max(min_spaces_left);
                    }
                } else if doc.lines[cell.first] > line_breaks {
                    // keep multiple existing line breaks
                    line_breaks = doc.lines[cell.first];
                }

                // if line length would be exceeded, move the 'overlength line break token' to
                // the next line, aligning it with one of the fallback tokens
                let mut overlength: Option<(usize, i64)> = None;
                if self.max_line_length > 0
                    && let Some((lb, fb1, fb2)) = self.lines[li].overlength
                    && cell.contains(lb)
                    && line_breaks == 0
                    && prev.is_some()
                    && !self.cols[ci].right_align
                    && !keep_multiline
                {
                    let line_last = self.lines[li].last_token().unwrap();
                    let remaining = condensed_width(doc, cell.first, line_last);
                    let prev_end = end_x(doc, prev.unwrap());
                    if prev_end + spaces_left + remaining >= self.max_line_length {
                        let required = condensed_width(doc, lb, line_last);
                        let indent =
                            overlength_indent(doc, fb1, fb2, required, self.max_line_length);
                        if lb == cell.first {
                            line_breaks = 1;
                            spaces_left = indent;
                        } else {
                            doc.lines[lb] = 1;
                            overlength = Some((lb, indent));
                        }
                    }
                }

                let used_width = if self.cols[ci].right_align {
                    let cell_width = if keep_multiline {
                        cell.multi_width(doc)
                    } else {
                        cell.mono_width(doc)
                    };
                    spaces_left += column_width - 1 - cell_width;
                    cell.set_whitespace(
                        doc,
                        line_breaks,
                        spaces_left,
                        keep_multiline,
                        condense_inner,
                        overlength,
                    );
                    column_width - 1
                } else {
                    cell.set_whitespace(
                        doc,
                        line_breaks,
                        spaces_left,
                        keep_multiline,
                        condense_inner,
                        overlength,
                    );
                    if keep_multiline {
                        cell.actual_multi_width(doc)
                    } else {
                        cell.actual_mono_width(doc)
                    }
                };
                if self.cols[ci].force_line_break_after {
                    line_breaks = 1;
                    spaces_left = doc.start_x(0, cell.first) + column_width;
                } else {
                    line_breaks = 0;
                    spaces_left = column_width - used_width;
                }
                column_indent += column_width;
            }
        }
    }
}

/// `Token.getCondensedWidthUpTo`: the width of the tokens from `first` to `last` with one space
/// between them (none for attached tokens), ignoring comments at the end.
fn condensed_width(doc: &Doc, first: usize, last: usize) -> i64 {
    let mut w = len(doc, first);
    for t in first + 1..=last {
        if is_comment(doc, t) && doc.lines[t] == 0 {
            continue;
        }
        if doc.lines[t] > 0 || doc.spaces[t] > 0 {
            w += 1;
        }
        w += len(doc, t);
    }
    w
}

/// `AlignOverlengthAction.getSpacesLeft`: the indent of the first fallback token if the remaining
/// text fits behind it, else that of the second fallback token.
fn overlength_indent(
    doc: &Doc,
    fb1: usize,
    fb2: Option<usize>,
    required: i64,
    max_len: i64,
) -> i64 {
    let x1 = doc.start_x(0, fb1);
    match fb2 {
        Some(f) if x1 + required > max_len => doc.start_x(0, f),
        _ => x1,
    }
}

/// A comment alone on its line (`Token.isCommentLine`).
pub fn is_comment_line(doc: &Doc, t: usize) -> bool {
    is_comment(doc, t)
        && doc.first_in_line(t)
        && (t + 1 >= doc.syn.tokens.len() || doc.lines[t + 1] > 0)
}

/// `Token.condenseUpTo`: puts the tokens after `first` up to `last` behind each other, breaking
/// lines at `indent` where `max_line_length` would be exceeded.
pub fn condense_up_to(
    doc: &mut Doc,
    first: usize,
    last: usize,
    max_line_length: i64,
    indent: i64,
    keep_comment_lines: bool,
) {
    if first == last {
        return;
    }
    let mut index_in_line = end_x(doc, first);
    let mut last_movable: Option<usize> = None;
    let n = doc.syn.tokens.len();
    let mut t = first + 1;
    while t < n {
        let attached = doc.lines[t] == 0 && doc.spaces[t] == 0;
        let ty = doc.syn.types[t];
        let spaces_left = i64::from(
            !(attached || matches!(ty, TokenType::Comma | TokenType::Period | TokenType::Colon)),
        );
        if spaces_left > 0 {
            last_movable = Some(t);
        }
        let comment_line = is_comment_line(doc, t);
        let asterisk_line = comment_line && doc.syn.text(t).starts_with('*');
        let quot_line = comment_line && doc.syn.text(t).starts_with('"');
        if asterisk_line || keep_comment_lines && quot_line {
            // do nothing
        } else if is_comment(doc, t - 1) {
            doc.lines[t] = 1;
            doc.spaces[t] = indent.max(0) as u32;
            last_movable = None;
            index_in_line = end_x(doc, t);
        } else if let Some(m) = last_movable
            && index_in_line + spaces_left + len(doc, t) > max_line_length
        {
            doc.lines[m] = 1;
            doc.spaces[m] = indent.max(0) as u32;
            if t != m {
                doc.lines[t] = 0;
                doc.spaces[t] = spaces_left as u32;
            }
            last_movable = None;
            index_in_line = end_x(doc, t);
        } else {
            doc.lines[t] = 0;
            doc.spaces[t] = spaces_left as u32;
            index_in_line += spaces_left + len(doc, t);
        }
        if t == last {
            break;
        }
        t += 1;
    }
}
