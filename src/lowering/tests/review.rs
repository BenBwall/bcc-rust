//! Executing and IR regressions for the PR #39 lowering reviews.
use super::*;
use crate::backend::interpreter::{
    Outcome,
    RuntimeValue,
    Termination,
    run_text,
};

#[test]
fn nested_overlapping_unions_lower_with_bounded_arena_use() {
    // The modest case fails the resource budget safely before a regressed
    // implementation can attempt the full review reproducer's 2^35 leaves.
    for depth in [12, 34, 128] {
        assert_nested_union_resources(depth);
    }
}

fn assert_nested_union_resources(depth: usize) {
    use std::fmt::Write as _;

    let mut source =
        String::from("#define U(n,p) union U##n { union U##p a, b; };\nunion U0 { int a, b; };\n");
    for level in 1..=depth {
        writeln!(source, "U({level},{})", level - 1).unwrap();
    }
    writeln!(
        source,
        "typedef char size_check[sizeof(union U{depth}) == 4 ? 1 : -1];\nint f(union U{depth} x) \
         {{ return 0; }}\nint main(void) {{ union U{depth} x = {{ 7 }}; return f(x); }}",
    )
    .unwrap();
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
    let mut scratch = Bump::new();
    let start = std::time::Instant::now();
    let module = lower_translation_unit(
        &context,
        &syntax,
        &sema,
        Target::LinuxGnu,
        &arena,
        &mut scratch,
    )
    .unwrap();
    eprintln!(
        "nested unions ({depth} levels): {:?}, {} IR bytes, {} scratch bytes",
        start.elapsed(),
        arena.high_water(),
        scratch.high_water()
    );
    assert!(arena.high_water() < 64 * 1024);
    assert!(scratch.high_water() < 64 * 1024);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(
        module
            .to_string()
            .contains("aggregate(size 4, align 4, [0: i32])")
    );
    let execution = Bump::new();
    let mut host = crate::backend::interpreter::DefaultHost::new_in(&execution);
    let outcome = crate::backend::interpreter::run(&module, "main", &[], &mut host).unwrap();
    assert_eq!(
        outcome.termination,
        Termination::Returned(Some(RuntimeValue::Int(0)))
    );
}

#[test]
fn aggregate_flattening_preserves_overlapping_scalar_types_and_offsets() {
    let module = lowered(
        "union U { float f; int i; }; union V { union U a, b; };\nstruct S { union V items[2]; \
         };\nint f(struct S x) { return x.items[0].a.i + x.items[1].b.i; }\nint main(void) { \
         struct S x = { .items[0].a.i = 7, .items[1].b.i = 11 };\nreturn f(x); }",
    );
    assert!(module.contains("aggregate(size 8, align 4, [0: f32, 0: i32, 4: f32, 4: i32])"));
    let outcome = run_text(&module, "main").result.unwrap();
    assert_eq!(
        outcome.termination,
        Termination::Returned(Some(RuntimeValue::Int(18)))
    );
}

#[test]
fn twenty_thousand_union_switches_lower_quickly_with_bounded_arena_use() {
    assert_array_union_switch_resources(false);
}

#[test]
fn twenty_thousand_scattered_union_switches_lower_with_bounded_resources() {
    assert_array_union_switch_resources(true);
}

fn assert_array_union_switch_resources(scattered: bool) {
    use std::fmt::Write as _;

    let mut source = String::from(
        "union U { long long old; struct { int x, y; } next; }; int main(void) { union U a[20000] \
         = {",
    );
    for position in 0..20_000 {
        let index = if scattered {
            if position % 2 == 0 {
                position / 2
            } else {
                19_999 - position / 2
            }
        } else {
            position
        };
        write!(source, "[{index}].old = -1, [{index}].next.x = 1,").unwrap();
    }
    source.push_str(
        "}; return a[0].next.x != 1 || a[0].next.y || a[9999].next.x != 1 || a[9999].next.y || \
         a[19999].next.x != 1 || a[19999].next.y; }",
    );
    assert_union_switch_resources(
        &source,
        if scattered {
            "scattered array of unions"
        } else {
            "array of unions"
        },
    );
}

#[test]
fn twenty_thousand_same_union_switches_lower_with_bounded_resources() {
    let source = format!(
        "union U {{ long long old; struct {{ int x, y; }} next; }}; int main(void) {{ union U u = \
         {{{}}}; return !(u.next.x == 1 && u.next.y == 0); }}",
        ".old = -1, .next.x = 1,".repeat(20_000)
    );
    assert_union_switch_resources(&source, "same union");
}

/// Both distributions must fit the same time and arena budgets, and leave
/// the last selected member's omitted fields zero in the interpreter.
fn assert_union_switch_resources(source: &str, name: &str) {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        tu.alloc_str(source),
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    assert!(sema.lowerable(&context));
    let arena = Bump::new();
    let mut scratch = Bump::new();
    let start = std::time::Instant::now();
    let module = lower_translation_unit(
        &context,
        &syntax,
        &sema,
        Target::LinuxGnu,
        &arena,
        &mut scratch,
    )
    .unwrap();
    let elapsed = start.elapsed();
    eprintln!(
        "20,000 switches ({name}): {elapsed:?}, {} IR bytes, {} scratch bytes",
        arena.high_water(),
        scratch.high_water()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "lowering took {elapsed:?}"
    );
    assert!(arena.high_water() < 128 * 1024 * 1024);
    assert!(scratch.high_water() < 128 * 1024 * 1024);
    let execution = Bump::new();
    let mut host = crate::backend::interpreter::DefaultHost::new_in(&execution);
    let outcome = crate::backend::interpreter::run(&module, "main", &[], &mut host).unwrap();
    assert_eq!(
        outcome.termination,
        Termination::Returned(Some(RuntimeValue::Int(0)))
    );
}

#[test]
fn ten_thousand_static_locals_use_linear_name_storage() {
    let source = format!(
        "int reserved __asm__(\"main.x.1\"); int main(void){{{}return 0;}}",
        "{static int x;}".repeat(10_000)
    );
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
    let scratch = Bump::new();
    let mut unit = Unit::new(&context, &sema, Target::LinuxGnu, &arena, &tables).unwrap();
    let mut errors = ArenaVec::new_in(&arena);
    unit.prepare_extensions(&mut errors);
    let functions = unit.declare_items(&syntax, &mut errors);
    let start = std::time::Instant::now();
    for (record, function) in functions {
        FunctionLowerer::lower(&mut unit, record, function.unwrap(), &scratch).unwrap();
    }
    assert!(errors.is_empty(), "{errors:?}");
    assert!(unit.module.symbol("main.x").is_some());
    for suffix in 2..=10_000 {
        assert!(unit.module.symbol(&format!("main.x.{suffix}")).is_some());
    }
    assert!(
        tables.high_water() < 16 * 1024 * 1024,
        "static name tables retained {} bytes",
        tables.high_water()
    );
    eprintln!(
        "10,000 statics: {} table bytes, {:?}",
        tables.high_water(),
        start.elapsed()
    );
}

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
    let mut unit = Unit::new(&context, &sema, Target::LinuxGnu, &arena, &tables).unwrap();
    // Pre-existing symbols must still be skipped by the suffix counter.
    for name in [".str", ".str.1", ".str.3"] {
        _ = unit.module.declare_global(
            name,
            GlobalDecl {
                linkage:      Linkage::Internal,
                size:         0,
                align:        Align::BYTE,
                constant:     true,
                thread_local: false,
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
        // destination copies; omitted ranges and a masked bit-field byte.
        assert_volatile_accesses(&text, "store.", 3);
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
        assert_volatile_accesses(&text, "store.", 6);
        assert_volatile_accesses(&text, "copy ", 1);
        // Only omitted padding gets range fills; shared bit-field bytes
        // use masked volatile accesses on both objects.
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
        assert_volatile_accesses(&text, "fill ", 2);
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

#[test]
fn aggregate_reinitialization_preserves_explicit_bit_fields() {
    check(
        "struct S { unsigned x:3, y:5; }; int main(void) { int n = 0; again:; struct S a = {.x = \
         n ? a.x + 1 : 1}; if (!n++) goto again; return a.x != 2 || a.y != 0; }",
    );
}

#[test]
fn m2_many_functions_and_declarations_have_linear_correspondence_work() {
    use std::fmt::Write as _;
    let mut source = String::new();
    for index in 0..10_000 {
        writeln!(
            source,
            "int f{index}(int n, int a[n]) {{ int x=1,y=2,z=3,u=4,v=5,w=6; return n+x+y+z+u+v+w; \
             }}"
        )
        .unwrap();
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
    let mut unit = Unit::new(&context, &sema, Target::LinuxGnu, &arena, &tables).unwrap();
    let mut errors = ArenaVec::new_in(&arena);
    unit.prepare_extensions(&mut errors);
    let functions = unit.declare_items(&syntax, &mut errors);
    assert!(errors.is_empty());
    assert_eq!(functions.len(), 10_000);
    let mut scratch = Bump::new();
    for &(record, function) in &functions {
        scratch.reset();
        FunctionLowerer::lower(&mut unit, record, function.unwrap(), &scratch).unwrap();
        assert!(
            unit.parameter_alias_visits <= 20_000,
            "parameter work grows with unrelated bindings: {} visits",
            unit.parameter_alias_visits
        );
    }
    assert_eq!(unit.module.functions().count(), 10_000);
    assert_eq!(unit.parameter_alias_visits, 20_000);
    assert_eq!(sema.correspondence.declarations.len(), 70_000);
    assert!(sema.correspondence.work <= sema.bindings.len());
    eprintln!(
        "M2 work: functions=10000 parameters=20000 declarations={} bindings={} construction={} \
         aliases={} TU used={} committed={}",
        sema.correspondence.declarations.len(),
        sema.bindings.len(),
        sema.correspondence.work,
        unit.parameter_alias_visits,
        tu.used(),
        tu.committed()
    );
}
