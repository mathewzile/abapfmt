//! Whole-file smoke test: how much of real code does the generated grammar type?
use abapfmt_grammar::{Class, Options, parse_statements};

#[test]
fn samples_are_mostly_matched() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    for f in [
        "cleanup-sample.abap",
        "test-1-input.abap",
        "test-2-input.abap",
    ] {
        let src = std::fs::read_to_string(root.join(f)).unwrap();
        let t = std::time::Instant::now();
        let stmts = parse_statements(&src, Options::default());
        let ok = stmts.iter().filter(|s| s.statement.is_some()).count();
        eprintln!(
            "{f}: {ok}/{} statements matched in {:?}",
            stmts.len(),
            t.elapsed()
        );
        for s in stmts.iter().filter(|s| s.statement.is_none()).take(8) {
            let a = s.tokens.first().unwrap().start as usize;
            let b = s.tokens.last().unwrap().end as usize;
            eprintln!("   unmatched: {}", src[a..b].lines().next().unwrap_or(""));
        }
        assert!(ok * 10 >= stmts.len() * 8, "{f}: fewer than 80% matched");
        assert!(
            stmts
                .iter()
                .flat_map(|s| &s.classes)
                .any(|c| *c == Some(Class::Keyword))
        );
    }
}
