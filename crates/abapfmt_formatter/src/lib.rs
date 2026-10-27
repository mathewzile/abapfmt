//! Layout/case rules producing text edits.
//!
//! A rule never changes tokens: it only edits whitespace, line layout and letter case. The
//! runtime guard for that is [`same_tokens`], which the CLI/LSP apply to every result.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

mod align;
mod doc;
mod reflow;
mod rules;

pub use doc::Doc;

use abapfmt_lexer::{TokenKind, lex};

/// A rule option value, as in the fixture files and (later) the config file.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    /// Strings and enum values (`"Type.VALUE"` as written by the upstream tests).
    Str(String),
}

/// The upstream default of every option of `rule` (`defaults.json`, read from the upstream rule
/// classes of ABAP cleaner v1.29.0). Options that are not set in a configuration take these values.
pub fn default_options(rule: &str) -> RuleConfig {
    static DEFAULTS: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let root = DEFAULTS.get_or_init(|| {
        serde_json::from_str(include_str!("../defaults.json")).expect("defaults.json is valid")
    });
    root[rule]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, v)| {
            let value = match v {
                serde_json::Value::Bool(b) => ConfigValue::Bool(*b),
                serde_json::Value::Number(n) => ConfigValue::Int(n.as_i64()?),
                serde_json::Value::String(s) => ConfigValue::Str(s.clone()),
                _ => return None,
            };
            Some((name.clone(), value))
        })
        .collect()
}

/// Option name (upstream `Config*` name without the prefix) to value.
pub type RuleConfig = BTreeMap<String, ConfigValue>;

/// Read helpers for rule options; fixtures always carry every option, the default only covers
/// hand-written configs.
pub trait ConfigExt {
    fn int(&self, name: &str, default: i64) -> i64;
    fn bool(&self, name: &str, default: bool) -> bool;
    /// The enum constant name, without any `Type.` prefix.
    fn choice(&self, name: &str, default: &str) -> String;
}

impl ConfigExt for RuleConfig {
    fn int(&self, name: &str, default: i64) -> i64 {
        match self.get(name) {
            Some(ConfigValue::Int(i)) => *i,
            _ => default,
        }
    }
    fn bool(&self, name: &str, default: bool) -> bool {
        match self.get(name) {
            Some(ConfigValue::Bool(b)) => *b,
            _ => default,
        }
    }
    fn choice(&self, name: &str, default: &str) -> String {
        match self.get(name) {
            Some(ConfigValue::Str(v)) => v.rsplit('.').next().unwrap_or(v).to_string(),
            _ => default.to_string(),
        }
    }
}

/// A single rule: edits the layout/case of the document.
pub type RuleFn = fn(doc: &mut Doc, config: &RuleConfig);

/// Rules implemented so far, keyed by upstream rule id (e.g. `"EMPTY_LINES_WITHIN_METHODS"`).
pub fn rules() -> &'static [(&'static str, RuleFn)] {
    rules::ALL
}

/// Run only the rule `rule_id`; `None` if it is not implemented. Source that cannot be
/// tokenized is returned unchanged.
pub fn run_rule(rule_id: &str, src: &str, config: &RuleConfig) -> Option<String> {
    let (_, rule) = rules().iter().find(|(id, _)| *id == rule_id)?;
    let mut text = src.to_string();
    // a rule that moved tokens asks to be run again on the parsed result
    for _ in 0..MAX_RERUNS {
        let Some(mut doc) = Doc::new(&text) else {
            return Some(text);
        };
        // DDL rules only apply to CDS sources and ABAP rules only to ABAP
        if rule_id.starts_with("DDL_") != doc.syn.ddl.is_some() {
            return Some(text);
        }
        rule(&mut doc, config);
        let rerun = doc.rerun;
        text = doc.render();
        if !rerun {
            break;
        }
    }
    Some(text)
}

/// How often a rule may ask to be run again on its own output.
const MAX_RERUNS: usize = 3;

/// Settings that a rule reads from another rule upstream: (rule, source rule, setting names).
const CROSS_RULE_SETTINGS: &[(&str, &str, &[&str])] = &[
    (
        "DDL_ALIGN_ENTITY_PARAMETERS",
        "DDL_POSITION_DEFINE",
        &["ParamsIndent"],
    ),
    (
        "DDL_ALIGN_SOURCE_PARAMETERS",
        "DDL_POSITION_DEFINE",
        &["ParamsIndent"],
    ),
    (
        "DDL_ALIGN_FUNCTION_PARAMETERS",
        "DDL_POSITION_DEFINE",
        &["ParamsIndent"],
    ),
    (
        "DDL_ALIGN_ENTITY_PARAMETERS",
        "DDL_SPACES_AROUND_SIGNS",
        &["SpaceBeforeColon", "SpaceAfterColon"],
    ),
    (
        "DDL_ALIGN_SOURCE_PARAMETERS",
        "DDL_SPACES_AROUND_SIGNS",
        &["SpaceBeforeColon", "SpaceAfterColon"],
    ),
    (
        "DDL_ALIGN_FUNCTION_PARAMETERS",
        "DDL_SPACES_AROUND_SIGNS",
        &["SpaceBeforeColon", "SpaceAfterColon"],
    ),
    (
        "DDL_ALIGN_ENTITY_PARAMETERS",
        "DDL_SPACES_AROUND_BRACKETS",
        &["SpacesInsideFuncParens"],
    ),
    (
        "DDL_ALIGN_SOURCE_PARAMETERS",
        "DDL_SPACES_AROUND_BRACKETS",
        &["SpacesInsideFuncParens"],
    ),
    (
        "DDL_ALIGN_FUNCTION_PARAMETERS",
        "DDL_SPACES_AROUND_BRACKETS",
        &["SpacesInsideFuncParens"],
    ),
    (
        "DDL_ALIGN_LOGICAL_EXPRESSIONS",
        "DDL_SPACES_AROUND_BRACKETS",
        &["SpacesInsideArithParens"],
    ),
];

/// `config` of rule `id`, completed with the settings it takes from other rules.
fn with_cross_rule_settings(
    id: &str,
    config: &RuleConfig,
    configs: &BTreeMap<String, Option<RuleConfig>>,
) -> RuleConfig {
    let mut result = config.clone();
    for (target, source, names) in CROSS_RULE_SETTINGS {
        if *target != id {
            continue;
        }
        if let Some(Some(source_config)) = configs.get(*source) {
            for name in *names {
                if let Some(v) = source_config.get(*name) {
                    result
                        .entry((*name).to_string())
                        .or_insert_with(|| v.clone());
                }
            }
        }
    }
    result
}

/// [`format`], guarded by [`same_tokens`]: an `Err` means a rule changed more than whitespace,
/// layout or letter case, and the original source must be kept.
pub fn format_checked(
    src: &str,
    configs: &BTreeMap<String, Option<RuleConfig>>,
    options: &Options,
) -> Result<String, String> {
    let configs = &with_tabular_width(configs, options.tabular_line_width);
    let tabular = options.tabular_line_width;
    let mut out = format(src, configs);
    if let Some(width) = options.line_width {
        let reflowed = reflow::reflow(&out, width, tabular);
        if reflowed != out {
            out = reflowed;
            // the rules lay out the broken lines again; repeat until the result is stable, so that
            // formatting the output once more changes nothing
            for _ in 0..MAX_REFLOW_ROUNDS {
                let next = reflow::reflow(&format(&out, configs), width, tabular);
                if next == out {
                    break;
                }
                out = next;
            }
        }
    }
    same_tokens(src, &out).map(|()| out)
}

/// Rounds of rules and reflow after the first reflow.
const MAX_REFLOW_ROUNDS: usize = 4;

/// Settings that do not belong to a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Statements with longer lines are broken after all rules have run; `None` switches the
    /// reflow off, so that only the per-rule line length options apply. Applies to ABAP only.
    pub line_width: Option<usize>,
    /// Maximum line length of the rows of tables in tabular style (`MaxLineLengthForSingleLine`
    /// of ALIGN_PARAMETERS, unless the rule configures its own value). The reflow lets lines
    /// that start with a row of such a table stay within this length.
    pub tabular_line_width: usize,
}

/// The default width of rows in tabular style, as in abap-cleaner: tabular style improves
/// readability, therefore some overlength is tolerated there.
pub const DEFAULT_TABULAR_LINE_WIDTH: usize = 160;

/// `configs` with `width` as the table row length of ALIGN_PARAMETERS, if not set there.
fn with_tabular_width(
    configs: &BTreeMap<String, Option<RuleConfig>>,
    width: usize,
) -> BTreeMap<String, Option<RuleConfig>> {
    let mut result = configs.clone();
    let entry = result
        .entry("ALIGN_PARAMETERS".to_string())
        .or_insert_with(|| Some(RuleConfig::new()));
    if let Some(c) = entry {
        c.entry("MaxLineLengthForSingleLine".to_string())
            .or_insert(ConfigValue::Int(width as i64));
    }
    result
}

/// The default line width, as recommended by the Clean ABAP style guide.
pub const DEFAULT_LINE_WIDTH: usize = 120;

impl Default for Options {
    fn default() -> Options {
        Options {
            line_width: Some(DEFAULT_LINE_WIDTH),
            tabular_line_width: DEFAULT_TABULAR_LINE_WIDTH,
        }
    }
}

/// A rule (or `"lineWidth"` for the reflow) that [`format_safe`] switched off, because its result
/// did not have the same tokens as its input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub rule: String,
    pub error: String,
}

/// The result of [`format_safe`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    /// Empty unless a rule had to be switched off to get a safe result.
    pub skipped: Vec<Skipped>,
}

/// [`format_checked`], but a rule whose result fails the token check is switched off instead of
/// rejecting the whole source. `Err` only if the source cannot be tokenized or no safe result is
/// found.
pub fn format_safe(
    src: &str,
    configs: &BTreeMap<String, Option<RuleConfig>>,
    options: &Options,
) -> Result<Formatted, String> {
    let first_error = match format_checked(src, configs, options) {
        Ok(text) => {
            return Ok(Formatted {
                text,
                skipped: Vec::new(),
            });
        }
        Err(e) => e,
    };
    // the source itself cannot be tokenized: there is nothing to compare a result with
    same_tokens(src, src)?;
    let configs = &with_tabular_width(configs, options.tabular_line_width);
    let (_, mut skipped) = format_with(rules(), src, configs, true);
    let mut reduced = configs.clone();
    for s in &skipped {
        reduced.insert(s.rule.clone(), None);
    }
    if let Ok(text) = format_checked(src, &reduced, options) {
        return Ok(Formatted { text, skipped });
    }
    // the rules are fine on their own, so the reflow (or its interplay with the rules) is at fault
    let without_reflow = Options {
        line_width: None,
        ..options.clone()
    };
    match format_checked(src, &reduced, &without_reflow) {
        Ok(text) => {
            skipped.push(Skipped {
                rule: "lineWidth".to_string(),
                error: first_error,
            });
            Ok(Formatted { text, skipped })
        }
        Err(e) => Err(e),
    }
}

/// Run all implemented rules in order. `configs` maps a rule id to its options (defaults for
/// absent ones); a rule mapped to `None` is switched off. The source is re-parsed between rules,
/// because a rule may move tokens.
pub fn format(src: &str, configs: &BTreeMap<String, Option<RuleConfig>>) -> String {
    format_with(rules(), src, configs, false).0
}

/// [`format`] with the given rules. With `guard`, the result of every rule that changed the text
/// is checked with [`same_tokens`]; a rule that fails is switched off for the rest of the run and
/// returned.
fn format_with(
    all: &[(&str, RuleFn)],
    src: &str,
    configs: &BTreeMap<String, Option<RuleConfig>>,
    guard: bool,
) -> (String, Vec<Skipped>) {
    let mut skipped: Vec<Skipped> = Vec::new();
    let mut text = src.to_string();
    let mut next = 0;
    // the source is parsed again only after a rule has changed it
    while next < all.len() {
        let Some(mut doc) = Doc::new(&text) else {
            break;
        };
        let is_ddl = doc.syn.ddl.is_some();
        let mut changed_at = None;
        for (k, (id, rule)) in all.iter().enumerate().skip(next) {
            // DDL rules only apply to CDS sources and ABAP rules only to ABAP
            if id.starts_with("DDL_") != is_ddl {
                continue;
            }
            // options that the configuration does not set have the upstream default
            let mut config = default_options(id);
            match configs.get(*id) {
                Some(None) => continue,
                Some(Some(c)) => config.extend(c.clone()),
                None => {}
            }
            if skipped.iter().any(|s| s.rule == *id) {
                continue;
            }
            let config = with_cross_rule_settings(id, &config, configs);
            let before = doc.snapshot();
            rule(&mut doc, &config);
            if !doc.is_unchanged_since(&before) {
                // a rule that asked to run again continues with itself on the changed source
                changed_at = Some((k, if doc.rerun { k } else { k + 1 }));
                break;
            }
        }
        let Some((rule_at, resume)) = changed_at else {
            break;
        };
        let out = doc.render();
        drop(doc);
        if guard && let Err(error) = same_tokens(&text, &out) {
            // continue with the next rule on the text before this one
            skipped.push(Skipped {
                rule: all[rule_at].0.to_string(),
                error,
            });
            next = rule_at + 1;
            continue;
        }
        text = out;
        next = resume;
    }
    (text, skipped)
}

/// Is `after` the same token stream as `before`? Only whitespace and the case of words,
/// operators and pragmas may differ. Code tokens (everything but comments) are compared in order;
/// comments and pragmas are compared as separate streams (comments ignoring whitespace inside them,
/// pragmas ignoring case), because rules may move a closing bracket or period in front of a
/// trailing comment or pragma and re-indent commented-out code.
/// ALIGN_PERFORM moves `IF FOUND` in front of the parameters of a PERFORM; this is the only
/// reordering of tokens that a rule makes, so `IF FOUND` is put to the end of every PERFORM
/// statement before the tokens are compared.
fn move_if_found_to_end(code: Vec<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(code.len());
    let mut statement: Vec<String> = Vec::new();
    for token in code {
        let ends = token == ".";
        statement.push(token);
        if !ends {
            continue;
        }
        if statement[0] == "PERFORM" {
            let mut moved = Vec::new();
            let mut kept = Vec::with_capacity(statement.len());
            let mut i = 0;
            while i < statement.len() {
                if statement[i] == "IF" && statement.get(i + 1).is_some_and(|t| t == "FOUND") {
                    moved.extend(["IF".to_string(), "FOUND".to_string()]);
                    i += 2;
                } else {
                    kept.push(std::mem::take(&mut statement[i]));
                    i += 1;
                }
            }
            let period = kept.pop();
            result.extend(kept);
            result.extend(moved);
            result.extend(period);
        } else {
            result.append(&mut statement);
        }
        statement.clear();
    }
    result.extend(statement);
    result
}

pub fn same_tokens(before: &str, after: &str) -> Result<(), String> {
    let tokenize = |src: &str| {
        if abapfmt_syntax::is_ddl_source(src) {
            abapfmt_syntax::parse_ddl(src).map(|s| s.tokens)
        } else {
            lex(src).ok()
        }
    };
    let (Some(a), Some(b)) = (tokenize(before), tokenize(after)) else {
        return Err("source cannot be tokenized".into());
    };
    let split = |src: &str, tokens: &[abapfmt_lexer::Token]| {
        let (mut code, mut comments, mut pragmas) = (Vec::new(), Vec::new(), Vec::new());
        for t in tokens {
            let text = t.text(src);
            match t.kind {
                TokenKind::Comment => comments.push(text.split_whitespace().collect::<String>()),
                TokenKind::Pragma => pragmas.push(text.to_ascii_uppercase()),
                TokenKind::Literal => code.push(text.to_string()),
                _ => code.push(text.to_ascii_uppercase()),
            }
        }
        (move_if_found_to_end(code), comments, pragmas)
    };
    let ((ca, ma, pa), (cb, mb, pb)) = (split(before, &a), split(after, &b));
    // DDL annotations may be written as one token ("@Anno.sub") that was several tokens before
    // ("@Anno. sub"); only the whitespace between their parts changes
    let ddl_joined = abapfmt_syntax::is_ddl_source(before) && ca.concat() == cb.concat();
    if ca.len() != cb.len() && !ddl_joined {
        return Err(format!(
            "code token count changed: {} -> {}",
            ca.len(),
            cb.len()
        ));
    }
    if let Some(i) = (0..ca.len()).find(|i| !ddl_joined && ca[*i] != cb[*i]) {
        return Err(format!(
            "code token {i} changed: `{}` -> `{}`",
            ca[i], cb[i]
        ));
    }
    if pa != pb {
        return Err("pragmas changed".into());
    }
    if ma != mb {
        return Err(format!(
            "comments changed: {} -> {} comment tokens",
            ma.len(),
            mb.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_and_case_are_not_token_changes() {
        assert!(same_tokens("data x type i.", "DATA  x\n  TYPE i.").is_ok());
        assert!(same_tokens("x = 'a'.", "x = 'A'.").is_err());
        assert!(same_tokens("x = 1.", "x = 2.").is_err());
        assert!(same_tokens("x = 1.", "x = 1. y = 2.").is_err());
    }

    /// A broken rule: renames the first identifier.
    fn rename_identifier(doc: &mut Doc, _: &RuleConfig) {
        let t = doc
            .syn
            .types
            .iter()
            .position(|t| *t == abapfmt_syntax::TokenType::Identifier);
        if let Some(t) = t {
            doc.text[t] = Some("renamed".to_string());
        }
    }

    #[test]
    fn a_rule_that_changes_tokens_is_skipped() {
        let case = rules()
            .iter()
            .find(|(id, _)| *id == "UPPER_AND_LOWER_CASE")
            .unwrap()
            .1;
        let all: &[(&str, RuleFn)] = &[
            ("BROKEN", rename_identifier),
            ("UPPER_AND_LOWER_CASE", case),
        ];
        let src = "data value type i.\n";
        let (out, skipped) = format_with(all, src, &BTreeMap::new(), true);
        assert_eq!(out, "DATA value TYPE i.\n");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].rule, "BROKEN");
        // unguarded, the broken result goes through (and format_checked would reject it)
        let (out, _) = format_with(all, src, &BTreeMap::new(), false);
        assert!(same_tokens(src, &out).is_err());
    }

    #[test]
    fn format_safe_skips_nothing_on_working_rules() {
        let src = "data value type i.\n";
        let f = format_safe(src, &BTreeMap::new(), &Options::default()).unwrap();
        assert_eq!(f.text, "DATA value TYPE i.\n");
        assert!(f.skipped.is_empty());
        assert!(
            format_safe(
                "DATA lv_chars(20 TYPE c.",
                &BTreeMap::new(),
                &Options::default()
            )
            .is_err()
        );
    }
}
