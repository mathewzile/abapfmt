//! Phase 2 go/no-go: the keyword-collision probe statements from the old repo's
//! `2026-10-02-abaplint-token-refiner.md` (variables named `value`, `text`, `key`, `index`, `line`,
//! `lines`, `name`, `mode`, `count`, `sum`...). Each word must come out KEYWORD or IDENTIFIER the
//! way the generated abaplint grammar decides it.
//!
//! The lexer below is a throwaway stand-in (abaplint's whitespace-flavoured token kinds, no
//! comments/pragmas/templates); the real one is phase 3.
use abapfmt_grammar::{Class, Options, Tk, Token, match_statement};

fn flavored(base: &str, wb: bool, wa: bool) -> Tk {
    let name = format!(
        "{}{base}{}",
        if wb { "W" } else { "" },
        if wa { "W" } else { "" }
    );
    Tk::from_name(&name).unwrap_or(Tk::Other)
}

fn lex(src: &str) -> Vec<(String, Tk)> {
    let c: Vec<char> = src.chars().collect();
    let white = |ch: Option<&char>| {
        ch.is_none_or(|x| matches!(x, ' ' | '\n' | '\t' | ':' | ',' | '.' | '"'))
    };
    let white_before = |i: usize| i > 0 && matches!(c[i - 1], ' ' | '\n' | '\t' | ':');
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if ch == '\'' || ch == '`' {
            i += 1;
            while i < c.len() && c[i] != ch {
                i += 1;
            }
            i += 1;
            out.push((c[start..i].iter().collect(), Tk::Other));
            continue;
        }
        let two: String = c[i..(i + 2).min(c.len())].iter().collect();
        let (len, base) = match (ch, two.as_str()) {
            (_, "->") => (2, Some("InstanceArrow")),
            (_, "=>") => (2, Some("StaticArrow")),
            ('(', _) => (1, Some("ParenLeft")),
            (')', _) => (1, Some("ParenRight")),
            ('[', _) => (1, Some("BracketLeft")),
            (']', _) => (1, Some("BracketRight")),
            ('-', _) => (1, Some("Dash")),
            ('+', _) => (1, Some("Plus")),
            ('@', _) => (1, Some("At")),
            ('.' | ',' | ':', _) => (1, None),
            _ => {
                let mut j = i;
                while j < c.len()
                    && !c[j].is_whitespace()
                    && !matches!(
                        c[j],
                        '.' | ',' | ':' | '(' | ')' | '[' | ']' | '+' | '@' | '\'' | '`'
                    )
                    && !(c[j] == '-' && j > i)
                    && !(j + 1 < c.len() && matches!((c[j], c[j + 1]), ('-', '>') | ('=', '>')))
                {
                    j += 1;
                }
                (j - i, None)
            }
        };
        let text: String = c[i..i + len].iter().collect();
        let kind = match base {
            Some(b) => flavored(b, white_before(i), white(c.get(i + len))),
            None => Tk::Other,
        };
        out.push((text, kind));
        i += len;
    }
    out
}

/// Split `PREFIX: a, b.` into the statements `PREFIX a` and `PREFIX b` (abaplint's chain handling).
fn statements(src: &str) -> Vec<Vec<(String, Tk)>> {
    let src = src.trim().trim_end_matches('.');
    match src.split_once(':') {
        Some((pre, rest)) => rest
            .split(',')
            .map(|s| lex(&format!("{pre} {s}")))
            .collect(),
        None => vec![lex(src)],
    }
}

fn check(src: &str) -> Result<(), String> {
    for toks in statements(src) {
        let tokens: Vec<Token> = toks
            .iter()
            .map(|(t, k)| Token { text: t, kind: *k })
            .collect();
        let m = match_statement(&tokens, Options::default())
            .ok_or_else(|| format!("{src}: no statement matched"))?;
        for (t, cls) in toks.iter().zip(&m.classes) {
            if !t
                .0
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '<')
            {
                continue;
            }
            // the probes write keywords in upper case and variables in lower case
            let want = if t.0.chars().any(|c| c.is_ascii_uppercase()) {
                Class::Keyword
            } else {
                Class::Identifier
            };
            if *cls != Some(want) {
                return Err(format!(
                    "{src} [{}]: `{}` is {cls:?}, want {want:?}",
                    m.statement, t.0
                ));
            }
        }
    }
    Ok(())
}

// Keywords upper case, variables lower case: the grammar must agree on every word.
const PROBES: &[&str] = &[
    "METHODS run IMPORTING value TYPE i key TYPE string EXPORTING text TYPE string.",
    "DATA(x) = COND #( WHEN value = 1 THEN text ELSE key ).",
    "DATA(y) = SWITCH #( mode WHEN 1 THEN text ELSE name ).",
    "READ TABLE lines INTO line INDEX index.",
    "READ TABLE lines INTO line WITH KEY id = key.",
    "READ TABLE lines INDEX index INTO line.",
    "LOOP AT lines INTO line WHERE id = key.",
    "WRITE: / text, key.",
    "DELETE lines INDEX index.",
    "INSERT line INTO lines INDEX index.",
    "MODIFY lines FROM line INDEX index.",
    "SPLIT text AT ',' INTO TABLE lines.",
    "SPLIT text AT ',' INTO key value.",
    "FIND text IN value.",
    "REPLACE ALL OCCURRENCES OF text IN value WITH key.",
    "CONDENSE text.",
    "TRANSLATE text TO UPPER CASE.",
    "SHIFT text LEFT DELETING LEADING '0'.",
    "ASSIGN COMPONENT name OF STRUCTURE line TO <fs>.",
    "SORT lines BY key.",
    "DESCRIBE TABLE lines LINES count.",
    "MESSAGE text TYPE 'E'.",
    "x = REDUCE i( INIT sum = 0 FOR line IN lines NEXT sum = sum + line-id ).",
    "x = VALUE #( FOR line IN lines ( line-id ) ).",
    "CASE mode.",
    "PERFORM run USING text CHANGING value.",
    "RETURN text.",
    // already handled by the heuristic refiner; must stay right
    "WRITE text.",
    "CLEAR: text, key.",
    "x = lines( lines ).",
    "IF text IS NOT INITIAL AND key = 1.",
    "obj->run( value = text key = 2 ).",
    "SELECT SINGLE * FROM ztab INTO @key WHERE id = @value.",
    "x = lines[ key = value ].",
];

fn run_probes() -> (usize, Vec<String>) {
    let mut failures = Vec::new();
    for src in PROBES {
        if let Err(e) = check(src) {
            failures.push(e);
        }
    }
    (PROBES.len(), failures)
}

#[test]
fn keyword_collision_probes() {
    // The grammar is recursive; give the matcher the stack a real run would have.
    let h = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(run_probes)
        .unwrap();
    let (total, failures) = h.join().unwrap();
    for f in &failures {
        eprintln!("FAIL {f}");
    }
    eprintln!(
        "{}/{} probes classified correctly",
        total - failures.len(),
        total
    );
    assert!(failures.is_empty(), "{} probe(s) failed", failures.len());
}
