//! Block structure of a command sequence: nesting level per command, derived from the grammar's
//! statement names. Mirrors abap-cleaner's level opener/closer tables (`Command.addLevelOpener`):
//! middle commands such as ELSE or WHEN close the level of their predecessor and open a new one,
//! and event blocks (`START-OF-SELECTION`, ...) are closed by the next event, CLASS, FORM or MODULE.
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
}

pub(crate) fn assign_levels(commands: &mut [Command]) {
    let mut stack: Vec<Entry> = Vec::new();
    let mut in_class_definition = false;
    let mut in_interface_definition = false;
    for c in commands.iter_mut() {
        assign_one(
            c,
            &mut stack,
            &mut in_class_definition,
            &mut in_interface_definition,
        );
        c.depth_after = stack.len() as u32;
    }
}

fn assign_one(
    c: &mut Command,
    stack: &mut Vec<Entry>,
    in_class_definition: &mut bool,
    in_interface_definition: &mut bool,
) {
    {
        c.level = stack.len() as u32;
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
                Some(
                    "ClassDefinition"
                        | "ClassImplementation"
                        | "Interface"
                        | "MethodImplementation"
                )
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
        }
        if name == "Else" && family == Family::OnChange {
            pos = Pos::Mid;
        }

        match pos {
            Pos::Open => {
                stack.push(Entry {
                    family,
                    event_block: is_event_opener,
                });
            }
            Pos::Mid => {
                if stack.last().is_some_and(|t| t.family == family) {
                    stack.pop();
                    c.level = stack.len() as u32;
                    stack.push(Entry {
                        family,
                        event_block: false,
                    });
                }
            }
            Pos::End => {
                if let Some(i) = stack.iter().rposition(|t| t.family == family) {
                    stack.truncate(i);
                }
                c.level = stack.len() as u32;
            }
        }
    }
}
