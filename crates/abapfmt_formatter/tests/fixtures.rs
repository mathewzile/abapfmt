//! Fixture harness over the upstream abap-cleaner rule tests in `fixtures/rules/**`.
//!
//! For every case:
//!  * an implemented rule must turn `src` into `exp` (ratchet: every case listed in
//!    `fixtures/baseline.json` must keep passing; `UPDATE_BASELINE=1` records new passes);
//!  * safety: an implemented rule's output must have the same tokens as `src`;
//!  * idempotence: an implemented rule applied to its own (passing) output changes nothing.
//!
//! The importer already wrapped `src`/`exp` in METHOD/CLASS/FORM as the upstream tests do, so the
//! texts are compared as they are.
use abapfmt_formatter::{ConfigValue, RuleConfig, run_rule, same_tokens};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

struct Case {
    key: String,
    rule: String,
    src: String,
    exp: String,
    config: RuleConfig,
    skipped: bool,
}

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

fn config_of(v: &Value) -> RuleConfig {
    v.as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    let val = match v {
                        Value::Bool(b) => ConfigValue::Bool(*b),
                        Value::Number(n) => ConfigValue::Int(n.as_i64()?),
                        Value::String(s) => ConfigValue::Str(s.clone()),
                        _ => return None,
                    };
                    Some((k.clone(), val))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn load(root: &Path) -> Vec<Case> {
    let mut files = Vec::new();
    collect(&root.join("rules"), &mut files);
    files.sort();
    let mut cases = Vec::new();
    for f in files {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let rel = f
            .strip_prefix(root.join("rules"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let rule = v["ruleId"].as_str().unwrap().to_string();
        for c in v["cases"].as_array().unwrap() {
            cases.push(Case {
                key: format!("{rel}::{}", c["name"].as_str().unwrap()),
                rule: rule.clone(),
                src: c["src"].as_str().unwrap_or_default().to_string(),
                exp: c["exp"].as_str().unwrap_or_default().to_string(),
                config: config_of(&c["config"]),
                skipped: c.get("skipped").is_some(),
            });
        }
    }
    cases
}

/// Cases whose expected output changes tokens (`fixtures/out-of-scope.json`: key -> reason). abapfmt
/// does not do this, so they are neither required to pass nor counted as failures.
fn out_of_scope(root: &Path) -> std::collections::BTreeMap<String, String> {
    serde_json::from_str(&std::fs::read_to_string(root.join("out-of-scope.json")).unwrap()).unwrap()
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

#[test]
fn fixture_ratchet() {
    let root = root();
    let baseline_path = root.join("baseline.json");
    let baseline: BTreeSet<String> = std::fs::read_to_string(&baseline_path)
        .ok()
        .map(|s| serde_json::from_str::<Vec<String>>(&s).unwrap())
        .unwrap_or_default()
        .into_iter()
        .collect();

    let cases = load(&root);
    let oos = out_of_scope(&root);
    let (mut passing, mut failing, mut no_rule, mut skipped) = (BTreeSet::new(), 0, 0, 0);
    let mut not_idempotent = Vec::new();
    let mut unsafe_out = Vec::new();
    for c in &cases {
        if c.skipped || oos.contains_key(&c.key) {
            skipped += 1;
            continue;
        }
        let Some(out) = run_rule(&c.rule, &c.src, &c.config) else {
            no_rule += 1;
            continue;
        };
        if let Err(e) = same_tokens(&c.src, &out) {
            unsafe_out.push(format!("{}: {e}", c.key));
        }
        if out == c.exp {
            passing.insert(c.key.clone());
            if run_rule(&c.rule, &out, &c.config).as_deref() != Some(out.as_str()) {
                not_idempotent.push(c.key.clone());
            }
        } else {
            failing += 1;
            if std::env::var_os("SHOW_FAILS")
                .is_some_and(|v| c.key.contains(v.to_str().unwrap_or("")))
            {
                eprintln!(
                    "FAIL {}
--- src
{}
--- exp
{}
--- got
{out}
",
                    c.key, c.src, c.exp
                );
            }
        }
    }

    eprintln!(
        "{} cases: {} pass, {failing} fail, {no_rule} without rule, {skipped} skipped or out of scope",
        cases.len(),
        passing.len()
    );

    if std::env::var_os("UPDATE_BASELINE").is_some() {
        let list: Vec<&String> = passing.iter().collect();
        std::fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&list).unwrap() + "\n",
        )
        .unwrap();
        eprintln!("baseline updated: {} passing cases", passing.len());
    } else {
        let regressions: Vec<&String> = baseline.iter().filter(|k| !passing.contains(*k)).collect();
        let gained = passing.iter().filter(|k| !baseline.contains(*k)).count();
        if gained > 0 {
            eprintln!(
                "{gained} case(s) pass that are not in the baseline; run with UPDATE_BASELINE=1"
            );
        }
        assert!(
            regressions.is_empty(),
            "ratchet: {} baseline case(s) regressed: {regressions:?}",
            regressions.len()
        );
    }
    assert!(
        unsafe_out.is_empty(),
        "rule output changed tokens: {unsafe_out:?}"
    );
    assert!(
        not_idempotent.is_empty(),
        "not idempotent: {not_idempotent:?}"
    );
}

/// The scope claim of the project: in the kept ABAP fixtures, `exp` differs from `src` only in
/// whitespace and letter case. Exceptions must be listed in `fixtures/out-of-scope.json` with a
/// reason, so a new one is a deliberate decision rather than an accident.
#[test]
fn expected_output_only_changes_whitespace_and_case() {
    let root = root();
    let oos = out_of_scope(&root);
    let mut violations = std::collections::BTreeSet::new();
    let mut checked = 0;
    for c in load(&root)
        .iter()
        .filter(|c| !c.skipped && !c.key.starts_with("ddl/"))
    {
        checked += 1;
        if let Err(e) = same_tokens(&c.src, &c.exp) {
            eprintln!("  {} ({}): {e}", c.key, c.rule);
            violations.insert(c.key.clone());
        }
    }
    eprintln!(
        "{checked} ABAP cases checked, {} change tokens, {} listed as out of scope",
        violations.len(),
        oos.len()
    );
    let unlisted: Vec<_> = violations
        .iter()
        .filter(|k| !oos.contains_key(*k))
        .collect();
    assert!(
        unlisted.is_empty(),
        "token-changing fixtures not in out-of-scope.json: {unlisted:?}"
    );
    let stale: Vec<_> = oos.keys().filter(|k| !violations.contains(*k)).collect();
    assert!(
        stale.is_empty(),
        "out-of-scope.json lists cases that no longer change tokens: {stale:?}"
    );
}
