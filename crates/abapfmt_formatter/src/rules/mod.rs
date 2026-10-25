//! The rules, one module per upstream rule group.
use crate::RuleFn;

mod align_abap_doc;
mod align_aliases_for;
mod align_assignments;
mod align_clear_free_and_sort;
mod align_cond_expressions;
pub(crate) mod align_declarations;
mod align_form_declaration;
mod align_logical_expressions;
mod align_methods_declaration;
mod align_methods_without_params;
mod align_parameters;
mod align_perform;
mod align_select_clauses;
mod align_select_from;
mod align_select_lists;
mod align_with_second_word;
mod case;
mod ddl_align;
mod ddl_empty_lines;
mod ddl_position;
mod ddl_spaces;
mod ddl_util;
mod empty_lines;
mod inset;
mod needless_spaces;
mod one_command_per_line;
mod select_query;
mod spaces;

/// In the order the rules run (upstream's registry order).
pub const ALL: &[(&str, RuleFn)] = &[
    ("DDL_ALIGN_DATA_SOURCES", ddl_align::data_sources),
    ("DDL_ALIGN_FIELD_LISTS", ddl_align::field_lists),
    ("DDL_ALIGN_ENTITY_PARAMETERS", ddl_align::entity_parameters),
    ("DDL_ALIGN_SOURCE_PARAMETERS", ddl_align::source_parameters),
    (
        "DDL_ALIGN_FUNCTION_PARAMETERS",
        ddl_align::function_parameters,
    ),
    ("DDL_POSITION_DEFINE", ddl_position::define),
    ("DDL_POSITION_SELECT", ddl_position::select),
    ("DDL_POSITION_BRACES", ddl_position::braces),
    ("DDL_POSITION_JOIN", ddl_position::join),
    ("DDL_POSITION_ASSOCIATION", ddl_position::association),
    ("DDL_POSITION_CLAUSES", ddl_position::clauses),
    ("DDL_EMPTY_LINES_BETWEEN", ddl_empty_lines::between_sections),
    ("DDL_EMPTY_LINES_WITHIN", ddl_empty_lines::within_sections),
    ("DDL_SPACES_AROUND_SIGNS", ddl_spaces::around_signs),
    ("DDL_SPACES_AROUND_BRACKETS", ddl_spaces::around_brackets),
    ("EMPTY_LINES_WITHIN_METHODS", empty_lines::within_methods),
    ("EMPTY_LINES_OUTSIDE_METHODS", empty_lines::outside_methods),
    (
        "EMPTY_LINES_IN_CLASS_DEFINITION",
        empty_lines::in_class_definition,
    ),
    (
        "ONE_COMMAND_PER_LINE",
        one_command_per_line::one_command_per_line,
    ),
    ("SPACE_BEFORE_PERIOD", spaces::space_before_period),
    (
        "SPACE_AROUND_COMMENT_SIGN",
        spaces::space_around_comment_sign,
    ),
    (
        "CLOSING_BRACKETS_POSITION",
        spaces::closing_brackets_position,
    ),
    (
        "SPACES_IN_EMPTY_BRACKETS",
        spaces::space_around_text_literal,
    ),
    ("NEEDLESS_SPACES", needless_spaces::needless_spaces),
    ("INSET", inset::inset),
    ("ALIGN_ASSIGNMENTS", align_assignments::align_assignments),
    (
        "ALIGN_WITH_SECOND_WORD",
        align_with_second_word::align_with_second_word,
    ),
    ("ALIGN_ALIASES_FOR", align_aliases_for::align_aliases_for),
    (
        "ALIGN_FORM_DECLARATION",
        align_form_declaration::align_form_declaration,
    ),
    ("ALIGN_PERFORM", align_perform::align_perform),
    (
        "ALIGN_CLEAR_FREE_AND_SORT",
        align_clear_free_and_sort::align_clear_free_and_sort,
    ),
    (
        "ALIGN_METHODS_DECLARATION",
        align_methods_declaration::align_methods_declaration,
    ),
    (
        "ALIGN_METHODS_FOR_TESTING",
        align_methods_without_params::align_methods_for_testing,
    ),
    (
        "ALIGN_METHODS_REDEFINITION",
        align_methods_without_params::align_methods_redefinition,
    ),
    ("ALIGN_ABAP_DOC", align_abap_doc::align_abap_doc),
    (
        "ALIGN_COND_EXPRESSIONS",
        align_cond_expressions::align_cond_expressions,
    ),
    (
        "ALIGN_SELECT_CLAUSES",
        align_select_clauses::align_select_clauses,
    ),
    ("ALIGN_SELECT_FROM", align_select_from::align_select_from),
    ("ALIGN_SELECT_LISTS", align_select_lists::align_select_lists),
    (
        "ALIGN_LOGICAL_EXPRESSIONS",
        align_logical_expressions::align_logical_expressions,
    ),
    ("ALIGN_PARAMETERS", align_parameters::align_parameters),
    ("ALIGN_DECLARATIONS", align_declarations::align_declarations),
    ("UPPER_AND_LOWER_CASE", case::upper_and_lower_case),
];
