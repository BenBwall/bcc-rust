//! M3 retained-interface coverage; no consumer syntax reconstruction.
use std::fmt::Write as _;

use super::*;
use crate::translation_phases::parsing::syntax::{
    StatementType,
    UnaryOperator,
};

#[test]
fn m3_calls_and_builtins_preserve_every_selection_wrapper() {
    for leaf in ["f", "__builtin_popcount", "__builtin_va_end"] {
        for callee in [
            String::from(leaf),
            format!("({leaf})"),
            format!("__extension__ ({leaf})"),
            format!("_Generic(0,int:{leaf},default:0)"),
            format!("__builtin_choose_expr(1,{leaf},0)"),
            format!("(__extension__ (_Generic(0,int:__builtin_choose_expr(1,({leaf}),0))))"),
        ] {
            let argument = if leaf == "__builtin_va_end" {
                "ap"
            } else {
                "3"
            };
            with_source(
                &format!("int f(int);void g(void){{__builtin_va_list ap;({callee})({argument});}}"),
                |context, sema| {
                    assert_eq!(
                        context.error_count(),
                        0,
                        "{callee}: {:?}",
                        context.take_pending_errors()
                    );
                    let call = sema
                        .expressions
                        .iter()
                        .find(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                        .unwrap();
                    let facts = sema.call_facts(call.expression).unwrap();
                    let binding = match facts.callee {
                        | Callee::Direct { binding, entity } if leaf == "f" => {
                            assert_eq!(
                                sema.bindings[entity.0].name.name,
                                sema.bindings[binding].name.name
                            );
                            binding
                        },
                        | Callee::Builtin { binding, .. } if leaf != "f" => binding,
                        | other => panic!("{callee}: {other:?}"),
                    };
                    assert_eq!(
                        context.string_cache.at(sema.bindings[binding].name.name),
                        leaf
                    );
                    let ExpressionType::Call {
                        function_expression,
                        ..
                    } = call.expression.kind
                    else {
                        unreachable!()
                    };
                    let designation = sema.designation(function_expression).unwrap();
                    assert!(matches!(
                        designation.leaf.kind,
                        ExpressionType::Identifier(_)
                    ));
                    assert_eq!(designation.binding, Some(binding));
                    assert!(!designation.converted_child);
                    assert_eq!(facts.arguments.len(), 1);
                    let arg = facts.arguments[0];
                    assert_eq!(
                        sema.conversions_of(arg.expression.0)
                            .last()
                            .map_or(sema.expressions[arg.expression.0].ty, |conversion| {
                                conversion.ty
                            }),
                        arg.ty
                    );
                },
            );
        }
    }
}

#[test]
fn m3_pointer_and_conversion_changing_callees_remain_indirect() {
    for callee in [
        "p",
        "(p)",
        "__extension__ p",
        "_Generic(0,int:p)",
        "__builtin_choose_expr(1,p,f)",
        "((int (*)(int))f)",
        "(*f)",
        "(&f)",
        "(0,f)",
        "(1?f:f)",
        "({f;})",
        "_Generic(0,int:({f;}))",
        "__builtin_choose_expr(1,((int (*)(int))f),f)",
    ] {
        with_source(
            &format!("int f(int);int g(int (*p)(int)){{return ({callee})(3);}}"),
            |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{callee}: {:?}",
                    context.take_pending_errors()
                );
                let call = sema
                    .expressions
                    .iter()
                    .find(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                    .unwrap();
                let facts = sema.call_facts(call.expression).unwrap();
                let Callee::Indirect(index) = facts.callee else {
                    panic!("{callee}: {facts:?}")
                };
                let ExpressionType::Call {
                    function_expression,
                    ..
                } = call.expression.kind
                else {
                    unreachable!()
                };
                assert!(std::ptr::eq(
                    sema.expressions[index.0].expression,
                    function_expression
                ));
                assert!(facts.prototype);
                assert_eq!(facts.fixed, 1);
            },
        );
    }
}

#[test]
fn m3_call_shape_keeps_point_of_use_and_argument_conversion_runs() {
    with_source(
        "int f();int a(void){return f((char)2,3.0f);}int f(int x,double y){return x+(int)y;}int \
         v(int,...);int b(void){return v((char)2,3.0f);}",
        |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let calls: Vec<_> = sema
                .expressions
                .iter()
                .filter_map(|info| sema.call_facts(info.expression))
                .collect();
            assert_eq!(calls.len(), 2);
            assert!(!calls[0].prototype);
            assert_eq!(calls[0].fixed, 2);
            assert!(matches!(
                sema.types.kind(calls[0].function_type.unwrap()),
                TypeKind::Function {
                    prototype: false,
                    ..
                }
            ));
            assert!(calls[1].prototype && calls[1].variadic);
            assert_eq!(calls[1].fixed, 1);
            for call in calls {
                assert_eq!(
                    sema.types.kind(call.arguments[0].ty),
                    TypeKind::Scalar(Scalar::Int)
                );
                assert_eq!(
                    sema.types.kind(call.arguments[1].ty),
                    TypeKind::Scalar(Scalar::Double)
                );
                assert_eq!(
                    sema.conversions_of(call.arguments[1].expression.0)
                        .last()
                        .unwrap()
                        .kind,
                    ConversionKind::DefaultArgument
                );
            }
        },
    );
}

#[test]
fn m3_implicit_declarations_are_discovered_through_transparent_wrappers() {
    use crate::configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
        LanguageMode,
    };
    for callee in ["missing", "(missing)", "__extension__ (missing)"] {
        let source = format!("int f(void){{return ({callee})(3);}}");
        for spelling in ["c89", "gnu99"] {
            let mode = LanguageMode::parse(spelling).unwrap();
            let configuration = CompilerConfiguration::new(mode.standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(mode.gnu);
            with_configuration(&source, configuration, |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{spelling}, {callee}: {:?}",
                    context.take_pending_errors()
                );
                let call = sema
                    .expressions
                    .iter()
                    .find_map(|info| sema.call_facts(info.expression))
                    .unwrap();
                assert!(matches!(call.callee, Callee::Direct { .. }));
                assert!(!call.prototype);
            });
        }
        with_source(&source, |context, sema| {
            assert!(!sema.lowerable(context));
            assert!(context.take_pending_errors().iter().any(|error| matches!(
                error,
                TranslationError::Semantic(error)
                    if error.kind == SemanticErrorKind::UndeclaredIdentifier
            )));
        });
    }
}

#[test]
fn m3_existing_child_or_extra_outer_conversions_force_indirect_calls() {
    for child_conversion in [false, true] {
        let tu = Bump::new();
        let scratch = Bump::new();
        let mut context = Context::new(&tu);
        let syntax = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<m3>"),
            tu.alloc_str("int f(int);int g(void){return ((f))(3);}"),
            crate::headers::HeaderSearch::default(),
        );
        let mut analyzer = Analyzer::new(&mut context, &scratch);
        for root in syntax.external_declarations() {
            analyzer.work.push(Work::Root(*root));
            while let Some(work) = analyzer.work.pop() {
                if let Work::ExpressionDone(expression) = work {
                    let operand = match expression.kind {
                        | ExpressionType::Parenthesized { expression } if child_conversion =>
                            Some(expression),
                        | ExpressionType::Call {
                            function_expression,
                            ..
                        } if !child_conversion => Some(function_expression),
                        | _ => None,
                    };
                    if let Some(operand) = operand {
                        let ty = analyzer.expression_info(operand).ty;
                        let pointer = analyzer.types.intern(TypeKind::Pointer(ty));
                        analyzer.convert(operand, pointer, ConversionKind::FunctionDecay);
                    }
                }
                analyzer.step(work);
            }
        }
        assert_eq!(analyzer.context.error_count(), 0);
        let call = *analyzer.resolved.retain().calls.values().next().unwrap();
        assert!(matches!(call.callee, Callee::Indirect(_)), "{call:?}");
    }
}

#[test]
fn m3_tails_keep_outer_labels_and_converted_results() {
    with_source(
        "void f(void){int a[2];({});({int x;});({{3;}});({(void)0;});({0;first:second:a;});}",
        |context, sema| {
            assert_eq!(context.error_count(), 0);
            let blocks: Vec<_> = sema
                .expressions
                .iter()
                .filter_map(|info| sema.statement_expression_facts(info.expression))
                .collect();
            assert_eq!(blocks.len(), 5);
            for (index, facts) in blocks[..3].iter().enumerate() {
                assert!(facts.outer_tail.is_none() && facts.value_tail.is_none());
                assert_eq!(facts.prefix, usize::from(index != 0));
                assert_eq!(
                    sema.types.kind(facts.result_type),
                    TypeKind::Scalar(Scalar::Void)
                );
            }
            assert!(blocks[3].value_tail.is_some());
            assert_eq!(blocks[3].prefix, 0);
            assert_eq!(
                sema.types.kind(blocks[3].result_type),
                TypeKind::Scalar(Scalar::Void)
            );
            let facts = blocks[4];
            assert!(matches!(
                facts.outer_tail.unwrap().kind,
                StatementType::Label(..)
            ));
            assert_eq!(facts.prefix, 1);
            assert!(matches!(
                sema.types.kind(facts.result_type),
                TypeKind::Pointer(_)
            ));
            assert_eq!(
                sema.expression_conversions(facts.value_tail.unwrap())[0].kind,
                ConversionKind::ArrayDecay
            );
        },
    );
}

#[test]
fn m3_member_facts_keep_owner_qualifiers_alignment_and_bit_fields() {
    with_source(
        "struct A{int same;unsigned bits:3;};struct B{char same;};void f(const volatile struct A \
         *a,struct B b){a->same;(__extension__ \
         (_Generic(0,int:__builtin_choose_expr(1,a->bits,0))));b.same;}",
        |context, sema| {
            assert_eq!(context.error_count(), 0);
            let members: Vec<_> = sema
                .expressions
                .iter()
                .filter(|info| {
                    matches!(
                        info.expression.kind,
                        ExpressionType::DirectMember { .. } | ExpressionType::IndirectMember { .. }
                    )
                })
                .collect();
            assert_eq!(members.len(), 3);
            let facts: Vec<_> = members
                .iter()
                .map(|info| sema.member_facts(info.expression).unwrap())
                .collect();
            assert_eq!(facts[0].tag, facts[1].tag);
            assert_ne!(facts[0].tag, facts[2].tag);
            assert_eq!(facts[1].bit_field, Some(3));
            for (info, fact) in members.iter().zip(&facts) {
                assert_eq!(fact.access_type, info.ty);
                assert_eq!(Some(fact.alignment), info.place_alignment());
                assert!(std::ptr::eq(
                    sema.selected_field(info.expression).unwrap(),
                    std::ptr::from_ref(&sema.types.tags[fact.tag].fields.get()[fact.field])
                ));
            }
            assert!(
                facts[0]
                    .access_type
                    .qualifiers
                    .contains(TypeQualifiers::CONST)
            );
            assert!(
                facts[0]
                    .access_type
                    .qualifiers
                    .contains(TypeQualifiers::VOLATILE)
            );
            let mut wrappers = 0;
            for info in sema.expressions {
                if !std::ptr::eq(info.expression, members[1].expression)
                    && sema
                        .designation(info.expression)
                        .is_some_and(|designation| {
                            std::ptr::eq(designation.leaf, members[1].expression)
                        })
                {
                    wrappers += 1;
                    let selected = sema.member_facts(info.expression).unwrap();
                    assert_eq!(
                        (selected.tag, selected.field),
                        (facts[1].tag, facts[1].field)
                    );
                    assert_eq!(selected.bit_field, Some(3));
                    assert!(std::ptr::eq(
                        sema.selected_field(info.expression).unwrap(),
                        sema.selected_field(members[1].expression).unwrap()
                    ));
                }
            }
            assert!(wrappers >= 4);
        },
    );
}

#[test]
fn m3_deep_wrappers_and_labeled_tails_have_bounded_work() {
    let wrappers = [
        ("(", ")"),
        ("__extension__ (", ")"),
        ("_Generic(0,int:(", "),default:0)"),
        ("__builtin_choose_expr(1,(", "),0)"),
    ];
    for depth in [128usize, 2_048, 16_384] {
        let mut source = String::from("int f(int);int g(void){");
        for i in 0..depth {
            source.push_str(wrappers[i % wrappers.len()].0);
        }
        source.push('f');
        for i in (0..depth).rev() {
            source.push_str(wrappers[i % wrappers.len()].1);
        }
        source.push_str("(3);return ({");
        for i in 0..depth {
            write!(source, "L{i}:").unwrap();
        }
        source.push_str("4;});}");
        with_source(&source, |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let work = sema.resolved.work;
            assert!(work[0] <= 2 * sema.expressions.len());
            assert_eq!(work[1], sema.control.statements.len());
            for info in sema.expressions {
                if matches!(
                    info.expression.kind,
                    ExpressionType::Parenthesized { .. }
                        | ExpressionType::Generic(_)
                        | ExpressionType::Unary {
                            operator: UnaryOperator::Extension,
                            ..
                        }
                ) {
                    assert!(sema.designation(info.expression).is_some());
                }
            }
            let call = sema
                .expressions
                .iter()
                .find_map(|info| sema.call_facts(info.expression))
                .unwrap();
            assert!(matches!(call.callee, Callee::Direct { .. }));
            let tail = sema
                .expressions
                .iter()
                .find_map(|info| sema.statement_expression_facts(info.expression))
                .unwrap();
            assert!(tail.value_tail.is_some());
            println!(
                "M3 depth={depth} expressions={} statements={} designation_steps={} tail_steps={} \
                 tu_used={} tu_committed={}",
                sema.expressions.len(),
                sema.control.statements.len(),
                work[0],
                work[1],
                context.tu_arena().used(),
                context.tu_arena().committed()
            );
        });
    }
    assert!(!std::mem::needs_drop::<facts::resolved::Designation<'_>>());
    assert!(!std::mem::needs_drop::<CallFacts<'_>>());
    assert!(!std::mem::needs_drop::<facts::resolved::MemberFacts>());
    assert!(!std::mem::needs_drop::<
        facts::resolved::StatementExpressionFacts<'_>,
    >());
}
