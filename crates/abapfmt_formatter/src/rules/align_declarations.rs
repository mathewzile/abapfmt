//! ALIGN_DECLARATIONS: aligns chains and consecutive declaration lines of CONSTANTS, DATA,
//! FIELD-SYMBOLS and TYPES (port of `AlignDeclarationsRule`).
use abapfmt_syntax::{Command, TokenType};

use crate::align::{Cell, Table, end_x, is_comment};
use crate::{ConfigExt, Doc, RuleConfig};

const KEYWORD: usize = 0;
const IDENTIFIER: usize = 1;
const TYPE: usize = 2;
const LENGTH: usize = 3;
const DECIMALS: usize = 4;
const VALUE: usize = 5;
const READ_ONLY: usize = 6;
const LINE_END_COMMENT: usize = 7;
const MAX_COLUMN_COUNT: usize = 8;

const DECLARATION_KEYWORDS: &[&str] = &[
    "TYPES",
    "CONSTANTS",
    "DATA",
    "FIELD-SYMBOLS",
    "CLASS-DATA",
    "STATICS",
    "RANGES",
];

#[derive(Clone, Copy, PartialEq)]
#[allow(clippy::enum_variant_names)]
enum Action {
    NameTypeLengthEtc,
    NameAndType,
    NameOnly,
}

fn action(config: &RuleConfig, key: &str) -> Action {
    match config.choice(key, "ALIGN_NAME_TYPE_LENGTH_ETC").as_str() {
        "ALIGN_NAME_AND_TYPE" => Action::NameAndType,
        "ALIGN_NAME_ONLY" => Action::NameOnly,
        _ => Action::NameTypeLengthEtc,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum StructureStyle {
    PerLevel,
    AcrossLevels,
    PerSection,
}

#[derive(Clone, Copy, PartialEq)]
enum TableKind {
    Normal,
    Structure,
    Enum,
}

// ---- token navigation (siblings = tokens on the same bracket level) ----

pub(crate) fn eq(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.text(t).eq_ignore_ascii_case(s)
}

pub(crate) fn is_kw(doc: &Doc, t: usize, s: &str) -> bool {
    doc.syn.types[t] != TokenType::Literal && eq(doc, t, s)
}

pub(crate) fn is_any_kw(doc: &Doc, t: usize, set: &[&str]) -> bool {
    set.iter().any(|s| is_kw(doc, t, s))
}

pub(crate) fn is_comma_or_period(doc: &Doc, t: usize) -> bool {
    matches!(doc.syn.types[t], TokenType::Comma | TokenType::Period)
}

/// The next sibling: a token that opens a level is followed by its closing token.
pub(crate) fn next_sibling(doc: &Doc, t: usize, last: usize) -> Option<usize> {
    let n = match doc.syn.bracket_match[t] {
        Some(close) if close > t => close,
        _ => t + 1,
    };
    (n <= last).then_some(n)
}

pub(crate) fn next_non_comment(doc: &Doc, t: usize, last: usize) -> Option<usize> {
    (t + 1..=last).find(|j| !is_comment(doc, *j))
}

pub(crate) fn next_non_comment_sibling(doc: &Doc, t: usize, last: usize) -> Option<usize> {
    let mut n = next_sibling(doc, t, last)?;
    while is_comment(doc, n) {
        n = next_sibling(doc, n, last)?;
    }
    Some(n)
}

/// The last token of a term that starts with `t` (`Term.createSimple`): brackets are included.
pub(crate) fn term_last(doc: &Doc, t: usize) -> usize {
    let mut last = t;
    while let Some(close) = doc.syn.bracket_match[last].filter(|c| *c > last) {
        last = close;
    }
    last
}

/// `matchesOnSiblings(true, pats...)`; each pattern is a `|`-separated list.
pub(crate) fn matches_seq(doc: &Doc, t: usize, last: usize, pats: &[&str]) -> bool {
    let mut cur = Some(t);
    for p in pats {
        let Some(c) = cur else { return false };
        if !p
            .split('|')
            .any(|a| doc.syn.text(c).eq_ignore_ascii_case(a))
        {
            return false;
        }
        cur = next_non_comment_sibling(doc, c, last);
    }
    true
}

/// `getLastTokenOnSiblings(true, pats...)` at exactly `t`: the last token of the match.
pub(crate) fn match_seq_last(doc: &Doc, t: usize, last: usize, pats: &[&str]) -> Option<usize> {
    let mut cur = t;
    for (i, p) in pats.iter().enumerate() {
        let ok = if *p == "@ID" {
            matches!(
                doc.syn.types[cur],
                TokenType::Identifier | TokenType::Literal
            )
        } else {
            p.split('|')
                .any(|a| doc.syn.text(cur).eq_ignore_ascii_case(a))
        };
        if !ok {
            return None;
        }
        if i + 1 < pats.len() {
            cur = next_non_comment_sibling(doc, cur, last)?;
        }
    }
    Some(cur)
}

/// `getLastTokenOnSiblings(true, ASTERISK, pats...)`: searches forward from `t` for the sequence.
pub(crate) fn find_seq_last(doc: &Doc, t: usize, last: usize, pats: &[&str]) -> Option<usize> {
    let mut cur = Some(t);
    while let Some(c) = cur {
        if let Some(l) = match_seq_last(doc, c, last, pats) {
            return Some(l);
        }
        cur = next_sibling(doc, c, last);
    }
    None
}

pub(crate) fn cmd_index_of(cmds: &[Command], t: usize) -> usize {
    cmds.iter()
        .position(|c| (c.first..=c.last).contains(&t))
        .unwrap()
}

pub(crate) fn is_simple_chain(doc: &Doc, cmd: &Command) -> bool {
    let (Some(first), Some(colon)) = (doc.first_code_token(cmd), cmd.colon) else {
        return false;
    };
    doc.syn.types[first] == TokenType::Keyword
        && doc.next_code_token(first, cmd.last) == Some(colon)
}

pub(crate) fn skip_command(doc: &Doc, cmd: &Command) -> bool {
    cmd.is_comment_only
        || (doc.syn.types[cmd.first] == TokenType::Period
            && (cmd.first == cmd.last
                || (cmd.first + 1 == cmd.last
                    && doc.lines[cmd.last] == 0
                    && is_comment(doc, cmd.last))))
}

fn block_level_diff(doc: &Doc, cmd: &Command) -> i32 {
    let mut diff = 0;
    for t in cmd.first..cmd.last {
        if is_comment(doc, t) {
            continue;
        }
        let begin = is_kw(doc, t, "BEGIN");
        let end = is_kw(doc, t, "END");
        if (begin || end) && next_non_comment(doc, t, cmd.last).is_some_and(|n| is_kw(doc, n, "OF"))
        {
            diff += if begin { 1 } else { -1 };
        }
    }
    diff
}

// ---- table set ----

struct TableSet {
    style: StructureStyle,
    additional_indent: i64,
    tables: Vec<Table>,
    kinds: Vec<TableKind>,
    stack: Vec<usize>,
}

impl TableSet {
    fn new(style: StructureStyle) -> TableSet {
        let mut s = TableSet {
            style,
            additional_indent: 0,
            tables: Vec::new(),
            kinds: Vec::new(),
            stack: Vec::new(),
        };
        s.add_table(TableKind::Normal);
        s
    }

    fn add_table(&mut self, kind: TableKind) {
        self.tables.push(Table::new(MAX_COLUMN_COUNT));
        self.kinds.push(kind);
        self.stack.push(self.tables.len() - 1);
    }

    fn current(&mut self) -> usize {
        if self.stack.is_empty() {
            self.add_table(TableKind::Normal);
        }
        *self.stack.last().unwrap()
    }

    fn begin_of_structure(&mut self, is_enum: bool) {
        if self.style == StructureStyle::AcrossLevels && self.additional_indent > 0 {
            // continue with the table started at the top-level BEGIN OF
        } else {
            self.add_table(if is_enum {
                TableKind::Enum
            } else {
                TableKind::Structure
            });
        }
        self.additional_indent += 2;
    }

    fn end_of_structure(&mut self) {
        if self.additional_indent > 0 {
            self.additional_indent -= 2;
        }
        if !self.stack.is_empty()
            && !(self.style == StructureStyle::AcrossLevels && self.additional_indent > 0)
        {
            self.stack.pop();
        }
        if self.style == StructureStyle::PerSection || self.additional_indent == 0 {
            // the table from the previous level is replaced with a new one
            self.stack.pop();
            self.add_table(if self.additional_indent != 0 {
                TableKind::Structure
            } else {
                TableKind::Normal
            });
        }
    }
}

struct TableStart {
    first_line_breaks: u32,
    keyword_indent: i64,
    basic_identifier_indent: i64,
}

// ---- the rule ----

pub fn align_declarations(doc: &mut Doc, config: &RuleConfig) {
    let on_class_def = config.bool("ExecuteOnClassDefAndInterfaces", true);
    for_each_section(
        doc,
        config,
        |doc, cmd| {
            !cmd.is_comment_only
                && doc.is_declaration(cmd)
                && (on_class_def || !(cmd.in_class_definition || cmd.in_interface_definition))
        },
        is_match_for_further,
        |doc, cmds, start, end, chain| execute_on(doc, config, cmds, start, end, chain),
    );
}

/// `AlignDeclarationSectionRuleBase.executeOn`: finds sections of consecutive declaration
/// commands (including a chain) and calls `exec(doc, cmds, start, end, is_chain)` for each.
pub(crate) fn for_each_section(
    doc: &mut Doc,
    config: &RuleConfig,
    matches_first: impl Fn(&Doc, &Command) -> bool,
    matches_further: impl Fn(&Doc, &Command, &str) -> bool,
    mut exec: impl FnMut(&mut Doc, &[Command], usize, usize, bool),
) {
    let across_empty = config.bool("AlignAcrossEmptyLines", true);
    let across_comments = config.bool("AlignAcrossCommentLines", true);

    let cmds = doc.commands().to_vec();
    let mut ci = 0;
    while ci < cmds.len() {
        let cmd = &cmds[ci];
        if !matches_first(doc, cmd) {
            ci += 1;
            continue;
        }
        let start = ci;
        let keyword = doc.first_code_text(cmd).unwrap();
        let start_is_chain = is_simple_chain(doc, cmd);
        let start_opens_block = block_level_diff(doc, cmd) > 0;
        let mut end = ci + 1;
        if !(start_is_chain && !start_opens_block) {
            let mut block_level = block_level_diff(doc, cmd);
            loop {
                if end >= cmds.len() {
                    break;
                }
                let c = &cmds[end];
                let diff = block_level_diff(doc, c);
                if block_level == 0 && diff > 0 {
                    break;
                }
                block_level += diff;
                let mut belongs_to_block = false;
                if block_level > 0 {
                    belongs_to_block = true;
                } else if block_level == 0 && diff < 0 {
                    end += 1;
                    break;
                } else {
                    if !across_empty && doc.lines[c.first] > 1 {
                        break;
                    }
                    if !across_comments && c.is_comment_only {
                        break;
                    }
                }
                let continues =
                    skip_command(doc, c) || belongs_to_block || matches_further(doc, c, &keyword);
                if !continues {
                    break;
                }
                end += 1;
            }
        }
        exec(doc, &cmds, start, end, start_is_chain);
        ci = end;
    }
}

fn is_match_for_further(doc: &Doc, cmd: &Command, keyword: &str) -> bool {
    if doc.first_code_text(cmd).as_deref() == Some(keyword) {
        !is_simple_chain(doc, cmd)
    } else {
        keyword == "TYPES" && doc.is_declaration_include(cmd)
    }
}

fn execute_on(
    doc: &mut Doc,
    config: &RuleConfig,
    cmds: &[Command],
    start: usize,
    end: usize,
    start_is_chain: bool,
) {
    let include_keyword = include_keyword_in_table(doc, cmds, start, end);
    let ts = determine_table_start(doc, cmds, start, end, include_keyword);
    let style = match config.choice("StructureAlignStyle", "PER_LEVEL").as_str() {
        "ACROSS_LEVELS" => StructureStyle::AcrossLevels,
        "PER_SECTION" => StructureStyle::PerSection,
        _ => StructureStyle::PerLevel,
    };
    let Some(mut set) = build_table(doc, cmds, start, end, include_keyword, style) else {
        return;
    };

    let chain_action = action(config, "AlignChainAction");
    let non_chain_action = action(config, "AlignNonChainsAction");
    let structure_action = action(config, "AlignStructureAction");
    let enum_name_only =
        config.choice("AlignEnumAction", "ALIGN_NAME_AND_VALUE") == "ALIGN_NAME_ONLY";
    let fill_ratio = config.int("FillPercentageToJustifyOwnColumn", 20) as f64 / 100.0;
    let condense = config.bool("CondenseInnerSpaces", true);
    let max_line_length = config.int("MaxLineLength", 120);

    for ti in 0..set.tables.len() {
        let kind = set.kinds[ti];
        let act = match kind {
            TableKind::Structure => structure_action,
            TableKind::Enum if enum_name_only => Action::NameOnly,
            TableKind::Enum => Action::NameTypeLengthEtc,
            TableKind::Normal if start_is_chain => chain_action,
            TableKind::Normal => non_chain_action,
        };
        join_columns(
            doc,
            &mut set.tables[ti],
            act,
            kind == TableKind::Enum,
            fill_ratio,
            condense,
        );
        if kind == TableKind::Enum {
            keep_enum_one_liners(doc, &mut set.tables[ti]);
        }
    }

    let mut first_line_breaks = ts.first_line_breaks;
    for table in &mut set.tables {
        if table.lines.is_empty() {
            continue;
        }
        if table.column_is_empty(READ_ONLY) && table.column_is_empty(LINE_END_COMMENT) {
            table.set_max_line_length(max_line_length);
        }
        let basic_indent = if table.column_is_empty(KEYWORD) {
            ts.basic_identifier_indent
        } else {
            ts.keyword_indent
        };
        if include_keyword {
            table.set_minimum_width(KEYWORD, ts.basic_identifier_indent - ts.keyword_indent - 1);
            for line in 0..table.lines.len() {
                if let Some(cell) = &table.lines[line].cells[KEYWORD] {
                    let cmd_last = cmds[cmd_index_of(cmds, cell.last)].last;
                    if let Some(next) = (cell.last < cmd_last).then_some(cell.last + 1) {
                        let asterisk = is_comment(doc, next) && doc.syn.text(next).starts_with('*');
                        if !asterisk && doc.lines[next] > 0 {
                            doc.lines[next] = 0;
                            doc.spaces[next] = 1;
                        }
                    }
                }
            }
        }
        table.align(doc, basic_indent, first_line_breaks, false, condense);
        first_line_breaks = 1;
    }
    align_generic_type_components(doc, config, cmds, start, end, structure_action);
    align_inner_comment_lines(doc, cmds, start, end);
}

fn keep_enum_one_liners(doc: &Doc, table: &mut Table) {
    if !table.column_is_empty(VALUE) {
        return;
    }
    let mut first_line = None;
    for (i, line) in table.lines.iter().enumerate() {
        if let Some(ident) = &line.cells[IDENTIFIER] {
            if first_line.is_none() {
                first_line = Some(i);
            } else if doc.lines[ident.first] > 0 {
                // at least one more identifier starts a new line
                return;
            }
        }
    }
    for i in (0..table.lines.len()).rev() {
        if table.lines[i].cells[IDENTIFIER].is_some() && Some(i) != first_line {
            table.remove_line_at(i);
        }
    }
}

fn include_keyword_in_table(doc: &Doc, cmds: &[Command], start: usize, end: usize) -> bool {
    let mut has_any_chain = false;
    let mut continues_behind_keyword = false;
    let keyword = doc.first_code_text(&cmds[start]).unwrap();
    for cmd in &cmds[start..end] {
        if skip_command(doc, cmd) || doc.first_code_text(cmd).as_deref() != Some(keyword.as_str()) {
            continue;
        }
        let first = doc.first_code_token(cmd).unwrap();
        let Some(mut ident) = next_non_comment_sibling(doc, first, cmd.last) else {
            continue;
        };
        if doc.syn.types[ident] == TokenType::Colon {
            if let Some(n) = next_non_comment_sibling(doc, ident, cmd.last) {
                ident = n;
            }
            has_any_chain = true;
        }
        if doc.lines[ident] == 0 {
            continues_behind_keyword = true;
        }
    }
    continues_behind_keyword || !has_any_chain
}

fn determine_table_start(
    doc: &Doc,
    cmds: &[Command],
    start: usize,
    end: usize,
    include_keyword: bool,
) -> TableStart {
    let first = cmds[start].first;
    let keyword_indent = doc.start_x(0, first);
    if include_keyword {
        let mut basic = end_x(doc, first) + 1;
        let keyword = doc.syn.text(first);
        for cmd in &cmds[start..end] {
            if doc.syn.text(cmd.first).eq_ignore_ascii_case(keyword)
                && is_simple_chain(doc, cmd)
                && doc.first_in_line(cmd.first)
                && let Some(colon) = next_non_comment_sibling(doc, cmd.first, cmd.last)
                && doc.syn.types[colon] == TokenType::Colon
                && doc.lines[colon] == 0
            {
                basic = basic.max(end_x(doc, colon) + 1);
            }
        }
        TableStart {
            first_line_breaks: doc.lines[first],
            keyword_indent,
            basic_identifier_indent: basic,
        }
    } else if is_simple_chain(doc, &cmds[start]) {
        let colon = next_non_comment(doc, first, cmds[start].last).unwrap();
        let offset = next_non_comment(doc, colon, cmds[start].last).unwrap();
        let lb = doc.lines[offset];
        TableStart {
            first_line_breaks: lb,
            keyword_indent,
            basic_identifier_indent: if lb > 0 {
                keyword_indent + 2
            } else {
                end_x(doc, colon) + 1
            },
        }
    } else {
        let offset = next_non_comment(doc, first, cmds[start].last).unwrap();
        TableStart {
            first_line_breaks: doc.lines[offset],
            keyword_indent,
            basic_identifier_indent: doc.start_x(0, offset),
        }
    }
}

fn end_of_structure_reached(doc: &Doc, t: usize, first: usize, last: usize) -> bool {
    let mut test = t;
    if test == first && is_any_kw(doc, test, DECLARATION_KEYWORDS) {
        let Some(n) = next_non_comment_sibling(doc, test, last) else {
            return false;
        };
        test = n;
        if doc.syn.types[test] == TokenType::Colon {
            let Some(n) = next_non_comment_sibling(doc, test, last) else {
                return false;
            };
            test = n;
        }
    }
    matches_seq(doc, test, last, &["END", "OF"])
}

fn build_table(
    doc: &Doc,
    cmds: &[Command],
    start: usize,
    end: usize,
    include_keyword: bool,
    style: StructureStyle,
) -> Option<TableSet> {
    let mut set = TableSet::new(style);
    let mut ci = start;
    let mut token = cmds[ci].first;
    loop {
        let cmd = &cmds[ci];
        let chain = is_simple_chain(doc, cmd);
        if chain {
            // skip comment lines between chain elements
            while token <= cmd.last
                && is_comment(doc, token)
                && doc.lines[token] > 0
                && (token == cmd.last || doc.lines[token + 1] > 0)
            {
                token += 1;
            }
            if token > cmd.last {
                break;
            }
        } else {
            while ci < end && skip_command(doc, &cmds[ci]) {
                ci += 1;
            }
            if ci >= end {
                break;
            }
            token = cmds[ci].first;
        }
        let cmd = &cmds[ci];
        if end_of_structure_reached(doc, token, cmd.first, cmd.last) {
            set.end_of_structure();
        }
        let table_index = set.current();
        let line = set.tables[table_index].add_line();

        if token == cmd.first && is_any_kw(doc, token, DECLARATION_KEYWORDS) {
            token = read_declaration_keyword(
                doc,
                cmds,
                &mut set.tables[table_index],
                line,
                token,
                include_keyword,
            );
        }
        if matches_seq(doc, token, cmd.last, &["BEGIN|END", "OF"])
            || matches_seq(doc, token, cmd.last, &["INCLUDE", "TYPE|STRUCTURE"])
        {
            token = read_structure_declaration(doc, cmd, &mut set, table_index, line, token)?;
        } else {
            token = read_declaration_line(
                doc,
                cmd,
                &mut set.tables[table_index],
                line,
                token,
                set.additional_indent,
            )?;
        }
        // comment at line end
        if token < cmd.last && is_comment(doc, token + 1) && doc.lines[token + 1] == 0 {
            token += 1;
            let cell = Cell::new(doc, token, token);
            set.tables[table_index].set_cell(line, LINE_END_COMMENT, cell);
        }
        token += 1;
        if chain && token <= cmd.last {
            continue;
        }
        ci += 1;
        if ci >= end {
            break;
        }
        // comment lines inside a command cannot be aligned across
        let c = &cmds[ci];
        if contains_inner_comment_line(doc, c) {
            return None;
        }
        token = c.first;
    }
    Some(set)
}

/// `Command.containsInnerCommentLine`: a comment line that is neither at the start nor the end
/// of the command, nor directly after `.`, `,` or `:`.
fn contains_inner_comment_line(doc: &Doc, cmd: &Command) -> bool {
    let n = doc.syn.tokens.len();
    let mut at_start_or_end = true;
    for t in cmd.first..=cmd.last {
        let is_line =
            is_comment(doc, t) && doc.first_in_line(t) && (t + 1 >= n || doc.lines[t + 1] > 0);
        if is_line {
            if !at_start_or_end {
                return true;
            }
        } else {
            at_start_or_end = matches!(doc.syn.text(t), "." | "," | ":");
        }
    }
    false
}

fn read_declaration_keyword(
    doc: &Doc,
    cmds: &[Command],
    table: &mut Table,
    line: usize,
    mut token: usize,
    include_keyword: bool,
) -> usize {
    let ci = cmd_index_of(cmds, token);
    let cmd = &cmds[ci];
    let chain = is_simple_chain(doc, cmd);
    let types_at_end_of_include_line = include_keyword
        && doc.lines[token] == 0
        && eq(doc, token, "TYPES")
        && ci > 0
        && doc.first_code_text(&cmds[ci - 1]).as_deref() == Some("INCLUDE");
    if include_keyword && !types_at_end_of_include_line {
        let cell = if chain {
            let colon = next_non_comment_sibling(doc, token, cmd.last).unwrap();
            Cell::new(doc, token, colon)
        } else {
            Cell::new(doc, token, token)
        };
        table.set_cell(line, KEYWORD, cell);
    }
    if chain {
        token = next_non_comment(doc, token, cmd.last).unwrap();
    }
    next_non_comment(doc, token, cmd.last).unwrap_or(cmd.last)
}

fn read_structure_declaration(
    doc: &Doc,
    cmd: &Command,
    set: &mut TableSet,
    table_index: usize,
    line: usize,
    token: usize,
) -> Option<usize> {
    let cell = Cell::special(doc, token, token, set.additional_indent, true);
    set.tables[table_index].set_cell(line, IDENTIFIER, cell);
    if matches_seq(doc, token, cmd.last, &["BEGIN", "OF"]) {
        let is_enum = matches_seq(doc, token, cmd.last, &["BEGIN", "OF", "ENUM"]);
        set.begin_of_structure(is_enum);
    }
    // skip the rest of the line, including a line-end comment
    let mut t = find_seq_last(doc, token, cmd.last, &[",|."])?;
    if t < cmd.last && is_comment(doc, t + 1) && doc.lines[t + 1] == 0 {
        t += 1;
    }
    Some(t)
}

fn read_declaration_line(
    doc: &Doc,
    cmd: &Command,
    table: &mut Table,
    line: usize,
    token: usize,
    additional_indent: i64,
) -> Option<usize> {
    let last = cmd.last;
    // identifier; may have the form "var(len)"
    let ident_first = token;
    let ident_last = term_last(doc, token);
    table.set_cell(
        line,
        IDENTIFIER,
        Cell::special(doc, token, ident_last, additional_indent, false),
    );
    let mut token = next_sibling(doc, ident_last, last)?;
    if is_comma_or_period(doc, token) {
        return Some(token);
    }
    if !is_any_kw(doc, token, &["TYPE", "LIKE", "VALUE"]) {
        return None;
    }
    let type_start = token;
    let mut type_end = token;
    let mut is_table = false;
    let mut skip_comment_at_line_end = false;
    while !is_any_kw(doc, type_end, &["LENGTH", "DECIMALS", "VALUE", "READ-ONLY"])
        && !(doc.syn.text(type_end) == "." || doc.syn.text(type_end) == ",")
    {
        if is_kw(doc, type_end, "TABLE") {
            is_table = true;
        }
        if is_kw(doc, type_end, "ASSOCIATION")
            || is_kw(doc, type_end, "WITH")
                && !matches_seq(doc, type_end, last, &["WITH", "EMPTY", "KEY"])
                && !matches_seq(doc, type_end, last, &["WITH", "DEFAULT", "KEY"])
                && !matches_seq(
                    doc,
                    type_end,
                    last,
                    &["WITH", "UNIQUE|NON-UNIQUE", "DEFAULT", "KEY"],
                )
                && !short_key(doc, type_end, last)
            || is_kw(doc, type_end, "CONTAINING")
        {
            // only align up to (but excluding) WITH / ASSOCIATION / CONTAINING
            let type_last = type_end - 1;
            table.set_cell(
                line,
                TYPE,
                Cell::special(doc, type_start, type_last, 0, true),
            );
            return find_seq_last(doc, type_end, last, &[",|."]);
        }
        type_end = next_sibling(doc, type_end, last)?;
    }
    if type_end != type_start {
        let mut type_last = type_end - 1;
        while is_comment(doc, type_last) {
            type_last -= 1;
        }
        table.set_cell(
            line,
            TYPE,
            Cell::special(doc, type_start, type_last, 0, is_table),
        );
        if is_table {
            skip_comment_at_line_end = true;
        }
    }
    token = type_end;
    if !is_comma_or_period(doc, token) {
        if let Some(l) = match_seq_last(doc, token, last, &["LENGTH", "@ID"]) {
            table.set_cell(line, LENGTH, Cell::new(doc, token, l));
            token = l + 1;
        }
        if let Some(l) = match_seq_last(doc, token, last, &["DECIMALS", "@ID"]) {
            table.set_cell(line, DECIMALS, Cell::new(doc, token, l));
            token = l + 1;
        }
        if is_kw(doc, token, "VALUE") {
            let value_start = token;
            let value_last = match match_seq_last(doc, token, last, &["VALUE", "IS", "INITIAL"]) {
                Some(l) => l,
                None => term_last(doc, next_sibling(doc, token, last)?),
            };
            table.set_cell(line, VALUE, Cell::new(doc, token, value_last));
            table.lines[line].overlength = Some((value_start, type_start, Some(ident_first)));
            token = value_last + 1;
        }
        if token <= last && is_kw(doc, token, "READ-ONLY") {
            table.set_cell(line, READ_ONLY, Cell::new(doc, token, token));
            token += 1;
        }
        // . or , (may be preceded by a pragma)
        token = find_seq_last(doc, token, last, &[",|."])?;
    }
    if skip_comment_at_line_end
        && token < last
        && is_comment(doc, token + 1)
        && doc.lines[token + 1] == 0
    {
        token += 1;
    }
    Some(token)
}

/// `WITH [UNIQUE|NON-UNIQUE] KEY comp [, ...]` short form: `KEY identifier` followed by `,` or `.`
fn short_key(doc: &Doc, t: usize, last: usize) -> bool {
    let mut cur = next_non_comment_sibling(doc, t, last);
    if cur.is_some_and(|c| is_any_kw(doc, c, &["UNIQUE", "NON-UNIQUE"])) {
        cur = cur.and_then(|c| next_non_comment_sibling(doc, c, last));
    }
    let Some(k) = cur.filter(|c| is_kw(doc, *c, "KEY")) else {
        return false;
    };
    let Some(ident) = next_non_comment_sibling(doc, k, last) else {
        return false;
    };
    doc.syn.types[ident] == TokenType::Identifier
        && next_non_comment_sibling(doc, ident, last).is_some_and(|n| is_comma_or_period(doc, n))
}

fn join_columns(
    doc: &mut Doc,
    table: &mut Table,
    act: Action,
    is_enum: bool,
    fill_ratio: f64,
    condense: bool,
) {
    let before: Vec<usize> = (0..=LINE_END_COMMENT)
        .map(|c| table.column_count(c))
        .collect();
    for i in (TYPE..=LINE_END_COMMENT).rev() {
        if table.column_is_empty(i) {
            continue;
        }
        let join = if act == Action::NameOnly {
            true
        } else if is_enum {
            false
        } else if act == Action::NameAndType {
            i != TYPE
        } else {
            before[i] <= 1 || (before[i] as f64) < (table.lines.len() as f64 * fill_ratio).floor()
        };
        if join {
            if i == LINE_END_COMMENT {
                table.remove_from_table(doc, i, condense);
            } else {
                table.join_into_previous_columns(doc, i, condense);
            }
        }
    }
    if !table.column_is_empty(LENGTH) && table.is_previous_column_fixed_width(doc, LENGTH) {
        table.join_into_previous_columns(doc, LENGTH, condense);
    }
}

fn prev_code_sibling(doc: &Doc, t: usize, first: usize) -> Option<usize> {
    // siblings are only walked backwards over closed groups when there are any; components of
    // generic types are plain tokens
    (first..t).rev().find(|j| !is_comment(doc, *j))
}

/// `finishLastLine`: the TYPE cell of the last component reaches up to before `end_token`.
fn finish_last_line(doc: &Doc, table: &mut Table, cmd_first: usize, end_token: usize, last: usize) {
    let Some(line) = table.lines.len().checked_sub(1) else {
        return;
    };
    let name = table.lines[line].cells[0].as_ref().unwrap().first;
    let type_token = next_non_comment_sibling(doc, name, last);
    let last_token = prev_code_sibling(doc, end_token, cmd_first);
    match (type_token, last_token) {
        (Some(a), Some(b)) if a <= b => table.set_cell(line, 1, Cell::new(doc, a, b)),
        _ => table.remove_line_at(line),
    }
}

/// Aligns the components of `TYPE ANY STRUCTURE CONTAINING ...`.
fn align_generic_type_components(
    doc: &mut Doc,
    config: &RuleConfig,
    cmds: &[Command],
    start: usize,
    end: usize,
    structure_action: Action,
) {
    let position = config.choice("ContainingPosition", "BELOW_KEYWORD_PLUS_2");
    for cmd in &cmds[start..end] {
        if cmd.is_comment_only || doc.first_code_text(cmd).as_deref() != Some("TYPES") {
            continue;
        }
        let keyword = doc.first_code_token(cmd).unwrap();
        let last = cmd.last;
        let mut search_from = Some(keyword);
        while let Some(from) = search_from {
            let Some(containing) =
                find_seq_last(doc, from, last, &["TYPE", "ANY", "STRUCTURE", "CONTAINING"])
            else {
                break;
            };
            let any_structure = prev_code_sibling(doc, containing, cmd.first).unwrap();
            let structure = any_structure;
            let any = prev_code_sibling(doc, structure, cmd.first).unwrap();
            let type_token = prev_code_sibling(doc, any, cmd.first).unwrap();
            let type_name = prev_code_sibling(doc, type_token, cmd.first);

            let mut table = Table::new(2);
            let mut prev = containing;
            let mut token = containing;
            let mut end_found = None;
            while let Some(n) = next_non_comment_sibling(doc, token, last) {
                token = n;
                if is_comma_or_period(doc, token) {
                    finish_last_line(doc, &mut table, cmd.first, token, last);
                    end_found = Some(token);
                    break;
                } else if is_kw(doc, token, "TYPE") {
                    finish_last_line(doc, &mut table, cmd.first, prev, last);
                    let line = table.add_line();
                    let cell = Cell::new(doc, prev, prev);
                    table.set_cell(line, 0, cell);
                }
                prev = token;
            }
            if !table.lines.is_empty() {
                if position != "UNCHANGED" {
                    let (mut spaces, mut lines) = (1i64, 0u32);
                    match position.as_str() {
                        "BELOW_KEYWORD_PLUS_2" => {
                            spaces = doc.start_x(0, keyword) + 2;
                            lines = 1;
                        }
                        "BELOW_IDENTIFIER_PLUS_2" => {
                            spaces = type_name.map_or(0, |n| doc.start_x(0, n)) + 2;
                            lines = 1;
                        }
                        "BELOW_TYPE" => {
                            spaces = doc.start_x(0, type_token);
                            lines = 1;
                        }
                        "BELOW_ANY_STRUCTURE" => {
                            spaces = doc.start_x(0, any);
                            lines = 1;
                        }
                        _ => {}
                    }
                    if lines == 0 && is_comment(doc, containing - 1) {
                        let code = prev_code_sibling(doc, containing, cmd.first).unwrap();
                        spaces += end_x(doc, code);
                        doc.lines[containing] = 1;
                    } else {
                        doc.lines[containing] = lines;
                    }
                    doc.spaces[containing] = spaces.max(0) as u32;
                }
                if structure_action == Action::NameOnly {
                    table.join_into_previous_columns(doc, 1, true);
                }
                let (basic_indent, first_line_breaks) = if position == "CONTINUE_AND_BREAK" {
                    (type_name.map_or(0, |n| doc.start_x(0, n)) + 2, 1)
                } else {
                    (end_x(doc, containing) + 1, 0)
                };
                table.align(doc, basic_indent, first_line_breaks, false, true);
            }
            // TYPES: may contain several comma-separated generic definitions
            search_from = end_found.and_then(|e| next_sibling(doc, e, last));
        }
    }
}

/// Quote-mark comment lines inside the section are aligned with the next code.
fn align_inner_comment_lines(doc: &mut Doc, cmds: &[Command], start: usize, end: usize) {
    let mut block_level = 0;
    for ci in start..end {
        let cmd = &cmds[ci];
        block_level += block_level_diff(doc, cmd);
        for t in cmd.first..=cmd.last {
            let is_line = is_comment(doc, t)
                && doc.first_in_line(t)
                && (t == cmd.last || doc.lines[t + 1] > 0 || t + 1 >= doc.syn.tokens.len())
                && doc.syn.text(t).starts_with('"');
            if !is_line {
                continue;
            }
            if let Some(next_code) = (t + 1..doc.syn.tokens.len()).find(|j| !is_comment(doc, *j)) {
                let nc_cmd = &cmds[cmd_index_of(cmds, next_code)];
                let end_of = matches_seq(doc, next_code, nc_cmd.last, &["END", "OF"]);
                doc.spaces[t] = doc.spaces[next_code] + if end_of { 2 } else { 0 };
            } else if block_level > 0
                && let Some(c) = cmds[ci + 1..].iter().find(|c| !c.is_comment_only)
            {
                doc.spaces[t] = doc.spaces[c.first];
            }
        }
    }
}
