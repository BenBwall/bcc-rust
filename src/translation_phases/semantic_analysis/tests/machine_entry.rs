//! x86 `interrupt` and `no_caller_saved_registers` and GCC's `naked`: each
//! compiler's rules in its words, as GCC 14.2 and Clang 23.1 report them.
//! GNU extension: GCC manual, "x86 Function Attributes".

use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    },
    target::Target,
    translation_phases::semantic_analysis::{
        MachineEntryFault as Fault,
        UnsupportedAttribute,
    },
};

fn configuration(identity: CompilerIdentity, general_regs: bool) -> CompilerConfiguration {
    CompilerConfiguration::default()
        .with_target(Target::LinuxGnu)
        .with_gnu_extensions(true)
        .with_compiler_identity(identity)
        .with_general_regs_only(general_regs)
}

/// The semantic diagnostics of `source` with their lines.
fn diagnostics(
    source: &str,
    identity: CompilerIdentity,
    general_regs: bool,
) -> Vec<(SemanticErrorKind, u32)> {
    let mut result = Vec::new();
    with_configuration(
        source,
        configuration(identity, general_regs),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    let position =
                        crate::translation_phases::GetPosition::position(&error, context);
                    result.push((error.kind, position.line));
                }
            }
        },
    );
    result
}

fn faults(source: &str, identity: CompilerIdentity) -> Vec<Fault> {
    diagnostics(source, identity, true)
        .into_iter()
        .filter_map(|(kind, _)| match kind {
            | SemanticErrorKind::InvalidMachineEntry(fault) => Some(fault),
            | _ => None,
        })
        .collect()
}

const VALID: &[&str] = &[
    "__attribute__((interrupt)) void f(void *p) {}",
    "struct frame; __attribute__((interrupt)) void f(struct frame *p, unsigned long e) {}",
    "__attribute__((interrupt)) void f(void *p, unsigned long long e) {}",
    "void f(void *p); __attribute__((interrupt)) void f(void *p) {}",
    "__attribute__((interrupt)) void f(void *p); void f(void *p) {}",
    "__attribute__((interrupt, no_caller_saved_registers)) void f(void *p) {}",
    "__attribute__((interrupt)) void (*pointer)(void *);",
    "__attribute__((interrupt)) void h(void *p); void (*pointer)(void *) = h;",
    "__attribute__((no_caller_saved_registers)) int f(int a) { return a; }",
    "__attribute__((no_caller_saved_registers)) void f(void);
     void g(void) { f(); }",
];

#[test]
fn valid_handlers_pass_under_both_identities() {
    for source in VALID {
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            assert_eq!(
                diagnostics(source, identity, true),
                [],
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn gcc_reports_each_fault_of_a_signature() {
    use Fault::{
        GccFirstParameter as First,
        GccParameterCount as Count,
        GccResult as Result_,
        GccSecondParameter as Second,
    };
    let gcc = CompilerIdentity::Gcc;
    let handler = |signature: &str| format!("__attribute__((interrupt)) {signature};");
    for (signature, expected) in [
        ("void f(void)", &[Count][..]),
        ("void f(int x)", &[First]),
        ("void f(void *p, int e)", &[Second]),
        ("void f(void *p, void *q)", &[Second]),
        ("void f(void *p, char e)", &[Second]),
        (
            "void f(void *p, unsigned long e, unsigned long z)",
            &[Count],
        ),
        ("int f(void *p)", &[Result_]),
        ("void f()", &[Count]),
        (
            "int f(int a, int b, int c)",
            &[First, Second, Count, Result_],
        ),
    ] {
        assert_eq!(faults(&handler(signature), gcc), expected, "{signature}");
    }
    // GCC takes any integer of the word's width, and variable arguments.
    for signature in ["void f(void *p, long e)", "void f(void *p, ...)"] {
        assert_eq!(faults(&handler(signature), gcc), [], "{signature}");
    }
}

#[test]
fn clang_stops_at_the_first_fault_of_a_signature() {
    use Fault::{
        ClangFirstParameter as First,
        ClangParameterCount as Count,
        ClangResult as Result_,
        ClangSecondParameter as Second,
    };
    let clang = CompilerIdentity::Clang;
    let handler = |signature: &str| format!("__attribute__((interrupt)) {signature};");
    for (signature, expected) in [
        ("void f(void)", &[Count][..]),
        ("void f(int x)", &[First]),
        ("void f(void *p, int e)", &[Second]),
        ("void f(void *p, long e)", &[Second]),
        (
            "void f(void *p, unsigned long e, unsigned long z)",
            &[Count],
        ),
        ("int f(void *p)", &[Result_]),
        ("int f(int a, int b, int c)", &[Result_]),
    ] {
        assert_eq!(faults(&handler(signature), clang), expected, "{signature}");
    }
    // Clang ignores the attribute on a function without a prototype, with a
    // warning, and takes variable arguments.
    assert_eq!(
        diagnostics(&handler("void f()"), clang, true),
        [(SemanticErrorKind::IgnoredAttribute, 1)]
    );
    assert_eq!(faults(&handler("void f(void *p, ...)"), clang), []);
}

#[test]
fn no_one_calls_a_handler_directly() {
    let source = "__attribute__((interrupt)) void h(void *p);
void f(void) { h(0); }
__attribute__((interrupt)) void g(void *p) { h(p); }
";
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            diagnostics(source, identity, true),
            [
                (SemanticErrorKind::InvalidMachineEntry(Fault::DirectCall), 2),
                (SemanticErrorKind::InvalidMachineEntry(Fault::DirectCall), 3),
            ],
            "{identity:?}"
        );
    }
}

#[test]
fn attributes_of_the_wrong_things_are_ignored_and_arguments_refused() {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            diagnostics("__attribute__((interrupt)) int x;", identity, true),
            [(SemanticErrorKind::IgnoredAttribute, 1)]
        );
        assert_eq!(
            diagnostics(
                "__attribute__((no_caller_saved_registers)) int x;",
                identity,
                true
            ),
            [(SemanticErrorKind::IgnoredAttribute, 1)]
        );
        assert_eq!(
            diagnostics(
                "__attribute__((no_caller_saved_registers(1))) void f(void);",
                identity,
                true
            ),
            [(SemanticErrorKind::AttributeArguments, 1)]
        );
    }
    let argument = "__attribute__((interrupt(1))) void f(void *p);";
    assert_eq!(
        diagnostics(argument, CompilerIdentity::Gcc, true),
        [(SemanticErrorKind::AttributeArguments, 1)]
    );
    assert_eq!(diagnostics(argument, CompilerIdentity::Clang, true), []);
}

#[test]
fn gcc_wants_general_registers_of_each_definition() {
    use SemanticErrorKind::InvalidMachineEntry as E;
    let gcc = CompilerIdentity::Gcc;
    for (source, fault) in [
        (
            "__attribute__((interrupt)) void f(void *p) {}",
            Fault::GccInterruptNeedsGeneralRegisters,
        ),
        (
            "__attribute__((interrupt)) void f(void *p, unsigned long e) {}",
            Fault::GccExceptionNeedsGeneralRegisters,
        ),
        (
            "__attribute__((no_caller_saved_registers)) void f(void) {}",
            Fault::GccNoCallerSavedNeedsGeneralRegisters,
        ),
    ] {
        assert_eq!(diagnostics(source, gcc, false), [(E(fault), 1)], "{source}");
        assert_eq!(diagnostics(source, gcc, true), [], "{source}");
        // Clang saves the SSE registers instead.
        assert_eq!(
            diagnostics(source, CompilerIdentity::Clang, false),
            [],
            "{source}"
        );
    }
    // A declaration compiles no body.
    let declaration = "__attribute__((interrupt)) void f(void *p);";
    assert_eq!(diagnostics(declaration, gcc, false), []);
    // A naked function has no saves to make.
    let naked = "__attribute__((no_caller_saved_registers, naked)) void f(void) {}";
    assert_eq!(diagnostics(naked, gcc, false), []);
}

#[test]
fn clang_warns_of_the_registers_a_call_makes_a_handler_save() {
    use SemanticErrorKind::ExcessiveRegsave as E;
    let source = "void plain(void);
__attribute__((no_caller_saved_registers)) void saving(void);
__attribute__((interrupt)) void h(void *p) { plain(); }
__attribute__((interrupt)) void i(void *p) { saving(); }
__attribute__((no_caller_saved_registers)) void j(void) { plain(); }
__attribute__((no_caller_saved_registers)) void k(void) { saving(); }
void l(void) { plain(); }
";
    assert_eq!(
        diagnostics(source, CompilerIdentity::Clang, false),
        [(E(true), 3), (E(false), 5)]
    );
    assert_eq!(diagnostics(source, CompilerIdentity::Gcc, true), []);
}

#[test]
fn gcc_refuses_interrupt_with_naked() {
    assert_eq!(
        diagnostics(
            "__attribute__((interrupt, naked)) void f(void *p) {}",
            CompilerIdentity::Gcc,
            true
        ),
        [(SemanticErrorKind::InvalidMachineEntry(Fault::Naked), 1)]
    );
}

/// The facts lowering reads, and bcc's refusal where it models no entry.
#[test]
fn the_convention_is_refused_where_bcc_does_not_model_it() {
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        for attribute in ["interrupt", "no_caller_saved_registers"] {
            let source = format!("__attribute__(({attribute})) void f(void *p) {{}}");
            with_configuration(
                &source,
                CompilerConfiguration::default().with_target(target),
                |_, sema| {
                    assert!(
                        sema.symbols
                            .refusals
                            .iter()
                            .any(|refusal| refusal.capability
                                == UnsupportedAttribute::CallingConvention),
                        "{attribute} {target:?}"
                    );
                },
            );
        }
    }
}
