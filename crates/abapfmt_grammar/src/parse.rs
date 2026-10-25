//! Statement splitting and typing, following abaplint's `StatementParser` (MIT): split the token
//! stream at `.`/`,`, expand `:` chains, drop pragmas and match each statement against the
//! generated grammar. Macros and native SQL are not expanded; such statements stay unmatched and
//! are left to the fallback refiner.
use std::collections::HashMap;
use std::sync::Mutex;

use crate::lexer::{GToken, LexClass, lex};
use crate::{Class, Options, Token, match_statement};

type Memo = (Option<&'static str>, Vec<Option<Class>>);

/// Results of the matcher by statement text. A formatter run parses the same statements again and
/// again, because its rules change only whitespace and letter case; the matcher is the expensive
/// part of a parse.
static MEMO: Mutex<Option<HashMap<String, Memo>>> = Mutex::new(None);
const MEMO_LIMIT: usize = 100_000;

fn memo_key(toks: &[Token], opts: Options) -> String {
    let mut key = format!("{opts:?}");
    for t in toks {
        key.push('\u{1}');
        key.push(char::from(t.kind as u8));
        key.push_str(t.text);
    }
    key
}

fn match_memoized(toks: &[Token], opts: Options) -> Memo {
    let key = memo_key(toks, opts);
    if let Some(m) = MEMO.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        return m.clone();
    }
    let result = match match_statement(toks, opts) {
        Some(m) => (Some(m.statement), m.classes),
        None => (None, Vec::new()),
    };
    let mut guard = MEMO.lock().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    if cache.len() >= MEMO_LIMIT {
        cache.clear();
    }
    cache.insert(key, result.clone());
    result
}

#[derive(Debug)]
pub struct ParsedStatement {
    /// All tokens of the statement, including pragmas and the closing `.`/`,`. For a chain
    /// segment this is the shared prefix followed by the segment's own tokens.
    pub tokens: Vec<GToken>,
    /// Matching statement, `None` if the grammar does not know it.
    pub statement: Option<&'static str>,
    /// One entry per token of `tokens` (`None` for pragmas, the closing punctuation and
    /// everything of an unmatched statement).
    pub classes: Vec<Option<Class>>,
}

/// Split into statements; comments are dropped. `src` may contain `\r`.
fn split(tokens: &[GToken], src: &str) -> Vec<Vec<GToken>> {
    let mut out = Vec::new();
    let mut add: Vec<GToken> = Vec::new();
    let mut pre: Vec<GToken> = Vec::new();
    let mut colon = false;
    for t in tokens {
        if t.class == LexClass::Comment {
            continue;
        }
        add.push(*t);
        match t.text(src) {
            "." => {
                out.push(pre.iter().chain(&add).copied().collect());
                add.clear();
                pre.clear();
                colon = false;
            }
            "," if !pre.is_empty() => {
                out.push(pre.iter().chain(&add).copied().collect());
                add.clear();
            }
            ":" if !colon => {
                colon = true;
                add.pop();
                pre.append(&mut add);
            }
            ":" => {
                add.pop();
            }
            _ => {}
        }
    }
    if !add.is_empty() {
        out.push(pre.iter().chain(&add).copied().collect());
    }
    out
}

fn parse_one(src: &str, tokens: Vec<GToken>, opts: Options) -> ParsedStatement {
    let n = tokens.len();
    let ends_with_punct = tokens
        .last()
        .is_some_and(|t| t.class == LexClass::Punctuation);
    let mut classes = vec![None; n];
    let mut statement = None;
    if ends_with_punct && n > 1 {
        // everything but the closing punctuation, without pragmas
        let idx: Vec<usize> = (0..n - 1)
            .filter(|i| tokens[*i].class != LexClass::Pragma)
            .collect();
        let toks: Vec<Token> = idx
            .iter()
            .map(|i| Token {
                text: tokens[*i].text(src),
                kind: tokens[*i].kind,
            })
            .collect();
        let (matched, m_classes) = match_memoized(&toks, opts);
        if let Some(name) = matched {
            statement = Some(name);
            for (k, i) in idx.iter().enumerate() {
                classes[*i] = m_classes[k];
            }
        }
    }
    ParsedStatement {
        tokens,
        statement,
        classes,
    }
}

/// Lex, split and match every statement of `src`. Runs the (deeply recursive) matcher on a
/// thread with a large stack.
pub fn parse_statements(src: &str, opts: Options) -> Vec<ParsedStatement> {
    let norm = src.replace('\r', " ");
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, || {
                let tokens = lex(&norm);
                split(&tokens, &norm)
                    .into_iter()
                    .map(|t| parse_one(&norm, t, opts))
                    .collect()
            })
            .expect("spawn parser thread")
            .join()
            .expect("parser thread panicked")
    })
}

/// Empties the statement cache (for benchmarks).
pub fn clear_memo() {
    *MEMO.lock().unwrap() = None;
}
