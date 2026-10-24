//! Statement splitting and typing, following abaplint's `StatementParser` (MIT): split the token
//! stream at `.`/`,`, expand `:` chains, drop pragmas and match each statement against the
//! generated grammar. Macros and native SQL are not expanded; such statements stay unmatched and
//! are left to the fallback refiner.
use crate::lexer::{GToken, LexClass, lex};
use crate::{Class, Options, Token, match_statement};

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
        if let Some(m) = match_statement(&toks, opts) {
            statement = Some(m.statement);
            for (k, i) in idx.iter().enumerate() {
                classes[*i] = m.classes[k];
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
