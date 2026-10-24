//! EMPTY_LINES_* rules.
use crate::{ConfigExt, Doc, RuleConfig};

/// EMPTY_LINES_OUTSIDE_METHODS: separate consecutive methods, classes and interfaces.
pub fn outside_methods(doc: &mut Doc, config: &RuleConfig) {
    let between_classes = config.int("EmptyLinesBetweenClasses", 2) as u32;
    let between_methods = config.int("EmptyLinesBetweenMethods", 1) as u32;
    let class_and_method = config.int("EmptyLinesBetweenClassAndMethod", 0) as u32;

    let cmds = doc.commands().to_vec();
    for (ci, cmd) in cmds.iter().enumerate() {
        let class_or_interface_start = cmd.is_class_or_interface_start();
        let class_end = cmd.is_class_end();
        let interface_end = cmd.is_interface_end();
        if !(class_or_interface_start
            || class_end
            || interface_end
            || cmd.is_method_function_or_form_start())
        {
            continue;
        }
        // move to the first comment line that possibly precedes the class or method
        let mut change = ci;
        while doc.lines[cmds[change].first] == 1 && change > 0 && cmds[change - 1].is_comment_only {
            change -= 1;
        }
        let Some(prev) = change.checked_sub(1).map(|p| &cmds[p]) else {
            continue;
        };
        let prev_matches = if class_or_interface_start {
            prev.is_class_or_interface_end()
        } else if class_end {
            prev.is_method_function_or_form_end()
        } else {
            prev.is_method_function_or_form_end() || prev.is_class_start()
        };
        if !prev_matches {
            continue;
        }
        let breaks = if class_or_interface_start {
            between_classes + 1
        } else if class_end || prev.is_class_start() {
            class_and_method + 1
        } else {
            between_methods + 1
        };
        doc.set_lines(cmds[change].first, breaks);
    }
}

/// EMPTY_LINES_WITHIN_METHODS: limit empty lines in methods and separate declarations from the
/// first executable statement.
#[allow(clippy::if_same_then_else)] // keeps the structure of the upstream condition chain
pub fn within_methods(doc: &mut Doc, config: &RuleConfig) {
    let max_at_start = config.int("MaxEmptyLinesAtMethodStart", 0) as u32;
    let max_within = config.int("MaxEmptyLinesWithinMethods", 1) as u32;
    let max_at_end = config.int("MaxEmptyLinesAtMethodEnd", 0) as u32;
    let empty_line_above_first_executable =
        config.bool("EmptyLineAboveFirstExecutable", true) && max_within > 0;

    let cmds = doc.commands().to_vec();
    // a "method" here includes event blocks in reports
    let mut in_method = !cmds
        .iter()
        .any(|c| c.is_method_function_form_or_event_block_start());
    let (mut declaration_found, mut executable_found) = (false, false);

    for (ci, cmd) in cmds.iter().enumerate() {
        if cmd.ends_event_block
            || cmd.is_class_or_interface_start()
            || cmd.is_class_or_interface_end()
        {
            in_method = false;
        }
        let is_decl = doc.is_declaration(cmd) || doc.is_declaration_include(cmd);
        if in_method && !declaration_found && is_decl {
            declaration_found = true;
        }
        if in_method {
            let insert_above = if !empty_line_above_first_executable {
                false
            } else if executable_found
                || is_decl
                || cmd.is_comment_only
                || cmd.is_method_function_or_form_end()
            {
                false
            } else {
                executable_found = true;
                declaration_found
            };

            // line breaks at method start / end
            let mut first = cmd.first;
            if ci > 0 {
                let prev = &cmds[ci - 1];
                if prev.is_method_function_form_or_event_block_start()
                    || cmd.is_method_function_or_form_end()
                {
                    let mut max_break = 1 + if cmd.is_method_function_or_form_end() {
                        max_at_end
                    } else {
                        max_at_start
                    };
                    // function modules keep extra lines for the comments SE37 generates
                    if prev.is_function_start() {
                        let has_parameters = doc
                            .first_code_token(prev)
                            .and_then(|f| doc.next_code_token(f, prev.last)) // function name
                            .and_then(|name| doc.next_code_token(name, prev.last))
                            .is_some_and(|after| {
                                doc.syn.types[after] != abapfmt_syntax::TokenType::Period
                            });
                        max_break += if has_parameters { 3 } else { 1 };
                    }
                    if doc.lines[first] > max_break {
                        doc.set_lines(first, max_break);
                    }
                    first += 1; // the first token is not subject to the "within" limit
                }
            }
            for t in first..=cmd.last {
                if doc.lines[t] > max_within + 1 {
                    doc.set_lines(t, max_within + 1);
                }
            }

            if insert_above {
                let mut change = ci;
                while doc.lines[cmds[change].first] == 1
                    && change > 0
                    && cmds[change - 1].is_comment_only
                {
                    change -= 1;
                }
                if doc.lines[cmds[change].first] == 1 {
                    doc.set_lines(cmds[change].first, 2);
                }
            }
        }

        if cmd.is_method_function_or_form_end() {
            in_method = false;
        }
        if cmd.is_method_function_form_or_event_block_start() {
            in_method = true;
            declaration_found = false;
            executable_found = false;
        }
    }
}
