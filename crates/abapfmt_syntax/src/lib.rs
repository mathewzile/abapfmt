//! Formatter-side view of ABAP code: the lossless tokens of `abapfmt_lexer`, each with a final
//! [`TokenType`], grouped into [`Command`]s (statements / chains) with bracket pairs.
//!
//! KEYWORD vs IDENTIFIER is decided by the generated abaplint grammar (`abapfmt_grammar`); words
//! in statements the grammar cannot parse fall back to a keyword-list guess. Nothing here
//! rewrites tokens.
#![forbid(unsafe_code)]

mod blocks;
mod ddl;
mod ddl_tables;
mod operators;

pub use ddl::{DdlInfo, is_ddl_source, parse_ddl};

use abapfmt_grammar::{Class, ParsedStatement, is_grammar_word, parse_statements};
use abapfmt_lexer::{LexError, Token, TokenKind, lex};

pub use abapfmt_grammar::{LanguageVersion, Options};

/// abap-cleaner's `TokenType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    Comment,
    Pragma,
    Literal,
    AssignmentOp,
    ComparisonOp,
    Keyword,
    Identifier,
    Period,
    Comma,
    Colon,
    OtherOp,
    /// Inside `EXEC SQL ... ENDEXEC`; never touched by any rule.
    NonAbap,
}

/// How the type of a word was decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Not a word, the type comes from the lexer.
    Lexer,
    Grammar,
    /// The grammar could not parse the statement; guessed from the keyword list.
    Fallback,
}

/// A statement (possibly a chain `A: b, c.`) as a token range `first..=last` that ends with its
/// period, or a single comment line.
#[derive(Clone, Debug)]
pub struct Command {
    pub first: usize,
    pub last: usize,
    /// Index of the chain colon, if the command is a chain.
    pub colon: Option<usize>,
    /// Grammar statement name of the first chain element (e.g. `"ReadTable"`).
    pub statement: Option<&'static str>,
    /// Every chain element was matched by the grammar.
    pub fully_matched: bool,
    pub is_comment_only: bool,
    /// Nesting level of the command itself (ELSE/WHEN/ENDIF... sit at the level of their opener).
    pub level: u32,
    /// Opens an event block (`START-OF-SELECTION`, `AT SELECTION-SCREEN`, ...).
    pub starts_event_block: bool,
    /// Closes an event block (the next event, CLASS, FORM or MODULE).
    pub ends_event_block: bool,
    /// From `CLASS ... DEFINITION` to its `ENDCLASS` (inclusive).
    pub in_class_definition: bool,
    /// From `INTERFACE` to its `ENDINTERFACE` (inclusive).
    pub in_interface_definition: bool,
    /// Inside (or opening) a CLASS, INTERFACE or METHOD.
    pub in_oo_context: bool,
    /// Number of open blocks after this command (the level of the code that follows it).
    pub depth_after: u32,
    /// Indentation from the enclosing blocks, in spaces, without the indentation of the first
    /// command of the code (see `root_no_base`).
    pub indent_rel: u32,
    /// The outermost command of the block structure is a CLASS, INTERFACE or introductory
    /// statement, so that the indentation of the first command of the code is not added.
    pub root_no_base: bool,
}

impl Command {
    /// A command of a DDL source; none of the ABAP block information applies.
    pub(crate) fn plain(first: usize, last: usize, is_comment_only: bool) -> Command {
        Command {
            first,
            last,
            colon: None,
            statement: None,
            fully_matched: true,
            is_comment_only,
            level: 0,
            starts_event_block: false,
            ends_event_block: false,
            in_class_definition: false,
            in_interface_definition: false,
            in_oo_context: false,
            depth_after: 0,
            indent_rel: 0,
            root_no_base: false,
        }
    }
    fn is(&self, names: &[&str]) -> bool {
        self.statement.is_some_and(|s| names.contains(&s))
    }
    pub fn is_class_start(&self) -> bool {
        self.is(&["ClassDefinition", "ClassImplementation"])
    }
    pub fn is_class_end(&self) -> bool {
        self.is(&["EndClass"])
    }
    pub fn is_interface_start(&self) -> bool {
        self.is(&["Interface"])
    }
    pub fn is_interface_end(&self) -> bool {
        self.is(&["EndInterface"])
    }
    pub fn is_class_or_interface_start(&self) -> bool {
        self.is_class_start() || self.is_interface_start()
    }
    pub fn is_class_or_interface_end(&self) -> bool {
        self.is_class_end() || self.is_interface_end()
    }
    pub fn is_method_start(&self) -> bool {
        self.is(&["MethodImplementation"])
    }
    pub fn is_method_end(&self) -> bool {
        self.is(&["EndMethod"])
    }
    pub fn is_function_start(&self) -> bool {
        self.is(&["FunctionModule"])
    }
    pub fn is_method_function_form_or_event_block_start(&self) -> bool {
        self.is_method_function_or_form_start() || self.starts_event_block
    }
    pub fn is_method_function_form_or_event_block_end(&self) -> bool {
        self.is_method_function_or_form_end() || self.ends_event_block
    }
    pub fn is_method_function_or_form_start(&self) -> bool {
        self.is(&["MethodImplementation", "FunctionModule", "Form"])
    }
    pub fn is_method_function_or_form_end(&self) -> bool {
        self.is(&["EndMethod", "EndFunction", "EndForm"])
    }
}

pub struct Syntax<'a> {
    pub src: &'a str,
    pub tokens: Vec<Token>,
    pub types: Vec<TokenType>,
    pub sources: Vec<Source>,
    pub commands: Vec<Command>,
    /// For a bracket-opening token the index of its closing token and vice versa.
    pub bracket_match: Vec<Option<usize>>,
    /// Innermost opening bracket token that encloses a token.
    pub parent: Vec<Option<usize>>,
    /// Command hierarchy, for CDS DDL / DCL sources only.
    pub ddl: Option<DdlInfo>,
}

impl Syntax<'_> {
    pub fn text(&self, i: usize) -> &str {
        self.tokens[i].text(self.src)
    }
}

fn opens_level(text: &str, kind: TokenKind) -> bool {
    match kind {
        TokenKind::Word | TokenKind::OtherOp => text.ends_with(['(', '[']),
        TokenKind::Literal => text.ends_with('{'),
        _ => false,
    }
}

fn closes_level(text: &str, kind: TokenKind) -> bool {
    match kind {
        TokenKind::Word | TokenKind::OtherOp => text.starts_with([')', ']']),
        TokenKind::Literal => text.starts_with('}'),
        _ => false,
    }
}

/// Words like `condense(` are built-in functions, not the keyword.
fn is_function_form(text: &str) -> bool {
    [
        "condense(",
        "escape(",
        "insert(",
        "match(",
        "replace(",
        "translate(",
    ]
    .iter()
    .any(|f| text.eq_ignore_ascii_case(f))
}

/// abap-cleaner's keyword guess for words the grammar could not place.
fn fallback_is_keyword(text: &str) -> bool {
    if is_function_form(text) {
        return false;
    }
    let bits: Vec<&str> = text
        .split(['-', ':', '('])
        .filter(|b| !b.is_empty())
        .collect();
    !bits.is_empty() && bits.iter().all(|b| !b.contains('_') && is_grammar_word(b))
}

/// Parse `src`. `Err` only if the lexer cannot continue (an unterminated `name(20`).
pub fn parse(src: &str, opts: Options) -> Result<Syntax<'_>, LexError> {
    let tokens = lex(src)?;
    let n = tokens.len();
    let find = |offset: u32| -> Option<usize> {
        let i = tokens.partition_point(|t| t.end <= offset);
        (i < n && tokens[i].start <= offset).then_some(i)
    };

    let stmts: Vec<ParsedStatement> = parse_statements(src, opts);

    // evidence from the grammar per formatter token
    let (mut kw, mut id) = (vec![false; n], vec![false; n]);
    let mut cmd_matched: Vec<(usize, bool, Option<&'static str>)> = Vec::new(); // (last token, matched, name)
    for s in &stmts {
        for (k, (g, c)) in s.tokens.iter().zip(&s.classes).enumerate() {
            let Some(i) = find(g.start) else { continue };
            // CLEAR ... WITH NULL: the grammar reads NULL as a field, but it is a keyword
            if s.statement == Some("Clear")
                && g.text(src).eq_ignore_ascii_case("NULL")
                && k > 0
                && s.tokens[k - 1].text(src).eq_ignore_ascii_case("WITH")
            {
                kw[i] = true;
                continue;
            }
            match c {
                Some(Class::Keyword) => kw[i] = true,
                Some(Class::Identifier) => id[i] = true,
                _ => {}
            }
        }
        if let Some(i) = s.tokens.last().and_then(|g| find(g.start)) {
            cmd_matched.push((i, s.statement.is_some() || s.tokens.len() == 1, s.statement));
        }
    }

    // commands: runs of own-line comments, and code up to the period
    let mut commands: Vec<Command> = Vec::new();
    let mut i = 0;
    while i < n {
        let start = i;
        if tokens[i].kind == TokenKind::Comment && (i + 1 == n || tokens[i + 1].lines_before > 0) {
            // a comment on its own line: one command per comment line, as in abap-cleaner
            commands.push(Command {
                first: start,
                last: i,
                colon: None,
                statement: None,
                fully_matched: true,
                is_comment_only: true,
                level: 0,
                starts_event_block: false,
                ends_event_block: false,
                in_class_definition: false,
                in_interface_definition: false,
                in_oo_context: false,
                depth_after: 0,
                indent_rel: 0,
                root_no_base: false,
            });
            i += 1;
            continue;
        }
        let mut colon = None;
        while i < n {
            match tokens[i].kind {
                TokenKind::Colon if colon.is_none() => colon = Some(i),
                TokenKind::Period => break,
                _ => {}
            }
            i += 1;
        }
        let last = i.min(n - 1);
        // a same-line comment or pragma after the period belongs to this command
        let mut end = last;
        while end + 1 < n
            && matches!(tokens[end + 1].kind, TokenKind::Comment | TokenKind::Pragma)
            && tokens[end + 1].lines_before == 0
        {
            end += 1;
        }
        i = end + 1;
        commands.push(Command {
            first: start,
            last: end,
            colon,
            statement: None,
            fully_matched: true,
            is_comment_only: false,
            level: 0,
            starts_event_block: false,
            ends_event_block: false,
            in_class_definition: false,
            in_interface_definition: false,
            in_oo_context: false,
            depth_after: 0,
            indent_rel: 0,
            root_no_base: false,
        });
    }
    // attach grammar results to commands
    for (last_tok, matched, name) in &cmd_matched {
        if let Some(c) = commands
            .iter_mut()
            .find(|c| (c.first..=c.last).contains(last_tok))
        {
            c.fully_matched &= *matched;
            if c.statement.is_none() {
                c.statement = *name;
            }
        }
    }

    // commands the grammar could not parse still shape the block structure: guess from the keyword
    for c in commands
        .iter_mut()
        .filter(|c| c.statement.is_none() && !c.is_comment_only)
    {
        let first = tokens[c.first..=c.last]
            .iter()
            .find(|t| t.kind == TokenKind::Word);
        c.statement = first.and_then(|t| blocks::guess_statement(t.text(src)));
        if c.statement.is_none()
            && first.is_some_and(|t| t.text(src).eq_ignore_ascii_case("AUTHORITY-CHECK"))
        {
            let words: Vec<String> = tokens[c.first..=c.last]
                .iter()
                .filter(|t| t.kind == TokenKind::Word)
                .take(3)
                .map(|t| t.text(src).to_ascii_uppercase())
                .collect();
            c.statement = match (
                words.get(1).map(String::as_str),
                words.get(2).map(String::as_str),
            ) {
                (Some("DISABLE"), Some("BEGIN")) => Some("AuthorityCheckDisableBegin"),
                (Some("DISABLE"), Some("END")) => Some("AuthorityCheckDisableEnd"),
                _ => None,
            };
        }
    }
    reclassify_select(&mut commands, &tokens, src);
    blocks::assign_levels(&mut commands, &tokens, src);

    // final types
    let mut types = Vec::with_capacity(n);
    let mut sources = Vec::with_capacity(n);
    for (idx, t) in tokens.iter().enumerate() {
        let (ty, from) = match t.kind {
            TokenKind::Comment => (TokenType::Comment, Source::Lexer),
            TokenKind::Pragma => (TokenType::Pragma, Source::Lexer),
            TokenKind::Literal => (TokenType::Literal, Source::Lexer),
            TokenKind::AssignmentOp => (TokenType::AssignmentOp, Source::Lexer),
            // a textual operator (BETWEEN, IN, CO...) is a name if the grammar only sees a name
            TokenKind::ComparisonOp
                if id[idx] && !kw[idx] && t.text(src).starts_with(|c: char| c.is_alphabetic()) =>
            {
                (TokenType::Identifier, Source::Grammar)
            }
            TokenKind::ComparisonOp => (TokenType::ComparisonOp, Source::Lexer),
            TokenKind::Period => (TokenType::Period, Source::Lexer),
            TokenKind::Comma => (TokenType::Comma, Source::Lexer),
            TokenKind::Colon => (TokenType::Colon, Source::Lexer),
            TokenKind::OtherOp => (TokenType::OtherOp, Source::Lexer),
            TokenKind::Word => {
                if id[idx] {
                    (TokenType::Identifier, Source::Grammar)
                } else if kw[idx] {
                    (TokenType::Keyword, Source::Grammar)
                } else if fallback_is_keyword(t.text(src)) {
                    (TokenType::Keyword, Source::Fallback)
                } else {
                    (TokenType::Identifier, Source::Fallback)
                }
            }
        };
        types.push(ty);
        sources.push(from);
    }

    // native SQL between EXEC SQL and ENDEXEC is not ABAP
    let mut native = false;
    for idx in 0..n {
        if native {
            if tokens[idx].kind == TokenKind::Word
                && tokens[idx].text(src).eq_ignore_ascii_case("ENDEXEC")
            {
                native = false;
            } else if tokens[idx].kind != TokenKind::Comment {
                types[idx] = TokenType::NonAbap;
                sources[idx] = Source::Lexer;
            }
        } else if commands
            .iter()
            .any(|c| c.last == idx && c.statement == Some("ExecSQL"))
        {
            native = true;
        }
    }

    // brackets
    let mut bracket_match = vec![None; n];
    let mut parent = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    for (idx, t) in tokens.iter().enumerate() {
        let text = t.text(src);
        if closes_level(text, t.kind) && !stack.is_empty() {
            let open = stack.pop().unwrap();
            bracket_match[open] = Some(idx);
            bracket_match[idx] = Some(open);
        }
        parent[idx] = stack.last().copied();
        if opens_level(text, t.kind) {
            stack.push(idx);
        }
        if t.kind == TokenKind::Period {
            stack.clear();
        }
    }

    // a word that is assigned to inside brackets, as in `replace_regexpr( value = text )`, is a name
    for idx in 0..n.saturating_sub(1) {
        if types[idx] == TokenType::Keyword
            && parent[idx].is_some()
            && types[idx + 1] == TokenType::AssignmentOp
            && tokens[idx + 1].text(src) == "="
        {
            types[idx] = TokenType::Identifier;
            sources[idx] = Source::Fallback;
        }
    }
    operators::distinguish_operators(src, &tokens, &commands, &bracket_match, &parent, &mut types);

    Ok(Syntax {
        src,
        tokens,
        types,
        sources,
        commands,
        bracket_match,
        parent,
        ddl: None,
    })
}

/// Does the select list contain a field that is not an aggregate (nor an alias)?
fn has_plain_field(top: &[String], aggregates: &[&str]) -> bool {
    const LIST_END: &[&str] = &[
        "FROM",
        "INTO",
        "APPENDING",
        "WHERE",
        "GROUP",
        "HAVING",
        "ORDER",
        "UP",
        "FOR",
        "UNION",
        "INTERSECT",
        "EXCEPT",
    ];
    // the FIELDS clause of the new syntax (not CORRESPONDING FIELDS OF)
    let start = (0..top.len())
        .find(|&p| top[p] == "FIELDS" && p > 0 && top[p - 1] != "CORRESPONDING")
        .map_or(1, |p| p + 1);
    let mut prev = "";
    for t in &top[start.min(top.len())..] {
        if LIST_END.contains(&t.as_str()) {
            break;
        }
        let skip = matches!(
            t.as_str(),
            "," | "AS" | "DISTINCT" | "UNIQUE" | "*" | "SINGLE"
        ) || prev == "AS"
            || t.starts_with(')')
            || aggregates.iter().any(|a| t.starts_with(a));
        if !skip {
            return true;
        }
        prev = t;
    }
    false
}

/// abaplint tells `SELECT ... ENDSELECT` loops from single statements, and `WITH ... ENDWITH`
/// likewise, only after parsing; this mirrors its `isSelectLoop` / `isWithLoop` on the top-level
/// tokens of the command.
fn reclassify_select(commands: &mut [Command], tokens: &[Token], src: &str) {
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
    ];
    for c in commands.iter_mut().filter(|c| !c.is_comment_only) {
        let is_with = matches!(c.statement, Some("With" | "WithLoop"));
        if !is_with && !matches!(c.statement, Some("Select" | "SelectLoop")) {
            continue;
        }
        let mut depth = 0i32;
        let mut top: Vec<String> = Vec::new();
        for t in tokens[c.first..=c.last]
            .iter()
            .filter(|t| t.kind != TokenKind::Comment)
        {
            let text = t.text(src);
            if closes_level(text, t.kind) {
                depth -= 1;
            }
            if depth <= 0 {
                top.push(text.to_ascii_uppercase());
            }
            if opens_level(text, t.kind) {
                depth += 1;
            }
        }
        let has = |w: &str| top.iter().any(|t| t == w);
        let followed = |a: &str, b: &str| top.windows(2).any(|p| p[0] == a && p[1] == b);
        let package_size = followed("PACKAGE", "SIZE");
        let into_table = top.iter().enumerate().any(|(k, t)| {
            if t != "INTO" && t != "APPENDING" {
                return false;
            }
            let mut next = k + 1;
            if top.get(next).is_some_and(|w| w == "CORRESPONDING") {
                next += 3; // CORRESPONDING FIELDS OF
            }
            top.get(next).is_some_and(|w| w == "TABLE")
        });
        let is_loop = if is_with {
            package_size || !into_table
        } else if top.get(1).is_some_and(|w| w == "SINGLE") {
            false
        } else if package_size {
            true
        } else if into_table {
            false
        } else {
            let into_any = has("INTO") || has("APPENDING");
            let group_by = followed("GROUP", "BY");
            let aggregation = top
                .iter()
                .any(|t| AGGREGATES.iter().any(|a| t.starts_with(a)))
                && !has_plain_field(&top, AGGREGATES);
            if !into_any && !group_by && !has("HAVING") && !followed("ORDER", "BY") && aggregation {
                false
            } else if has("UNION") || has("INTERSECT") || has("EXCEPT") {
                true
            } else {
                group_by || !aggregation
            }
        };
        c.statement = Some(match (is_with, is_loop) {
            (true, true) => "WithLoop",
            (true, false) => "With",
            (false, true) => "SelectLoop",
            (false, false) => "Select",
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenType::*;

    fn types(src: &str) -> Vec<(String, TokenType)> {
        let s = parse(src, Options::default()).unwrap();
        (0..s.tokens.len())
            .map(|i| (s.text(i).to_string(), s.types[i]))
            .collect()
    }

    #[test]
    fn keyword_collisions_follow_the_grammar() {
        let t = types("READ TABLE lines INTO line INDEX index.");
        let want = [
            ("READ", Keyword),
            ("TABLE", Keyword),
            ("lines", Identifier),
            ("INTO", Keyword),
            ("line", Identifier),
            ("INDEX", Keyword),
            ("index", Identifier),
            (".", Period),
        ];
        assert_eq!(t, want.map(|(a, b)| (a.to_string(), b)));
    }

    #[test]
    fn fused_tokens_take_the_identifier_side() {
        let t = types("DATA(x) = lo->m( 1 ).");
        assert_eq!(t[0], ("DATA(".into(), Keyword));
        assert_eq!(t[1], ("x".into(), Identifier));
        assert_eq!(t[4], ("lo->m(".into(), Identifier));
    }

    #[test]
    fn chains_and_commands() {
        let src = "DATA: a TYPE i, b TYPE string. \" tail\n* own line\nCLEAR a.\n";
        let s = parse(src, Options::default()).unwrap();
        assert_eq!(s.commands.len(), 3);
        assert_eq!(s.commands[0].colon, Some(1));
        assert_eq!(s.commands[0].statement, Some("Data"));
        assert!(s.commands[0].fully_matched);
        assert!(s.commands[0].last > s.commands[0].colon.unwrap());
        assert_eq!(s.text(s.commands[0].last), "\" tail");
        assert!(s.commands[1].is_comment_only);
        assert_eq!(s.commands[2].statement, Some("Clear"));
    }

    #[test]
    fn unparseable_statements_use_the_fallback() {
        let s = parse("FOO BAR value.", Options::default()).unwrap();
        assert!(!s.commands[0].fully_matched);
        assert!(s.sources.contains(&Source::Fallback));
        // `value` is a grammar word, so the guess can only be a keyword: the known weakness
        assert_eq!(s.types[2], Keyword);
    }

    #[test]
    fn brackets_pair_up() {
        let s = parse("x = foo( bar( 1 ) ).", Options::default()).unwrap();
        let outer = s
            .tokens
            .iter()
            .position(|t| t.text(s.src) == "foo(")
            .unwrap();
        let close = s.bracket_match[outer].unwrap();
        assert_eq!(s.text(close), ")");
        assert_eq!(s.bracket_match[close], Some(outer));
        let inner = s
            .tokens
            .iter()
            .position(|t| t.text(s.src) == "bar(")
            .unwrap();
        assert_eq!(s.parent[inner], Some(outer));
    }
}
