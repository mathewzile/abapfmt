//! Lists the words that the grammar could not classify: `cargo run --example fallback -- files...`
use std::collections::BTreeMap;

use abapfmt_syntax::{Options, Source, TokenType, parse};

fn main() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0;
    for path in std::env::args().skip(1) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(syn) = parse(&src, Options::default()) else {
            continue;
        };
        for i in 0..syn.tokens.len() {
            if syn.sources[i] == Source::Fallback {
                let kind = if syn.types[i] == TokenType::Keyword {
                    "K"
                } else {
                    "I"
                };
                *counts
                    .entry(format!("{kind} {}", syn.text(i).to_ascii_uppercase()))
                    .or_default() += 1;
                total += 1;
            }
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("{total} fallback words");
    for (w, n) in v.iter().take(40) {
        println!("{n:>5} {w}");
    }
}
