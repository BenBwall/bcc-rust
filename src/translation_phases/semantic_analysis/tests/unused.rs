//! `-Wunused-*` against GCC 14.2 and Clang 23.1.1: the two compilers count
//! uses differently, so most forms pin both.

use super::{
    super::unused::UnusedRule as U,
    *,
};
use crate::configuration::{
    CompilerConfiguration,
    CompilerIdentity,
    DiagnosticGroup,
    WarningOption,
};

fn rules(source: &str, configuration: CompilerConfiguration) -> Vec<U> {
    let mut rules = Vec::new();
    with_exact_configuration(source, configuration, |context, _| {
        rules.extend(
            context
                .take_pending_errors()
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::Unused(rule),
                        ..
                    }) => Some(*rule),
                    | _ => None,
                }),
        );
    });
    rules.sort_by_key(|rule| format!("{rule:?}"));
    rules
}

fn gcc() -> CompilerConfiguration {
    CompilerConfiguration::default()
        .with_compiler_identity(CompilerIdentity::Gcc)
        .with_gnu_extensions(true)
        .with_all_warnings(true)
        .with_extra_warnings(true)
}

fn clang() -> CompilerConfiguration {
    CompilerConfiguration::default()
        .with_compiler_identity(CompilerIdentity::Clang)
        .with_gnu_extensions(true)
        .with_all_warnings(true)
        .with_extra_warnings(true)
}

/// Each form both compilers accept in silence; a false positive here would
/// stop a `-Werror` build.
#[test]
fn used_things_are_silent() {
    for source in [
        "int f(void) { int a = 1; return a; }",
        "void f(void) { int a; (void)a; }",
        "void f(void) { int a; int *p = &a; (void)p; }",
        "int f(void) { int a; return sizeof a; }",
        "int f(void) { int a; a = 1; return sizeof a; }",
        "void f(void) { int a; __typeof__(a) b; (void)b; }",
        "void f(void) { int n = 1; int a[n]; (void)a; }",
        "void f(void) { int a; __asm__(\"\" : \"=r\"(a)); }",
        "void f(void) { int a = 1; __asm__(\"\" : : \"r\"(a)); }",
        "void f(void) { int a; ({ a = 1; }); }",
        "int f(void) { int a; return ({ a = 1; }); }",
        "void f(void) { int a = ({ int b = 1; b; }); (void)a; }",
        "void f(void) { int *p = 0; p[0] = 1; }",
        "void f(void) { struct S { int x; } *p = 0; p->x = 1; }",
        "void f(void) { for (int i = 0; i < 1; i++) {} }",
        "void f(void) { int a __attribute__((unused)); }",
        "void f(void) { __attribute__((unused)) int a; }",
        "void f(void) { int a __attribute__((unused)) = 1; a = 2; }",
        "void f(void) { [[maybe_unused]] int a; }",
        "void f(void) { typedef int T __attribute__((unused)); T a; }",
        "void f(void) { struct __attribute__((unused)) S { int x; }; struct S s; }",
        "void g(int *p); void f(void) { int a __attribute__((cleanup(g))); a = 1; }",
        "void f(void) { void g(int a); }",
        "void g(int a);",
        "void f(int a) { (void)a; }",
        "void f(int a __attribute__((unused))) {}",
        "void f(__attribute__((unused)) int a) {}",
        "int f(int a, int b[a]) { return b[0]; }",
        "void f(int a, int b[a]) { (void)b; }",
        "void f(void) { l:; goto l; }",
        "void f(void) { l: __attribute__((unused)); }",
        "void f(void) { void *p = &&l; (void)p; l:; }",
        "static void f(void) {} void g(void) { f(); }",
        "static void f(void) {} void g(void) { void (*p)(void) = f; (void)p; }",
        "static void f(void) __attribute__((unused)); static void f(void) {}",
        "__attribute__((unused)) static void f(void) {}",
        "__attribute__((used)) static void f(void) {}",
        "static __attribute__((constructor)) void f(void) {}",
        "static void f(void); static void g(void) { f(); } static void f(void) { g(); } void \
         h(void) { g(); }",
        "static int a; int g(void) { return a; }",
        "static const int a = 1; int g(void) { return a; }",
        "static int a __attribute__((unused));",
        "__attribute__((used)) static int a;",
        "static int q; static int *p = &q; int *g(void) { return p; }",
        "extern int a;",
        "int a;",
        "const int a = 1;",
        "typedef int T;",
        "struct S { int x; };",
        "enum { A };",
        "void f(int a) { a = 1, (void)a; }",
        "void f(int a) { (void)a, a = 1; }",
        "void f(void) { (void)0; }",
        "int g(int n) __attribute__((enable_if(n > 0, \"positive\")));",
        "void g(const char *, ...) __attribute__((format(printf, 1, 2)));",
        "void f(volatile int *p) { *p; }",
        "void f(volatile int a) { a; }",
        "void f(int a) { a ? a = 1 : (a = 2); }",
        "void f(int a) { a && (a = 1); }",
        "int g(void); void f(void) { g(); }",
        "int g(void); void f(void) { (void)g(); }",
        "void f(void) { __builtin_trap(); }",
        "void f(int a) { a ? (void)0 : (void)1; }",
        "#define M(x) ((void)0)\nvoid f(int a) { (void)a; M(a); }",
    ] {
        assert_eq!(rules(source, gcc()), [], "GCC: {source}");
        assert_eq!(rules(source, clang()), [], "Clang: {source}");
    }
}

#[test]
fn each_compiler_counts_uses_its_own_way() {
    for (source, in_gcc, in_clang) in [
        (
            "void f(void) { int a; }",
            &[U::Variable][..],
            &[U::Variable][..],
        ),
        ("void f(void) { int a[2]; }", &[U::Variable], &[U::Variable]),
        (
            "void f(void) { static int a; }",
            &[U::Variable],
            &[U::Variable],
        ),
        (
            "void f(void) { const int a = 1; }",
            &[U::Variable],
            &[U::Variable],
        ),
        ("void f(void) { extern int a; }", &[U::Variable], &[]),
        ("void f(void) { extern int a; a = 1; }", &[], &[]),
        (
            "void f(void) { int a; a = 1; }",
            &[U::ButSetVariable],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { static int a; a = 1; }",
            &[U::ButSetVariable],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { volatile int a; a = 1; }",
            &[U::ButSetVariable],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { int a; (a) = 1; }",
            &[U::ButSetVariable],
            &[],
        ),
        (
            "void f(void) { int a[2]; a[0] = 1; }",
            &[U::ButSetVariable],
            &[],
        ),
        (
            "void f(void) { struct { int x; } s; s.x = 1; }",
            &[U::ButSetVariable],
            &[],
        ),
        (
            "void f(void) { int a = 0; a++; }",
            &[],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { int a = 0; --a; }",
            &[],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { int a = 0; a += 1; }",
            &[],
            &[U::ButSetVariable],
        ),
        ("void f(void) { volatile int a = 0; a++; }", &[], &[]),
        (
            "void f(void) { int a, b; a = b = 1; }",
            &[U::ButSetVariable],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { int a, b; a = 1, b = 2; }",
            &[U::ButSetVariable, U::ButSetVariable],
            &[U::ButSetVariable],
        ),
        (
            "void f(void) { for (int i = 0;;) { i = 1; } }",
            &[U::ButSetVariable],
            &[U::ButSetVariable],
        ),
        ("void f(int a) {}", &[U::Parameter], &[U::Parameter]),
        ("void f(a) int a; {}", &[U::Parameter], &[U::Parameter]),
        ("inline void f(int a) {}", &[U::Parameter], &[U::Parameter]),
        (
            "void f(int a) { a = 1; }",
            &[U::ButSetParameter],
            &[U::ButSetParameter],
        ),
        ("void f(int a) { a++; }", &[], &[U::ButSetParameter]),
        ("void f(void) { l:; }", &[U::GccLabel], &[U::ClangLabel]),
        (
            "void f(void) { l: m:; goto m; }",
            &[U::GccLabel],
            &[U::ClangLabel],
        ),
        (
            "static void f(void) {}",
            &[U::GccFunction],
            &[U::ClangFunction],
        ),
        ("static inline void f(void) {}", &[], &[U::ClangFunction]),
        ("static void f(void) { f(); }", &[U::GccFunction], &[]),
        (
            "static void f(void) {} void g(void) { (void)sizeof(f); }",
            &[],
            &[],
        ),
        ("static int a;", &[U::GccFileObject], &[U::Variable]),
        (
            "static int a[2] = {1, 2};",
            &[U::GccFileObject],
            &[U::Variable],
        ),
        ("static volatile int a;", &[], &[U::Variable]),
        (
            "static const int a = 1;",
            &[U::GccFileConstant],
            &[U::ClangFileConstant],
        ),
        (
            "static const char s[] = \"x\";",
            &[U::GccFileConstant],
            &[U::ClangFileConstant],
        ),
        (
            "static int a; void g(void) { a = 1; }",
            &[],
            &[U::ButSetGlobal],
        ),
        ("void f(int a) { a; }", &[U::GccNoEffect], &[U::ClangResult]),
        (
            "void f(int a) { a + 1; }",
            &[U::GccNoEffect],
            &[U::ClangResult],
        ),
        (
            "void f(int a) { a == 1; }",
            &[U::GccNoEffect],
            &[U::ClangEquality],
        ),
        (
            "void f(int a) { a != 1; }",
            &[U::GccNoEffect],
            &[U::ClangInequality],
        ),
        (
            "void f(int a) { a < 1; }",
            &[U::GccNoEffect],
            &[U::ClangRelational],
        ),
        (
            "void f(int a) { -a; }",
            &[U::GccNoEffect],
            &[U::ClangResult],
        ),
        (
            "void f(int *p) { *p; }",
            &[U::GccNoEffect],
            &[U::ClangResult],
        ),
        (
            "void f(int *p) { (void *)p; }",
            &[U::GccNoEffect],
            &[U::ClangVoidPointerCast],
        ),
        (
            "void f(int a) { a, a = 1; }",
            &[U::GccCommaLeft],
            &[U::ClangCommaLeft],
        ),
        (
            "void f(int a) { a = 1, a; }",
            &[U::GccCommaRight],
            &[U::ClangResult],
        ),
        (
            "int g(void); void f(void) { g() + 1; }",
            &[U::GccValueComputed],
            &[U::ClangResult],
        ),
        (
            "int g(void); void f(int a) { a && g(); }",
            &[U::GccValueComputed],
            &[],
        ),
        ("void f(int a) { ({ a; }); }", &[], &[]),
        (
            "int f(int a) { return ({ a; a + 1; }); }",
            &[U::GccNoEffect],
            &[U::ClangResult],
        ),
        (
            "void f(int a) { for (;; a + 1) {} }",
            &[U::GccNoEffect],
            &[U::ClangResult],
        ),
    ] {
        let sorted = |expected: &[U]| {
            let mut expected = expected.to_vec();
            expected.sort_by_key(|rule| format!("{rule:?}"));
            expected
        };
        assert_eq!(rules(source, gcc()), sorted(in_gcc), "GCC: {source}");
        assert_eq!(rules(source, clang()), sorted(in_clang), "Clang: {source}");
    }
}

/// Clang leaves an expression a macro's body wrote alone; its argument is the
/// user's own.
#[test]
fn clang_skips_a_macro_body() {
    let body = "#define Z(x) (x) + 1\nvoid f(int a) { Z(a); }";
    assert_eq!(rules(body, clang()), []);
    assert_eq!(rules(body, gcc()), [U::GccNoEffect]);
    let argument = "#define M(x) x\nvoid f(int a) { M(a); }";
    assert_eq!(rules(argument, clang()), [U::ClangResult]);
}

#[test]
fn options_hold_their_groups_as_each_compiler_does() {
    let base = |identity| CompilerConfiguration::default().with_compiler_identity(identity);
    let parameter = "void f(int a) {}";
    let variable = "void f(void) { int a; }";
    let off = WarningOption::Enable(false);
    let on = WarningOption::Enable(true);
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(rules(variable, base(identity)), []);
        assert_eq!(rules(parameter, base(identity)), []);
        assert_eq!(rules(parameter, base(identity).with_all_warnings(true)), []);
        assert_eq!(
            rules(
                parameter,
                base(identity).with_warning_option(DiagnosticGroup::UnusedParameter, on)
            ),
            [U::Parameter]
        );
        assert_eq!(
            rules(variable, base(identity).with_all_warnings(true)),
            [U::Variable]
        );
        assert_eq!(
            rules(
                variable,
                base(identity).with_warning_option(DiagnosticGroup::Unused, on)
            ),
            [U::Variable]
        );
        assert_eq!(
            rules(
                variable,
                base(identity)
                    .with_all_warnings(true)
                    .with_warning_option(DiagnosticGroup::UnusedVariable, off)
            ),
            []
        );
        assert_eq!(
            rules(
                variable,
                base(identity)
                    .with_all_warnings(true)
                    .with_warning_option(DiagnosticGroup::Unused, off)
            ),
            []
        );
    }
    // GCC's `-Wextra` holds the parameter groups only with `-Wunused`, and
    // Clang's holds them alone.
    assert_eq!(
        rules(
            parameter,
            base(CompilerIdentity::Gcc).with_extra_warnings(true)
        ),
        []
    );
    assert_eq!(
        rules(
            parameter,
            base(CompilerIdentity::Gcc)
                .with_extra_warnings(true)
                .with_warning_option(DiagnosticGroup::Unused, on)
        ),
        [U::Parameter]
    );
    assert_eq!(
        rules(
            parameter,
            base(CompilerIdentity::Clang).with_extra_warnings(true)
        ),
        [U::Parameter]
    );
    // GCC's own option wins in either order; Clang's `-Wall` later overrides.
    let earlier = |identity| {
        base(identity)
            .with_warning_option(DiagnosticGroup::UnusedVariable, off)
            .with_all_warnings(true)
    };
    assert_eq!(rules(variable, earlier(CompilerIdentity::Gcc)), []);
    assert_eq!(
        rules(variable, earlier(CompilerIdentity::Clang)),
        [U::Variable]
    );
}

/// A discarded value is a warning by default in Clang alone.
#[test]
fn only_clang_warns_about_values_by_default() {
    let base = |identity| {
        CompilerConfiguration::default()
            .with_compiler_identity(identity)
            .with_gnu_extensions(true)
    };
    let source = "void f(int a) { a; }";
    assert_eq!(rules(source, base(CompilerIdentity::Gcc)), []);
    assert_eq!(
        rules(source, base(CompilerIdentity::Clang)),
        [U::ClangResult]
    );
    for identity in [CompilerIdentity::Tcc, CompilerIdentity::Msvc] {
        assert_eq!(
            rules(source, base(identity).with_all_warnings(true)),
            [],
            "{identity:?}"
        );
    }
}
