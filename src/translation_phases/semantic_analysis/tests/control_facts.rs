//! M1 retained-interface and scaling acceptance, C99 §6.2.4p6 and §6.8.6.1p1.

use std::fmt::Write;

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn m1_h1_label_identity_and_cleanup_share_the_semantic_regions() {
    with_source(
        "int main(void){int n=4,a[n];a[0]=42;({goto L;L:;0;});return a[0]!=42;}",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let f = s.functions[0];
            assert_eq!(
                s.control.regions[f.control.root.0].lexical_scope,
                ScopeId(f.scope)
            );
            let jump = s
                .control
                .jumps
                .iter()
                .find(|j| matches!(j.target, ControlTarget::Label(_)))
                .unwrap();
            let ControlTarget::Label(id) = jump.target else {
                unreachable!()
            };
            let label = s.control.labels[id.0];
            assert_eq!(label.id, id);
            assert_eq!(context.string_cache.at(label.name.unwrap().name), "L");
            assert!(!label.local);
            assert_eq!(label.point, Some(jump.destination));
            assert_eq!(
                s.control
                    .statement(label.definition.unwrap())
                    .unwrap()
                    .label,
                Some(id)
            );
            assert_eq!(jump.transition.left.first, jump.transition.left.stop_before);
            let region = s.control.regions[jump.source.region.0];
            let vm = region.nearest_vm.unwrap();
            assert!(matches!(
                s.control.regions[vm.0].cleanup,
                CleanupKind::AutomaticVla(_)
            ));
            let expression = region.nearest_statement_expression.unwrap();
            let RegionKind::StatementExpression(id) = s.control.regions[expression.0].kind else {
                unreachable!()
            };
            assert!(matches!(
                s.expressions[id.0].expression.kind,
                ExpressionType::StatementExpression(_)
            ));
            assert!(
                s.control
                    .expression_point(s.expressions[id.0].expression)
                    .is_some()
            );
            assert!(s.control.statements.iter().any(|statement| {
                s.control.regions[statement.exit.left.first.0].nearest_cleanup == Some(vm)
                    && statement.exit.left.stop_before == f.control.root
            }));
        },
    );
}

#[test]
fn m1_vm_typedef_and_pointer_regions_protect_entry_without_cleanup() {
    for declaration in ["typedef int T[n];", "int (*p)[n];"] {
        let source = format!("void f(int n){{goto L;{declaration}{{int a[n];L:;}}}}");
        with_source(&source, |context, s| {
            let errors = context.take_pending_errors();
            let [TranslationError::Semantic(error)] = errors.as_slice() else {
                panic!("{errors:?}")
            };
            assert_eq!(error.kind, SemanticErrorKind::JumpIntoVariableScope);
            let jump = s.control.jumps[0];
            let mut region = jump.transition.entered.first;
            let mut outermost = None;
            while region != jump.transition.entered.stop_before {
                let fact = s.control.regions[region.0];
                if matches!(fact.kind, RegionKind::VmDeclaration(_)) {
                    outermost = Some(fact);
                }
                region = fact.parent.unwrap();
            }
            let protected = outermost.unwrap();
            assert_eq!(protected.cleanup, CleanupKind::None);
            assert_eq!(error.previous, Some(protected.source));
        });
    }
}

#[test]
fn m1_loop_header_jumps_resolve_to_the_outer_body_context() {
    with_source(
        "void f(int n){while(n){while(({if(n)break;0;})){continue;}}}",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert_eq!(s.control.loops.len(), 2);
            let outer = s.control.loops[0];
            let inner = s.control.loops[1];
            let break_jump = s
                .control
                .jumps
                .iter()
                .find(|j| matches!(j.target, ControlTarget::LoopBreak(_)))
                .unwrap();
            assert_eq!(break_jump.target, ControlTarget::LoopBreak(outer.statement));
            assert_eq!(break_jump.destination, outer.exit);
            let continue_jump = s
                .control
                .jumps
                .iter()
                .find(|j| matches!(j.target, ControlTarget::LoopContinue(_)))
                .unwrap();
            assert_eq!(
                continue_jump.target,
                ControlTarget::LoopContinue(inner.statement)
            );
            assert_eq!(continue_jump.destination, inner.step);
            assert_ne!(outer.body, outer.header);
            assert_ne!(inner.body, inner.header);
        },
    );
}

#[test]
fn m1_switch_cases_retain_converted_endpoints_order_and_owner() {
    with_configuration(
        "void f(unsigned n){switch(n){case -1:;case 2 ... 4:switch(n){case \
         3:;default:;}break;case 0:;default:;}}",
        CompilerConfiguration::default()
            .with_gnu_extensions(true)
            .with_extension_policy(ExtensionPolicy::Allow),
        |context, s| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            assert_eq!(s.control.switches.len(), 2);
            let outer = s
                .control
                .switches
                .iter()
                .find(|switch| switch.cases.len() == 3)
                .unwrap();
            assert_eq!(
                s.control.statements[outer.statement.0]
                    .switch
                    .map(|id| id.0),
                Some(
                    s.control
                        .switches
                        .iter()
                        .position(|switch| switch.statement == outer.statement)
                        .unwrap()
                )
            );
            assert!(s.control.expression_point(outer.control).is_some());
            assert_eq!(
                s.types.kind(outer.ty),
                TypeKind::Scalar(Scalar::UnsignedInt)
            );
            assert_eq!(outer.cases[0].lower.to_u64(), Some(u64::from(u32::MAX)));
            assert_eq!(outer.cases[1].lower.to_i128(), Some(2));
            assert_eq!(outer.cases[1].upper.to_i128(), Some(4));
            assert!(outer.cases[1].range);
            assert_eq!(outer.sorted, &[2, 1, 0]);
            for (ordinal, case) in outer.cases.iter().enumerate() {
                assert_eq!(case.ordinal, ordinal);
                assert_eq!(
                    s.control.statement(case.statement).unwrap().label,
                    Some(case.label)
                );
                assert_eq!(case.dispatch.unwrap().left.first, outer.dispatch.region);
                assert_eq!(case.dispatch.unwrap().entered.first, case.point.region);
            }
            assert!(outer.default_dispatch.is_some());
        },
    );
}

#[test]
fn round4_empty_case_ranges_retain_labels_without_dispatch_edges() {
    with_configuration(
        "void f(int n){switch(n){case 1:;case 9 ... 3:;case 2:;}}",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, s| {
            assert!(s.lowerable(context), "{:?}", context.take_pending_errors());
            let switch = s.control.switches[0];
            assert_eq!(switch.cases.len(), 3);
            assert_eq!(switch.sorted, &[0, 2]);
            let empty = switch.cases[1];
            assert!(empty.dispatch.is_none());
            assert_eq!(empty.lower.to_i128(), Some(9));
            assert_eq!(empty.upper.to_i128(), Some(3));
            assert_eq!(
                s.control.statement(empty.statement).unwrap().label,
                Some(empty.label)
            );
        },
    );
}

#[test]
fn round4_nested_header_jumps_release_inner_vlas_and_preserve_outer_continue() {
    for jump in ["continue", "break"] {
        let source = format!(
            "void f(int n){{for(int a[n];; ){{for(int b[n];({{int c[n];{jump};0;}}); ){{}}}}}}"
        );
        with_source(&source, |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let outer = s.control.loops[0];
            let fact = s.control.jumps[0];
            assert_eq!(
                fact.target,
                if jump == "continue" {
                    ControlTarget::LoopContinue(outer.statement)
                } else {
                    ControlTarget::LoopBreak(outer.statement)
                }
            );
            let mut region = fact.transition.left.first;
            let mut released = 0;
            while region != fact.transition.left.stop_before {
                let data = s.control.regions[region.0];
                released += usize::from(matches!(data.cleanup, CleanupKind::AutomaticVla(_)));
                region = data.parent.unwrap();
            }
            assert_eq!(released, if jump == "continue" { 2 } else { 3 });
        });
    }
}

#[test]
fn m1_parameter_actions_and_cast_effects_are_retained_once() {
    with_source(
        "int f(int n,int (*p)[n],__typeof__((n++,p)) q){(void)((__typeof__(p++))q);return n;}",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let f = s.functions[0];
            assert_eq!(f.parameter_facts.len(), 3);
            assert_eq!(f.parameter_facts[0].site.unwrap().bounds.len(), 0);
            assert_eq!(f.parameter_facts[1].site.unwrap().bounds.len(), 1);
            assert_eq!(
                f.parameter_facts[2].site.unwrap().operand_evaluations.len(),
                1
            );
            assert_eq!(f.parameter_facts[2].site.unwrap().bounds.len(), 0);
            for parameter in f.parameter_facts {
                let site = parameter.site.unwrap();
                for &expression in site.operand_evaluations {
                    assert_eq!(
                        s.control.expression_point(expression).unwrap().function,
                        f.control.id
                    );
                }
                for &bound in site.bounds {
                    assert_eq!(
                        s.control
                            .expression_point(s.runtime_bounds[bound].expression)
                            .unwrap()
                            .function,
                        f.control.id
                    );
                }
            }
            let cast = s
                .expressions
                .iter()
                .find_map(|info| {
                    let ExpressionType::Cast { target_type, .. } = info.expression.kind else {
                        return None;
                    };
                    let site = s.type_name_site(target_type)?;
                    (!site.operand_evaluations.is_empty()).then_some(info.expression)
                })
                .unwrap();
            assert_eq!(
                s.expression_actions(cast)
                    .unwrap()
                    .operand_evaluations
                    .len(),
                1
            );
            for context in [EffectContext::Value, EffectContext::Designation] {
                assert!(
                    s.effect_summary(cast, context)
                        .is_some_and(|effects| !effects.is_empty())
                );
            }
            assert_eq!(s.expressions.len(), s.effects.summaries.len());
        },
    );
}

#[test]
fn m1_deep_region_construction_and_jump_queries_are_bounded() {
    for depth in [128usize, 2_048, 16_384] {
        for enters in [false, true] {
            let mut source = String::from("void f(int n){outside:;");
            if enters {
                source.push_str("goto inside;");
            }
            for level in 0..depth {
                write!(source, "{{typedef int T{level}[n];").unwrap();
                if !enters {
                    source.push_str("goto outside;");
                }
            }
            source.push_str("inside:;");
            if enters {
                for _ in 0..depth {
                    source.push_str("goto inside;");
                }
            }
            for _ in 0..=depth {
                source.push('}');
            }
            with_source(&source, |context, s| {
                assert_eq!(context.pending_error_count(), usize::from(enters));
                let facts = &s.control;
                assert_eq!(facts.jumps.len(), depth + usize::from(enters));
                assert_eq!(facts.construction_work, 5 * facts.regions.len());
                let queries = facts.statements.len() + facts.jumps.len() + usize::from(enters);
                let logarithm = usize::try_from(facts.regions.len().bit_width()).unwrap();
                assert!(
                    facts.query_work <= queries * (3 * logarithm + 4),
                    "{} for {queries} queries",
                    facts.query_work
                );
                println!(
                    "M1 depth={depth} enters={enters} regions={} statements={} jumps={} \
                     construction={} query={} tu_used={} tu_committed={}",
                    facts.regions.len(),
                    facts.statements.len(),
                    facts.jumps.len(),
                    facts.construction_work,
                    facts.query_work,
                    context.tu_arena().used(),
                    context.tu_arena().committed()
                );
            });
        }
    }
    assert!(!std::mem::needs_drop::<Region>());
    assert!(!std::mem::needs_drop::<StatementFacts<'_>>());
    assert!(!std::mem::needs_drop::<JumpFacts>());
    assert!(!std::mem::needs_drop::<SwitchFacts<'_>>());
    assert!(!std::mem::needs_drop::<BoundSiteFacts<'_>>());
}
