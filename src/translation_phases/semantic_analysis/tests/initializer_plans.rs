//! M8: the current-object checker's retained placement plans. Each plan is
//! rendered one operation per line: offsets and sizes in bytes, `*` on a
//! value a range repeats, `[bit offset:width]` on a bit-field store.

use std::fmt::Write;

use super::*;
use crate::translation_phases::{
    ErrorSeverity,
    GetSeverity,
    semantic_analysis::{
        InitOperation,
        InitializerPlan,
    },
};

/// Fails on an error-severity diagnostic; warnings such as
/// `-Winitializer-overrides` are expected.
fn assert_accepted(context: &mut Context<'_>, source: &str) {
    let errors = context.take_pending_errors();
    assert!(
        !errors.iter().any(|e| e.severity() == ErrorSeverity::Error),
        "{source}: {errors:?}"
    );
}

/// The plan of the initializer of the object named `name`.
fn plan_of(source: &str, name: &str) -> String {
    let mut rendered = None;
    with_source(source, |context, s| {
        assert_accepted(context, source);
        let initializer = s
            .correspondence
            .declarations
            .iter()
            .rev()
            .find(|fact| {
                fact.binding
                    .is_some_and(|b| context.string_cache.at(s.bindings[b].name.name) == name)
                    && fact.initializer.is_some()
            })
            .and_then(|fact| fact.initializer)
            .unwrap_or_else(|| panic!("{name} has an initializer"));
        let plan = s
            .initializers
            .plan(initializer)
            .unwrap_or_else(|| panic!("{name} has a plan"));
        rendered = Some(render(s, plan));
    });
    rendered.unwrap()
}

fn render(s: &SemanticTranslationUnit<'_>, plan: &InitializerPlan<'_>) -> String {
    let size = |ty| s.types.layout(ty).map_or(0, |layout| layout.size);
    let mut out = String::new();
    for operation in plan.operations {
        match *operation {
            | InitOperation::Zero {
                destination, end, ..
            } => writeln!(
                out,
                "zero {}+{} end {end}",
                destination.offset,
                size(destination.ty)
            ),
            | InitOperation::Select {
                destination,
                member,
                scope,
                ..
            } => writeln!(
                out,
                "select {} member {member} scope {scope}",
                destination.offset
            ),
            | InitOperation::Store {
                value,
                destination,
                bit_field,
                ..
            } => writeln!(
                out,
                "store {} v{}{}{}",
                destination.offset,
                value.0,
                if plan.values[value.0 as usize].repeated {
                    "*"
                } else {
                    ""
                },
                bit_field.map_or(String::new(), |bits| format!(
                    " [{}:{}]",
                    bits.bit_offset, bits.width
                ))
            ),
            | InitOperation::Copy {
                value,
                destination,
                enclosing_unions,
            } => writeln!(
                out,
                "copy {} v{} unions {}",
                destination.offset,
                value.0,
                enclosing_unions.len()
            ),
            | InitOperation::String { destination, .. } => writeln!(
                out,
                "string {}+{}",
                destination.offset,
                size(destination.ty)
            ),
            | InitOperation::Repeat {
                count,
                stride,
                length,
            } => writeln!(out, "repeat {count}x{stride} len {length}"),
            | InitOperation::EmptyScalar { destination, .. } =>
                writeln!(out, "empty {}", destination.offset),
            | InitOperation::Unmodeled { destination, .. } =>
                writeln!(out, "unmodeled {}", destination.offset),
        }
        .unwrap();
    }
    out
}

#[test]
fn ranges_are_one_repeat_at_their_first_element() {
    assert_eq!(
        plan_of(
            "struct P { int x, y; } a[4] = { [1 ... 2] = { 5 }, [3].y = 7 };",
            "a"
        ),
        "zero 0+32 end 5\nrepeat 2x8 len 2\nzero 8+8 end 4\nstore 8 v0*\nstore 28 v1\n"
    );
    // Nested ranges repeat inside each other; one evaluation serves all six.
    assert_eq!(
        plan_of("int a[2][3] = { [0 ... 1] = { [0 ... 2] = 7 } };", "a"),
        "zero 0+24 end 5\nrepeat 2x12 len 3\nzero 0+12 end 5\nrepeat 3x4 len 1\nstore 0 v0*\n"
    );
    // A one-element range repeats nothing; brace elision continues after the
    // range's last element.
    assert_eq!(
        plan_of(
            "struct P { int x, y; } b[3] = { [2 ... 2] = 1, [0 ... 1] = 2, 3 };",
            "b"
        ),
        "zero 0+24 end 6\nrepeat 1x8 len 1\nstore 16 v0\nrepeat 2x8 len 1\nstore 0 v1*\nstore 12 \
         v2\n"
    );
}

#[test]
fn union_members_and_replacements_are_explicit() {
    assert_eq!(
        plan_of(
            "union U { int i; char c[8]; } u = { .i = 1, .c = \"ab\" };",
            "u"
        ),
        "zero 0+8 end 5\nselect 0 member 0 scope 0\nstore 0 v0\nselect 0 member 1 scope 0\nstring \
         0+8\n"
    );
    // A designator through an anonymous union selects its member there; a
    // copy keeps the selection of the union around it.
    assert_eq!(
        plan_of(
            "struct S { int a; } g; union W { struct S s; int i; };
             void f(void) { union W w = { .s = g }; (void)w; }",
            "w"
        ),
        "zero 0+4 end 3\nselect 0 member 0 scope 0\ncopy 0 v0 unions 1\n"
    );
    assert_eq!(
        plan_of(
            "struct T { int k; union { short h; int w; }; } t = { .w = 3 };",
            "t"
        ),
        "zero 0+8 end 3\nselect 4 member 1 scope 0\nstore 4 v0\n"
    );
}

#[test]
fn bit_fields_elision_and_braced_scalars_place_stores() {
    assert_eq!(
        plan_of(
            "struct B { unsigned a : 3, b : 5; int c; } b = { 1, 2, 3 };",
            "b"
        ),
        "zero 0+8 end 4\nstore 0 v0 [0:3]\nstore 0 v1 [3:5]\nstore 4 v2\n"
    );
    assert_eq!(
        plan_of("int m[2][3] = { 1, 2, 3, 4 };", "m"),
        "zero 0+24 end 5\nstore 0 v0\nstore 4 v1\nstore 8 v2\nstore 12 v3\n"
    );
    assert_eq!(plan_of("int x = { { 5 } };", "x"), "store 0 v0\n");
    assert_eq!(plan_of("int y = {};", "y"), "empty 0\n");
    assert_eq!(
        plan_of("struct C { int a : 4; } c = { { 6 } };", "c"),
        "zero 0+4 end 2\nstore 0 v0 [0:4]\n"
    );
}

#[test]
fn arrays_of_unknown_size_and_flexible_members_are_completed() {
    assert_eq!(plan_of("char s[] = \"hello\";", "s"), "string 0+6\n");
    assert_eq!(plan_of("char t[] = { \"hi\" };", "t"), "string 0+3\n");
    assert_eq!(
        plan_of("int a[] = { [9] = 1 };", "a"),
        "zero 0+40 end 2\nstore 36 v0\n"
    );
    assert_eq!(
        plan_of(
            "struct F { int n; int v[]; }; static struct F f = { 3, { 1, 2, 3 } };",
            "f"
        ),
        "zero 0+4 end 6\nstore 0 v0\nzero 4+12 end 6\nstore 4 v1\nstore 8 v2\nstore 12 v3\n"
    );
    // A later list replaces the member; its zero scope keeps room for every
    // element written.
    assert_eq!(
        plan_of(
            "struct F { int n; int v[]; }; static struct F f = { .v = { 1, 2, 3 }, .v = { 4 } };",
            "f"
        ),
        "zero 0+4 end 7\nzero 4+12 end 5\nstore 4 v0\nstore 8 v1\nstore 12 v2\nzero 4+12 end \
         7\nstore 4 v3\n"
    );
    // Another structure's flexible member takes only an empty list, which
    // stores nothing.
    assert_eq!(
        plan_of(
            "struct F { int n; int v[]; }; struct G { int k; struct F f; } g = { 1, { 2, {} } };",
            "g"
        ),
        "zero 0+8 end 4\nstore 0 v0\nzero 4+4 end 4\nstore 4 v1\n"
    );
}

#[test]
fn subobjects_keep_their_containers_qualifiers() {
    with_source(
        "const struct { int a; volatile int b[2]; } c = { 1, { 2, 3 } };",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let (_, plan) = s.initializers.plans.iter().next().unwrap();
            let stores = plan
                .operations
                .iter()
                .filter_map(|operation| match operation {
                    | InitOperation::Store { destination, .. } => Some(destination.ty.qualifiers),
                    | _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(stores.len(), 3);
            assert!(stores.iter().all(|q| q.contains(TypeQualifiers::CONST)));
            assert!(!stores[0].contains(TypeQualifiers::VOLATILE));
            assert!(
                stores[1..]
                    .iter()
                    .all(|q| q.contains(TypeQualifiers::VOLATILE))
            );
        },
    );
}

#[test]
fn compound_literals_and_rejected_initializers() {
    with_source(
        "int *p = (int[]){ 1, 2, 3 }; int bad[1] = { 1, 2 };",
        |context, s| {
            assert!(context.pending_error_count() > 0);
            // The compound literal's plan; the rejected array has none.
            assert_eq!(s.initializers.plans.len(), 2);
            let mut rendered = s
                .initializers
                .plans
                .values()
                .map(|plan| render(s, plan))
                .collect::<Vec<_>>();
            rendered.sort();
            assert_eq!(
                rendered,
                [
                    "store 0 v0\n",
                    "zero 0+12 end 4\nstore 0 v0\nstore 4 v1\nstore 8 v2\n"
                ]
            );
        },
    );
}

/// The plan work (`review_steps[3]`) and operations of `source`.
fn plan_work(source: &str) -> (usize, usize) {
    let tu = Bump::new();
    let scratch = Bump::new();
    let source = tu.alloc_str(&format!("{source}\n"));
    let mut context = Context::new(&tu);
    let unit = crate::pipeline::parse_translation_unit(
        &mut context,
        Path::new("<input>"),
        source,
        crate::headers::HeaderSearch::default(),
    );
    let mut analyzer = Analyzer::new(&mut context, &scratch);
    for &root in unit.external_declarations().iter().rev() {
        analyzer.work.push(Work::Root(root));
    }
    while let Some(work) = analyzer.work.pop() {
        analyzer.step(work);
    }
    assert_accepted(analyzer.context, source);
    (
        analyzer.review_steps.get()[3],
        analyzer.initializers.totals().1,
    )
}

#[test]
fn deep_initializers_scale_with_their_nesting() {
    for n in [128, 512] {
        let braced = format!(
            "int a{}={}1{};",
            "[1]".repeat(n),
            "{".repeat(n),
            "}".repeat(n)
        );
        let (work, operations) = plan_work(&braced);
        assert_eq!(
            operations,
            n + 1,
            "one zero scope per brace pair and the store"
        );
        assert!(work <= 4 * n, "{work}");
        // Brace elision descends the same depth from one element.
        let elided = format!("int a{}=1;", "[1]".repeat(n)).replace("=1", "={1}");
        let (work, operations) = plan_work(&elided);
        assert_eq!(operations, 2);
        assert!(work <= 4 * n, "{work}");
    }
}

#[test]
fn sparse_and_overlapping_range_initializers_do_not_expand() {
    // Megabyte extents: work and operations follow the written elements.
    for n in [16, 64] {
        let mut sparse = String::from("static int s[1 << 20] = {");
        for i in 0..n {
            write!(sparse, "[{}] = {i},", i * 16_000).unwrap();
        }
        sparse.push_str("};");
        let (work, operations) = plan_work(&sparse);
        assert_eq!(operations, n + 1);
        assert!(work <= 4 * n + 4, "{work}");
        let mut ranges = String::from("static int r[1 << 20] = {");
        for i in 0..n {
            write!(
                ranges,
                "[0 ... (1 << 20) - 1] = {i}, [{i} ... {}] = {i},",
                i + 1000
            )
            .unwrap();
        }
        ranges.push_str("};");
        let (work, operations) = plan_work(&ranges);
        assert_eq!(operations, 4 * n + 1);
        assert!(work <= 8 * n + 4, "{work}");
    }
}

#[test]
fn plan_records_stay_small() {
    use crate::translation_phases::semantic_analysis::{
        InitValue,
        Subobject,
    };
    // Recorded in docs/research/semantic-facts-m8.md.
    eprintln!(
        "Subobject {} InitValue {} InitOperation {} InitializerPlan {}",
        size_of::<Subobject>(),
        size_of::<InitValue<'_>>(),
        size_of::<InitOperation<'_>>(),
        size_of::<InitializerPlan<'_>>()
    );
    assert!(size_of::<Subobject>() <= 24);
    assert!(size_of::<InitValue<'_>>() <= 16);
    assert!(size_of::<InitOperation<'_>>() <= 56);
    assert!(size_of::<InitializerPlan<'_>>() <= 56);
}
