//! M5 retained expression effects and effects-only recipes, queried through
//! the same interface lowering consumes.
use std::fmt::Write as _;

use super::{
    facts::effects::{
        EffectSet as F,
        EffectUse,
    },
    *,
};
use crate::translation_phases::parsing::syntax::{
    BlockItem,
    ExpressionSlot,
    StatementType,
};

const PRELUDE: &str = "int f(int); int g; volatile int v; int *p; volatile int *vp; struct S { \
                       int m; volatile int vm; } s, *ps; void *q;";

/// Runs `check` on the final expression statement of a probe function.
fn probe(
    declarations: &str,
    expression: &str,
    check: impl for<'tu> FnOnce(&SemanticTranslationUnit<'tu>, &'tu Expression<'tu>),
) {
    probe_in(
        crate::configuration::CompilerConfiguration::default(),
        declarations,
        expression,
        check,
    );
}

/// [`probe`] in GNU C23, for its predefined constants.
fn probe_c23(
    expression: &str,
    check: impl for<'tu> FnOnce(&SemanticTranslationUnit<'tu>, &'tu Expression<'tu>),
) {
    probe_in(
        crate::configuration::CompilerConfiguration::new(
            CStandard::C23,
            crate::configuration::ExtensionPolicy::Allow,
        )
        .with_gnu_extensions(true),
        "",
        expression,
        check,
    );
}

fn probe_in(
    configuration: crate::configuration::CompilerConfiguration,
    declarations: &str,
    expression: &str,
    check: impl for<'tu> FnOnce(&SemanticTranslationUnit<'tu>, &'tu Expression<'tu>),
) {
    with_configuration(
        &format!("{PRELUDE}\nvoid probe(void){{ {declarations} {expression}; }}"),
        configuration,
        |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{expression}: {:?}",
                context.take_pending_errors()
            );
            let body = sema.functions.last().unwrap().syntax.body;
            let StatementType::Compound { items } = body.kind else {
                panic!("probe body");
            };
            let Some(&BlockItem::Statement(statement)) = items.last() else {
                panic!("probe statement");
            };
            let StatementType::Expression(ExpressionSlot::Parsed(e)) = statement.kind else {
                panic!("probe expression");
            };
            check(sema, e);
        },
    );
}

fn summary(declarations: &str, expression: &str) -> [F; 2] {
    let mut result = [F::empty(); 2];
    probe(declarations, expression, |sema, e| {
        result = [
            sema.effect_summary(e, EffectContext::Value).unwrap(),
            sema.effect_summary(e, EffectContext::Designation).unwrap(),
        ];
    });
    result
}

#[test]
fn m5_effect_classes_follow_c_evaluation_and_registry_contracts() {
    let none = F::empty();
    for (declarations, expression, value, designation) in [
        // Calls, volatile accesses and address-only uses.
        ("", "f(1)", F::CALL, F::CALL),
        ("", "g", none, none),
        ("", "v", F::VOLATILE, none),
        ("", "&v", none, none),
        ("", "*vp", F::VOLATILE, none),
        ("", "&*vp", none, none),
        ("", "s.vm", F::VOLATILE, none),
        ("", "&ps->vm", none, none),
        ("", "ps->m", none, none),
        // Stores.
        ("", "g = 1", F::WRITE, F::WRITE),
        ("", "v = 1", F::WRITE | F::VOLATILE, F::WRITE | F::VOLATILE),
        ("", "g++", F::WRITE, F::WRITE),
        ("", "--g", F::WRITE, F::WRITE),
        ("", "g += f(0)", F::WRITE | F::CALL, F::WRITE | F::CALL),
        // Statement expressions summarize their bodies.
        ("", "({ g++; 0; })", F::WRITE, F::WRITE),
        ("", "({ 1; })", none, none),
        ("", "({ L: g; })", none, none),
        ("", "({ L: v; })", F::VOLATILE, F::VOLATILE),
        ("", "({ f(0); })", F::CALL, F::CALL),
        ("", "({ while (g) ; 0; })", F::CONTROL, F::CONTROL),
        ("", "({ if (g) g; 0; })", none, none),
        ("", "({ int t = 0; t; })", F::UNKNOWN, F::UNKNOWN),
        // Only the selected arm of a generic or choose-expression executes.
        ("", "_Generic(0, int: g, default: f(1))", none, none),
        ("", "_Generic(0, int: f(1), default: g)", F::CALL, F::CALL),
        ("", "_Generic(0, int: v, default: f(1))", F::VOLATILE, none),
        ("", "__builtin_choose_expr(1, g, f(1))", none, none),
        ("", "__builtin_choose_expr(0, g, f(1))", F::CALL, F::CALL),
        // Unevaluated sizeof and typeof operands; evaluated VLA operands.
        ("", "sizeof(f(1))", none, none),
        ("", "sizeof(g++)", none, none),
        ("", "(__typeof__(f(1)))g", none, none),
        ("", "sizeof(int[g])", none, none),
        ("", "sizeof(int[g++])", F::WRITE, F::WRITE),
        ("int b[g][g];", "sizeof(b[f(0)])", F::CALL, F::CALL),
        ("int b[g][g];", "sizeof(b[0])", none, none),
        ("", "(int (*)[g++])q", F::WRITE, F::WRITE),
        (
            "int b[g][g], (*r)[g] = b;",
            "(__typeof__(r++))q",
            F::WRITE,
            F::WRITE,
        ),
        // Constants: label addresses, C23 predefined constants, and C2y
        // `_Countof`, which evaluates only an operand of variable length.
        ("l:;", "&&l", none, none),
        ("int a[3];", "_Countof(a)", none, none),
        ("int a[g];", "_Countof(a)", none, none),
        ("int b[g][g];", "_Countof(b[f(0)])", F::CALL, F::CALL),
        ("int b[g][3];", "_Countof(b[f(0)])", none, none),
        ("", "_Countof(int[3][g++])", none, none),
        ("", "_Countof(int[g++])", F::WRITE, F::WRITE),
        ("", "_Countof(g)", F::UNKNOWN, F::UNKNOWN),
        // Comma, conditional and short circuit operands.
        ("", "(g, f(1))", F::CALL, F::CALL),
        ("", "(g, v)", F::VOLATILE, F::VOLATILE),
        ("", "g ? f(1) : 0", F::CALL, F::CALL),
        ("", "g ? g : 1", none, none),
        ("", "g ?: g++", F::WRITE, F::WRITE),
        ("", "g && f(1)", F::CALL, F::CALL),
        ("", "g || g", none, none),
        // Builtins through the registry's effect classes and operand modes.
        ("", "__builtin_expect(g, 1)", none, none),
        ("", "__builtin_expect(f(1), 1)", F::CALL, F::CALL),
        ("", "__builtin_popcount(g)", none, none),
        ("", "__builtin_trap()", F::CONTROL, F::CONTROL),
        ("", "__builtin_unreachable()", F::CONTROL, F::CONTROL),
        ("", "__builtin_alloca(g)", F::LIFETIME, F::LIFETIME),
        ("", "__builtin_add_overflow(g, 1, &g)", F::WRITE, F::WRITE),
        ("", "__builtin_add_overflow_p(g, 1, (int)0)", none, none),
        ("", "__builtin_add_overflow_p(g, 1, *p)", none, none),
        (
            "",
            "__builtin_add_overflow_p(g, 1, *vp)",
            F::VOLATILE,
            F::VOLATILE,
        ),
        ("", "__builtin_add_overflow_p(g, 1, f(1))", F::CALL, F::CALL),
        (
            "l:;",
            "__builtin_add_overflow_p(g, 1, (long)&&l)",
            none,
            none,
        ),
        ("", "__builtin_object_size(p, 0)", none, none),
        ("", "__builtin_strlen(\"x\")", F::CALL, F::CALL),
        (
            "__builtin_va_list ap;",
            "__builtin_va_end(ap)",
            F::WRITE,
            F::WRITE,
        ),
    ] {
        assert_eq!(
            summary(declarations, expression),
            [value, designation],
            "{declarations} {expression}"
        );
    }
    for expression in ["true", "false", "nullptr"] {
        probe_c23(expression, |sema, e| {
            for context in [EffectContext::Value, EffectContext::Designation] {
                assert_eq!(sema.effect_summary(e, context), Some(none), "{expression}");
                assert_eq!(
                    sema.effect_recipe(e, context),
                    Some(EffectRecipe::None),
                    "{expression}"
                );
            }
        });
    }
}

#[test]
fn m5_recipes_schedule_only_effects_and_conditions() {
    fn call<'tu>(sema: &SemanticTranslationUnit<'tu>, e: &'tu Expression<'tu>) -> usize {
        let index = sema.expression_index(e).unwrap();
        let ExpressionType::Call { .. } = sema.expressions[index].expression.kind else {
            panic!("not a call");
        };
        index
    }
    fn recipe<'tu>(
        sema: &SemanticTranslationUnit<'tu>,
        e: &'tu Expression<'tu>,
    ) -> EffectRecipe<'tu> {
        sema.effect_recipe(e, EffectContext::Value).unwrap()
    }
    // Short circuits retain one branch on the left operand.
    for (expression, and) in [("g && f(1)", true), ("g || f(1)", false)] {
        probe("", expression, |sema, e| {
            let ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } = e.kind
            else {
                panic!("binary");
            };
            let right = EffectUse {
                expression: ExprId(call(sema, right_expression)),
                context:    EffectContext::Value,
            };
            assert_eq!(
                recipe(sema, e),
                EffectRecipe::Conditional {
                    condition: ExprId(sema.expression_index(left_expression).unwrap()),
                    then:      and.then_some(right),
                    otherwise: (!and).then_some(right),
                }
            );
        });
    }
    // A pure right operand needs no branch; neither does a pure read.
    probe("", "g && g", |sema, e| {
        assert_eq!(recipe(sema, e), EffectRecipe::None);
    });
    // Both arms with effects; an omitted conditional reuses its condition.
    probe("", "g ? f(1) : g++", |sema, e| {
        let EffectRecipe::Conditional {
            then: Some(_),
            otherwise: Some(_),
            ..
        } = recipe(sema, e)
        else {
            panic!("two-armed recipe");
        };
    });
    probe("", "g ?: f(1)", |sema, e| {
        let ExpressionType::OmittedConditional(c) = e.kind else {
            panic!("omitted conditional");
        };
        assert_eq!(
            recipe(sema, e),
            EffectRecipe::Conditional {
                condition: ExprId(sema.expression_index(c.condition_expression).unwrap()),
                then:      None,
                otherwise: Some(EffectUse {
                    expression: ExprId(call(sema, c.else_expression)),
                    context:    EffectContext::Value,
                }),
            }
        );
    });
    probe("", "g ? g : 1", |sema, e| {
        assert_eq!(recipe(sema, e), EffectRecipe::None);
    });
    // Volatile reads evaluate; their addresses do not.
    probe("", "v", |sema, e| {
        let index = sema.expression_index(e).unwrap();
        assert_eq!(recipe(sema, e), EffectRecipe::EvaluateValue(ExprId(index)));
        assert_eq!(
            sema.effect_recipe(e, EffectContext::Designation),
            Some(EffectRecipe::None)
        );
    });
    probe("", "&v", |sema, e| {
        assert_eq!(recipe(sema, e), EffectRecipe::None);
    });
    // Arithmetic keeps only the effectful operand; ICE operands do nothing.
    probe("", "f(1) + g", |sema, e| {
        let ExpressionType::Binary {
            left_expression, ..
        } = e.kind
        else {
            panic!("binary");
        };
        assert_eq!(
            recipe(sema, e),
            EffectRecipe::Sequence(&[EffectUse {
                expression: ExprId(call(sema, left_expression)),
                context:    EffectContext::Value,
            }])
        );
    });
    for (declarations, expression) in [
        ("", "sizeof(f(1))"),
        ("l:;", "&&l"),
        ("", "_Countof(int[3][g++])"),
    ] {
        probe(declarations, expression, |sema, e| {
            assert_eq!(recipe(sema, e), EffectRecipe::None, "{expression}");
        });
    }
    probe("", "_Countof(int[g++])", |sema, e| {
        let index = sema.expression_index(e).unwrap();
        assert_eq!(recipe(sema, e), EffectRecipe::EvaluateValue(ExprId(index)));
    });
    // Selected arms forward; unselected arms are never named.
    probe("", "_Generic(0, int: f(1), default: g++)", |sema, e| {
        let EffectRecipe::Sequence(&[selected]) = recipe(sema, e) else {
            panic!("selected arm");
        };
        assert!(matches!(
            sema.expressions[selected.expression.0].expression.kind,
            ExpressionType::Call { .. }
        ));
    });
    probe("", "({ g++; 0; })", |sema, e| {
        let index = sema.expression_index(e).unwrap();
        assert_eq!(recipe(sema, e), EffectRecipe::StatementBlock(ExprId(index)));
    });
    // A VM cast keeps its own bound action even when its operand is pure.
    probe("", "(int (*)[g])q", |sema, e| {
        assert_eq!(recipe(sema, e), EffectRecipe::Sequence(&[]));
        assert_eq!(sema.expression_actions(e).unwrap().bounds.len(), 1);
    });
}

/// Production is a constant number of steps per expression and statement,
/// for deep operators, selections and nested statement expressions alike.
#[test]
fn m5_deep_effect_production_is_linear() {
    for depth in [128usize, 2_048, 16_384] {
        let mut source = String::from("int f(int);int g;int t(void){return ");
        for i in 0..depth {
            source.push_str(match i % 4 {
                | 0 => "g&&(",
                | 1 => "g?(",
                | 2 => "_Generic(0,int:(",
                | _ => "({g++;",
            });
        }
        source.push_str("f(g++)");
        for i in (0..depth).rev() {
            source.push_str(match i % 4 {
                | 0 => ")",
                | 1 => "):g++",
                | 2 => "),default:f(0))",
                | _ => ";})",
            });
        }
        source.push_str(";}");
        with_source(&source, |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let work = sema.effects.work;
            let expressions = sema.expressions.len();
            let statements = sema.control.statements.len();
            assert!(
                work <= 4 * expressions + statements,
                "{work} effect steps for {expressions} expressions and {statements} statements"
            );
            let mut line = String::new();
            write!(
                line,
                "M5 depth={depth} expressions={expressions} statements={statements} \
                 effect_steps={work} tu_used={}",
                context.tu_arena().used()
            )
            .unwrap();
            println!("{line}");
        });
    }
    assert!(!std::mem::needs_drop::<EffectRecipe<'_>>());
    assert_eq!(
        size_of::<facts::effects::ExpressionEffects>(),
        4,
        "two contexts of one effect byte and one recipe byte"
    );
}

/// A label address in the destination operand of an overflow predicate is
/// effect-free, so the predicate folds to an integer constant expression.
/// GNU extension: GCC Integer Overflow Builtins and Labels as Values.
#[test]
fn m5_overflow_predicate_folds_over_label_addresses() {
    probe(
        "l:;",
        "__builtin_add_overflow_p(1, 2, (long)&&l) + __builtin_mul_overflow_p(1 << 30, 4, \
         (int)(&&l - &&l))",
        |sema, e| {
            let index = sema.expression_index(e).unwrap();
            let info = &sema.expressions[index];
            assert!(info.ice);
            assert_eq!(info.integer.map(|integer| integer.value), Some(1));
        },
    );
}
