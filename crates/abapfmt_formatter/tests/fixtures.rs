//! Phase 1 smoke test: every kept upstream fixture file parses and has the expected shape.
//! The real fixture harness (running rules against `src`/`exp`) arrives in phase 4.
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
}

#[test]
fn fixture_files_are_well_formed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rules");
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(
        files.len() >= 40,
        "expected the kept fixture set, found {}",
        files.len()
    );

    let mut cases = 0;
    for file in &files {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert!(
            v["ruleId"].is_string(),
            "{}: missing ruleId",
            file.display()
        );
        for case in v["cases"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: no cases", file.display()))
        {
            assert!(case["name"].is_string());
            if case.get("skipped").is_none() {
                assert!(
                    case["src"].is_string() && case["exp"].is_string(),
                    "{}",
                    file.display()
                );
            }
            cases += 1;
        }
    }
    eprintln!("{} files, {cases} cases", files.len());
}
