//! What lowering refuses: units with errors, and valid C outside the
//! supported subset, each reported at its syntax rather than lowered wrong.

use super::*;
use crate::diagnostics::ToDiagnostic;

#[test]
fn asm_collision_keeps_checking_later_function_bodies() {
    let errors = lower(
        "int a __asm__(\"same\")=1; int b __asm__(\"same\")=2;\nint f(int n){{static int same; \
         int *p=&same;}int a[n]; return 0;}\nint g(void){__asm__(\"\" :: \"t\"(1.0L));return 1;}",
    )
    .unwrap_err();
    assert_eq!(errors.len(), 2, "{errors:?}");
    for construct in [
        "assembler labels",
        "inline assembly operands of this register class or type",
    ] {
        assert!(
            errors.iter().any(|error| error.ends_with(construct)),
            "{errors:?}"
        );
    }
}

/// The single error lowering reports for `source`.
#[track_caller]
fn refusal(source: &str) -> String {
    match lower(source) {
        | Ok(ir) => panic!("lowering accepted unsupported code:\n{source}\n{ir}"),
        | Err(errors) => {
            assert_eq!(errors.len(), 1, "{errors:?}");
            errors[0].clone()
        },
    }
}

#[test]
fn a_unit_with_errors_is_refused_whole() {
    assert_eq!(
        lower("int f(void) { return undeclared; }"),
        Err(vec![
            "code is not generated for a translation unit with errors".to_owned()
        ])
    );
}

#[test]
fn unsupported_constructs_name_themselves() {
    let cases = [
        (
            "__float128 f(__float128 x) { return x; }",
            "binary128 floating-point types",
        ),
        (
            "double _Complex f(void) { _Atomic(double _Complex) z; return z; }",
            "atomic complex values",
        ),
        (
            "void f(void) { __asm__(\"\" :: \"t\"(1.0L)); }",
            "inline assembly operands of this register class or type",
        ),
    ];
    for (source, construct) in cases {
        let message = refusal(source);
        assert!(
            message.ends_with(construct),
            "{source}: expected {construct}, got {message}"
        );
    }
}

/// C23 (N3220) §6.5.3.6p7: a `static` or `thread_local` compound literal
/// in a function outlives its block, and a thread-local one at file scope
/// is per thread. Lowering names them instead of allocating them in the
/// frame or as one shared object.
#[test]
fn compound_literals_with_static_or_thread_storage_name_themselves() {
    for source in [
        "int *f(void) { return &(static int){1}; }",
        "int *f(void) { return &(static _Thread_local int){1}; }",
        "int *p = &(_Thread_local int){1};",
    ] {
        let message = refusal(source);
        assert!(
            message.ends_with("compound literals with static or thread storage duration"),
            "{source}: {message}"
        );
    }
}

/// A type semantic analysis leaves unanalyzed is refused as unsupported,
/// naming why, rather than as a type that "was not analyzed".
#[test]
fn unmodeled_extension_types_are_refused_as_unsupported() {
    let message = refusal("int f(void) { _BitInt(9) b = 1; return b; }");
    assert_eq!(message, "not yet supported by lowering: _BitInt types");
}

#[test]
fn every_failing_function_is_reported() {
    let errors = lower(
        "__float128 f(__float128 x) { return x; }
int ok(int a) { return a; }
int g(int n) { __asm__(\"\" :: \"t\"(1.0L)); return n; }",
    )
    .expect_err("two functions are unsupported");
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn errors_point_at_their_syntax_and_render() {
    let tu = Bump::new();
    let source = tu.alloc_str("int f(int n) {\n  __asm__(\"\" :: \"t\"(1.0L));\n  return 0;\n}\n");
    let mut context = Context::new(&tu);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let arena = Bump::new();
    let mut scratch = Bump::new();
    let errors = lower_translation_unit(
        &context,
        &unit,
        &sema,
        Target::LinuxGnu,
        &arena,
        &mut scratch,
    )
    .expect_err("x87 assembly operands are unsupported");
    let error = errors.errors[0];
    assert_eq!(
        error.kind,
        LoweringErrorKind::Unsupported(Construct::InlineAssemblyOperand)
    );
    let span = error.source.expect("the error has a source");
    let diagnostic = error.to_diagnostic(&context, span);
    assert_eq!(
        diagnostic.message,
        "not yet supported by lowering: inline assembly operands of this register class or type"
    );
}

#[test]
fn unsupported_type_diagnostics_retain_the_actual_construct() {
    for (source, construct) in [
        ("_BitInt(9) value;", "_BitInt types"),
        ("int f(void){return sizeof(_BitInt(9));}", "_BitInt types"),
        ("int f(void){return _Alignof(_BitInt(9));}", "_BitInt types"),
        ("__typeof__(sizeof(_BitInt(9))) value;", "_BitInt types"),
        ("int f(void){return -(_BitInt(9))1;}", "_BitInt types"),
        ("int f(void){return (_BitInt(9))1+1;}", "_BitInt types"),
        ("int f(void){return 1?(_BitInt(9))1:1;}", "_BitInt types"),
        ("int f(void){return (int)(_BitInt(9))1;}", "_BitInt types"),
        (
            "struct S {_BitInt(9) x;}; extern struct S s; int f(void){return s.x;}",
            "_BitInt types",
        ),
        (
            "typedef _BitInt(9) Small; Small values[2];",
            "_BitInt types",
        ),
        (
            "int f(void) { constexpr int value=1; return value; }",
            "constexpr objects",
        ),
        ("_Decimal32 value;", "_Decimal32 types"),
        ("_Decimal64 value;", "_Decimal64 types"),
        ("_Decimal128 value;", "_Decimal128 types"),
        ("auto value=1;", "inferred declaration types"),
        (
            "typedef int Ext __attribute__((ext_vector_type(4))); Ext value;",
            "ext_vector_type types",
        ),
        (
            "__attribute__((ext_vector_type(4))) int value;",
            "ext_vector_type types",
        ),
        (
            "enum E : unsigned int { A }; enum E value;",
            "fixed-underlying enum types",
        ),
        (
            "struct S { _BitInt(9) value; }; struct S object;",
            "_BitInt types",
        ),
    ] {
        let errors = lower_with(
            source,
            crate::configuration::CompilerConfiguration::new(
                crate::configuration::CStandard::C23,
                crate::configuration::ExtensionPolicy::Allow,
            )
            .with_gnu_extensions(true),
        )
        .expect_err("specific construct is unsupported");
        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
        let message = &errors[0];
        assert_eq!(
            message,
            &format!("not yet supported by lowering: {construct}"),
            "{source}"
        );
    }
    // These supported constructs were listed in the old generic refusal note.
    // They must still compile beside an unrelated unsupported type.
    let message = refusal("_Thread_local int tls; __typeof__(int) ordinary; _BitInt(9) value;");
    assert!(message.ends_with("_BitInt types"), "{message}");
}

/// File-scope basic assembly reaches the module in source order, with
/// concatenated literals joined, between whatever functions and objects
/// surround it (GCC manual, "Basic Asm").
#[test]
fn file_scope_asm_reaches_the_module_in_source_order() {
    let ir = lowered(
        "__asm__(\".globl s\\ns: ret\");\nint f(void) { return 1; }\n__asm__(\"# two\" \" \
         joined\");\nint x = 3;\n__asm__(\"# \\\"three\\\"\");",
    );
    let lines: Vec<&str> = ir
        .lines()
        .filter(|line| line.starts_with("module asm"))
        .collect();
    assert_eq!(
        lines,
        [
            "module asm \".globl s\\u{a}s: ret\"",
            "module asm \"# two joined\"",
            "module asm \"# \\u{22}three\\u{22}\"",
        ],
        "{ir}"
    );
}

/// Clang rejects qualifiers, operands and wide or Unicode templates at file
/// scope; lowering reports each in Clang's words, as invalid rather than as
/// valid C it cannot translate. A template with NUL or bytes that are not
/// UTF-8 is valid but cannot reach the assembler as written, so it is
/// unsupported. None is dropped.
#[test]
fn file_scope_asm_that_cannot_be_carried_is_reported() {
    for (source, message) in [
        (
            "__asm__ volatile(\"nop\");",
            "meaningless 'volatile' on asm outside function",
        ),
        (
            "__asm__ inline(\"nop\");",
            "meaningless 'inline' on asm outside function",
        ),
        (
            "__asm__(\"nop\" : );",
            "file-scope asm cannot have operands, clobbers or labels",
        ),
        (
            "__asm__(L\"nop\");",
            "cannot use wide string literal in 'asm'",
        ),
        (
            "__asm__(\"a\\0b\");",
            "not yet supported by lowering: file-scope assembly templates containing NUL or bytes \
             that are not UTF-8",
        ),
        (
            "__asm__(\"\\xff\");",
            "not yet supported by lowering: file-scope assembly templates containing NUL or bytes \
             that are not UTF-8",
        ),
    ] {
        assert_eq!(lower(source), Err(vec![message.to_owned()]), "{source}");
    }
}

/// An invalid file-scope asm renders as an error at its syntax without the
/// note that calls unsupported code valid C.
#[test]
fn invalid_file_scope_asm_does_not_claim_to_be_valid_c() {
    let tu = Bump::new();
    let source = tu.alloc_str("__asm__ volatile(\"nop\");\n");
    let mut context = Context::new(&tu);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let arena = Bump::new();
    let mut scratch = Bump::new();
    let errors = lower_translation_unit(
        &context,
        &unit,
        &sema,
        Target::LinuxGnu,
        &arena,
        &mut scratch,
    )
    .expect_err("volatile file-scope asm is invalid");
    let error = errors.errors[0];
    let span = error.source.expect("the error has a source");
    let diagnostic = error.to_diagnostic(&context, span);
    assert_eq!(
        diagnostic.message,
        "meaningless 'volatile' on asm outside function"
    );
    let rendered = format!("{diagnostic:?}");
    assert!(!rendered.contains("valid C"), "{rendered}");
}
