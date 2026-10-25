//! DDL_ANNO_LAYOUT: standardizes spaces, one-liners and alignment of DDL annotations (port of
//! `DdlAnnotationLayoutRule`, `DdlAnnotationScope`, `DdlAnnotation`, `DdlAnnotationNestingRange` and
//! the annotation writers; the nesting rule is not part of this port, so existing nesting is kept).
//!
//! Upstream re-creates the annotation commands from the annotation model and swaps them into the
//! code. Here the re-created commands are only used to take over whitespace: if they do not match
//! the existing commands token for token (ignoring whitespace), the section is left unchanged.
use super::ddl_util::*;
use crate::{ConfigExt, Doc, RuleConfig};

// ---- annotation model ----

struct Elem {
    name: String,
    started_nesting_in_original: bool,
    starts_array: bool,
    /// used temporarily if `starts_array`
    anonymous_object_count: u32,
    nesting_range: Option<usize>,
    end_index: i64,
}

impl Elem {
    fn new(name: String) -> Elem {
        Elem {
            name,
            started_nesting_in_original: false,
            starts_array: false,
            anonymous_object_count: 0,
            nesting_range: None,
            end_index: 0,
        }
    }

    fn anonymous(number: u32) -> Elem {
        Elem::new(format!("*{}", 1000 + number))
    }

    fn is_anonymous_array_object(&self) -> bool {
        self.name.starts_with('*')
    }

    fn matches(&self, other: &Elem) -> bool {
        self.name.eq_ignore_ascii_case(&other.name) && self.starts_array == other.starts_array
    }

    fn copy_of(&self) -> Elem {
        Elem {
            name: self.name.clone(),
            started_nesting_in_original: self.started_nesting_in_original,
            starts_array: self.starts_array,
            ..Elem::new(String::new())
        }
    }
}

struct Anno {
    elems: Vec<Elem>,
    value: Option<String>,
    original_line_breaks: u32,
    value_end_index: i64,
    /// the length of the longest value in a table of structure-identical objects in an array
    value_column_width: usize,
}

impl Anno {
    fn level_count(&self) -> usize {
        self.elems.len()
    }

    fn level_of_first_diff(&self, other: &Anno) -> usize {
        let min = self.elems.len().min(other.elems.len());
        (0..min)
            .find(|i| !self.elems[*i].matches(&other.elems[*i]))
            .unwrap_or(min)
    }

    fn has_array_start_at(&self, level: usize) -> bool {
        self.elems.get(level).is_some_and(|e| e.starts_array)
    }

    fn starts_nesting_after_level(&self, level: usize) -> bool {
        self.elems
            .get(level)
            .is_some_and(|e| e.nesting_range.is_some())
    }

    fn original_started_nesting_after_level(&self, level: usize) -> bool {
        self.elems
            .get(level)
            .is_some_and(|e| e.started_nesting_in_original)
    }

    fn inner_nesting_depth_after_level(&self, level: usize) -> usize {
        (level..self.elems.len())
            .filter(|i| self.starts_nesting_after_level(*i) || self.has_anonymous_object_at(*i))
            .count()
    }

    fn has_anonymous_object_at(&self, level: usize) -> bool {
        self.elems
            .get(level)
            .is_some_and(|e| e.is_anonymous_array_object())
    }

    fn elem_name(&self, level: usize) -> &str {
        self.elems.get(level).map_or("", |e| e.name.as_str())
    }

    fn is_value_in_array(&self) -> bool {
        !self.elems.is_empty() && self.has_array_start_at(self.elems.len() - 1)
    }

    fn starts_nesting_or_array_before(&self, level: usize) -> bool {
        (0..level).any(|i| self.starts_nesting_after_level(i))
    }

    fn is_elem_followed_by_dot(&self, level: usize) -> bool {
        !self.starts_nesting_after_level(level) && level + 1 < self.level_count()
    }
}

struct NestingRange {
    nest_after_level: usize,
    is_array: bool,
    annos: Vec<usize>,
    column_width: usize,
    max_inner_elem_depth: usize,
    max_inner_nesting_depth: usize,
    write_as_one_liner: bool,
}

struct Layout {
    spaces_after_colon: i64,
    spaces_inside_braces: i64,
    spaces_inside_brackets: i64,
    max_line_length: i64,
    max_one_liner_elem_count_main: usize,
    max_one_liner_elem_count_list: usize,
    align_values: bool,
    align_tables_in_arrays: bool,
    initial_line_breaks: u32,
    basic_indent: i64,
}

#[derive(Default)]
struct Scope {
    annos: Vec<Anno>,
    ranges: Vec<NestingRange>,
    /// the annotation commands, in order
    commands: Vec<usize>,
}

impl Scope {
    /// `DdlAnnotationScope.add`; `None` if the command cannot be processed (e.g. due to comments).
    fn add(&mut self, doc: &Doc, ci: usize) -> Option<()> {
        self.commands.push(ci);
        let (first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
        let mut elems: Vec<Elem> = Vec::new();
        let mut has_content = false;
        let mut expecting_value = false;
        let mut line_breaks = doc.lines[first];
        let mut t = first;
        while t <= last {
            if is_comment(doc, t) {
                return None;
            }
            let text = doc.syn.text(t);
            match text {
                "{" => {
                    expecting_value = false;
                    let l = elems.last_mut()?;
                    if l.starts_array {
                        l.anonymous_object_count += 1;
                        let n = l.anonymous_object_count;
                        elems.push(Elem::anonymous(n));
                    }
                    elems.last_mut()?.started_nesting_in_original = true;
                }
                "." => expecting_value = false,
                ":" => expecting_value = true,
                "[" => {
                    expecting_value = true;
                    let l = elems.last_mut()?;
                    if l.starts_array {
                        l.anonymous_object_count += 1;
                        let n = l.anonymous_object_count;
                        elems.push(Elem::anonymous(n));
                    }
                    elems.last_mut()?.starts_array = true;
                }
                "," | "}" | "]" => {
                    if has_content {
                        self.annos
                            .push(create_anno(&elems, expecting_value, line_breaks));
                    }
                    // after a comma, return to the beginning of the last { or [; after } or ],
                    // return to the beginning of the { or [ before the last one
                    let mut pass = text != ",";
                    while let Some(l) = elems.last() {
                        if l.starts_array || l.started_nesting_in_original {
                            if !pass {
                                break;
                            }
                            pass = false;
                        }
                        elems.pop();
                    }
                    has_content = false;
                    expecting_value = elems.last().is_some_and(|l| l.starts_array);
                    if text == "," {
                        line_breaks = (t + 1..=last)
                            .find(|j| !is_comment(doc, *j))
                            .map_or(0, |n| doc.lines[n]);
                    }
                }
                _ if expecting_value => {
                    let mut value = text.to_string();
                    // unchecked expressions #(...) are put on one line
                    if text == "#"
                        && let Some(open) = (t + 1..=last).find(|j| !is_comment(doc, *j))
                        && doc.syn.text(open) == "("
                        && let Some(close) = doc.syn.bracket_match[open]
                    {
                        while t < close {
                            t += 1;
                            let spaces = if doc.lines[t] > 0 { 1 } else { doc.spaces[t] };
                            value.push_str(&" ".repeat(spaces as usize));
                            value.push_str(doc.syn.text(t));
                        }
                    }
                    elems.push(Elem::new(value));
                    has_content = true;
                }
                _ => {
                    for part in text.split('.') {
                        let part = part.trim();
                        let part = part.strip_prefix('@').unwrap_or(part);
                        if !part.is_empty() {
                            elems.push(Elem::new(part.to_string()));
                        }
                    }
                    // a Boolean annotation may implicitly use its default value
                    has_content = true;
                }
            }
            t += 1;
        }
        if has_content {
            self.annos
                .push(create_anno(&elems, expecting_value, line_breaks));
        }
        Some(())
    }

    fn max_level_count(&self) -> usize {
        self.annos.iter().map(Anno::level_count).max().unwrap_or(0)
    }

    /// `useExistingNesting`: nesting is kept as in the original code.
    fn use_existing_nesting(&mut self) {
        for i in 0..self.annos.len() {
            for level in 0..self.annos[i].level_count() {
                self.check_nesting_for(i, level);
            }
        }
        for r in 0..self.ranges.len() {
            self.finish_range(r);
        }
    }

    fn check_nesting_for(&mut self, anno_index: usize, level: usize) {
        let anno = &self.annos[anno_index];
        // was nesting already planned when processing an earlier annotation?
        if anno.starts_nesting_after_level(level) {
            return;
        }
        let (array, anonymous, original) = (
            anno.has_array_start_at(level),
            anno.has_anonymous_object_at(level),
            anno.original_started_nesting_after_level(level),
        );
        if array {
            // array elements must always be 'nested'
            self.create_range(anno_index, level, true, true);
        } else if anonymous {
            // anonymous objects inside an array must always be 'nested'
            self.create_range(anno_index, level, true, false);
        } else if original {
            // nesting is enforced if (and only if) the original code used it here
            self.create_range(anno_index, level, false, false);
        }
    }

    /// creates a nesting range for all surrounding annotations with matching element names
    fn create_range(&mut self, anno_index: usize, level: usize, fill_up: bool, is_array: bool) {
        let can_include = |scope: &Scope, other: usize| {
            let o = &scope.annos[other];
            scope.annos[anno_index].level_of_first_diff(o) > level
                && (fill_up || o.original_started_nesting_after_level(level))
        };
        let mut start = anno_index;
        while start > 0 && can_include(self, start - 1) {
            start -= 1;
        }
        let mut end = anno_index + 1;
        while end < self.annos.len() && can_include(self, end) {
            end += 1;
        }
        let idx = self.ranges.len();
        self.ranges.push(NestingRange {
            nest_after_level: level,
            is_array,
            annos: (start..end).collect(),
            column_width: 0,
            max_inner_elem_depth: 0,
            max_inner_nesting_depth: 0,
            write_as_one_liner: false,
        });
        for a in start..end {
            self.annos[a].elems[level].nesting_range = Some(idx);
        }
    }

    /// `DdlAnnotationNestingRange.finishBuild`
    fn finish_range(&mut self, r: usize) {
        let (nest_after, is_array) = (self.ranges[r].nest_after_level, self.ranges[r].is_array);
        let nested = nest_after + 1;
        let (mut followed_by_dot, mut column_width, mut elem_depth, mut nesting_depth) =
            (false, 0, 0, 0);
        for &a in &self.ranges[r].annos {
            let anno = &self.annos[a];
            followed_by_dot |= anno.is_elem_followed_by_dot(nested);
            if !is_array {
                column_width = column_width.max(anno.elem_name(nested).chars().count());
            }
            elem_depth = elem_depth.max(anno.level_count().saturating_sub(nested));
            nesting_depth = nesting_depth.max(anno.inner_nesting_depth_after_level(nested));
        }
        let range = &mut self.ranges[r];
        range.column_width = if followed_by_dot { 0 } else { column_width };
        range.max_inner_elem_depth = elem_depth;
        range.max_inner_nesting_depth = nesting_depth;
    }

    fn belongs_to_main_annotations(&self, doc: &Doc) -> bool {
        self.commands
            .first()
            .is_none_or(|c| cmd_parent(doc, *c).is_none())
    }

    /// `determineOneLiners`
    fn determine_one_liners(&mut self, doc: &Doc, layout: &Layout) {
        let max_count = if self.belongs_to_main_annotations(doc) {
            layout.max_one_liner_elem_count_main
        } else {
            layout.max_one_liner_elem_count_list
        };
        if max_count < 2 {
            return;
        }
        let mut found = false;
        for r in &mut self.ranges {
            r.write_as_one_liner = r.max_inner_nesting_depth == 0 && r.annos.len() <= max_count;
            found |= r.write_as_one_liner;
        }
        if !found {
            return;
        }
        // simulate writing the annotations
        let mut writer = Writer::new(layout, self.max_level_count());
        self.write_to(&mut writer);
        for r in 0..self.ranges.len() {
            if !self.ranges[r].write_as_one_liner {
                continue;
            }
            let nested = self.ranges[r].nest_after_level + 1;
            let mut max_end = 0;
            for &a in &self.ranges[r].annos {
                let anno = &self.annos[a];
                max_end = max_end.max(anno.elems.get(nested).map_or(0, |e| e.end_index));
                max_end = max_end.max(anno.value_end_index);
            }
            if max_end > layout.max_line_length {
                self.ranges[r].write_as_one_liner = false;
            }
        }
    }

    /// `determineTablesInArrays`
    fn determine_tables_in_arrays(&mut self) {
        for r in 0..self.ranges.len() {
            let range = &self.ranges[r];
            if range.is_array
                && range.max_inner_elem_depth <= 2
                && range.max_inner_nesting_depth == 1
            {
                self.determine_table_in_array(r);
            }
        }
    }

    /// Checks whether 'tabular' alignment is possible inside an array of nested objects with the
    /// same elements, and stores the maximum value widths on the annotations.
    fn determine_table_in_array(&mut self, r: usize) -> bool {
        let level = self.ranges[r].nest_after_level + 1;
        let mut first_obj: Option<String> = None;
        let mut last_obj: Option<String> = None;
        let mut last_obj_start = 0;
        let mut column_names: Vec<String> = Vec::new();
        let mut max_widths: Vec<usize> = Vec::new();
        for (index, &a) in self.ranges[r].annos.iter().enumerate() {
            let anno = &self.annos[a];
            if !anno.has_anonymous_object_at(level) || !self.continue_on_same_line(anno, level + 1)
            {
                return false;
            }
            let Some(value) = &anno.value else {
                return false;
            };
            let obj = anno.elem_name(level).to_string();
            let column = anno.elem_name(level + 1).to_string();
            let width = value.chars().count();
            if first_obj.as_ref().is_none_or(|f| *f == obj) {
                first_obj = Some(obj.clone());
                last_obj = Some(obj);
                column_names.push(column);
                max_widths.push(width);
            } else {
                // later objects must have the same element names in the same sequence (but may
                // have fewer elements)
                if last_obj.as_ref().is_none_or(|l| *l != obj) {
                    last_obj = Some(obj);
                    last_obj_start = index;
                }
                let in_obj = index - last_obj_start;
                if in_obj >= column_names.len() || column != column_names[in_obj] {
                    return false;
                }
                max_widths[in_obj] = max_widths[in_obj].max(width);
            }
        }
        if column_names.len() < 2 {
            return false;
        }
        let mut last_obj: Option<String> = None;
        let mut in_obj = 0;
        let members = self.ranges[r].annos.clone();
        for a in members {
            let obj = self.annos[a].elem_name(level).to_string();
            if last_obj.as_ref().is_none_or(|l| *l != obj) {
                last_obj = Some(obj);
                in_obj = 0;
            } else {
                in_obj += 1;
            }
            self.annos[a].value_column_width = max_widths[in_obj];
        }
        true
    }

    fn continue_on_same_line(&self, anno: &Anno, nesting_start_level: usize) -> bool {
        nesting_start_level > 0
            && anno.elems.get(nesting_start_level - 1).is_some_and(|e| {
                e.nesting_range
                    .is_some_and(|r| self.ranges[r].write_as_one_liner)
            })
    }

    fn is_nested_with(&self, anno: usize, other: usize, level: usize) -> bool {
        let (a, o) = (&self.annos[anno], &self.annos[other]);
        a.elems
            .get(level)
            .is_some_and(|e| e.nesting_range.is_some())
            && o.elems
                .get(level)
                .is_some_and(|x| x.nesting_range == a.elems[level].nesting_range)
    }

    fn column_width(&self, anno: usize, level: usize) -> usize {
        if level == 0 {
            return 0;
        }
        match self.annos[anno].elems[level - 1].nesting_range {
            Some(r) if !self.ranges[r].write_as_one_liner => self.ranges[r].column_width,
            _ => 0,
        }
    }

    fn value_column_width(&self, anno: usize) -> usize {
        // only if the last element belongs to a one-liner nesting inside an array
        let a = &self.annos[anno];
        let Some(nested_after) = a.elems.len().checked_sub(2) else {
            return 0;
        };
        if a.starts_nesting_after_level(nested_after)
            && nested_after > 0
            && a.has_array_start_at(nested_after - 1)
        {
            match a.elems[nested_after].nesting_range {
                Some(r) if self.ranges[r].write_as_one_liner => a.value_column_width,
                _ => 0,
            }
        } else {
            0
        }
    }

    fn will_command_be_multi_line(&self, start: usize) -> bool {
        for i in start + 1..self.annos.len() {
            let first_diff = self.annos[i].level_of_first_diff(&self.annos[i - 1]);
            let mut nesting_start = first_diff;
            while nesting_start > 0 && !self.is_nested_with(i - 1, i, nesting_start - 1) {
                nesting_start -= 1;
            }
            if !self.annos[i - 1].starts_nesting_or_array_before(nesting_start) {
                return false;
            } else if !self.continue_on_same_line(&self.annos[i - 1], nesting_start) {
                return true;
            }
        }
        false
    }

    /// `writeTo`: writes the annotations with the determined nesting.
    fn write_to(&mut self, w: &mut Writer) {
        for i in 0..=self.annos.len() {
            self.write_anno(w, i);
        }
    }

    fn write_anno(&mut self, w: &mut Writer, i: usize) {
        let at_end = i == self.annos.len();
        let prev = (i > 0).then(|| i - 1);
        let mut nesting_start = 0;
        let mut starts_new_main = false;
        if let Some(p) = prev {
            let first_diff = if at_end {
                0
            } else {
                self.annos[i].level_of_first_diff(&self.annos[p])
            };
            starts_new_main = first_diff == 0;
            nesting_start = first_diff;
            while nesting_start > 0 && (at_end || !self.is_nested_with(p, i, nesting_start - 1)) {
                nesting_start -= 1;
            }
            // close the brackets and braces down to the nesting start level
            let prev_anno = &self.annos[p];
            let mut level = prev_anno.level_count() as i64 - 1;
            while level >= nesting_start as i64 {
                let l = level as usize;
                if prev_anno.has_array_start_at(l) {
                    w.close_bracket();
                } else if prev_anno.has_anonymous_object_at(l)
                    || prev_anno.starts_nesting_after_level(l)
                {
                    w.close_brace();
                }
                level -= 1;
            }
            // start a new line or even a new command
            if !self.annos[p].starts_nesting_or_array_before(nesting_start) {
                w.finish_command();
                nesting_start = 0;
            } else {
                let continue_on_line = self.continue_on_same_line(&self.annos[p], nesting_start);
                w.add_comma(nesting_start, continue_on_line);
            }
        }
        if at_end {
            return;
        }
        if nesting_start == 0 {
            let multi = self.will_command_be_multi_line(i);
            w.start_command(multi);
        }
        // add the required annotation levels
        let mut add_dot = false;
        w.start_annotation(self.annos[i].original_line_breaks);
        let _ = starts_new_main;
        for level in nesting_start..self.annos[i].level_count() {
            w.start_level(level);
            if !self.annos[i].has_anonymous_object_at(level) {
                if level == 0 {
                    w.add_annotation_sign();
                } else if add_dot {
                    w.add_dot();
                }
                let column_width = self.column_width(i, level);
                let name = self.annos[i].elem_name(level).to_string();
                let end = w.add_anno_elem_name(&name, column_width);
                self.annos[i].elems[level].end_index = end;
            }
            // open bracket or brace
            add_dot = false;
            if self.annos[i].starts_nesting_after_level(level) {
                if !self.annos[i].has_anonymous_object_at(level) {
                    w.add_colon();
                }
                if self.annos[i].has_array_start_at(level) {
                    w.open_bracket();
                } else {
                    w.open_brace();
                }
            } else {
                add_dot = true;
            }
        }
        // write the value
        if let Some(value) = self.annos[i].value.clone() {
            if !self.annos[i].is_value_in_array() {
                w.add_colon();
            }
            w.start_level(self.annos[i].level_count());
            let width = self.value_column_width(i);
            let end = w.add_value(&value, width);
            self.annos[i].value_end_index = end;
        }
    }
}

fn create_anno(elems: &[Elem], last_is_value: bool, line_breaks: u32) -> Anno {
    let count = if last_is_value {
        elems.len().saturating_sub(1)
    } else {
        elems.len()
    };
    Anno {
        elems: elems[..count].iter().map(Elem::copy_of).collect(),
        value: last_is_value.then(|| elems[count].name.clone()),
        original_line_breaks: line_breaks,
        value_end_index: 0,
        value_column_width: 0,
    }
}

// ---- writer ----

/// A token that was written: line breaks, spaces left and text.
type WrittenToken = (u32, u32, String);

struct Writer<'a> {
    layout: &'a Layout,
    start_pos_of_level: Vec<i64>,
    written_index_in_line: i64,
    is_first_command: bool,
    is_new_command: bool,
    last_command_was_multi_line: bool,
    is_in_command: bool,
    next_line_breaks: u32,
    next_spaces_left: i64,
    last_was_elem_name: bool,
    current: Vec<WrittenToken>,
    commands: Vec<Vec<WrittenToken>>,
}

impl<'a> Writer<'a> {
    fn new(layout: &'a Layout, max_level: usize) -> Writer<'a> {
        Writer {
            layout,
            start_pos_of_level: vec![0; max_level + 2],
            written_index_in_line: 0,
            is_first_command: true,
            is_new_command: false,
            last_command_was_multi_line: false,
            is_in_command: false,
            next_line_breaks: layout.initial_line_breaks,
            next_spaces_left: layout.basic_indent,
            last_was_elem_name: false,
            current: Vec::new(),
            commands: Vec::new(),
        }
    }

    fn start_command(&mut self, will_be_multi_line: bool) {
        self.is_new_command = true;
        self.last_command_was_multi_line = will_be_multi_line;
    }

    /// Empty lines are kept as in the original code (`DdlAnnotationEmptyLines.KEEP_AS_IS`).
    fn start_annotation(&mut self, original_line_breaks: u32) {
        if self.is_first_command {
            self.is_first_command = false;
        } else if self.is_new_command && original_line_breaks > 1 {
            self.next_line_breaks += 1;
        }
        self.is_new_command = false;
    }

    fn add_token_ex(
        &mut self,
        add_spaces_left: i64,
        text: &str,
        spaces_right: i64,
        is_elem_name: bool,
    ) -> i64 {
        let len = text.chars().count() as i64;
        if is_elem_name
            && self.last_was_elem_name
            && self.next_spaces_left == 0
            && add_spaces_left == 0
        {
            if let Some(last) = self.current.last_mut() {
                last.2.push_str(text);
            }
            self.written_index_in_line += len;
        } else {
            let spaces = (self.next_spaces_left + add_spaces_left).max(0) as u32;
            self.current
                .push((self.next_line_breaks, spaces, text.to_string()));
            self.is_in_command = true;
            if self.next_line_breaks > 0 {
                self.written_index_in_line = 0;
            }
            self.written_index_in_line += self.next_spaces_left + len;
        }
        self.next_line_breaks = 0;
        self.next_spaces_left = spaces_right;
        self.last_was_elem_name = is_elem_name || text == "@";
        self.written_index_in_line
    }

    fn add_token(&mut self, add_spaces_left: i64, text: &str, spaces_right: i64) -> i64 {
        self.add_token_ex(add_spaces_left, text, spaces_right, false)
    }

    fn open_bracket(&mut self) -> i64 {
        self.add_token(0, "[", self.layout.spaces_inside_brackets)
    }

    fn open_brace(&mut self) -> i64 {
        self.add_token(0, "{", self.layout.spaces_inside_braces)
    }

    fn add_annotation_sign(&mut self) -> i64 {
        self.add_token(0, "@", 0)
    }

    fn start_level(&mut self, level: usize) {
        // remember the current indentation for this level
        self.start_pos_of_level[level] = if self.is_in_command {
            if self.next_line_breaks > 0 {
                self.next_spaces_left
            } else {
                self.written_index_in_line + self.next_spaces_left
            }
        } else {
            0
        };
    }

    fn add_anno_elem_name(&mut self, name: &str, column_width: usize) -> i64 {
        let len = name.chars().count();
        let spaces_right = if self.layout.align_values && column_width > len {
            (column_width - len) as i64
        } else {
            0
        };
        self.add_token_ex(0, name, spaces_right, true)
    }

    fn add_dot(&mut self) -> i64 {
        self.add_token_ex(0, ".", 0, true)
    }

    fn add_colon(&mut self) -> i64 {
        // if next_spaces_left were set to align columns, attach the colon and move the extra
        // spaces behind it
        let spaces_right = self.next_spaces_left + self.layout.spaces_after_colon;
        self.next_spaces_left = 0;
        self.add_token(0, ":", spaces_right)
    }

    fn add_value(&mut self, value: &str, value_column_width: usize) -> i64 {
        let len = value.chars().count();
        let spaces_right = if self.layout.align_tables_in_arrays && value_column_width > len {
            (value_column_width - len) as i64
        } else {
            0
        };
        self.add_token(0, value, spaces_right)
    }

    fn add_comma(&mut self, new_level: usize, continue_on_line: bool) -> i64 {
        let spaces_right = if continue_on_line {
            self.next_spaces_left + 1
        } else {
            0
        };
        self.next_spaces_left = 0;
        let result = self.add_token(0, ",", spaces_right);
        if !continue_on_line {
            let indent = self.start_pos_of_level[new_level];
            self.add_line_feed(indent);
        }
        result
    }

    fn close_bracket(&mut self) -> i64 {
        self.add_token(self.layout.spaces_inside_brackets, "]", 0)
    }

    fn close_brace(&mut self) -> i64 {
        self.add_token(self.layout.spaces_inside_braces, "}", 0)
    }

    fn finish_command(&mut self) {
        if !self.current.is_empty() {
            self.commands.push(std::mem::take(&mut self.current));
        }
        self.is_in_command = false;
        self.last_was_elem_name = false;
        let indent = self.layout.basic_indent;
        self.add_line_feed(indent);
    }

    fn add_line_feed(&mut self, new_indent: i64) {
        self.next_line_breaks = 1;
        self.next_spaces_left = new_indent;
        self.last_was_elem_name = false;
    }
}

// ---- rule ----

pub fn layout(doc: &mut Doc, config: &RuleConfig) {
    let n = doc.syn.commands.len();
    let mut scope = Scope::default();
    let mut skip_section = false;
    let mut ci = 0;
    while ci < n {
        if is_comment_line(doc, ci) {
            // comment lines are skipped
        } else if !is_annotation_before_list_element(doc, ci) {
            // process this annotation section, then start a new section
            if !skip_section && !scope.annos.is_empty() {
                execute_on_scope(doc, config, &mut scope);
            }
            scope = Scope::default();
            skip_section = false;
        } else if !skip_section && scope.add(doc, ci).is_none() {
            // this scope cannot be processed, e.g. due to a comment within an annotation command
            skip_section = true;
        }
        ci += 1;
    }
    if !skip_section && !scope.annos.is_empty() {
        execute_on_scope(doc, config, &mut scope);
    }
}

fn is_annotation_before_list_element(doc: &Doc, ci: usize) -> bool {
    is_annotation(doc, ci)
        && !first_code(doc, ci).is_some_and(|f| doc.syn.text(f).starts_with("@<"))
}

fn execute_on_scope(doc: &mut Doc, config: &RuleConfig, scope: &mut Scope) {
    let first_token = doc.syn.commands[scope.commands[0]].first;
    let layout = Layout {
        spaces_after_colon: i64::from(config.bool("SpaceAfterColon", true)),
        spaces_inside_braces: i64::from(config.bool("SpaceInsideBraces", true)),
        spaces_inside_brackets: i64::from(config.bool("SpaceInsideBrackets", true)),
        max_line_length: config.int("MaxLineLength", 120),
        max_one_liner_elem_count_main: config.int("MaxOneLinerElemCountMain", 4).max(0) as usize,
        max_one_liner_elem_count_list: config.int("MaxOneLinerElemCountList", 4).max(0) as usize,
        align_values: config.bool("AlignValues", false),
        align_tables_in_arrays: config.bool("AlignTablesInArrays", true),
        initial_line_breaks: doc.lines[first_token],
        basic_indent: i64::from(doc.spaces[first_token]),
    };
    scope.use_existing_nesting();
    scope.determine_one_liners(doc, &layout);
    scope.determine_tables_in_arrays();

    let mut writer = Writer::new(&layout, scope.max_level_count());
    scope.write_to(&mut writer);
    transfer_whitespace(doc, scope, &writer.commands);
}

/// Takes over the whitespace of the re-created commands, provided they match the existing
/// commands token for token (ignoring whitespace); otherwise nothing is changed.
fn transfer_whitespace(doc: &mut Doc, scope: &Scope, new_commands: &[Vec<WrittenToken>]) {
    if new_commands.len() != scope.commands.len() {
        return;
    }
    let mut changes: Vec<(usize, u32, u32)> = Vec::new();
    for (new, &ci) in new_commands.iter().zip(&scope.commands) {
        let (first, last) = (doc.syn.commands[ci].first, doc.syn.commands[ci].last);
        let mut t = first;
        for (lines, spaces, text) in new {
            // the new token may span several old tokens, e.g. "@Anno.sub" for "@ Anno. sub"
            if t > last {
                return;
            }
            changes.push((t, *lines, *spaces));
            let mut matched = String::new();
            while matched.len() < text.len() {
                if t > last || !text.starts_with(&format!("{matched}{}", doc.syn.text(t))) {
                    return;
                }
                matched.push_str(doc.syn.text(t));
                t += 1;
                if matched.len() < text.len() {
                    changes.push((t, 0, 0));
                }
            }
        }
        if t != last + 1 {
            return;
        }
    }
    for (t, lines, spaces) in changes {
        doc.set_lines(t, lines);
        doc.set_spaces(t, spaces);
    }
}
