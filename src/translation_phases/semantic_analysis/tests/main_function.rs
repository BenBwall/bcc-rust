//! `main`'s signature under each compiler's rules, pinned against GCC 14.2
//! and Clang 23.1.1 (see `main_function.rs`).

use super::{
    super::main_function::MainRule as M,
    *,
};
use crate::configuration::{
    CompilerConfiguration,
    CompilerIdentity,
    DiagnosticGroup,
    WarningOption,
};

fn rules(source: &str, configuration: CompilerConfiguration) -> Vec<M> {
    let mut rules = Vec::new();
    with_configuration(source, configuration, |context, _| {
        rules.extend(
            context
                .take_pending_errors()
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::Main(rule),
                        ..
                    }) => Some(*rule),
                    | _ => None,
                }),
        );
    });
    rules
}

fn gcc() -> CompilerConfiguration {
    CompilerConfiguration::default().with_compiler_identity(CompilerIdentity::Gcc)
}

fn clang() -> CompilerConfiguration {
    CompilerConfiguration::default().with_compiler_identity(CompilerIdentity::Clang)
}

/// The forms real programs use, which neither compiler says anything about
/// even under `-Wall` and `-Wpedantic`: GNU make's three-parameter `main`
/// and the `const` forms among them.
#[test]
fn portable_forms_and_common_extensions_are_silent() {
    for source in [
        "int main(void) { return 0; }",
        "int main() { return 0; }",
        "int main(int argc, char **argv) { return 0; }",
        "int main(int argc, char *argv[]) { return 0; }",
        "int main(int argc, char **argv, char **envp) { return 0; }",
        "int main(int argc, const char **argv) { return 0; }",
        "int main(int argc, char *const *argv) { return 0; }",
        "int main(int argc, const char *const *argv) { return 0; }",
        "int main(int argc, char **argv, const char **envp) { return 0; }",
        "int main(const int argc, char **argv) { return 0; }",
        "int main(int argc, char **restrict argv) { return 0; }",
        "int main(argc, argv) int argc; char **argv; { return 0; }",
        "typedef int I; typedef char **A; I main(I argc, A argv) { return 0; }",
        "void f(int main);",
        "void f(int main(void));",
        "void f(void) { int main; (void)main; }",
        "typedef int main;",
        "static int main;",
        "struct S { int main; };",
    ] {
        for configuration in [
            gcc().with_all_warnings(true),
            gcc().with_pedantic(true),
            clang(),
            clang().with_all_warnings(true),
        ] {
            assert_eq!(rules(source, configuration), [], "{source}");
        }
    }
}

#[test]
fn gcc_reports_forms_only_under_its_option() {
    let source = "void main(void) {}";
    assert_eq!(rules(source, gcc()), []);
    let expected = [M::GccReturnType];
    assert_eq!(rules(source, gcc().with_all_warnings(true)), expected);
    assert_eq!(rules(source, gcc().with_pedantic(true)), expected);
    assert_eq!(
        rules(
            source,
            gcc().with_warning_option(DiagnosticGroup::Main, WarningOption::Enable(true))
        ),
        expected
    );
    // The group's own option wins in GCC, in either order.
    let off = WarningOption::Enable(false);
    for configuration in [
        gcc()
            .with_warning_option(DiagnosticGroup::Main, off)
            .with_all_warnings(true),
        gcc()
            .with_all_warnings(true)
            .with_warning_option(DiagnosticGroup::Main, off),
        gcc().with_all_warnings(true).with_all_warnings(false),
    ] {
        assert_eq!(rules(source, configuration), []);
    }
}

#[test]
fn gcc_rules_match_each_form() {
    for (source, expected) in [
        ("void main(void) {}", &[M::GccReturnType][..]),
        ("const int main(void) { return 0; }", &[]),
        (
            "_Atomic int main(void) { return 0; }",
            &[M::GccAtomicReturn],
        ),
        ("int main(int a) { return 0; }", &[M::GccParameterCount]),
        (
            "int main(long a) { return 0; }",
            &[M::GccFirstParameter, M::GccParameterCount],
        ),
        (
            "int main(long a, char **b) { return 0; }",
            &[M::GccFirstParameter],
        ),
        (
            "int main(int a, signed char **b) { return 0; }",
            &[M::GccSecondParameter],
        ),
        ("int main(int a, volatile char **b) { return 0; }", &[]),
        (
            "int main(int a, char *const *volatile b) { return 0; }",
            &[],
        ),
        // GCC checks a nested definition too, which is not public.
        (
            "void f(void) { int main(void) { return 0; } (void)main; }",
            &[M::GccStatic],
        ),
        (
            "void f(void) { int main(int a) { return a; } (void)main; }",
            &[M::GccParameterCount, M::GccStatic],
        ),
        (
            "void f(void) { void main(void) {} (void)main; }",
            &[M::GccReturnType, M::GccStatic],
        ),
        (
            "void f(void) { int main(a) int a; { return a; } (void)main; }",
            &[M::GccStatic],
        ),
        (
            "int main(int a, char **b, int c) { return 0; }",
            &[M::GccThirdParameter],
        ),
        (
            "int main(int a, char **b, char **c, char **d) { return 0; }",
            &[M::GccParameterCount],
        ),
        (
            "int main(int a, char **b, ...) { return 0; }",
            &[M::GccVariadic],
        ),
        ("static int main(void) { return 0; }", &[M::GccStatic]),
        (
            "static int main(void); int main(void) { return 0; }",
            &[M::GccStatic],
        ),
        // A declaration and an old-style definition's parameters are not
        // read.
        ("void main(void);", &[]),
        ("int main(long a);", &[]),
        ("int main(a, b) long a; char **b; { return 0; }", &[]),
        ("long main(a) int a; { return 0; }", &[M::GccReturnType]),
        ("int main;", &[M::GccObject]),
        ("extern int main;", &[M::GccObject]),
        ("void f(void) { extern int main; }", &[M::GccObject]),
        ("inline int main(void);", &[M::GccInline]),
        ("_Noreturn int main(void);", &[M::GccNoreturn]),
    ] {
        assert_eq!(
            rules(source, gcc().with_all_warnings(true)),
            expected,
            "{source}"
        );
    }
}

#[test]
fn clang_rules_match_each_form() {
    let int_rule = |source| rules(source, clang().with_gnu_extensions(true));
    for (source, expected) in [
        ("void main(void) {}", &[M::ClangReturnType][..]),
        ("long main();", &[M::ClangReturnType]),
        ("void f(void) { long main(); }", &[M::ClangReturnType]),
        ("const int main(void) { return 0; }", &[]),
        ("int main(int a) { return 0; }", &[M::ClangOneParameter]),
        ("int main(a) int a; { return 0; }", &[M::ClangOneParameter]),
        ("int main(long a) { return 0; }", &[M::ClangFirstParameter]),
        (
            "int main(a, b) long a; char **b; { return 0; }",
            &[M::ClangFirstParameter],
        ),
        (
            "int main(int a, volatile char **b) { return 0; }",
            &[M::ClangSecondParameter],
        ),
        // Only the qualifiers a declaration writes count: outer ones, not
        // those on an array's brackets or inside a typedef name.
        ("int main(int a, char **volatile b) { return 0; }", &[]),
        ("int main(int a, char *const *const b) { return 0; }", &[]),
        (
            "int main(int a, char *const b[volatile]) { return 0; }",
            &[],
        ),
        (
            "typedef char *const *volatile T; int main(int a, T b) { return 0; }",
            &[],
        ),
        (
            "typedef char *volatile P; int main(int a, P *b) { return 0; }",
            &[],
        ),
        (
            "int main(int a, char *const *volatile b) { return 0; }",
            &[M::ClangSecondParameter],
        ),
        (
            "int main(int a, char *const *restrict b);",
            &[M::ClangSecondParameter],
        ),
        (
            "int main(int a, char *volatile b[]) { return 0; }",
            &[M::ClangSecondParameter],
        ),
        (
            "typedef char *const *T; int main(int a, volatile T b) { return 0; }",
            &[M::ClangSecondParameter],
        ),
        (
            "typedef const char C; int main(int a, C **b) { return 0; }",
            &[M::ClangSecondParameter],
        ),
        (
            "int main(a, b) int a; char *const *volatile b; { return 0; }",
            &[M::ClangSecondParameter],
        ),
        (
            "int main(int a, char **b, char *const *volatile c) { return 0; }",
            &[M::ClangThirdParameter],
        ),
        (
            "int main(int a, char **b, int c) { return 0; }",
            &[M::ClangThirdParameter],
        ),
        (
            "int main(int a, char **b, char **c, char **d) { return 0; }",
            &[M::ClangSurplusParameters(4)],
        ),
        ("static int main(void) { return 0; }", &[M::ClangStatic]),
        ("inline int main(void) { return 0; }", &[M::ClangInline]),
        ("_Noreturn int main(void);", &[M::ClangNoreturn]),
        (
            "int main(int a, char **b, ...) { return 0; }",
            &[M::ClangVariadic],
        ),
        ("int main;", &[M::ClangObject]),
        ("void f(void) { extern int main; }", &[M::ClangObject]),
    ] {
        assert_eq!(int_rule(source), expected, "{source}");
    }
    // Strict modes refuse any other return type, even a qualified `int`.
    let strict = clang().with_gnu_extensions(false);
    assert_eq!(rules("void main(void) {}", strict), [M::ClangMustReturnInt]);
    assert_eq!(
        rules("const int main(void) { return 0; }", strict),
        [M::ClangMustReturnInt]
    );
}

/// A freestanding startup function is the implementation's, C99 §5.1.2.1p1,
/// so only GCC's explicit `-Wmain` reads one, and `-Wall` does not.
#[test]
fn freestanding_checks_only_under_an_explicit_gcc_option() {
    let source = "void main(void) {}";
    assert_eq!(rules(source, clang().with_hosted(false)), []);
    assert_eq!(
        rules(source, gcc().with_hosted(false).with_all_warnings(true)),
        []
    );
    assert_eq!(
        rules(
            source,
            gcc()
                .with_hosted(false)
                .with_warning_option(DiagnosticGroup::Main, WarningOption::Enable(true))
        ),
        [M::GccReturnType]
    );
    assert_eq!(rules("int main;", gcc().with_hosted(false)), []);
}

/// TCC has no rule for the name, and `cl`'s are not modeled.
#[test]
fn other_identities_are_silent() {
    for identity in [CompilerIdentity::Tcc, CompilerIdentity::Msvc] {
        for source in [
            "void main(void) {}",
            "int main(long a, char **b, int c, int d) { return 0; }",
            "static int main;",
            "int main;",
        ] {
            let configuration = CompilerConfiguration::default()
                .with_compiler_identity(identity)
                .with_all_warnings(true);
            assert_eq!(rules(source, configuration), [], "{source}");
        }
    }
}
