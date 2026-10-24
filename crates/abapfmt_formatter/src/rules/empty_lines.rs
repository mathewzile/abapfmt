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
