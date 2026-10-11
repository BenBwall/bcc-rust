//! Executing and IR regressions for the PR #39 lowering reviews.
use super::*;
use crate::backend::interpreter::{
    Outcome,
    RuntimeValue,
    Termination,
    run_text,
};

#[test]
fn thousands_of_string_symbols_use_linear_arena_space() {
    use std::fmt::Write as _;

    use crate::ir::{
        Align,
        GlobalDecl,
        Linkage,
    };
    let mut source = String::new();
    for index in 0..4096 {
        writeln!(source, "char *text{index} = \"literal{index}\";").unwrap();
    }
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        tu.alloc_str(&source),
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    assert!(sema.lowerable(&context));
    let arena = Bump::new();
    let tables = Bump::new();
    let mut unit = Unit::new(&context, &sema, Target::LinuxGnu, &arena, &tables);
    // Pre-existing symbols must still be skipped by the suffix counter.
    for name in [".str", ".str.1", ".str.3"] {
        _ = unit.module.declare_global(
            name,
            GlobalDecl {
                linkage:  Linkage::Internal,
                size:     0,
                align:    Align::BYTE,
                constant: true,
            },
        );
    }
    let mut errors = ArenaVec::new_in(&arena);
    drop(unit.declare_items(&syntax, &mut errors));
    assert!(errors.is_empty(), "{errors:?}");
    for suffix in (2..=4098).filter(|&suffix| suffix != 3) {
        assert!(
            unit.module.symbol(&format!(".str.{suffix}")).is_some(),
            "missing unique string symbol {suffix}"
        );
    }
    assert!(
        tables.used() < 4 * 1024 * 1024,
        "retained {} table bytes",
        tables.used()
    );
}

fn check(source: &str) {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let text = lower_for(source, target).unwrap();
        let result = run_text(&text, "main");
        assert!(
            matches!(
                result.result,
                Ok(Outcome {
                    termination: Termination::Returned(Some(RuntimeValue::Int(0))),
                    ..
                })
            ),
            "{target:?}: {result:?}"
        );
    }
}

/// Both the lowered and optimized IR must retain observable accesses.
fn access_texts(source: &str) -> [String; 2] {
    let text = lowered(source);
    let arena = Bump::new();
    let mut module = parse_module(&arena, &text).unwrap();
    let scratch = Bump::new();
    _ = crate::optimizer::optimize(
        &mut module,
        &crate::optimizer::OptimizerOptions::default(),
        &scratch,
    );
    let verifier = Bump::new();
    assert!(
        verify_module(&module, Profile::PreAbi, &verifier).is_empty(),
        "{module}"
    );
    [text, module.to_string()]
}

#[test]
fn replacing_string_initializer_clears_automatic_suffix() {
    check_replacing_string("");
}

#[test]
fn replacing_string_initializer_clears_static_suffix() {
    check_replacing_string("static ");
}

fn check_replacing_string(storage: &str) {
    let source = format!(
        "int main(void) {{ {storage}char a[2][4] = {{ [1] = \"abc\", [0] = \"abc\", [0] = \"x\" \
         }}; {storage}char b[1][3] = {{ [0] = \"abc\", [0] = \"x\" }}; return a[0][0] != 'x' || \
         a[0][1] || a[0][2] || a[0][3] || a[1][0] != 'a' || a[1][1] != 'b' || a[1][2] != 'c' || \
         a[1][3] || b[0][0] != 'x' || b[0][1] || b[0][2]; }}"
    );
    check(&source);
    for text in access_texts(&source) {
        let result = run_text(&text, "main");
        assert!(
            matches!(
                result.result,
                Ok(Outcome {
                    termination: Termination::Returned(Some(RuntimeValue::Int(0))),
                    ..
                })
            ),
            "{result:?}\n{text}"
        );
    }
}

#[test]
fn replacing_volatile_string_initializer_clears_suffix_observably() {
    let source = "int main(void) { volatile char a[2][4] = { [0] = \"abc\", [0] = \"x\" }; return \
                  a[0][2]; }";
    check(source);
    for text in access_texts(source) {
        // Whole-object zeroing plus the replacement string's implicit tail.
        assert_volatile_accesses(&text, "fill ", 2);
        // Both explicit string initializations remain observable.
        assert_volatile_accesses(&text, "copy ", 2);
    }
}

#[test]
fn repeated_braced_subobject_clears_implicit_members() {
    check(
        "struct S { int a[2]; int b; }; static struct S s = { .a = {1, 2}, .b = 7, .a = {3} }; \
         int main(void) { struct S t = { .a = {1, 2}, .b = 7, .a = {3} }; return s.a[1] != 0 || \
         t.a[1] != 0 || s.a[0] != 3 || t.a[0] != 3 || s.b != 7 || t.b != 7; }",
    );
}

#[test]
fn nested_designators_restart_for_each_braced_list() {
    check(
        "static int s[2][3] = {{[1] = 1, 3}, {[1] = 2, 4}}; struct S { int a, b; }; static struct \
         S t[2] = {{.b = 5}, {.b = 6}}; int main(void) { int a[2][3] = {{[1] = 1, 3}, {[1] = 2, \
         4}}; struct S u[2] = {{.b = 5}, {.b = 6}}; return s[0][1] != 1 || s[1][1] != 2 || \
         s[1][0] != 0 || s[0][2] != 3 || s[1][2] != 4 || a[0][1] != 1 || a[1][1] != 2 || a[1][0] \
         != 0 || a[0][2] != 3 || a[1][2] != 4 || t[1].b != 6 || t[1].a != 0 || u[1].b != 6 || \
         u[1].a != 0; }",
    );
}

#[test]
fn static_unsigned_integers_convert_by_magnitude() {
    check(
        "double x = (unsigned __int128)1 << 127; float f = 0xffffffffffffffffULL; double d = \
         0xffffffffffffffffULL; double neg = -42; int main(void) { return x < 0 || x != 0x1p127 \
         || f != 0x1p64f || d != 0x1p64 || neg != -42; }",
    );
}

#[test]
fn static_unsigned_long_long_converts_exactly_to_x87() {
    let text = lower_for("long double l = 0xffffffffffffffffULL;", Target::LinuxGnu).unwrap();
    assert!(text.contains("ffffffffffffffff3e40"), "{text}");
}

#[test]
fn static_overwrites_replace_pointer_relocations() {
    check(
        "int a, b; int *p[1] = { [0] = &a, [0] = 0 }; int *q[1] = { [0] = &a, [0] = &b }; union U \
         { int *p; unsigned char bytes[8]; }; union U u = { .p = &a, .bytes[1] = 0 }; int \
         main(void) { for (int i = 0; i < 8; i++) if (u.bytes[i]) return 3; return p[0] != 0 || \
         q[0] != &b; }",
    );
}

#[test]
fn incomplete_extern_array_is_completed_before_initialized_definition() {
    check("extern int a[]; int *p = a; int a[3] = {1, 2, 3}; int main(void) { return p[2] != 3; }");
}

#[test]
fn incomplete_extern_array_is_completed_before_zero_definition() {
    check(
        "extern int a[]; int *p = a; int a[3]; int main(void) { if (p[2] != 0) return 1; p[2] = \
         7; return a[2] != 7; }",
    );
}

#[test]
fn tentative_array_uses_final_composite_type() {
    check(
        "int a[]; int *p = a; int a[2]; int main(void) { if (p[1] != 0) return 1; p[1] = 9; \
         return a[1] != 9; }",
    );
}

#[test]
fn forward_declared_struct_uses_completed_layout() {
    check(
        "struct S; extern struct S s; struct S *p = &s; struct S { int x; long y; }; struct S s = \
         {1, 42}; int main(void) { return p->y != 42; }",
    );
}

#[test]
fn volatile_scalar_initialization_is_an_observable_store() {
    for text in access_texts("void f(void) { volatile int x = 1; }") {
        assert!(text.contains("store.i32 volatile"), "{text}");
    }
}

#[test]
fn volatile_aggregate_initialization_preserves_all_destination_accesses() {
    for text in access_texts(
        "struct S { unsigned x:3; int a[2]; char s[3]; }; void f(void) { volatile struct S x = \
         {1, {2}, \"hi\"}; volatile struct S y = x; }",
    ) {
        // Bit-field and array-element stores; string, source-read and
        // destination copies; zeroing x, its array member and y.
        assert_volatile_accesses(&text, "store.", 2);
        assert_volatile_accesses(&text, "copy ", 3);
        assert_volatile_accesses(&text, "fill ", 3);
    }
}

#[test]
fn volatile_members_and_compound_literals_keep_initialization_qualifiers() {
    for text in access_texts(
        "struct S { volatile int x; volatile unsigned b:3; }; void f(void) { struct S s = {1, 2}; \
         (volatile struct S){3, 4}; }",
    ) {
        assert_volatile_accesses(&text, "store.", 4);
        assert_volatile_accesses(&text, "copy ", 1);
        // The unqualified object's whole-object fill is not volatile;
        // the compound literal's whole-object fill must be observable.
        assert_eq!(
            text.lines()
                .filter(|line| line.trim_start().starts_with("fill "))
                .count(),
            2,
            "{text}"
        );
        assert_volatile_accesses(&text, "fill volatile ", 1);
    }
}

#[test]
fn volatile_aggregate_reads_are_observable_when_copied_or_passed() {
    for source in [
        "struct S { int x; }; volatile struct S s = {42}; int main(void) { struct S t = s; return \
         0; }",
        "struct S { int x; }; volatile struct S s = {42}; void sink(struct S); void f(void) { \
         sink(s); }",
        "struct S { int x; }; volatile struct S s = {42}; struct S f(void) { return s; }",
        "struct S { int x; }; volatile struct S s = {42}; void f(void) { struct S t; t = s; }",
    ] {
        for text in access_texts(source) {
            assert!(
            text.lines()
                .any(|line| line.trim_start().starts_with("copy ") && line.contains("volatile")),
            "{text}"
            );
        }
    }
}

#[test]
fn volatile_array_initialization_keeps_range_accesses() {
    for text in access_texts(
        "void f(void) { volatile char s[3] = \"hi\"; volatile int a[2][2] = {{1}, {2}}; }",
    ) {
        assert_volatile_accesses(&text, "store.", 2);
        assert_volatile_accesses(&text, "copy ", 1);
        assert_volatile_accesses(&text, "fill ", 4);
    }
}

/// Count the required accesses before checking their flags, so deleting an
/// initialization cannot make the regression pass vacuously.
fn assert_volatile_accesses(text: &str, prefix: &str, expected: usize) {
    let accesses: Vec<_> = text
        .lines()
        .filter(|line| line.trim_start().starts_with(prefix))
        .collect();
    assert_eq!(accesses.len(), expected, "{prefix}:\n{text}");
    for line in accesses {
        assert!(line.contains("volatile"), "{line}\n{text}");
    }
}
