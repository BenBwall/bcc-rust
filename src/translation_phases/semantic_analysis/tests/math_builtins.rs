//! Real math builtin contracts, static math constants and operand diagnostics.

use std::fmt::Write as _;

use super::*;
use crate::target::Target;

#[test]
fn math_builtins_have_all_c99_real_precision_signatures() {
    let fixture = include_str!("../../../../tests/fixtures/programs/host_math.c");
    let declarations = fixture.split("static int test_float(void)").next().unwrap();
    let mut source = String::from(declarations);
    // Header declarations and builtin aliases must have compatible function
    // types, including output pointers, target long, and nexttoward's second
    // long-double operand. The fixture independently inventories every family.
    for declaration in declarations.lines().filter(|line| line.ends_with(';')) {
        let Some((prefix, _)) = declaration.split_once('(') else {
            continue;
        };
        let name = prefix.split_whitespace().last().unwrap();
        if name == "printf" || name.starts_with("copysign") {
            continue;
        }
        writeln!(
            source,
            "int check_{name}[__builtin_types_compatible_p(__typeof__({name}), \
             __typeof__(__builtin_{name})) ? 1 : -1];"
        )
        .unwrap();
    }
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        with_configuration(
            &source,
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(
                    unit.errors,
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                );
            },
        );
    }
}

#[test]
fn math_header_constants_initialize_static_objects_in_each_precision() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        with_configuration(
            r#"
static float infinity = __builtin_inff(), hugef = __builtin_huge_valf(), nanf = __builtin_nanf("");
static double inf = __builtin_inf(), huge = __builtin_huge_val(), nan = __builtin_nan("0x123");
static long double infl = __builtin_infl(), hugel = __builtin_huge_vall(), nanl = __builtin_nanl("017");
int signatures[__builtin_types_compatible_p(__typeof__(__builtin_inff()), float)
    && __builtin_types_compatible_p(__typeof__(__builtin_inf()), double)
    && __builtin_types_compatible_p(__typeof__(__builtin_infl()), long double)
    && __builtin_types_compatible_p(__typeof__(__builtin_nanf("")), float)
    && __builtin_types_compatible_p(__typeof__(__builtin_nan("")), double)
    && __builtin_types_compatible_p(__typeof__(__builtin_nanl("")), long double) ? 1 : -1];
int classifications[__builtin_isnan(__builtin_nan(""))
    && __builtin_isnanf(__builtin_nanf("")) && __builtin_isnanl(__builtin_nanl(""))
    && __builtin_isinf(__builtin_inff()) && __builtin_isinf(__builtin_huge_vall()) ? 1 : -1];
"#,
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(
                    unit.errors,
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                );
            },
        );
    }
}

#[test]
fn math_builtin_constraints_report_invalid_calls_and_keep_following_declarations() {
    for source in [
        "void f(void) { __builtin_sqrt(); }",
        "void f(void) { __builtin_sqrt((void *)0); }",
        "void f(void) { __builtin_pow(1.0); }",
        "void f(void) { __builtin_fmaf(1.0f, 2.0f); }",
        "void f(void) { __builtin_frexpl(1.0L, (float *)0); }",
        "void f(void) { __builtin_modff(1.0f, (double *)0); }",
        "void f(void) { __builtin_inff(1); }",
        "void f(void) { __builtin_nanf(1.0f); }",
        "void f(void) { __builtin_nanl(); }",
        "void f(void) { __builtin_isnanf((void *)0); }",
        "void f(void) { __builtin_rotateleft32(1); }",
        "void f(void) { __builtin_rotateleft32(1, (void *)0); }",
    ] {
        with_source(&format!("{source} int following;"), |context, unit| {
            assert_eq!(
                unit.errors,
                1,
                "{source}: {:?}",
                context.take_pending_errors()
            );
            assert!(
                unit.bindings
                    .iter()
                    .any(|binding| context.string_cache.at(binding.name.name) == "following")
            );
        });
    }
}

#[test]
fn dynamic_nan_tags_do_not_become_static_constants() {
    with_source(
        "char tag[] = \"123\"; double value = __builtin_nan(tag); int following;",
        |context, unit| {
            assert_eq!(unit.errors, 1, "{:?}", context.take_pending_errors());
        },
    );
}

#[test]
fn constant_rotate_counts_wrap_at_the_operand_width() {
    with_source(
        "static unsigned value = __builtin_rotateleft32(0x12345678u, 36); int \
         counts[__builtin_rotateleft8(0x81, 9) == 3 && __builtin_rotateright16(3, 17) == 0x8001 \
         && __builtin_rotateleft32(0x12345678u, -4) == 0x81234567u && \
         __builtin_rotateleft64(1ull, 64) == 1 ? 1 : -1];",
        |context, unit| assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors()),
    );
}

#[test]
fn dynamic_nan_retains_library_call_decay_and_argument_evaluation() {
    with_source(
        "double f(char *tag) { return (__builtin_choose_expr(1, __builtin_nan, \
         __builtin_nan))(tag); }",
        |context, unit| {
            assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
            let expression = unit
                .expressions
                .iter()
                .find(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                .unwrap()
                .expression;
            let facts = unit.call_facts(expression).unwrap();
            assert!(matches!(
                facts.callee,
                Callee::Builtin {
                    direct: Some(_),
                    ..
                }
            ));
            let builtin = unit.builtin_call_facts(expression).unwrap();
            assert_eq!(builtin.descriptor, crate::builtins::BuiltinId::LibraryNan);
            assert_eq!(
                builtin.operands[0].evaluation,
                crate::builtins::ArgumentEvaluation::Value
            );
            assert_eq!(
                builtin.effects,
                crate::builtins::BuiltinEffectClass::Library
            );
        },
    );
}
