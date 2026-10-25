//! Layout/case rules producing text edits.
//!
//! A rule never changes tokens: it only edits whitespace, line layout and letter case. The
//! runtime guard for that is [`same_tokens`], which the CLI/LSP apply to every result.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

mod align;
mod doc;
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
    let Some(mut doc) = Doc::new(src) else {
        return Some(src.to_string());
    };
    // DDL rules only apply to CDS sources and ABAP rules only to ABAP
    if rule_id.starts_with("DDL_") != doc.syn.ddl.is_some() {
        return Some(src.to_string());
    }
    rule(&mut doc, config);
    Some(doc.render())
}

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
) -> Result<String, String> {
    let out = format(src, configs);
    same_tokens(src, &out).map(|()| out)
}

/// Run all implemented rules in order. `configs` maps a rule id to its options (defaults for
/// absent ones); a rule mapped to `None` is switched off. The source is re-parsed between rules,
/// because a rule may move tokens.
pub fn format(src: &str, configs: &BTreeMap<String, Option<RuleConfig>>) -> String {
    let default = RuleConfig::new();
    let mut text = src.to_string();
    for (id, _) in rules() {
        let config = match configs.get(*id) {
            Some(None) => continue,
            Some(Some(c)) => c,
            None => &default,
        };
        let config = with_cross_rule_settings(id, config, configs);
        if let Some(out) = run_rule(id, &text, &config) {
            text = out;
        }
    }
    text
}

/// Is `after` the same token stream as `before`? Only whitespace and the case of words,
/// operators and pragmas may differ. Code tokens (everything but comments) are compared in order;
/// comments and pragmas are compared as separate streams (comments ignoring whitespace inside them,
/// pragmas ignoring case), because rules may move a closing bracket or period in front of a
/// trailing comment or pragma and re-indent commented-out code.
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
        (code, comments, pragmas)
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
}
