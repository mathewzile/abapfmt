//! The lexer must never panic on real code, and its spans must stay in order with only
//! whitespace between them (so nothing is ever lost or rewritten).
use std::path::Path;

fn check(src: &str, what: &str) {
    let Ok(tokens) = abapfmt_lexer::lex(src) else {
        return;
    };
    let mut last = 0;
    for t in &tokens {
        assert!(
            t.start as usize >= last && t.end >= t.start,
            "{what}: span order at {}",
            t.start
        );
        assert!(
            src[last..t.start as usize].chars().all(char::is_whitespace),
            "{what}: gap at {last}"
        );
        last = t.end as usize;
    }
    assert!(src[last..].chars().all(char::is_whitespace), "{what}: tail");
}

#[test]
fn samples_and_fixtures_lex_losslessly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    for f in [
        "samples/cleanup-sample.abap",
        "samples/test-1-input.abap",
        "samples/test-2-input.abap",
    ] {
        check(&std::fs::read_to_string(root.join(f)).unwrap(), f);
    }
    let mut n = 0;
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
                for c in v["cases"].as_array().unwrap() {
                    for k in ["src", "exp"] {
                        if let Some(s) = c[k].as_str() {
                            check(s, &p.display().to_string());
                            n += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!("{n} fixture snippets lexed");
    assert!(n > 500);
}
