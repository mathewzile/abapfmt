//! UPPER_AND_LOWER_CASE: keywords, identifiers and pragmas to upper or lower case, depending on
//! whether they are inside a CLASS/INTERFACE definition or anywhere else.
use abapfmt_syntax::{Command, TokenType};

use crate::{ConfigExt, Doc, RuleConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Unchanged,
    Lower,
    Upper,
}

fn style_of(name: &str) -> Style {
    match name {
        "LOWER_CASE" => Style::Lower,
        "UPPER_CASE" => Style::Upper,
        _ => Style::Unchanged,
    }
}

fn style_for(upper_minus_lower: i32) -> Style {
    match upper_minus_lower {
        n if n > 0 => Style::Upper,
        n if n < 0 => Style::Lower,
        _ => Style::Unchanged,
    }
}

fn apply(text: &str, style: Style) -> String {
    match style {
        Style::Lower => text.to_lowercase(),
        Style::Upper => text.to_uppercase(),
        Style::Unchanged => text.to_string(),
    }
}

fn is_mixed_case(text: &str) -> bool {
    text.to_uppercase() != text && text.to_lowercase() != text
}

fn pragma_parameters(text: &str) -> &str {
    text.find('[').map_or("", |p| &text[p..])
}

fn pragma_without_parameters(text: &str) -> &str {
    text.find('[').map_or(text, |p| &text[..p])
}

/// `TEXT-001`, `TEXT-a01`: a keyword, a dash and a three character id.
fn may_be_text_symbol(text: &str) -> bool {
    text.len() == 8 && text[..5].eq_ignore_ascii_case("TEXT-")
}

fn is_textual_comparison_op(doc: &Doc, i: usize) -> bool {
    doc.syn.types[i] == TokenType::ComparisonOp
        && doc.syn.text(i).starts_with(|c: char| c.is_alphabetic())
}

/// abap-cleaner's `ABAP.isCharAllowedForVariableNames`
fn is_var_char(chars: &[char], pos: usize, first: bool, oo: bool) -> bool {
    let c = chars[pos];
    if c.is_ascii_alphabetic() || c == '_' || c == '/' {
        return true;
    }
    if !first && c.is_ascii_digit() {
        return true;
    }
    if c == '%' && chars.get(pos + 1) == Some(&'_') {
        return true;
    }
    if oo {
        return false;
    }
    if matches!(c, '%' | '$' | '*' | '?') {
        return true;
    }
    if first && matches!(c, '&' | '<') {
        return true;
    }
    if !first && c == '#' {
        return true;
    }
    c == '>' && (pos + 1 == chars.len() || !is_var_char(chars, pos + 1, false, oo))
}

/// Split `any_class=>any_method(` into runs of variable-name chars and of the characters between.
fn split_identifier(id: &str, oo: bool) -> Vec<String> {
    let chars: Vec<char> = id.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    if chars.first() == Some(&'@') {
        out.push("@".to_string());
        start = 1;
    }
    while start < chars.len() {
        let is_var = is_var_char(&chars, start, true, oo);
        let mut end = start + 1;
        while end < chars.len() && is_var == is_var_char(&chars, end, false, oo) {
            end += 1;
        }
        out.push(chars[start..end].iter().collect());
        start = end;
    }
    out
}

fn change_identifier_keeping_mixed_case(text: &str, style: Style, oo: bool) -> String {
    split_identifier(text, oo)
        .into_iter()
        .map(|bit| {
            if is_mixed_case(&bit) {
                bit
            } else {
                apply(&bit, style)
            }
        })
        .collect()
}

struct Styles {
    def_keyword: Style,
    def_identifier: Style,
    impl_keyword: Style,
    impl_identifier: Style,
    pragma: Style,
    pragma_parameter: Style,
}

fn is_in_definition_section(c: &Command) -> bool {
    c.in_class_definition || c.in_interface_definition
}

fn count_upper_minus_lower(
    doc: &Doc,
    cmd: &Command,
    kind: TokenType,
    all: bool,
    parameters_only: bool,
) -> i32 {
    let mut n = 0;
    for i in cmd.first..=cmd.last {
        let ty = doc.syn.types[i];
        if ty == TokenType::Comment {
            continue;
        }
        if ty == kind || (kind == TokenType::Keyword && is_textual_comparison_op(doc, i)) {
            let mut text = doc.syn.text(i);
            if kind == TokenType::Pragma {
                text = if parameters_only {
                    pragma_parameters(text)
                } else {
                    pragma_without_parameters(text)
                };
                if text.is_empty() {
                    continue;
                }
            }
            let (upper, lower) = (text.to_uppercase(), text.to_lowercase());
            // only fully upper or fully lower case counts (not CamelCase, nor the field symbol `<>`)
            if text == upper && text != lower {
                n += 1;
            } else if text == lower && text != upper {
                n -= 1;
            }
            if !all && n != 0 {
                break;
            }
        }
    }
    n
}

fn derive_styles(doc: &Doc, all: bool) -> Styles {
    let (mut dk, mut di, mut ik, mut ii, mut fk, mut fi, mut pu, mut pp) = (0, 0, 0, 0, 0, 0, 0, 0);
    let (mut def_found, mut impl_found, mut pragma_found) = (false, false, false);
    let mut in_impl = false;
    for cmd in doc.commands().iter().filter(|c| !c.is_comment_only) {
        if !all && def_found && impl_found && pragma_found {
            break;
        }
        let is_class_impl_start = cmd.statement == Some("ClassImplementation");
        if is_class_impl_start {
            in_impl = true;
        }
        if is_in_definition_section(cmd) {
            if all || dk == 0 {
                dk += count_upper_minus_lower(doc, cmd, TokenType::Keyword, all, false);
            }
            if all || di == 0 {
                di += count_upper_minus_lower(doc, cmd, TokenType::Identifier, all, false);
            }
            def_found = !all && dk != 0 && di != 0;
        } else if is_class_impl_start || (cmd.is_class_end() && in_impl) {
            // SE80 upper-cases CLASS ... IMPLEMENTATION and ENDCLASS only, so they only serve as a fallback
            if all || fk == 0 {
                fk += count_upper_minus_lower(doc, cmd, TokenType::Keyword, all, false);
            }
            if all || fi == 0 {
                fi += count_upper_minus_lower(doc, cmd, TokenType::Identifier, all, false);
            }
        } else {
            if all || ik == 0 {
                ik += count_upper_minus_lower(doc, cmd, TokenType::Keyword, all, false);
            }
            if all || ii == 0 {
                ii += count_upper_minus_lower(doc, cmd, TokenType::Identifier, all, false);
            }
            impl_found = !all && ik != 0 && ii != 0;
        }
        if cmd.is_class_end() {
            in_impl = false;
        }
        if all || pu == 0 {
            pu += count_upper_minus_lower(doc, cmd, TokenType::Pragma, all, false);
        }
        if all || pp == 0 {
            pp += count_upper_minus_lower(doc, cmd, TokenType::Pragma, all, true);
        }
        pragma_found = !all && pu != 0 && pp != 0;
    }
    if ik == 0 {
        ik = fk;
    }
    if ii == 0 {
        ii = fi;
    }
    Styles {
        def_keyword: style_for(dk),
        def_identifier: style_for(di),
        impl_keyword: style_for(ik),
        impl_identifier: style_for(ii),
        pragma: style_for(pu),
        pragma_parameter: style_for(pp),
    }
}

pub fn upper_and_lower_case(doc: &mut Doc, config: &RuleConfig) {
    let styles = match config.choice("DeriveCaseMethod", "NONE").as_str() {
        "FROM_FIRST_TOKEN" => derive_styles(doc, false),
        "FROM_MAJORITY" => derive_styles(doc, true),
        _ => Styles {
            def_keyword: style_of(&config.choice("DefinitionKeywordStyle", "UPPER_CASE")),
            def_identifier: style_of(&config.choice("DefinitionIdentifierStyle", "LOWER_CASE")),
            impl_keyword: style_of(&config.choice("ImplementationKeywordStyle", "UPPER_CASE")),
            impl_identifier: style_of(
                &config.choice("ImplementationIdentifierStyle", "LOWER_CASE"),
            ),
            pragma: style_of(&config.choice("PragmaStyle", "UPPER_CASE")),
            pragma_parameter: style_of(&config.choice("PragmaParameterStyle", "UPPER_CASE")),
        },
    };
    let keep_mixed = config.bool("KeepMixedCaseInIdentifiers", true);

    let cmds = doc.commands().to_vec();
    for cmd in cmds.iter().filter(|c| !c.is_comment_only) {
        let in_def = is_in_definition_section(cmd);
        let keyword_style = if in_def {
            styles.def_keyword
        } else {
            styles.impl_keyword
        };
        let identifier_style = if in_def {
            styles.def_identifier
        } else {
            styles.impl_identifier
        };
        let oo = cmd.in_oo_context;
        for i in cmd.first..=cmd.last {
            let ty = doc.syn.types[i];
            let text = doc.syn.text(i).to_string();
            let is_word = matches!(ty, TokenType::Keyword | TokenType::Identifier);
            let changed = if is_word && may_be_text_symbol(&text) {
                // TEXT-001: keyword TEXT, dash, alphanumeric id
                let (prefix, id) = text.split_at(5);
                Some(format!(
                    "{}{}",
                    apply(prefix, keyword_style),
                    apply(id, identifier_style)
                ))
            } else if (ty == TokenType::Keyword || is_textual_comparison_op(doc, i))
                && keyword_style != Style::Unchanged
            {
                Some(apply(&text, keyword_style))
            } else if ty == TokenType::Identifier && identifier_style != Style::Unchanged {
                Some(if keep_mixed {
                    change_identifier_keeping_mixed_case(&text, identifier_style, oo)
                } else {
                    apply(&text, identifier_style)
                })
            } else if ty == TokenType::Pragma
                && (styles.pragma != Style::Unchanged
                    || styles.pragma_parameter != Style::Unchanged)
            {
                let mut pragma = pragma_without_parameters(&text).to_string();
                let mut parameters = pragma_parameters(&text).to_string();
                if styles.pragma != Style::Unchanged {
                    pragma = apply(&pragma, styles.pragma);
                }
                if styles.pragma_parameter != Style::Unchanged && !parameters.is_empty() {
                    parameters = if keep_mixed {
                        change_identifier_keeping_mixed_case(
                            &parameters,
                            styles.pragma_parameter,
                            oo,
                        )
                    } else {
                        apply(&parameters, styles.pragma_parameter)
                    };
                }
                Some(pragma + &parameters)
            } else {
                None
            };
            if let Some(new) = changed
                && new != text
            {
                doc.text[i] = Some(new);
            }
        }
    }
}
