//! Phase 3 gate: the whole pipeline (lexer -> grammar -> typed commands) over every ABAP fixture
//! snippet and sample. Reports how much the grammar types versus the fallback.
use abapfmt_syntax::{Options, Source, parse};
use std::path::Path;

fn snippets() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut out = Vec::new();
    for f in [
        "cleanup-sample.abap",
        "test-1-input.abap",
        "test-2-input.abap",
        "test-2-output.abap",
    ] {
        out.push((
            f.to_string(),
            std::fs::read_to_string(root.join("samples").join(f)).unwrap(),
        ));
    }
    let mut stack = vec![root.join("rules")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().contains("ddl")
            {
                let v: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
                let rule = v["ruleId"].as_str().unwrap().to_string();
                for c in v["cases"].as_array().unwrap() {
                    for k in ["src", "exp"] {
                        if let Some(s) = c[k].as_str() {
                            out.push((
                                format!("{rule}/{}/{k}", c["name"].as_str().unwrap_or("?")),
                                s.to_string(),
                            ));
                        }
                    }
                }
            }
        }
    }
    out
}

#[test]
fn corpus_is_typed_without_loss() {
    let (mut cmds, mut matched, mut words, mut fallback_words, mut lex_errors) = (0, 0, 0, 0, 0);
    let mut worst: Vec<(String, String)> = Vec::new();
    let all = snippets();
    let t = std::time::Instant::now();
    for (name, src) in &all {
        let Ok(s) = parse(src, Options::default()) else {
            lex_errors += 1;
            continue;
        };
        for c in s.commands.iter().filter(|c| !c.is_comment_only) {
            cmds += 1;
            if c.fully_matched {
                matched += 1;
            } else if worst.len() < 15 {
                let a = s.tokens[c.first].start as usize;
                let b = s.tokens[c.last].end as usize;
                worst.push((
                    name.clone(),
                    src[a..b].lines().next().unwrap_or("").to_string(),
                ));
            }
        }
        for (i, t) in s.tokens.iter().enumerate() {
            if t.kind == abapfmt_lexer::TokenKind::Word {
                words += 1;
                if s.sources[i] == Source::Fallback {
                    fallback_words += 1;
                }
            }
        }
    }
    eprintln!(
        "{} snippets in {:?}: {matched}/{cmds} commands fully matched by the grammar; \
         {fallback_words}/{words} words via fallback; {lex_errors} lexer errors",
        all.len(),
        t.elapsed()
    );
    for (n, l) in &worst {
        eprintln!("  unmatched in {n}: {l}");
    }
    assert!(cmds > 1000);
    assert!(
        matched * 100 >= cmds * 85,
        "grammar should type at least 85% of commands"
    );
}
