//! abapfmt configuration (`abapfmt.json`) and abap-cleaner profile (`.cfj`) import.
//!
//! `abapfmt.json`:
//!
//! ```json
//! { "rules": { "ALIGN_PARAMETERS": { "MaxLineLength": 100 }, "INSET": { "enabled": false } } }
//! ```
//!
//! Rule ids and option names are those of abap-cleaner; enum options are written by name.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use abapfmt_formatter::{ConfigValue, RuleConfig};
use serde_json::Value;

/// Where a directory may keep its configuration, in order of precedence.
pub const CONFIG_LOCATIONS: &[&str] = &["abapfmt.json", ".vscode/abapfmt.json"];

/// The options per rule id; a rule mapped to `None` is switched off, absent rules use defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub rules: BTreeMap<String, Option<RuleConfig>>,
    /// Maximum line width (default 120); `None` switches the reflow off.
    pub line_width: Option<usize>,
    /// Maximum length of table rows in tabular style (default 160).
    pub tabular_line_width: usize,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            rules: BTreeMap::new(),
            line_width: Some(abapfmt_formatter::DEFAULT_LINE_WIDTH),
            tabular_line_width: abapfmt_formatter::DEFAULT_TABULAR_LINE_WIDTH,
        }
    }
}

impl Config {
    /// The formatter options of this configuration.
    pub fn options(&self) -> abapfmt_formatter::Options {
        abapfmt_formatter::Options {
            line_width: self.line_width,
            // the rule option, if set, wins over the top-level setting
            tabular_line_width: match self
                .rules
                .get("ALIGN_PARAMETERS")
                .and_then(|r| r.as_ref())
                .and_then(|r| r.get("MaxLineLengthForSingleLine"))
            {
                Some(abapfmt_formatter::ConfigValue::Int(w)) if *w > 0 => *w as usize,
                _ => self.tabular_line_width,
            },
        }
    }
}

/// A loaded configuration together with things the user should know about.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
enum Kind {
    Bool,
    Int,
    Str,
    Enum(Vec<String>),
}

type Settings = BTreeMap<String, BTreeMap<String, Kind>>;

/// Rule id to option name to option type, generated from the upstream rule classes.
fn settings() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let root: Value = serde_json::from_str(include_str!("../settings.json"))
            .expect("settings.json is valid JSON");
        let mut result = Settings::new();
        for (rule, options) in root.as_object().into_iter().flatten() {
            let mut map = BTreeMap::new();
            for (name, spec) in options.as_object().into_iter().flatten() {
                let kind = match spec["t"].as_str() {
                    Some("bool") => Kind::Bool,
                    Some("int") => Kind::Int,
                    Some("enum") => Kind::Enum(
                        spec["names"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect(),
                    ),
                    _ => Kind::Str,
                };
                map.insert(name.clone(), kind);
            }
            result.insert(rule.clone(), map);
        }
        result
    })
}

fn is_implemented(rule: &str) -> bool {
    abapfmt_formatter::rules().iter().any(|(id, _)| *id == rule)
}

/// Converts a JSON value to an option value of the given type.
fn convert(kind: &Kind, value: &Value) -> Option<ConfigValue> {
    match (kind, value) {
        (Kind::Bool, Value::Bool(b)) => Some(ConfigValue::Bool(*b)),
        (Kind::Int, Value::Number(n)) => n.as_i64().map(ConfigValue::Int),
        (Kind::Str, Value::String(s)) => Some(ConfigValue::Str(s.clone())),
        (Kind::Enum(names), Value::String(s)) => {
            let name = s.rsplit('.').next().unwrap_or(s);
            names
                .iter()
                .find(|n| n.eq_ignore_ascii_case(name))
                .map(|n| ConfigValue::Str(n.clone()))
        }
        _ => None,
    }
}

/// Converts a `.cfj` setting, which is always stored as text (enums as their ordinal).
fn convert_cfj(kind: &Kind, text: &str) -> Option<ConfigValue> {
    match kind {
        Kind::Bool => match text {
            "true" => Some(ConfigValue::Bool(true)),
            "false" => Some(ConfigValue::Bool(false)),
            _ => None,
        },
        Kind::Int => text.trim().parse().ok().map(ConfigValue::Int),
        Kind::Str => Some(ConfigValue::Str(text.to_string())),
        Kind::Enum(names) => match text.trim().parse::<usize>() {
            Ok(i) => names.get(i).map(|n| ConfigValue::Str(n.clone())),
            Err(_) => convert(kind, &Value::String(text.to_string())),
        },
    }
}

/// Parses the text of an `abapfmt.json`.
pub fn parse_json(text: &str) -> Result<Loaded, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = root
        .as_object()
        .ok_or("the configuration must be a JSON object")?;
    let mut loaded = Loaded::default();
    for (key, value) in obj {
        match key.as_str() {
            "$schema" => {}
            "lineWidth" => {
                loaded.config.line_width = match value {
                    Value::String(s) if s == "off" => None,
                    Value::Number(n) => match n.as_u64() {
                        Some(w) if (40..=1000).contains(&w) => Some(w as usize),
                        _ => return Err("\"lineWidth\" must be between 40 and 1000".into()),
                    },
                    _ => return Err("\"lineWidth\" must be a number or \"off\"".into()),
                };
            }
            "tabularLineWidth" => match value.as_u64() {
                Some(w) if (40..=1000).contains(&w) => {
                    loaded.config.tabular_line_width = w as usize;
                }
                _ => return Err("\"tabularLineWidth\" must be between 40 and 1000".into()),
            },
            "rules" => {
                let rules = value.as_object().ok_or("\"rules\" must be an object")?;
                for (id, options) in rules {
                    read_rule(&mut loaded, id, options)?;
                }
            }
            other => return Err(format!("unknown setting \"{other}\"")),
        }
    }
    Ok(loaded)
}

fn read_rule(loaded: &mut Loaded, id: &str, options: &Value) -> Result<(), String> {
    let Some(known) = settings().get(id) else {
        return Err(format!("unknown rule \"{id}\""));
    };
    if !is_implemented(id) {
        loaded
            .warnings
            .push(format!("rule {id} is not supported and ignored"));
        return Ok(());
    }
    // a plain boolean switches the rule on or off
    if let Value::Bool(enabled) = options {
        if !enabled {
            loaded.config.rules.insert(id.to_string(), None);
        }
        return Ok(());
    }
    let options = options
        .as_object()
        .ok_or_else(|| format!("rule {id}: expected an object or a boolean"))?;
    let mut config = RuleConfig::new();
    let mut enabled = true;
    for (name, value) in options {
        if name == "enabled" {
            enabled = value
                .as_bool()
                .ok_or_else(|| format!("rule {id}: \"enabled\" must be a boolean"))?;
            continue;
        }
        let kind = known
            .get(name)
            .ok_or_else(|| format!("rule {id}: unknown option \"{name}\""))?;
        let converted = convert(kind, value)
            .ok_or_else(|| format!("rule {id}: invalid value for \"{name}\": {value}"))?;
        config.insert(name.clone(), converted);
    }
    loaded
        .config
        .rules
        .insert(id.to_string(), enabled.then_some(config));
    Ok(())
}

/// Imports an abap-cleaner profile (`.cfj`). Rules that abapfmt does not implement are ignored.
pub fn import_cfj(text: &str) -> Result<Loaded, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("invalid profile: {e}"))?;
    let rules = root["rules"]
        .as_array()
        .ok_or("the profile contains no \"rules\" array")?;
    let mut loaded = Loaded::default();
    let mut ignored = Vec::new();
    for rule in rules {
        let Some(id) = rule["ruleID"].as_str() else {
            continue;
        };
        let active = rule["isActive"].as_bool().unwrap_or(false);
        let Some(known) = settings().get(id).filter(|_| is_implemented(id)) else {
            if active {
                ignored.push(id.to_string());
            }
            continue;
        };
        if !active {
            loaded.config.rules.insert(id.to_string(), None);
            continue;
        }
        let mut config = RuleConfig::new();
        for (name, value) in rule["settings"].as_object().into_iter().flatten() {
            let (Some(kind), Some(text)) = (known.get(name), value.as_str()) else {
                continue;
            };
            if let Some(v) = convert_cfj(kind, text) {
                config.insert(name.clone(), v);
            }
        }
        loaded.config.rules.insert(id.to_string(), Some(config));
    }
    if !ignored.is_empty() {
        loaded.warnings.push(format!(
            "ignored {} rules that abapfmt does not support: {}",
            ignored.len(),
            ignored.join(", ")
        ));
    }
    Ok(loaded)
}

/// Loads a configuration file; `.cfj` files are imported as abap-cleaner profiles.
pub fn load_file(path: &Path) -> Result<Loaded, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let result = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cfj"))
    {
        import_cfj(&text)
    } else {
        parse_json(&text)
    };
    result.map_err(|e| format!("{}: {e}", path.display()))
}

/// The configuration file of `dir` itself (see [`CONFIG_LOCATIONS`]).
pub fn config_in_dir(dir: &Path) -> Option<PathBuf> {
    CONFIG_LOCATIONS
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

/// The nearest configuration file in `dir` or one of its ancestors: for files in a local
/// checkout.
pub fn find_config(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find_map(config_in_dir)
}

/// The configuration file of the first workspace folder that has one: for documents that are not
/// local files (remote file systems), which have no directory to search upwards from.
pub fn find_config_in_workspace(folders: &[PathBuf]) -> Option<PathBuf> {
    folders.iter().find_map(|f| config_in_dir(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_options_are_typed() {
        let l = parse_json(r#"{"rules":{"ALIGN_PARAMETERS":{"MaxLineLength":100},"INSET":false}}"#)
            .unwrap();
        assert_eq!(
            l.config.rules["ALIGN_PARAMETERS"].as_ref().unwrap()["MaxLineLength"],
            ConfigValue::Int(100)
        );
        assert_eq!(l.config.rules["INSET"], None);
    }

    #[test]
    fn line_width_defaults_to_120_and_can_be_changed_or_switched_off() {
        assert_eq!(Config::default().line_width, Some(120));
        assert_eq!(parse_json("{}").unwrap().config.line_width, Some(120));
        assert_eq!(
            parse_json(r#"{"lineWidth":100}"#)
                .unwrap()
                .config
                .line_width,
            Some(100)
        );
        assert_eq!(
            parse_json(r#"{"lineWidth":"off"}"#)
                .unwrap()
                .config
                .line_width,
            None
        );
    }

    #[test]
    fn tabular_line_width_defaults_to_160_and_the_rule_option_wins() {
        assert_eq!(Config::default().options().tabular_line_width, 160);
        let c = parse_json(r#"{"tabularLineWidth":140}"#).unwrap().config;
        assert_eq!(c.options().tabular_line_width, 140);
        let c = parse_json(
            r#"{"tabularLineWidth":140,"rules":{"ALIGN_PARAMETERS":{"MaxLineLengthForSingleLine":130}}}"#,
        )
        .unwrap()
        .config;
        assert_eq!(c.options().tabular_line_width, 130);
        assert!(parse_json(r#"{"tabularLineWidth":"off"}"#).is_err());
    }

    #[test]
    fn config_is_found_in_the_vscode_folder() {
        let root = std::env::temp_dir().join(format!("abapfmt-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".vscode")).unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join(".vscode/abapfmt.json"), "{}").unwrap();
        let found = find_config(&root.join("src/deep")).unwrap();
        assert!(found.ends_with(std::path::Path::new(".vscode").join("abapfmt.json")));
        assert_eq!(
            find_config_in_workspace(&[root.join("src"), root.clone()]),
            Some(found)
        );
        // abapfmt.json in the folder itself wins
        std::fs::write(root.join("abapfmt.json"), "{}").unwrap();
        assert!(config_in_dir(&root).unwrap().ends_with("abapfmt.json"));
        assert!(
            !config_in_dir(&root)
                .unwrap()
                .to_string_lossy()
                .contains(".vscode")
        );
    }

    #[test]
    fn json_rejects_bad_values() {
        assert!(parse_json(r#"{"rules":{"ALIGN_PARAMETERS":{"MaxLineLength":"x"}}}"#).is_err());
        assert!(parse_json(r#"{"rules":{"ALIGN_PARAMETERS":{"Nope":1}}}"#).is_err());
        assert!(parse_json(r#"{"rules":{"NOPE":{}}}"#).is_err());
        assert!(parse_json(r#"{"lineWidth":10}"#).is_err());
        assert!(parse_json(r#"{"lineWidth":"wide"}"#).is_err());
        assert!(parse_json(r#"{"unknown":1}"#).is_err());
    }

    #[test]
    fn cfj_converts_ordinals_and_ignores_unsupported_rules() {
        let cfj = r#"{"requiredVersion":1,"ruleCount":2,"rules":[
            {"ruleID":"DDL_ALIGN_FIELD_LISTS","isActive":true,"settingCount":2,
             "settings":{"MaxLineLength":"100","NameListPos":"1"}},
            {"ruleID":"CAMEL_CASE_NAME","isActive":true,"settingCount":0,"settings":{}}]}"#;
        let l = import_cfj(cfj).unwrap();
        let c = l.config.rules["DDL_ALIGN_FIELD_LISTS"].as_ref().unwrap();
        assert_eq!(c["MaxLineLength"], ConfigValue::Int(100));
        assert_eq!(
            c["NameListPos"],
            ConfigValue::Str("LINE_START_PLUS_2".into())
        );
        assert_eq!(l.warnings.len(), 1);
        assert!(l.warnings[0].contains("CAMEL_CASE_NAME"));
    }
}
