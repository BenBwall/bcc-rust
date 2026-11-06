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
        // An attribute's argument is an expression that refers to its name.
        "static void c(int *p) { (void)p; } void f(void) { int x __attribute__((cleanup(c))) = 0; \
         }",
        "static void c(int *p) { (void)p; } void f(void) { int x __attribute__((__cleanup__(c))); \
         }",
        "static void c(int *); void f(void) { int x __attribute__((cleanup(c))); } static void \
         c(int *p) { (void)p; }",
        "static inline void c(int *p) { (void)p; } void f(void) { int x \
         __attribute__((cleanup(c))); }",
        "static void d(void *p) { (void)p; } void *m(void) __attribute__((malloc(d)));",
        "static void d(void *p) { (void)p; } void *m(void) __attribute__((malloc(d, 1)));",
        "static void *r(void) { return 0; } void f(void) __attribute__((ifunc(\"r\")));",
    ] {
        assert_eq!(rules(source, gcc()), [], "GCC: {source}");
        assert_eq!(rules(source, clang()), [], "Clang: {source}");
    }
}

#[test]
fn each_compiler_counts_uses_its_own_way() {
    for (source, in_gcc, in_clang) in [
        // A cleanup function's own variable is exempt, not the function that
        // nothing else refers to.
        (
            "static void c(int *p) { (void)p; } static void f(void) { int x \
             __attribute__((cleanup(c))) = 0; }",
            &[U::GccFunction][..],
            &[U::ClangFunction][..],
        ),
        // An alias or a weak reference's target is a use to GCC alone.
        (
            "static void t(void) {} void w(void) __attribute__((alias(\"t\")));",
            &[],
            &[U::ClangFunction],
        ),
        (
            "static int t; extern int w __attribute__((alias(\"t\")));",
            &[],
            &[U::Variable],
        ),
        (
            "static void t(void) {} static void w(void) __attribute__((weakref(\"t\")));",
            &[],
            &[U::ClangFunction],
        ),
        // Clang ignores an attribute after the definition, and says so.
        (
            "static void g(void) {} static void g(void) __attribute__((used));",
            &[],
            &[U::ClangFunction],
        ),
        // GCC takes an attribute it does not know for an expression too.
        (
            "static void d(void) {} void g(void) __attribute__((nosuch(d)));",
            &[],
            &[U::ClangFunction],
        ),
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

/// `unused` on a type, against GCC 14.2 and Clang 23.1.1: GCC's variable takes
/// it from the type (`TREE_USED`), at file scope too, as long as its declarator
/// derives nothing. Clang looks only at a local variable, at its last typedef
/// or its tag, whose array it looks through. A parameter gets nothing from
/// either.
#[test]
fn unused_on_a_type_reaches_the_variable() {
    const U: &str = "typedef int U __attribute__((unused)); ";
    const T: &str = "struct __attribute__((unused)) T { int a; }; ";
    let (file, constant) = (&[U::GccFileObject][..], &[U::GccFileConstant][..]);
    let local = &[U::Variable][..];
    for (source, in_gcc, in_clang) in
        [
            // The finding: the file-scope record, by typedef and by anonymous
            // tag.
            (format!("{U}static U x;"), &[][..], local),
            (
                "static struct __attribute__((unused)) { int a; } x;".to_owned(),
                &[],
                local,
            ),
            // Every spelling of the typedef and every path from it.
            (
                "typedef int __attribute__((unused)) U; static U x;".to_owned(),
                &[],
                local,
            ),
            (
                "__attribute__((unused)) typedef int U; static U x;".to_owned(),
                &[],
                local,
            ),
            (format!("{U}typedef U V; static V x;"), &[], local),
            (
                format!("{U}static const U x = 1;"),
                &[],
                &[U::ClangFileConstant],
            ),
            (format!("{U}static U (x);"), &[], local),
            (format!("{U}static __typeof__(U) x;"), &[], local),
            (format!("{U}U y; static __typeof__(y) x;"), &[], local),
            (format!("{U}static U x; static int x;"), &[], local),
            (format!("{U}static int x; static U x;"), &[], local),
            (format!("{U}typedef int U; static U x;"), &[], local),
            (
                "typedef int *V __attribute__((unused)); static V x;".to_owned(),
                &[],
                local,
            ),
            // What a declarator derives no longer carries it.
            (format!("{U}static U *x;"), file, local),
            (format!("{U}static U x[2];"), file, local),
            (format!("{U}typedef U *V; static V x;"), file, local),
            (
                "static struct __attribute__((unused)) { int a; } *x;".to_owned(),
                file,
                local,
            ),
            (
                "static struct __attribute__((unused)) { int a; } x[2];".to_owned(),
                file,
                local,
            ),
            // Tags and enumerations, whose definition alone gives it to GCC's
            // type.
            (format!("{T}static struct T x;"), &[], local),
            (
                format!("{T}static const struct T x = {{1}};"),
                &[],
                &[U::ClangFileConstant],
            ),
            (
                "enum __attribute__((unused)) E { A }; static enum E x;".to_owned(),
                &[],
                local,
            ),
            (
                "static enum __attribute__((unused)) { A } x;".to_owned(),
                &[],
                local,
            ),
            (
                "struct __attribute__((unused)) T; struct T { int a; }; static struct T x;"
                    .to_owned(),
                file,
                local,
            ),
            (
                "struct T { int a; }; static struct __attribute__((unused)) T x;".to_owned(),
                file,
                local,
            ),
            (
                "typedef struct T T; struct T { int a; } __attribute__((unused)); static T x;"
                    .to_owned(),
                file,
                local,
            ),
            // A tag of the same name elsewhere is another type.
            (
                format!("{T}void f(void) {{ struct T {{ int b; }} x; }}"),
                local,
                local,
            ),
            (
                "void f(void) { struct __attribute__((unused)) T { int a; } y; } static struct T \
                 {              int b; } x;"
                    .to_owned(),
                file,
                local,
            ),
            // A spelling GCC's typedef does not read.
            (
                "[[maybe_unused]] typedef int U; static U x;".to_owned(),
                file,
                local,
            ),
            (
                "[[maybe_unused]] typedef int U; void f(void) { U x; }".to_owned(),
                local,
                &[],
            ),
            // Locals.
            (format!("{U}void f(void) {{ U x; }}"), &[], &[]),
            (format!("{U}void f(void) {{ U *x; }}"), local, local),
            (format!("{U}void f(void) {{ U x[2]; }}"), local, local),
            (
                format!("{U}typedef U V; void f(void) {{ V x; }}"),
                &[],
                local,
            ),
            (
                format!("{U}void f(void) {{ U x; x = 1; }}"),
                &[],
                &[U::ButSetVariable],
            ),
            (format!("{T}void f(void) {{ struct T x[2]; }}"), local, &[]),
            (format!("{T}void f(void) {{ struct T *x; }}"), local, local),
            (
                format!("{T}void f(void) {{ struct T x[2]; x[0].a = 1; }}"),
                &[U::ButSetVariable],
                &[],
            ),
            (
                "enum __attribute__((unused)) E { A }; void f(void) { enum E x; x = A; }"
                    .to_owned(),
                &[],
                &[],
            ),
            (
                "struct __attribute__((unused)) T; struct T { int a; }; void f(void) { struct T \
                 x; }"
                    .to_owned(),
                local,
                &[],
            ),
            // Parameters.
            (
                format!("{U}void f(U a) {{}}"),
                &[U::Parameter],
                &[U::Parameter],
            ),
            (
                format!("{T}void f(struct T a) {{}}"),
                &[U::Parameter],
                &[U::Parameter],
            ),
        ]
    {
        assert_eq!(rules(&source, gcc()), in_gcc, "GCC: {source}");
        assert_eq!(rules(&source, clang()), in_clang, "Clang: {source}");
    }
    assert_eq!(rules("static const int x = 1;", gcc()), constant);
}

/// GCC reports what the end of a unit leaves unused as it compiles the unit,
/// and Clang whenever no error has been reported; each of them also reports
/// `static void f(void);` as it pops the file scope, and GCC a local `extern`.
#[test]
fn end_of_unit_reports_follow_each_compiler() {
    let syntax_only = || gcc().with_syntax_only(true);
    for source in [
        "static int x;",
        "static const int x = 1;",
        "static void f(void) {}",
        "static int x; static int g(void) { return x; }",
    ] {
        assert_ne!(rules(source, gcc()), [], "{source}");
        assert_eq!(rules(source, syntax_only()), [], "{source}");
        assert_ne!(rules(source, clang()), [], "{source}");
        assert_ne!(
            rules(source, clang().with_syntax_only(true)),
            [],
            "{source}"
        );
    }
    // Local reports stay, and a label's, value's or parameter's.
    for (source, expected) in [
        ("void f(void) { int a; }", U::Variable),
        ("void f(void) { static int a; }", U::Variable),
        ("void f(void) { int a; a = 1; }", U::ButSetVariable),
        ("void f(int a) {}", U::Parameter),
        ("static int x; void f(void) { extern int x; }", U::Variable),
    ] {
        assert_eq!(rules(source, syntax_only()), [expected], "{source}");
    }
    // A block-scope `extern` stands for the name's other reports, once.
    for source in [
        "static int x; void f(void) { extern int x; }",
        "static int x; void f(void) { extern int x; } void g(void) { extern int x; }",
        "static int x; void f(void) { extern int x; } int g(void) { return x; }",
    ] {
        assert_eq!(rules(source, gcc()), [U::Variable], "{source}");
    }
    assert_eq!(
        rules("static int x; void f(void) { extern int x; }", clang()),
        [U::Variable]
    );
    // Clang drops the end of the unit after an error, which a warning made an
    // error counts as; GCC's is the same either way.
    for source in [
        "static int x; int f(void) { return y; }",
        "static void g(void) {} int f(void) { return y; }",
    ] {
        assert_eq!(rules(source, clang()), [], "{source}");
        assert_ne!(rules(source, gcc()), [], "{source}");
        assert_eq!(rules(source, syntax_only()), [], "{source}");
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

/// Clang keeps its value warnings, comparisons aside, out of code it can tell
/// is unreachable; GCC does not look.
#[test]
fn clang_skips_unreachable_values() {
    for source in [
        "void f(int x) { return; x + 1; }",
        "void f(int x) { if (0) x + 1; }",
        "void f(int x) { if (1) return; x + 1; }",
        "void f(int x) { for (;;) {} x + 1; }",
        "void g(void) __attribute__((noreturn)); void f(int x) { g(); x + 1; }",
        "void f(int x) { __builtin_unreachable(); x + 1; }",
        "void f(int x) { __builtin_abort(); x + 1; }",
        "void f(int x) { goto l; x + 1; l:; }",
    ] {
        assert_eq!(rules(source, clang()), [], "Clang: {source}");
        assert_eq!(rules(source, gcc()), [U::GccNoEffect], "GCC: {source}");
    }
    assert_eq!(
        rules("void f(int x) { return; x == 1; }", clang()),
        [U::ClangEquality]
    );
    assert_eq!(
        rules(
            "int g(void); void f(int x) { return; int y = (x + 1, g()); (void)y; }",
            clang()
        ),
        []
    );
    assert_eq!(
        rules("void f(int x) { (void)sizeof(({ x + 1; 1; })); }", clang()),
        []
    );
    assert_eq!(
        rules("void f(int x) { (void)(0 ? (x + 1, 1) : 1); }", clang()),
        []
    );
    // A label makes the code reachable again.
    assert_eq!(
        rules("void f(int x) { if (0) { l: x + 1; } goto l; }", clang()),
        [U::ClangResult]
    );
}

/// A variable length array's `sizeof` evaluates its bound and so has an
/// effect for GCC, which keeps quiet about it however it is used; Clang warns.
/// C99: §6.5.3.4p2, p. 80; PDF p. 92.
#[test]
fn gcc_counts_what_a_variably_modified_type_evaluates() {
    for source in [
        "int g(void); void f(void) { sizeof(int[g()]); }",
        "void f(int n) { sizeof(int[n]); }",
        "void f(int n) { int a[n]; sizeof a; }",
        "void f(int n) { int a[2][n]; sizeof(a[0]); }",
        "void f(int n, int (*p)[n]) { sizeof(*p); }",
        "int g(void); void f(void) { sizeof(__typeof__(int[g()])); }",
        "int g(void); void f(void) { (sizeof(int[g()])); }",
        "int g(void); void f(void) { sizeof(int[g()]) + 1; }",
        "int g(void); void f(void) { !sizeof(int[g()]); }",
        "int g(void); void f(void) { (long)sizeof(int[g()]); }",
        "int g(void); void f(int c) { c ? sizeof(int[g()]) : 0; }",
        "void f(int n, void *p) { (int (*)[n])p; }",
    ] {
        assert_eq!(rules(source, gcc()), [], "{source}");
        assert_ne!(rules(source, clang()), [], "{source}");
    }
    // A constant size evaluates nothing, even with a bound in its operand's
    // pointer type or an element of constant size.
    for source in [
        "void f(int n) { int a[n][2]; sizeof(a[0]); }",
        "void f(int n, int (*p)[n]) { sizeof(p); }",
        "int g(void); void f(void) { sizeof(int (*)[g()]); }",
        "int g(void); void f(void) { sizeof(g()); }",
        "int g(void); void f(int *x) { sizeof(x[g()]); }",
        "int g(void); void f(void) { _Alignof(int[g()]); }",
    ] {
        assert_eq!(rules(source, gcc()), [U::GccNoEffect], "{source}");
    }
    assert_eq!(
        rules("int g(void); void f(void) { sizeof(int[g()]), 0; }", gcc()),
        [U::GccCommaRight]
    );
    assert_eq!(
        rules("int g(void); void f(void) { 0, sizeof(int[g()]); }", gcc()),
        [U::GccCommaLeft]
    );
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
    // GCC's `-Wunused` outranks `-Wall` in either order.
    assert_eq!(
        rules(
            variable,
            base(CompilerIdentity::Gcc)
                .with_warning_option(DiagnosticGroup::Unused, off)
                .with_all_warnings(true)
        ),
        []
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
    let comparison = "void f(int a) { a == 1; }";
    assert_eq!(rules(comparison, base(CompilerIdentity::Gcc)), []);
    assert_eq!(
        rules(comparison, base(CompilerIdentity::Clang)),
        [U::ClangEquality]
    );
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

/// Clang's CFG, and not code generation's insertion point, decides a
/// discarded value: each form here is silent in Clang 23.1.1, and warned about
/// by GCC 14.2, which does not ask.
#[test]
fn unreachable_discarded_values_are_silent_for_clang_only() {
    for source in [
        // Loops that never run, by any spelling of the constant.
        "void f(void) { while (0) { 42; } }",
        "void f(void) { for (; 0;) { 42; } }",
        "void f(void) { for (; 0; 42); }",
        "void f(void) { while (1 - 1) 42; }",
        "enum { Z }; void f(void) { while (Z) { 42; } }",
        "void f(void) { while ((void *)0) { 42; } }",
        "void f(void) { while (0.0) { 42; } }",
        "void f(void) { const int c = 0; while (c) { 42; } }",
        // Loops that never end.
        "void f(void) { for (; 1;) {} 42; }",
        "void f(void) { do {} while (1); 42; }",
        "void f(void) { for (;; 42) { break; } }",
        // A condition the CFG decides though it holds a variable.
        "void f(int a) { while (a && 0) { 42; } }",
        "void f(int a) { while (a || 1) {} 42; }",
        "void f(int a) { if (a & 0) { 42; } }",
        "void f(int a) { if (a * 0) { 42; } }",
        "void f(int a) { if (!(a && 0)) {} else { 42; } }",
        "int g(void); void f(void) { if (g(), 0) { 42; } }",
        "void f(int a) { if ((a = 1, 0)) { 42; } }",
        "void f(int a, int b) { if (a && (b & 0)) { 42; } }",
        // The tautologies it recognizes.
        "void f(int a) { if (a | 1) {} else { 42; } }",
        "void f(int a) { if (a == 1 && a == 2) { 42; } }",
        "void f(int a) { if (a != 1 || a != 2) {} else { 42; } }",
        "void f(int a) { if ((a & 8) == 1) { 42; } }",
        "void f(int a) { if ((a | 1) == 0) { 42; } }",
        "void f(int a) { if ((a < 1) == 2) { 42; } }",
        "void f(int a) { if ((a < 1) < 2) {} else { 42; } }",
        // Calls that do not return, wherever they are.
        "void f(void) { __builtin_trap(); 42; }",
        "void f(void) { __builtin_trap(); { 42; } }",
        "void f(int a) { if (a) __builtin_trap(); else __builtin_trap(); 42; }",
        "void f(int a) { a ? __builtin_trap() : __builtin_trap(); 42; }",
        "void f(void) { __builtin_trap(), 42; }",
        "void f(void) { int x = (__builtin_trap(), 0); (void)x; 42; }",
        "void f(void) { do { __builtin_trap(); } while (0); 42; }",
        "void f(void) { while (__builtin_trap(), 1) { 42; } }",
        "void f(void) { __builtin_assume(0); 42; }",
        "void f(void) { ({ __builtin_trap(); }); 42; }",
        "void f(void) { int x = ({ return; 0; }); (void)x; 42; }",
        // Labels nothing reachable jumps to.
        "void f(void) { return; x: 42; }",
        "void f(void) { return; x: 42; goto x; }",
        "void f(void) { goto y; x: 42; y: return; }",
        "void f(void) { if (0) { x: 42; } }",
        "void f(void) { while (0) { x: 42; } }",
        "void *p; void f(void) { return; x: 42; p = &&x; }",
        "void f(void) { while (0) { ({ __label__ x; x: 42; 0; }); } ({ __label__ x; goto x; x:; \
         }); }",
        // Switches.
        "void f(int a) { switch (a) { case 0: return; default: return; } 42; }",
        "void f(void) { switch (1) { case 0: 42; default:; } }",
        "void f(void) { switch (1) { case 0 ... 2: return; } 42; }",
        "void f(void) { switch (1) { default: 42; case 1:; } }",
        "enum E { A, B }; void f(enum E e) { switch (e) { case A: return; case B: return; } 42; }",
        "enum E { A, B }; void f(enum E e) { switch (e) { case A: break; case B: break; default: \
         42; } }",
        "enum E { A, B }; void f(enum E *e) { switch (*e) { case A ... B: return; } 42; }",
        // A statement expression's value, and an integer's address.
        "void f(int a) { if (({ (void)a; 0; })) { 42; } }",
        "int g(void); void f(void) { if (({ g(); 0; })) { 42; } }",
        "void f(int a) { while (({ a = 1; 0; })) { 42; } }",
        "void f(void) { if (({ int x = 0; 0; })) { 42; } }",
        "void f(void) { if (({ {} 0; })) { 42; } }",
        "void f(void) { if ((char *)1) {} else { 42; } }",
        "void f(void) { while ((void *)-1) {} 42; }",
        "void f(void) { if ((char *)1 == 0) { 42; } }",
        // The left operand of a comma.
        "void f(int a) { while (0) { 42, a = 1; } }",
        "void f(int a) { __builtin_trap(); 42, a = 1; }",
    ] {
        let unused_value = |rules: &[U]| {
            rules
                .iter()
                .filter(|rule| rule.group() == DiagnosticGroup::UnusedValue)
                .count()
        };
        assert_eq!(unused_value(&rules(source, clang())), 0, "{source}");
        assert_ne!(unused_value(&rules(source, gcc())), 0, "{source}");
    }
}

/// What Clang's CFG does reach, against
/// [`unreachable_discarded_values_are_silent_for_clang_only`].
#[test]
fn reachable_discarded_values_still_warn_for_clang() {
    for source in [
        "void f(void) { 42; return; }",
        "void f(void) { while (1) { 42; } }",
        "void f(void) { while (1) { break; } 42; }",
        "void f(int a) { while (a) {} 42; }",
        "void f(void) { for (;;) { 42; } }",
        "void f(void) { for (; 1;) { 42; } }",
        "void f(void) { for (;; 42) { continue; } }",
        "void f(void) { for (42; 0;); }",
        "void f(void) { do { 42; } while (0); }",
        "void f(void) { do { break; } while (1); 42; }",
        "void f(int a) { do { if (a) break; } while (1); 42; }",
        "void f(void) { while (sizeof(int) == 8) {} 42; }",
        "void f(void) { if (0) 42; else 42; }",
        "void f(int a) { if (a) return; 42; }",
        "void f(int a) { if ((a = 0)) { 42; } }",
        // Inside parentheses only the evaluation sees through.
        "void f(int a) { if ((a & 0)) { 42; } }",
        "void f(int a) { if ((a | 1)) {} else { 42; } }",
        // A statement expression that does not evaluate.
        "void f(int a) { if (({ int x = a; 0; })) { 42; } }",
        "void f(int a) { if (({ if (a); 0; })) { 42; } }",
        "void f(int a) { if (({ if (a) return; 0; })) { 42; } }",
        // Not what the tautologies recognize.
        "void f(int a) { if (a > 2 && a < 5) { 42; } }",
        "void f(int a) { if ((a & 3) == 1) { 42; } }",
        "void f(int a) { if ((a < 1) == 1) { 42; } }",
        "void f(int a) { if ((a < 1) > -1) {} else { 42; } }",
        "void f(int *p) { if (*p == 1 && *p == 2) { 42; } }",
        "enum { A }; enum { B = 1 }; void f(int a) { if (a == A && a == B) { 42; } }",
        // Labels something reaches.
        "void f(void) { goto x; x: 42; }",
        "void f(void) { goto y; x: 42; return; y: goto x; }",
        "void f(void) { if (0) { x: 42; } goto x; }",
        "void f(void) { while (0) { x: 42; } goto x; }",
        "void f(void) { goto x; while (0) { x:; } 42; }",
        "void *p; void f(void) { p = &&x; goto *p; x: 42; }",
        "void f(void) { ({ __label__ x; goto x; x:; }); ({ x: 42; 0; }); }",
        // Calls that return, or that may not run.
        "void g(void); void f(void) { g(); 42; }",
        "void f(int a) { if (a) __builtin_trap(); 42; }",
        "void f(void) { __builtin_debugtrap(); 42; }",
        "void f(void) { __builtin_assume(1); 42; }",
        "void f(void) { (void)sizeof(__builtin_trap(), 1); 42; }",
        "void f(void) { (void)_Generic(0, int: 0, default: (__builtin_trap(), 0)); 42; }",
        "void f(void) { (void)__builtin_choose_expr(1, 0, (__builtin_trap(), 0)); 42; }",
        // Switches.
        "void f(int a) { switch (a) { case 0: 42; } }",
        "void f(int a) { switch (a) { case 0: return; } 42; }",
        "void f(void) { switch (1) { case 1: 42; } }",
        "void f(void) { switch (1) { case 0:; default: 42; } }",
        "void f(void) { switch (0) { case 1: return; } 42; }",
        "enum E { A, B }; void f(enum E e) { switch (e) { case A: return; } 42; }",
        "enum E { A, B }; void f(enum E e) { switch ((int)e) { case A: return; case B: return; } \
         42; }",
        "enum E { A, B }; void f(enum E e) { switch (e + 0) { case A: return; case B: return; } \
         42; }",
        "enum E { A, B }; void f(enum E e) { switch (e) { case A: break; case B: return; } 42; }",
    ] {
        assert!(
            rules(source, clang())
                .iter()
                .any(|rule| rule.group() == DiagnosticGroup::UnusedValue),
            "{source}",
        );
    }
}

/// Clang's comparison warning reports at once, without asking.
#[test]
fn discarded_comparisons_warn_even_when_unreachable() {
    for source in [
        "void f(int a) { while (0) { a == 1; } }",
        "void f(int a) { __builtin_trap(); a < 1; }",
        "void f(int a) { return; a != 1; }",
    ] {
        assert_eq!(rules(source, clang()).len(), 1, "{source}");
    }
}

/// Reachability says nothing about what counts as used: a variable, function
/// or label named only in unreachable code is used for both compilers.
#[test]
fn uses_in_unreachable_code_still_count() {
    for source in [
        "void f(void) { int a; while (0) { (void)a; } }",
        "void f(void) { int a = 0; __builtin_trap(); (void)a; }",
        "static void g(void) {} void f(void) { while (0) { g(); } }",
        "static void g(void) {} void f(void) { __builtin_trap(); g(); }",
        "static int g; int f(void) { return 0; return g; }",
        "void f(void) { while (0) { goto x; x:; } }",
    ] {
        assert_eq!(rules(source, clang()), [], "{source}");
        assert_eq!(rules(source, gcc()), [], "{source}");
    }
}

/// Clang asks about the expression its warning is about: a conditional's last
/// arm, so that a condition known true silences the warning, and known false
/// does not, whatever the arms are.
#[test]
fn a_discarded_conditional_asks_about_its_last_arm() {
    for (source, warns) in [
        ("void f(int a) { 1 ? 0 : 42; }", false),
        ("void f(int a) { 0 ? 0 : 42; }", true),
        ("void f(int a) { a || 1 ? a : a; }", false),
        ("void f(int a) { (a || 1) ? 0 : 42; }", false),
        ("void f(int a) { a && 0 ? 0 : 42; }", true),
        ("void f(int a) { a ? 0 : 42; }", true),
        ("void f(int a, int b) { a || b ? a : b; }", true),
    ] {
        let found = rules(source, clang())
            .iter()
            .any(|rule| rule.group() == DiagnosticGroup::UnusedValue);
        assert_eq!(found, warns, "{source}");
    }
}

/// A statement expression's last statement gives it its value, and discards
/// nothing, wherever the statement expression is.
#[test]
fn a_statement_expression_value_is_not_discarded_in_unreachable_code() {
    for source in [
        "void f(int n) { return; int z = ({ n; }); (void)z; }",
        "void f(int n) { return; int z = ({ (void)0; n; }); (void)z; }",
        "void f(int n) { return; (void)({ (void)0; n; }); }",
        "void f(int n) { return; int z[({ n; })]; (void)z; }",
        "void f(int n) { return; int (*z)[({ n; })]; (void)z; }",
    ] {
        let unused_value = |rules: &[U]| {
            rules
                .iter()
                .filter(|rule| rule.group() == DiagnosticGroup::UnusedValue)
                .count()
        };
        assert_eq!(unused_value(&rules(source, gcc())), 0, "{source}");
        assert_eq!(unused_value(&rules(source, clang())), 0, "{source}");
    }
}
