//! Block structure of a command sequence: nesting level per command, derived from the grammar's
//! statement names. Mirrors abap-cleaner's level opener/closer tables (`Command.addLevelOpener`):
//! middle commands such as ELSE or WHEN close the level of their predecessor and open a new one,
//! and event blocks (`START-OF-SELECTION`, ...) are closed by the next event, CLASS, FORM or MODULE.
use abapfmt_lexer::{Token, TokenKind};

use crate::Command;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    Function,
    Method,
    Module,
    Form,
    Define,
    Class,
    Interface,
    Do,
    While,
    Loop,
    At,
    Case,
    If,
    Try,
    Select,
    With,
    TestSeam,
    TestInjection,
    Enhancement,
    EnhancementSection,
    Provide,
    OnChange,
    CatchSystemExceptions,
    Event,
    AuthorityCheckDisable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pos {
    Open,
    Mid,
    End,
}

const EVENTS: &[&str] = &[
    "AtLineSelection",
    "AtSelectionScreen",
    "AtUserCommand",
    "EndOfPage",
    "Initialization",
    "LoadOfProgram",
    "StartOfSelection",
    "EndOfSelection",
    "TopOfPage",
];

fn role(name: &str) -> Option<(Family, Pos)> {
    use Family::*;
    use Pos::*;
    Some(match name {
        "FunctionModule" => (Function, Open),
        "EndFunction" => (Function, End),
        "MethodImplementation" => (Method, Open),
        "EndMethod" => (Method, End),
        "Module" => (Module, Open),
        "EndModule" => (Module, End),
        "Form" => (Form, Open),
        "EndForm" => (Form, End),
        "Define" => (Define, Open),
        "EndOfDefinition" => (Define, End),
        "ClassDefinition" | "ClassImplementation" => (Class, Open),
        "Public" | "Protected" | "Private" => (Class, Mid),
        "EndClass" => (Class, End),
        "Interface" => (Interface, Open),
        "EndInterface" => (Interface, End),
        "Do" => (Do, Open),
        "EndDo" => (Do, End),
        "While" => (While, Open),
        "EndWhile" => (While, End),
        "Loop" | "LoopAtScreen" | "LoopExtract" => (Loop, Open),
        "EndLoop" => (Loop, End),
        "At" | "AtFirst" | "AtLast" => (At, Open),
        "EndAt" => (At, End),
        "Case" | "CaseType" => (Case, Open),
        "When" | "WhenOthers" | "WhenType" => (Case, Mid),
        "EndCase" => (Case, End),
        "If" => (If, Open),
        "ElseIf" | "Else" => (If, Mid),
        "EndIf" => (If, End),
        "Try" => (Try, Open),
        "Catch" | "Cleanup" => (Try, Mid),
        "EndTry" => (Try, End),
        "SelectLoop" => (Select, Open),
        "EndSelect" => (Select, End),
        "WithLoop" => (With, Open),
        "EndWith" => (With, End),
        "TestSeam" => (TestSeam, Open),
        "EndTestSeam" => (TestSeam, End),
        "TestInjection" => (TestInjection, Open),
        "EndTestInjection" => (TestInjection, End),
        "Enhancement" => (Enhancement, Open),
        "EndEnhancement" => (Enhancement, End),
        "EnhancementSection" => (EnhancementSection, Open),
        "EndEnhancementSection" => (EnhancementSection, End),
        "Provide" => (Provide, Open),
        "EndProvide" => (Provide, End),
        "OnChange" => (OnChange, Open),
        "EndOn" => (OnChange, End),
        "CatchSystemExceptions" => (CatchSystemExceptions, Open),
        "EndCatch" => (CatchSystemExceptions, End),
        "AuthorityCheckDisableBegin" => (AuthorityCheckDisable, Open),
        "AuthorityCheckDisableEnd" => (AuthorityCheckDisable, End),
        "Report" => (Event, Open),
        n if EVENTS.contains(&n) => (Event, Open),
        _ => return None,
    })
}

/// Statement name for the block keywords of a command the grammar could not parse.
pub(crate) fn guess_statement(first_word: &str) -> Option<&'static str> {
    Some(match first_word.to_ascii_uppercase().as_str() {
        "FUNCTION" => "FunctionModule",
        "ENDFUNCTION" => "EndFunction",
        "METHOD" => "MethodImplementation",
        "ENDMETHOD" => "EndMethod",
        "FORM" => "Form",
        "ENDFORM" => "EndForm",
        "MODULE" => "Module",
        "ENDMODULE" => "EndModule",
        "ENDCLASS" => "EndClass",
        "ENDINTERFACE" => "EndInterface",
        "IF" => "If",
        "ELSEIF" => "ElseIf",
        "ELSE" => "Else",
        "ENDIF" => "EndIf",
        "DO" => "Do",
        "ENDDO" => "EndDo",
        "WHILE" => "While",
        "ENDWHILE" => "EndWhile",
        "LOOP" => "Loop",
        "ENDLOOP" => "EndLoop",
        "CASE" => "Case",
        "WHEN" => "When",
        "ENDCASE" => "EndCase",
        "TRY" => "Try",
        "CATCH" => "Catch",
        "CLEANUP" => "Cleanup",
        "ENDTRY" => "EndTry",
        _ => return None,
    })
}

struct Entry {
    family: Family,
    event_block: bool,
    /// Indentation its children get relative to it.
    step: u32,
    /// Extra indentation for WHEN / CATCH / CLEANUP / PUBLIC SECTION and everything below.
    extra: u32,
    /// The outermost command of a nesting is a CLASS, INTERFACE or introductory statement.
    no_base: bool,
}

const INTRODUCTORY: &[&str] = &[
    "CLASS-POOL",
    "FUNCTION-POOL",
    "INTERFACE-POOL",
    "PROGRAM",
    "REPORT",
    "TYPE-POOL",
];

fn first_word<'a>(c: &Command, tokens: &[Token], src: &'a str) -> &'a str {
    tokens[c.first..=c.last]
        .iter()
        .find(|t| t.kind == TokenKind::Word)
        .map_or("", |t| t.text(src))
}

fn own_no_base(c: &Command, word: &str) -> bool {
    matches!(
        c.statement.filter(|_| !c.is_comment_only),
        Some("ClassDefinition" | "ClassImplementation" | "EndClass" | "Interface" | "EndInterface")
    ) || word.eq_ignore_ascii_case("CLASS")
        || INTRODUCTORY.iter().any(|w| word.eq_ignore_ascii_case(w))
}

/// Number of `BEGIN OF` / `END OF` sequences in a selection screen element.
fn begin_end_counts(c: &Command, tokens: &[Token], src: &str) -> (i32, i32) {
    let words: Vec<&str> = tokens[c.first..=c.last]
        .iter()
        .filter(|t| t.kind != TokenKind::Comment)
        .map(|t| t.text(src))
        .collect();
    let count = |a: &str| {
        words
            .windows(2)
            .filter(|w| w[0].eq_ignore_ascii_case(a) && w[1].eq_ignore_ascii_case("OF"))
            .count() as i32
    };
    (count("BEGIN"), count("END"))
}

fn is_selection_screen_element(word: &str) -> bool {
    ["SELECTION-SCREEN", "PARAMETERS", "SELECT-OPTIONS"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
}

pub(crate) fn assign_levels(commands: &mut [Command], tokens: &[Token], src: &str) {
    let mut stack: Vec<Entry> = Vec::new();
    let mut in_class_definition = false;
    let mut in_interface_definition = false;
    let mut indent_add = 0i32;
    let mut prev_counts: Option<(i32, i32)> = None;
    for c in commands.iter_mut() {
        let word = first_word(c, tokens, src);
        let no_base = own_no_base(c, word);
        assign_one(
            c,
            no_base,
            &mut stack,
            &mut in_class_definition,
            &mut in_interface_definition,
        );
        c.depth_after = stack.len() as u32;

        // BEGIN OF ... END OF blocks of selection screen elements indent their content
        if let Some((b, e)) = prev_counts.take()
            && b > e
        {
            indent_add += b - e;
        }
        if !c.is_comment_only && is_selection_screen_element(word) {
            let (b, e) = begin_end_counts(c, tokens, src);
            if e > b {
                indent_add -= e - b;
            }
            prev_counts = Some((b, e));
        }
        c.indent_rel = (c.indent_rel as i32 + 2 * indent_add).max(0) as u32;
    }
}

fn set_indent(c: &mut Command, stack: &[Entry], own_extra: u32, own_no_base: bool) {
    c.indent_rel = stack.iter().map(|e| e.step + e.extra).sum::<u32>() + own_extra;
    c.root_no_base = stack.first().map_or(own_no_base, |e| e.no_base);
}

fn assign_one(
    c: &mut Command,
    own_no_base: bool,
    stack: &mut Vec<Entry>,
    in_class_definition: &mut bool,
    in_interface_definition: &mut bool,
) {
    c.level = stack.len() as u32;
    set_indent(c, stack, 0, own_no_base);
    match c.statement.filter(|_| !c.is_comment_only) {
        Some("ClassDefinition") => *in_class_definition = true,
        Some("ClassImplementation") => *in_class_definition = false,
        _ => {}
    }
    c.in_class_definition = *in_class_definition;
    if c.statement.filter(|_| !c.is_comment_only) == Some("Interface") {
        *in_interface_definition = true;
    }
    c.in_interface_definition = *in_interface_definition;
    if c.statement == Some("EndInterface") {
        *in_interface_definition = false;
    }
    c.in_oo_context = stack
        .iter()
        .any(|e| matches!(e.family, Family::Class | Family::Interface | Family::Method))
        || matches!(
            c.statement.filter(|_| !c.is_comment_only),
            Some("ClassDefinition" | "ClassImplementation" | "Interface" | "MethodImplementation")
        );
    if c.statement == Some("EndClass") {
        *in_class_definition = false;
    }
    let Some(name) = c.statement.filter(|_| !c.is_comment_only) else {
        return;
    };
    let Some((mut family, mut pos)) = role(name) else {
        return;
    };
    let is_event_opener = family == Family::Event && EVENTS.contains(&name);
    c.starts_event_block = is_event_opener;

    // ELSE belongs to the innermost IF or ON CHANGE OF
    if name == "Else"
        && let Some(top) = stack.last()
        && top.family == Family::OnChange
    {
        family = Family::OnChange;
    }
    // an event block ends where the next event, CLASS, FORM or MODULE begins
    let ends_events = (family == Family::Event
        || matches!(family, Family::Module | Family::Form)
        || (family == Family::Class && pos == Pos::Open))
        && pos == Pos::Open;
    if ends_events {
        while let Some(top) = stack.last() {
            if top.family != Family::Event {
                break;
            }
            c.ends_event_block |= top.event_block;
            stack.pop();
        }
        c.level = stack.len() as u32;
        set_indent(c, stack, 0, own_no_base);
    }
    if name == "Else" && family == Family::OnChange {
        pos = Pos::Mid;
    }

    match pos {
        Pos::Open => {
            stack.push(Entry {
                family,
                event_block: is_event_opener,
                // the body of TRY is indented by 4; lines directly following REPORT are not indented
                step: match name {
                    "Try" => 4,
                    "Report" => 0,
                    _ => 2,
                },
                extra: 0,
                no_base: own_no_base,
            });
        }
        Pos::Mid => {
            if let Some(popped) = stack.pop_if(|t| t.family == family) {
                c.level = stack.len() as u32;
                let extra = if matches!(
                    name,
                    "When"
                        | "WhenOthers"
                        | "WhenType"
                        | "Catch"
                        | "Cleanup"
                        | "Public"
                        | "Protected"
                        | "Private"
                ) {
                    2
                } else {
                    0
                };
                set_indent(c, stack, extra, popped.no_base);
                stack.push(Entry {
                    family,
                    event_block: false,
                    step: 2,
                    extra,
                    no_base: popped.no_base,
                });
            }
        }
        Pos::End => {
            if let Some(i) = stack.iter().rposition(|t| t.family == family) {
                stack.truncate(i);
            }
            c.level = stack.len() as u32;
            set_indent(c, stack, 0, own_no_base);
        }
    }
}
