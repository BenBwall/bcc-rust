//! What lowering refuses: units with errors, and valid C outside the
//! supported subset, each reported at its syntax rather than lowered wrong.

use super::*;
use crate::diagnostics::ToDiagnostic;

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
            "double f(double x) { return x * 2; }",
            "floating-point arithmetic",
        ),
        (
            "struct s { int a : 3; }; int f(struct s *p) { return p->a; }",
            "bit-fields",
        ),
        (
            "int f(int n) { int a[n]; a[0] = 1; return a[0]; }",
            "variable length arrays",
        ),
        (
            "struct s { int a; }; int f(struct s v) { return v.a; }",
            "structures and unions passed by value",
        ),
        (
            "struct s { int a; }; struct s f(void) { struct s v = { 1 }; return v; }",
            "structures and unions returned by value",
        ),
        ("int f(void) { return ({ 1; }); }", "statement expressions"),
        ("void f(void) { __asm__(\"nop\"); }", "inline assembly"),
        ("_Atomic int a; int f(void) { return a; }", "atomic types"),
        (
            "int f(void) { void *p = &&l; l: return p != 0; }",
            "label addresses",
        ),
        (
            "int f(int x) { return x ?: 1; }",
            "conditionals with an omitted operand",
        ),
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

#[test]
fn every_failing_function_is_reported() {
    let errors = lower(
        "double f(double x) { return x; }
int ok(int a) { return a; }
struct s { int b : 2; }; int g(struct s *p) { return p->b; }",
    )
    .expect_err("two functions are unsupported");
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn errors_point_at_their_syntax_and_render() {
    let tu = Bump::new();
    let source = tu.alloc_str("int f(int n) {\n  int a[n];\n  return 0;\n}\n");
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
    .expect_err("a variable length array is unsupported");
    let error = errors.errors[0];
    assert_eq!(
        error.kind,
        LoweringErrorKind::Unsupported(Construct::VariableLengthArray)
    );
    let span = error.source.expect("the error has a source");
    let diagnostic = error.to_diagnostic(&context, span);
    assert_eq!(
        diagnostic.message,
        "not yet supported by lowering: variable length arrays"
    );
}
