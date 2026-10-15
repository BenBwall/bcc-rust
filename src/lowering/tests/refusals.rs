//! What lowering refuses: units with errors, and valid C outside the
//! supported subset, each reported at its syntax rather than lowered wrong.

use super::*;
use crate::diagnostics::ToDiagnostic;

#[test]
fn asm_collision_keeps_checking_later_function_bodies() {
    let errors = lower(
        "int a __asm__(\"same\")=1; int b __asm__(\"same\")=2;\nint f(int n){{static int same; \
         int *p=&same;}int a[n]; return 0;}\nint g(void){__asm__(\"nop\");return 1;}",
    )
    .unwrap_err();
    assert_eq!(errors.len(), 2, "{errors:?}");
    for construct in ["assembler labels", "inline assembly"] {
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
        ("void f(void) { __asm__(\"nop\"); }", "inline assembly"),
        ("_Atomic int a; int f(void) { return a; }", "atomic types"),
        (
            "_Complex double z; int f(void) { return __real__ z; }",
            "complex arithmetic",
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
    assert_eq!(
        message,
        "not yet supported by lowering: a type built from an extension semantic analysis does not \
         model"
    );
}

#[test]
fn every_failing_function_is_reported() {
    let errors = lower(
        "__float128 f(__float128 x) { return x; }
int ok(int a) { return a; }
int g(int n) { __asm__(\"nop\"); return n; }",
    )
    .expect_err("two functions are unsupported");
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn errors_point_at_their_syntax_and_render() {
    let tu = Bump::new();
    let source = tu.alloc_str("int f(int n) {\n  __asm__(\"nop\");\n  return 0;\n}\n");
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
    .expect_err("inline assembly is unsupported");
    let error = errors.errors[0];
    assert_eq!(
        error.kind,
        LoweringErrorKind::Unsupported(Construct::InlineAssembly)
    );
    let span = error.source.expect("the error has a source");
    let diagnostic = error.to_diagnostic(&context, span);
    assert_eq!(
        diagnostic.message,
        "not yet supported by lowering: inline assembly"
    );
}
