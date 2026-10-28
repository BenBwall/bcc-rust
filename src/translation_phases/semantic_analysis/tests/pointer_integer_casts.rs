//! Casts between a pointer and an integer of another width: GCC's
//! `-Wint-to-pointer-cast` and `-Wpointer-to-int-cast`, and Clang's finer
//! groups that split off `void *` and enumeration destinations, each under
//! its identity's rule. C99 §6.3.2.3p5-6.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    DiagnosticGroup as G,
    ExtensionPolicy,
    WarningOption,
};

/// The warning groups of `source`'s diagnostics in GNU C17 under
/// `identity` and `options`, and how many of them are errors.
fn groups(
    source: &str,
    identity: CompilerIdentity,
    options: &[(G, WarningOption)],
) -> (Vec<G>, usize) {
    let configuration = options.iter().fold(
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity),
        |configuration, &(group, option)| configuration.with_warning_option(group, option),
    );
    let mut result = (Vec::new(), 0);
    with_configuration(source, configuration, |context, unit| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::PointerConstraint(group),
                    ..
                }) => result.0.push(group),
                | error => panic!("{source}: unexpected {error:?}"),
            }
        }
        result.1 = unit.errors;
    });
    result
}

const PRELUDE: &str = "enum E { A } e; int x; void f(void); __int128 q; int i; char c; _Bool b; \
                       unsigned short us; void *p; char *cp;";

#[test]
fn each_identity_warns_for_its_own_width_rule() {
    // The cast, GCC's group and Clang's group.
    for (cast, gcc, clang) in [
        (
            "(void *)i",
            Some(G::IntToPointerCast),
            Some(G::IntToVoidPointerCast),
        ),
        (
            "(const void *)i",
            Some(G::IntToPointerCast),
            Some(G::IntToVoidPointerCast),
        ),
        (
            "(char *)i",
            Some(G::IntToPointerCast),
            Some(G::IntToPointerCast),
        ),
        (
            "(char *)c",
            Some(G::IntToPointerCast),
            Some(G::IntToPointerCast),
        ),
        (
            "(char *)us",
            Some(G::IntToPointerCast),
            Some(G::IntToPointerCast),
        ),
        (
            "(void (*)(void))i",
            Some(G::IntToPointerCast),
            Some(G::IntToPointerCast),
        ),
        (
            "(void *)(int){0}",
            Some(G::IntToPointerCast),
            Some(G::IntToVoidPointerCast),
        ),
        // Even an unevaluated operand that is no constant counts.
        (
            "(char *)(1 ? 2 : i)",
            Some(G::IntToPointerCast),
            Some(G::IntToPointerCast),
        ),
        (
            "(char *)(i * 0)",
            Some(G::IntToPointerCast),
            Some(G::IntToPointerCast),
        ),
        // GCC also warns for a wider integer, Clang only for a narrower one.
        ("(char *)q", Some(G::IntToPointerCast), None),
        // Neither warns for `_Bool`, an enumeration or a constant, and GCC
        // counts an address constant as one.
        ("(char *)b", None, None),
        ("(char *)e", None, None),
        ("(char *)A", None, None),
        ("(char *)(char)300", None, None),
        ("(char *)(int)1.5", None, None),
        ("(char *)(int)(long)&x", None, Some(G::IntToPointerCast)),
        ("(char *)(long)i", None, None),
        (
            "(int)p",
            Some(G::PointerToIntCast),
            Some(G::VoidPointerToIntCast),
        ),
        (
            "(int)cp",
            Some(G::PointerToIntCast),
            Some(G::PointerToIntCast),
        ),
        (
            "(char)cp",
            Some(G::PointerToIntCast),
            Some(G::PointerToIntCast),
        ),
        (
            "(int)f",
            Some(G::PointerToIntCast),
            Some(G::PointerToIntCast),
        ),
        (
            "(int)(void *)0",
            Some(G::PointerToIntCast),
            Some(G::VoidPointerToIntCast),
        ),
        ("(enum E)p", None, Some(G::VoidPointerToEnumCast)),
        ("(enum E)cp", None, Some(G::PointerToEnumCast)),
        ("(__int128)p", Some(G::PointerToIntCast), None),
        ("(_Bool)p", None, None),
        ("(long)p", None, None),
    ] {
        let source = format!("{PRELUDE} void g(void) {{ (void){cast}; }}");
        for (identity, expected) in [
            (CompilerIdentity::Gcc, gcc),
            (CompilerIdentity::Clang, clang),
        ] {
            assert_eq!(
                groups(&source, identity, &[]),
                (expected.into_iter().collect(), 0),
                "{identity:?}: {cast}"
            );
        }
    }
}

#[test]
fn clang_parent_groups_control_their_children() {
    let source = format!(
        "{PRELUDE} void g(void) {{ (void)(void *)i; (void)(char *)i; (void)(int)p; (void)(int)cp; \
         (void)(enum E)p; (void)(enum E)cp; }}"
    );
    let (all, _) = groups(&source, CompilerIdentity::Clang, &[]);
    assert_eq!(all.len(), 6, "{all:?}");
    for (option, remaining) in [
        (
            (G::IntToPointerCast, WarningOption::Enable(false)),
            &[
                G::VoidPointerToIntCast,
                G::PointerToIntCast,
                G::VoidPointerToEnumCast,
                G::PointerToEnumCast,
            ][..],
        ),
        (
            (G::PointerToIntCast, WarningOption::Enable(false)),
            &[G::IntToVoidPointerCast, G::IntToPointerCast][..],
        ),
        (
            (G::PointerToEnumCast, WarningOption::Enable(false)),
            &[
                G::IntToVoidPointerCast,
                G::IntToPointerCast,
                G::VoidPointerToIntCast,
                G::PointerToIntCast,
            ][..],
        ),
        (
            (G::VoidPointerToIntCast, WarningOption::Enable(false)),
            &[
                G::IntToVoidPointerCast,
                G::IntToPointerCast,
                G::PointerToIntCast,
                G::PointerToEnumCast,
            ][..],
        ),
    ] {
        let (found, _) = groups(&source, CompilerIdentity::Clang, &[option]);
        assert_eq!(found, remaining, "{:?}", option.0);
    }
    assert_eq!(
        groups(
            "int i; void g(void) { (void)(void *)i; (void)(char *)i; }",
            CompilerIdentity::Clang,
            &[(G::IntToPointerCast, WarningOption::Error(true))]
        ),
        (vec![G::IntToVoidPointerCast, G::IntToPointerCast], 2)
    );
    // GCC has neither child group; its own group stays in force.
    assert_eq!(
        groups(
            "int i; void g(void) { (void)(void *)i; }",
            CompilerIdentity::Gcc,
            &[(G::IntToVoidPointerCast, WarningOption::Enable(false))]
        ),
        (vec![G::IntToPointerCast], 0)
    );
    assert_eq!(
        groups(
            "int i; void *p; void g(void) { (void)(void *)i; (void)(int)p; }",
            CompilerIdentity::Gcc,
            &[
                (G::IntToPointerCast, WarningOption::Enable(false)),
                (G::PointerToIntCast, WarningOption::Enable(false)),
            ]
        ),
        (vec![], 0)
    );
}

#[test]
fn casts_in_static_initializers_follow_the_same_rules() {
    // A constant integer is no warning toward a pointer; the reverse is.
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            groups(
                "static char *g = (char *)(int)2; static int k = (int)(char *)0;",
                identity,
                &[]
            ),
            (vec![G::PointerToIntCast], 0),
            "{identity:?}"
        );
    }
}

#[test]
fn bit_int_operands_count_with_their_width() {
    // Both compilers compare a `_BitInt`'s width; GCC also a wider one.
    for (identity, expected) in [
        (
            CompilerIdentity::Gcc,
            vec![
                G::IntToPointerCast,
                G::PointerToIntCast,
                G::PointerToIntCast,
            ],
        ),
        (
            CompilerIdentity::Clang,
            vec![G::IntToVoidPointerCast, G::VoidPointerToIntCast],
        ),
    ] {
        let mut found = Vec::new();
        with_configuration(
            "void f(int x, void *p) { (void)(void *)(_BitInt(32))x; (void)(_BitInt(32))p; \
             (void)(void *)(_BitInt(64))x; (void)(_BitInt(128))p; }",
            CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
                .with_compiler_identity(identity),
            |context, _| {
                for error in context.take_pending_errors() {
                    if let TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::PointerConstraint(group),
                        ..
                    }) = error
                    {
                        found.push(group);
                    }
                }
            },
        );
        assert_eq!(found, expected, "{identity:?}");
    }
}
