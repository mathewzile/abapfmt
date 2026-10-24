//! Layout/case rules producing text edits.
//!
//! A rule never changes tokens: it only edits whitespace, line layout and letter case. The
//! runtime guard for that is [`same_tokens`], which the CLI/LSP apply to every result.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

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
    rule(&mut doc, config);
    Some(doc.render())
}

/// Is `after` the same token stream as `before`? Only whitespace and the case of words,
/// operators and pragmas may differ. Code tokens (everything but comments) are compared in order;
/// comments and pragmas are compared as separate streams (comments ignoring whitespace inside them,
/// pragmas ignoring case), because rules may move a closing bracket or period in front of a
/// trailing comment or pragma and re-indent commented-out code.
pub fn same_tokens(before: &str, after: &str) -> Result<(), String> {
    let (Ok(a), Ok(b)) = (lex(before), lex(after)) else {
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
    if ca.len() != cb.len() {
        return Err(format!(
            "code token count changed: {} -> {}",
            ca.len(),
            cb.len()
        ));
    }
    if let Some(i) = (0..ca.len()).find(|i| ca[*i] != cb[*i]) {
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
