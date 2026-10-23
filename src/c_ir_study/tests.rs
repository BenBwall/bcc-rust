//! Observable comparisons use the existing interpreter in both ABI profiles.
//! UB and policy cases are checked against the new contract, never by
//! demanding agreement on undefined program output.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Test fixtures and diagnostic text may use global collections."
)]

use std::path::Path;

use super::{
    arithmetic::Value,
    eval::{
        EventKind,
        Limits,
        evaluate,
    },
    lower,
    model::*,
};
use crate::{
    backend::interpreter::{
        self,
        DefaultHost,
        RuntimeValue,
        Termination,
    },
    configuration::CompilerConfiguration,
    headers::HeaderSearch,
    ir::Profile,
    lowering::tests::programs::PROGRAMS,
    optimizer::OptimizerOptions,
    pipeline::{
        analyze_translation_unit,
        lower_translation_unit,
        parse_translation_unit,
    },
    target::Target,
    translation_phases::Context,
    util::bump::Bump,
};

fn compile<'ir>(source: &str, target: Target, arena: &'ir Bump) -> Result<Module<'ir>, Error> {
    let tu = Bump::new();
    let mut context =
        Context::with_configuration(&tu, CompilerConfiguration::default().with_target(target));
    let source = tu.alloc_str(source);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<c-ir-study>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    assert!(
        sema.lowerable(&context),
        "frontend rejected {source}: {:?}",
        context.take_pending_errors()
    );
    lower(&context, &sema, arena)
}

fn legacy(source: &str, target: Target, optimized: bool, profile: Profile) -> i128 {
    let tu = Bump::new();
    let mut context =
        Context::with_configuration(&tu, CompilerConfiguration::default().with_target(target));
    let source = tu.alloc_str(source);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<c-ir-study>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    let arena = Bump::new();
    let options = OptimizerOptions::default();
    let module = lower_translation_unit(
        &context,
        &syntax,
        &sema,
        &arena,
        optimized.then_some(&options),
        profile,
    )
    .unwrap_or_else(|e| panic!("legacy lowering: {e:?}\n{source}"));
    let host_arena = Bump::new();
    let mut host = DefaultHost::for_target(&host_arena, target);
    let outcome = interpreter::run_main(&module, &mut host)
        .unwrap_or_else(|e| panic!("legacy execution: {e:?}"));
    match outcome.termination {
        | Termination::Returned(Some(RuntimeValue::Int(bits))) => i128::from(bits as u32 as i32),
        | other => panic!("unexpected legacy termination: {other:?}"),
    }
}

const FILE_PROGRAMS: &[(&str, &str, i128)] = &[
    (
        "loops",
        include_str!("../../test-programs/ir-design-study/loops.c"),
        34,
    ),
    (
        "calls",
        include_str!("../../test-programs/ir-design-study/calls.c"),
        141,
    ),
    (
        "values",
        include_str!("../../test-programs/ir-design-study/values.c"),
        18,
    ),
];

#[test]
fn defined_programs_agree_with_both_interpreter_profiles_and_optimizer() {
    let selected = [
        "sum_to_ten",
        "recursive_fibonacci",
        "euclid_gcd",
        "nested_loops_break_continue",
        "recursive_factorial",
        "unsigned_wraparound",
        "collatz_do_while",
    ];
    let programs = FILE_PROGRAMS.iter().copied().chain(
        PROGRAMS
            .iter()
            .filter(|p| selected.contains(&p.name))
            .map(|p| (p.name, p.source, i128::from(p.expected))),
    );
    let mut count = 0;
    for (name, source, expected) in programs {
        count += 1;
        for target in [Target::LinuxGnu, Target::WindowsMsvc] {
            let arena = Bump::new();
            let module =
                compile(source, target, &arena).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            // compile() has already dropped the syntax, sema, context and TU
            // arena.
            let execution_arena = Bump::new();
            let result = evaluate(&module, "main", &[], Limits::default(), &execution_arena);
            assert_eq!(
                result
                    .result
                    .unwrap_or_else(|e| panic!("{name}: {e:?}"))
                    .number(),
                expected,
                "{name} {target:?}"
            );
            for profile in [Profile::PreAbi, Profile::PostAbi] {
                for optimized in [false, true] {
                    assert_eq!(
                        legacy(source, target, optimized, profile),
                        expected,
                        "{name} {target:?} {profile:?} optimized={optimized}"
                    );
                }
            }
        }
    }
    assert_eq!(count, 10, "fixture selection stays explicit");
}

fn outcome(source: &str) -> Result<i128, Failure> {
    let arena = Bump::new();
    let module = compile(source, Target::LinuxGnu, &arena).unwrap_or_else(|e| panic!("{e:?}"));
    let execution_arena = Bump::new();
    let result = evaluate(&module, "main", &[], Limits::default(), &execution_arena);
    if let Err(e) = result.result {
        assert!(e.origin.is_some(), "execution failure has a source handle");
    }
    result.result.map(Value::number).map_err(|e| e.failure)
}

#[test]
fn undefined_operations_fail_even_when_the_result_is_discarded() {
    for (body, expected) in [
        ("int x=2147483647; x+1;", Undefined::SignedOverflow),
        ("int x=(-2147483647-1); -x;", Undefined::SignedOverflow),
        ("int z=0; 1/z;", Undefined::DivisionByZero),
        ("int n=32; 1<<n;", Undefined::InvalidShift),
        ("int x=-1; x<<1;", Undefined::InvalidShift),
        ("int x=(-2147483647-1); x/-1;", Undefined::SignedOverflow),
        ("int x=0; x++ + x++;", Undefined::UnsequencedAccess),
        ("int x=0; x=x++;", Undefined::UnsequencedAccess),
    ] {
        assert_eq!(
            outcome(&format!("int main(void){{{body}return 0;}}")),
            Err(Failure::Undefined(expected)),
            "{body}"
        );
    }
}

#[test]
fn sequencing_short_circuit_and_conversion_rules_are_distinct() {
    for (source, expected) in [
        ("int main(void){int x=1; return (x++, x++);}", 2),
        ("int main(void){int x=0; return (x=1) && (x=2);}", 1),
        ("int main(void){int x=1; return 0 ? 1/--x : 7;}", 7),
        ("int main(void){int x=1; x=x+1; return x;}", 2),
        ("int main(void){int x=1; x=(x++,3); return x;}", 3),
        ("int main(void){int x=1; {int x=3; ++x;} return x;}", 1),
        ("int main(void){unsigned u=4294967295u; return (int)u;}", -1),
        ("int main(void){_Bool b=256; return b;}", 1),
        ("int main(void){int n=-8; return n>>2;}", -2),
    ] {
        assert_eq!(outcome(source), Ok(expected), "{source}");
    }
    assert!(
        matches!(
            outcome("int main(void){int i=0; return (i++, i) + i;}"),
            Err(Failure::Unsupported(_))
        ),
        "complex sequencing is refused instead of overdiagnosed"
    );
}

#[test]
fn indeterminate_read_is_not_reported_as_mandatory_iso_ub() {
    assert!(
        matches!(
            outcome("int main(void){int x; return x;}"),
            Err(Failure::IndeterminateRead { .. })
        ),
        "indeterminate state keeps its own category"
    );
}

#[test]
fn object_instances_begin_at_block_entry_and_end_on_jumps() {
    let arena = Bump::new();
    let module = compile(
        "int main(void){int n=0; while(n<3){int first=1; int x=n; n++; if(n==2)continue; \
         if(n==3)break;} return n;}",
        Target::LinuxGnu,
        &arena,
    )
    .unwrap();
    let runtime = Bump::new();
    let execution = evaluate(&module, "main", &[], Limits::default(), &runtime);
    assert_eq!(execution.result.unwrap().number(), 3);
    let mut live = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let x = module.functions[0]
        .locals
        .iter()
        .position(|l| l.name == "x")
        .unwrap();
    let first = module.functions[0]
        .locals
        .iter()
        .position(|l| l.name == "first")
        .unwrap();
    let mut begins = 0;
    let mut x_live = false;
    for event in execution.events {
        assert!(event.origin.is_some(), "events retain provenance handles");
        let spans = module
            .source_spans(event.origin.unwrap())
            .expect("copied source segments");
        assert!(
            !spans.is_empty() && spans[0].line > 0,
            "source location survives frontend teardown"
        );
        match event.kind {
            | EventKind::Begin { local, instance } => {
                assert!(seen.insert(instance), "every lifetime has a new identity");
                assert!(live.insert((event.activation, instance)), "begin once");
                if local == x {
                    begins += 1;
                    x_live = true;
                }
            },
            | EventKind::End { local, instance } => {
                assert!(
                    live.remove(&(event.activation, instance)),
                    "end a live instance"
                );
                if local == x {
                    x_live = false;
                }
            },
            | EventKind::Write { local, .. } if local == first =>
                assert!(x_live, "x lives before its declaration is reached"),
            | _ => {},
        }
    }
    assert_eq!(begins, 3);
    assert!(
        live.is_empty(),
        "return/break/continue release all instances"
    );
}

#[test]
fn execution_is_bounded_in_steps_calls_and_trace_storage() {
    for (source, limits, reason) in [
        (
            "int main(void){while(1){} return 0;}",
            Limits {
                steps: 1000,
                ..Limits::default()
            },
            "steps",
        ),
        (
            "int f(void){return f();} int main(void){return f();}",
            Limits {
                call_depth: 5,
                ..Limits::default()
            },
            "call depth",
        ),
        (
            "int main(void){int x=1;return x;}",
            Limits {
                events: 3,
                ..Limits::default()
            },
            "trace events",
        ),
    ] {
        let arena = Bump::new();
        let module = compile(source, Target::LinuxGnu, &arena).unwrap();
        let runtime = Bump::new();
        let result = evaluate(&module, "main", &[], limits, &runtime);
        assert_eq!(result.result.unwrap_err().failure, Failure::Limit(reason));
        assert!(
            result.steps <= limits.steps && result.events.len() <= limits.events,
            "bounded retained execution"
        );
    }
}

#[test]
fn unsupported_constructs_never_fall_back_to_the_scalar_pipeline() {
    for source in [
        "int main(void){int a[2];return 0;}",
        "int main(void){volatile int a=0;return a;}",
        "int main(void){static int a;return a;}",
        "int f(int); int main(void){return f(1);}",
        "int main(void){goto done;done:return 0;}",
        "int main(void){switch(1){default:return 0;}}",
    ] {
        let arena = Bump::new();
        assert!(
            matches!(
                compile(source, Target::LinuxGnu, &arena)
                    .unwrap_err()
                    .failure,
                Failure::Unsupported(_)
            ),
            "{source}"
        );
    }
}

#[test]
fn text_is_deterministic_and_invalid_control_references_are_rejected() {
    let a = Bump::new();
    let b = Bump::new();
    let first = compile(FILE_PROGRAMS[0].1, Target::LinuxGnu, &a).unwrap();
    let second = compile(FILE_PROGRAMS[0].1, Target::LinuxGnu, &b).unwrap();
    assert_eq!(first.to_string(), second.to_string());
    assert!(
        first.to_string().contains("object 0 n"),
        "logical parameter survives"
    );
    let bad_code = [Instruction {
        kind:   InstructionKind::Jump {
            label: usize::MAX,
            keep:  0,
        },
        origin: first.functions[0].signature.origin,
    }];
    let bad_function = Function {
        code: &bad_code,
        ..first.functions[0]
    };
    let functions = [bad_function];
    let bad = Module {
        functions: &functions,
        origins:   first.origins,
    };
    assert_eq!(bad.verify().unwrap_err().failure, Failure::InvalidIr);

    let underflow = [
        Instruction {
            kind:   InstructionKind::Exit,
            origin: first.functions[0].signature.origin,
        },
        Instruction {
            kind:   InstructionKind::Fallthrough,
            origin: first.functions[0].signature.origin,
        },
    ];
    let f = Function {
        code: &underflow,
        labels: &[],
        ..first.functions[0]
    };
    assert_eq!(
        Module {
            functions: &[f],
            origins:   first.origins,
        }
        .verify()
        .unwrap_err()
        .failure,
        Failure::InvalidIr
    );

    let result_type = IntType {
        bits: 0,
        ..first.functions[0].signature.result
    };
    let f = Function {
        signature: Signature {
            result: result_type,
            ..first.functions[0].signature
        },
        ..first.functions[0]
    };
    let malformed = Module {
        functions: &[f],
        origins:   first.origins,
    };
    let runtime = Bump::new();
    assert_eq!(
        evaluate(&malformed, "sum", &[], Limits::default(), &runtime)
            .result
            .unwrap_err()
            .failure,
        Failure::InvalidIr
    );
}
