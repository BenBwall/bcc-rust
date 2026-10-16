//! `__func__`, `__FUNCTION__` and `__PRETTY_FUNCTION__`: types, sharing,
//! file scope, shadowing, the text of `__PRETTY_FUNCTION__` and the modes in
//! which each is accepted. C99: §6.4.2.2 paragraph 1, p. 52; PDF p. 64.
//! GNU extension: GCC manual, "Function Names as Strings".

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        TranslationError,
        parsing::declaration_syntax::TypeQualifiers,
    },
};

fn kinds(source: &str, configuration: CompilerConfiguration) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, configuration, |context, _| {
        kinds.extend(
            context
                .take_pending_errors()
                .iter()
                .filter_map(|e| match e {
                    | TranslationError::Semantic(e) => Some(e.kind),
                    | _ => None,
                }),
        );
    });
    kinds
}

fn kinds_of(source: &str) -> Vec<SemanticErrorKind> {
    kinds(source, CompilerConfiguration::default())
}

/// The `__PRETTY_FUNCTION__` text of each function that names it, in order.
fn pretty(source: &str) -> Vec<String> {
    let mut texts = Vec::new();
    with_source(source, |context, unit| {
        assert_eq!(
            context.pending_error_count(),
            0,
            "{source}: {:?}",
            context.take_pending_errors()
        );
        for definition in unit.definitions {
            if let DefinitionKind::PrettyFunctionName { text, .. } = definition.kind {
                texts.push(String::from(context.string_cache.at(text)));
            }
        }
    });
    texts
}

fn one(source: &str) -> String {
    let mut texts = pretty(source);
    assert_eq!(texts.len(), 1, "{source}");
    texts.remove(0)
}

#[test]
fn the_three_names_are_static_const_char_arrays() {
    with_source(
        "int a = sizeof __func__, b = sizeof __FUNCTION__, c = sizeof __PRETTY_FUNCTION__;\nvoid \
         fun(int x) { int a = sizeof __func__, b = sizeof __FUNCTION__, c = sizeof \
         __PRETTY_FUNCTION__; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 3);
            let sizes: Vec<_> = unit
                .expressions
                .iter()
                .filter(|e| matches!(e.expression.kind, ExpressionType::SizeofExpr(_)))
                .map(|e| e.integer.unwrap().value)
                .collect();
            // File scope: the empty string and `top level`; then `fun`.
            assert_eq!(sizes, [1, 1, 10, 4, 4, 14]);
        },
    );
    with_source(
        "void fun(void) { const char *a = __FUNCTION__, *b = __PRETTY_FUNCTION__; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            for name in ["__func__", "__PRETTY_FUNCTION__"] {
                let binding = unit
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .unwrap();
                assert_eq!(binding.duration, Duration::Static);
                assert_eq!(binding.linkage, Linkage::None);
                assert!(matches!(
                    unit.types.nodes[binding.ty.index],
                    TypeKind::Array(element, ArrayBound::Constant(_))
                        if element.qualifiers.contains(TypeQualifiers::CONST)
                ));
            }
        },
    );
}

#[test]
fn function_is_the_same_object_as_func() {
    with_source(
        "void f(void) { const char *a = __func__, *b = __FUNCTION__, *c = __func__, *d = \
         __FUNCTION__; { const char *e = __FUNCTION__; } }\nvoid g(void) { const char *a = \
         __FUNCTION__; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let named: Vec<_> = unit
                .bindings
                .iter()
                .enumerate()
                .filter(|(_, b)| context.string_cache.at(b.name.name) == "__func__")
                .map(|(index, _)| index)
                .collect();
            assert_eq!(named.len(), 2);
            let used: Vec<_> = unit
                .expressions
                .iter()
                .filter(|e| matches!(e.expression.kind, ExpressionType::Identifier(_)))
                .filter_map(|e| e.binding)
                .filter(|b| named.contains(b))
                .collect();
            assert_eq!(
                used,
                [named[0], named[0], named[0], named[0], named[0], named[1]]
            );
            // No separate `__FUNCTION__` object exists.
            assert!(
                unit.bindings
                    .iter()
                    .all(|b| context.string_cache.at(b.name.name) != "__FUNCTION__")
            );
        },
    );
}

#[test]
fn pretty_function_is_one_object_per_function() {
    with_source(
        "void f(void) { const char *a = __PRETTY_FUNCTION__; { const char *b = \
         __PRETTY_FUNCTION__; } const char *c = __PRETTY_FUNCTION__; }\nvoid g(void) { const char \
         *a = __PRETTY_FUNCTION__; } void h(void) {}",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let objects = unit
                .definitions
                .iter()
                .filter(|d| matches!(d.kind, DefinitionKind::PrettyFunctionName { .. }))
                .count();
            assert_eq!(objects, 2, "h never names it");
        },
    );
}

#[test]
fn pretty_function_spells_the_signature_as_clang_does() {
    for (source, expected) in [
        ("void f(void) { __PRETTY_FUNCTION__; }", "void f(void)"),
        ("void f() { __PRETTY_FUNCTION__; }", "void f()"),
        (
            "int f(int a, char b) { __PRETTY_FUNCTION__; return 0; }",
            "int f(int, char)",
        ),
        (
            "int f(a, b) int a; char b; { __PRETTY_FUNCTION__; return 0; }",
            "int f()",
        ),
        (
            "void f(unsigned u, unsigned long ul, short s, _Bool b) { __PRETTY_FUNCTION__; }",
            "void f(unsigned int, unsigned long, short, _Bool)",
        ),
        (
            "void f(const char *a, char *const b, const char *const c, volatile int *restrict d, \
             void *e) { __PRETTY_FUNCTION__; }",
            "void f(const char *, char *const, const char *const, volatile int *restrict, void *)",
        ),
        (
            "struct S { int a; }; union U { int a; }; enum E { A };\nvoid f(struct S a, union U \
             *b, enum E c, const struct S *d) { __PRETTY_FUNCTION__; }",
            "void f(struct S, union U *, enum E, const struct S *)",
        ),
        (
            "void f(int a[3], int b[], int c[2][3], int n, int d[n]) { __PRETTY_FUNCTION__; }",
            "void f(int *, int *, int (*)[3], int, int *)",
        ),
        (
            "void f(int (*a)(int, char), void (*b)(void), int (*c)(), int (*d)(int, ...)) { \
             __PRETTY_FUNCTION__; }",
            "void f(int (*)(int, char), void (*)(void), int (*)(), int (*)(int, ...))",
        ),
        (
            "void f(int n, ...) { __PRETTY_FUNCTION__; }",
            "void f(int, ...)",
        ),
        (
            "static const char *f(int x) { __PRETTY_FUNCTION__; return 0; }",
            "const char *f(int)",
        ),
        (
            "inline unsigned f(unsigned x) { __PRETTY_FUNCTION__; return 0; }",
            "unsigned int f(unsigned int)",
        ),
        (
            "int (*f(int a))(char) { __PRETTY_FUNCTION__; return 0; }",
            "int (*f(int))(char)",
        ),
        (
            "int (*f(void))[4] { __PRETTY_FUNCTION__; return 0; }",
            "int (*f(void))[4]",
        ),
        (
            "void (*f(int a))(void) { __PRETTY_FUNCTION__; return 0; }",
            "void (*f(int))(void)",
        ),
        (
            "float _Complex f(double _Complex a, long double _Complex b) { __PRETTY_FUNCTION__; \
             return 0; }",
            "_Complex float f(_Complex double, _Complex long double)",
        ),
        (
            "void f(__int128 a, unsigned __int128 b, _Atomic(int) *c) { __PRETTY_FUNCTION__; }",
            "void f(__int128, unsigned __int128, _Atomic(int) *)",
        ),
        (
            "void f(int x __attribute__((vector_size(16))), long long *y \
             __attribute__((vector_size(8)))) { __PRETTY_FUNCTION__; }",
            "void f(__attribute__((__vector_size__(4 * sizeof(int)))) int, \
             __attribute__((__vector_size__(1 * sizeof(long long)))) long long *)",
        ),
    ] {
        assert_eq!(one(source), expected, "{source}");
    }
}

#[test]
fn pretty_function_keeps_the_typedef_names_as_written() {
    for (source, expected) in [
        (
            "typedef int T; T shadow2(void){int T; __PRETTY_FUNCTION__; return 0;}",
            "T shadow2(void)",
        ),
        (
            "typedef int T; void shadow_param(T x){int T; __PRETTY_FUNCTION__;}",
            "void shadow_param(T)",
        ),
        (
            "typedef unsigned long count_t; typedef int (*callback_t)(int); typedef struct { int \
             a; } box_t; typedef const int fixed_t;\nvoid f(count_t a, count_t *b, const count_t \
             *c, callback_t d, callback_t *e, box_t f, box_t *g, fixed_t h, const callback_t i) { \
             __PRETTY_FUNCTION__; }",
            "void f(count_t, count_t *, const count_t *, callback_t, callback_t *, box_t, box_t \
             *, fixed_t, const callback_t)",
        ),
        (
            "typedef unsigned long count_t; count_t f(void) { __PRETTY_FUNCTION__; return 0; }",
            "count_t f(void)",
        ),
        (
            "typedef unsigned long count_t; count_t *f(count_t (*p)[2]) { __PRETTY_FUNCTION__; \
             return 0; }",
            "count_t *f(count_t (*)[2])",
        ),
        // The array type adjusts to a pointer before it is printed.
        (
            "typedef int row_t[3]; void f(row_t a, row_t *b) { __PRETTY_FUNCTION__; }",
            "void f(int *, row_t *)",
        ),
        (
            "typedef void callback_t(int); void f(callback_t *a, callback_t b) { \
             __PRETTY_FUNCTION__; }",
            "void f(callback_t *, callback_t *)",
        ),
        (
            "typedef __SIZE_TYPE__ size_t; typedef __PTRDIFF_TYPE__ ptrdiff_t; void f(size_t a, \
             ptrdiff_t b, __int128_t c, __uint128_t *d) { __PRETTY_FUNCTION__; }",
            "void f(size_t, ptrdiff_t, __int128_t, __uint128_t *)",
        ),
    ] {
        assert_eq!(one(source), expected, "{source}");
    }
}

#[test]
fn pretty_function_spells_the_va_list_parameter_of_each_target() {
    use crate::target::Target;
    let source =
        "typedef __builtin_va_list list; void f(__builtin_va_list a, list b,                   \
         __builtin_va_list *c) { __PRETTY_FUNCTION__; }";
    for (target, expected) in [
        (
            Target::LinuxGnu,
            "void f(struct __va_list_tag *, struct __va_list_tag *, __builtin_va_list *)",
        ),
        (
            Target::WindowsGnu,
            "void f(__builtin_va_list, list, __builtin_va_list *)",
        ),
    ] {
        let mut texts = Vec::new();
        with_configuration(
            source,
            CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(context.pending_error_count(), 0);
                texts.extend(unit.definitions.iter().filter_map(|d| match d.kind {
                    | DefinitionKind::PrettyFunctionName { text, .. } =>
                        Some(String::from(context.string_cache.at(text))),
                    | _ => None,
                }));
            },
        );
        assert_eq!(texts, [expected], "{target:?}");
    }
}

#[test]
fn pretty_function_in_nested_constructs_names_the_innermost_function() {
    assert_eq!(
        pretty(
            "void outer(int a) { void inner(char b) { __PRETTY_FUNCTION__; } inner(0); \
             __PRETTY_FUNCTION__; }"
        ),
        ["void inner(char)", "void outer(int)"]
    );
    assert_eq!(
        pretty("int f(void) { return ({ int x = 1; sizeof __PRETTY_FUNCTION__; }); }"),
        ["int f(void)"]
    );
}

#[test]
fn outside_a_function_each_name_warns_and_is_empty() {
    for source in [
        "const char *s = __func__;",
        "const char *s = __FUNCTION__;",
        "const char *s = __PRETTY_FUNCTION__;",
        "char a[sizeof __func__];",
        "void f(int n[sizeof __PRETTY_FUNCTION__]);",
    ] {
        assert_eq!(
            kinds_of(source),
            [SemanticErrorKind::PredefinedOutsideFunction],
            "{source}"
        );
    }
    // Every use warns, and one object serves them all.
    assert_eq!(
        kinds_of("char a[sizeof __func__ + sizeof __FUNCTION__ + sizeof __func__];"),
        [SemanticErrorKind::PredefinedOutsideFunction; 3]
    );
    with_source(
        "const char *a = __func__, *b = __FUNCTION__, *c = __PRETTY_FUNCTION__, *d = \
         __PRETTY_FUNCTION__;",
        |context, unit| {
            let texts: Vec<_> = unit
                .definitions
                .iter()
                .filter_map(|d| match d.kind {
                    | DefinitionKind::FunctionName(text) =>
                        Some(("name", String::from(context.string_cache.at(text)))),
                    | DefinitionKind::PrettyFunctionName { function, text } => {
                        assert!(function.is_none());
                        Some(("pretty", String::from(context.string_cache.at(text))))
                    },
                    | _ => None,
                })
                .collect();
            assert_eq!(
                texts,
                [
                    ("name", String::new()),
                    ("pretty", String::from("top level"))
                ]
            );
        },
    );
}

#[test]
fn a_declaration_of_the_name_wins_over_the_predefined_object() {
    for source in [
        "int __PRETTY_FUNCTION__; int f(void) { return __PRETTY_FUNCTION__; }",
        "int f(void) { int __PRETTY_FUNCTION__ = 1; return __PRETTY_FUNCTION__; }",
        "int f(void) { { int __FUNCTION__ = 1; return __FUNCTION__; } }",
        "int f(void) { sizeof __FUNCTION__; { int __FUNCTION__ = 1; return __FUNCTION__; } }",
        "int f(void) { { int __PRETTY_FUNCTION__ = 1; } return sizeof __PRETTY_FUNCTION__ == 15; }",
    ] {
        assert_eq!(kinds_of(source), [], "{source}");
    }
}

#[test]
fn the_names_cannot_be_modified() {
    for name in ["__func__", "__FUNCTION__", "__PRETTY_FUNCTION__"] {
        let kinds = kinds_of(&format!("void f(void) {{ {name}[0] = 1; {name} = 0; }}"));
        assert_eq!(kinds.len(), 2, "{name}: {kinds:?}");
    }
    assert_eq!(
        kinds_of("void f(void) { char *p = __PRETTY_FUNCTION__; }").len(),
        1
    );
}

#[test]
fn the_gnu_spellings_are_accepted_silently_in_every_mode() {
    let source = "void f(void) { const char *a = __FUNCTION__, *b = __PRETTY_FUNCTION__; (void)a; \
                  (void)b; }";
    for gnu in [false, true] {
        for standard in [
            CStandard::C89,
            CStandard::C99,
            CStandard::C11,
            CStandard::C23,
        ] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                let configuration =
                    CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu);
                assert_eq!(
                    kinds(source, configuration),
                    [],
                    "{standard:?} {policy:?} {gnu}"
                );
            }
        }
    }
}
