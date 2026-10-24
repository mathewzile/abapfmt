//! The rules, one module per upstream rule group.
use crate::RuleFn;

mod empty_lines;

pub const ALL: &[(&str, RuleFn)] = &[
    ("EMPTY_LINES_OUTSIDE_METHODS", empty_lines::outside_methods),
    ("EMPTY_LINES_WITHIN_METHODS", empty_lines::within_methods),
];
