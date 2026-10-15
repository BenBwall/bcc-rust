//! GNU labels as values: `&&label` typing, computed-goto conversions and
//! control facts, label differences in static initializers, and the jump
//! constraints Clang applies to every possible target.
//! GNU extension: GCC "Labels as Values"; C99 §6.8.6.1p1, p. 137; PDF p. 149.

use std::fmt::Write as _;

use super::*;

fn kinds(source: &str) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_source(source, |context, _| {
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

fn clean(source: &str) {
    assert_eq!(kinds(source), [], "{source}");
}

/// The expression of the `index`th retained record whose syntax is `&&`.
fn label_addresses<'tu>(s: &SemanticTranslationUnit<'tu>) -> Vec<&'tu Expression<'tu>> {
    s.expressions
        .iter()
        .map(|info| info.expression)
        .filter(|e| matches!(e.kind, ExpressionType::LabelAddress(_)))
        .collect()
}

#[test]
fn label_addresses_are_void_pointer_address_constants() {
    with_source(
        "int f(void) { void *p = &&a; static void *t[] = { &&b, &&a }; goto *p; a: b: return 0; }",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let addresses = label_addresses(s);
            assert_eq!(addresses.len(), 3);
            for &e in &addresses {
                let info = s.expression_info(e).unwrap();
                assert_eq!(info.constant, ConstantClass::Address);
                assert_eq!(info.category, ValueCategory::Rvalue);
                let TypeKind::Pointer(target) = s.types.kind(info.ty) else {
                    panic!("&&label is a pointer");
                };
                assert!(matches!(
                    s.types.kind(target),
                    TypeKind::Scalar(Scalar::Void)
                ));
            }
            // Address order follows the first `&&` of each label.
            let function = s.functions[0].control.id;
            let taken = s.control.address_taken(function);
            let names: Vec<_> = taken
                .iter()
                .map(|id| {
                    context
                        .string_cache
                        .at(s.control.labels[id.0].name.unwrap().name)
                })
                .collect();
            assert_eq!(names, ["a", "b"]);
            for (index, id) in taken.iter().enumerate() {
                assert_eq!(
                    s.control.labels[id.0].address,
                    Some(u32::try_from(index).unwrap())
                );
            }
            // Both `&&a` resolve to one label identity.
            assert_eq!(
                s.control.label_address(addresses[0]),
                s.control.label_address(addresses[2])
            );
            assert_ne!(
                s.control.label_address(addresses[0]),
                s.control.label_address(addresses[1])
            );
        },
    );
}

#[test]
fn computed_gotos_convert_to_const_void_pointers_and_jump_indirectly() {
    with_source(
        "int f(char *q) { void *t[1] = { &&a }; goto *q; goto *t[0]; a: return 0; }",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let function = s.functions[0].control.id;
            let jumps: Vec<_> = s
                .control
                .jumps
                .iter()
                .filter(|j| j.target == ControlTarget::IndirectGoto(function))
                .collect();
            assert_eq!(jumps.len(), 2);
            for jump in jumps {
                // No transition: like Clang, a computed goto releases nothing.
                assert_eq!(jump.transition.left.first, jump.transition.left.stop_before);
                assert_eq!(
                    jump.transition.entered.first,
                    jump.transition.entered.stop_before
                );
            }
            let gotos: Vec<_> = s
                .expressions
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    s.conversions_of(*index).iter().any(|conversion| {
                        conversion.kind == ConversionKind::Assignment
                            && matches!(s.types.kind(conversion.ty), TypeKind::Pointer(target)
                            if target.qualifiers.contains(
                                TypeQualifiers::CONST
                            ))
                    })
                })
                .collect();
            assert_eq!(gotos.len(), 2, "both operands convert to const void *");
        },
    );
}

#[test]
fn label_differences_are_link_time_constants_of_32_to_64_bits() {
    with_source(
        "int f(void) { static int d[] = { &&a - &&b, (int)(&&b - &&a), (long)((&&a - &&b)) }; \
         static unsigned long long e = &&b - &&b; a: b: return d[0] + (int)e; }",
        |context, s| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let differences: Vec<_> = s
                .expressions
                .iter()
                .filter(|info| info.constant == ConstantClass::LabelDifference)
                .collect();
            // Two subtractions, a cast, a cast of a parenthesized
            // subtraction, its parentheses and the last subtraction.
            assert!(differences.len() >= 6, "{}", differences.len());
            for info in differences {
                assert_eq!(info.integer, None, "no folded value");
                let (label, base) = s.control.label_difference(info.expression).unwrap();
                assert_eq!(
                    s.control.labels[label.0].function,
                    s.control.labels[base.0].function
                );
            }
        },
    );
    // Clang refuses differences narrower than 32 or wider than 64 bits,
    // `_Bool`, a narrowed difference widened again, bit-field destinations
    // and arithmetic on a difference.
    for source in [
        "int f(void) { static short s = &&a - &&b; a: b: return 0; }",
        "int f(void) { static int s = (short)(&&a - &&b); a: b: return 0; }",
        "int f(void) { static int s = &&a - &&b + 1; a: b: return 0; }",
        "int f(void) { static char s = &&a - &&a; a: return 0; }",
        "int f(void) { static __int128 s = &&b - &&a; a: b: return 0; }",
        "int f(void) { static unsigned __int128 s = &&b - &&a; a: b: return 0; }",
        "int f(void) { static long s = (long)(__int128)(&&b - &&a); a: b: return 0; }",
        "int f(void) { static long long s = (long long)(int)(&&b - &&a); a: b: return 0; }",
        "int f(void) { static _Bool s = &&b - &&a; a: b: return 0; }",
        "int f(void) { static struct { int x : 32; } s = { &&b - &&a }; a: b: return 0; }",
        "int f(void) { static struct { int y; long long x : 40; } s = { .x = &&b - &&a }; a: b: \
         return 0; }",
    ] {
        assert_eq!(
            kinds(source),
            [SemanticErrorKind::NonConstantInitializer],
            "{source}"
        );
    }
    // A label address plus a constant and a cast stay address constants.
    clean("int f(void) { static void *r = &&a + 1; static char *t = (char *)&&a; a: return 0; }");
    // Narrowing a 64-bit difference stays constant, as in Clang.
    clean(
        "int f(void) { static int n = (int)(long long)(&&b - &&a); static unsigned long long u = \
         &&b - &&a; static struct { int x; } s = { &&b - &&a }; a: b: return n + (int)u + s.x; }",
    );
}

#[test]
fn computed_gotos_cannot_enter_protected_scopes_at_any_target() {
    // Clang: "cannot jump from this indirect goto statement to one of its
    // possible targets".
    for source in [
        "void f(int n) { void *p = &&in; goto *p; { int v[n]; in: v[0] = 1; } }",
        "void f(int n) { void *p = &&in; goto *p; { typedef int A[n]; in:; } }",
        "void f(int n) { void *p = &&out; { int v[n]; out:; } goto *p; }",
    ] {
        assert_eq!(
            kinds(source),
            [SemanticErrorKind::IndirectJumpIntoVariableScope],
            "{source}"
        );
    }
    assert_eq!(
        kinds("void f(void) { void *p = &&in; goto *p; ({ in: 1; }); }"),
        [SemanticErrorKind::IndirectJumpIntoStatementExpression]
    );
    // One error per goto, for its first protected target.
    assert_eq!(
        kinds(
            "void f(int n) { void *p = &&a; void *q = &&b; goto *p; goto *q; { int v[n]; a:; b:; \
             } }"
        ),
        [
            SemanticErrorKind::IndirectJumpIntoVariableScope,
            SemanticErrorKind::IndirectJumpIntoVariableScope
        ]
    );
    // Leaving a VLA scope, a goto inside the scope, and jumps out of
    // statement expressions are allowed.
    clean("void f(int n) { void *p = &&out; { int v[n]; v[0] = 0; goto *p; } out:; }");
    clean("void f(int n) { { int v[n]; void *p = &&in; v[0] = 0; goto *p; in:; } }");
    clean(
        "int f(int x) { void *p = &&out; int r = ({ if (x) goto *p; 1; }); return r; out: return \
         0; }",
    );
}

#[test]
fn label_address_and_computed_goto_constraints() {
    assert_eq!(
        kinds("void *p = &&x;"),
        [SemanticErrorKind::LabelAddressOutsideFunction]
    );
    assert_eq!(
        kinds("int f(long k) { void *p = &&a; goto *k; a: return 0; }"),
        [SemanticErrorKind::InvalidComputedGoto]
    );
    assert_eq!(
        kinds("int f(double d) { void *p = &&a; goto *d; a: return 0; }"),
        [SemanticErrorKind::InvalidComputedGoto]
    );
    assert_eq!(
        kinds("int f(void *p) { if (p) goto *p; return 0; }"),
        [SemanticErrorKind::IndirectGotoWithoutLabels]
    );
    assert_eq!(
        kinds("int f(void) { return &&missing != 0; }"),
        [SemanticErrorKind::UndefinedLabel]
    );
    // GNU local labels resolve by scope; their addresses are ordinary.
    clean("int f(int x) { __label__ a; void *p = &&a; goto *p; a: return x; }");
}

#[test]
fn void_pointer_arithmetic_is_a_gnu_extension_of_element_size_one() {
    with_source(
        "int f(void *p, void *q) { void *r = p + 3; r -= 1; r++; return (int)(r - p) + (int)(q - \
         p); }",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert!(s.expressions.iter().any(|info| matches!(
                info.expression.kind,
                ExpressionType::Binary {
                    operator: BinaryOperator::Subtraction,
                    ..
                }
            ) && matches!(
                s.types.kind(info.ty),
                TypeKind::Scalar(_)
            )));
        },
    );
    // Subscripts and constant differences use element size 1 too, as in
    // Clang: `&p[i]` is an address, `p[i]` a `void` rvalue that is never
    // read, and differences of `void *` address constants fold.
    clean("int f(void *p) { void *q = &p[2]; (void)p[1]; return (int)((char *)q - (char *)p); }");
    with_source(
        "int arr[8]; static long d = (void *)&arr[3] - (void *)&arr[0]; static void *g = &((void \
         *)arr)[2];",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert!(
                s.expressions.iter().any(|info| info.folded_address()
                    && info.integer.is_some_and(|value| value.value == 12)),
                "the difference folds to 12 bytes"
            );
        },
    );
}

#[test]
fn computed_goto_checks_visit_only_protected_labels() {
    // Many gotos and many plain labels: no pairwise check at all; one
    // protected label: one check per goto.
    for (protected, expected) in [(false, 0), (true, 200)] {
        let mut source = String::from("void f(int n) { static void *t[] = {");
        for label in 0..200 {
            write!(source, "&&l{label},").unwrap();
        }
        source.push_str(if protected { "&&in}; " } else { "}; " });
        for label in 0..200 {
            write!(source, "l{label}: goto *t[n];").unwrap();
        }
        if protected {
            source.push_str("{ int v[n]; in: v[0] = 0; }");
        }
        source.push_str("}\n");
        let tu = Bump::new();
        let scratch = Bump::new();
        let mut context = Context::new(&tu);
        let unit = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<input>"),
            tu.alloc_str(&source),
            crate::headers::HeaderSearch::default(),
        );
        let mut analyzer = Analyzer::new(&mut context, &scratch);
        for &root in unit.external_declarations().iter().rev() {
            analyzer.work.push(Work::Root(root));
        }
        while let Some(work) = analyzer.work.pop() {
            analyzer.step(work);
        }
        analyzer.finish_translation_unit();
        assert_eq!(analyzer.statements.indirect_checks.get(), expected);
        assert_eq!(
            analyzer.context.error_count(),
            usize::from(protected) * 200,
            "each goto may enter the VLA scope"
        );
    }
}
