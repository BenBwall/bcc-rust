//! Registry migration contracts and checked-instance coverage.
use std::fmt::Write as _;

use super::*;
use crate::target::Target;

#[test]
fn m4_registry_ids_suffixes_and_preprocessing_support_agree() {
    use crate::builtins::{
        NamingPolicy,
        SyntaxForm,
        lookup,
    };
    let mut source = String::new();
    let mut ids = std::collections::HashSet::new();
    for descriptor in crate::builtins::test_descriptors() {
        assert!(ids.insert(descriptor.id));
        assert_eq!(lookup(descriptor.spelling).unwrap().id, descriptor.id);
        assert_eq!(descriptor.id.descriptor().spelling, descriptor.spelling);
        let advertised = descriptor.availability.naming == NamingPolicy::Reserved;
        write!(
            source,
            "#if __has_builtin({}) != {}\n#error support\n#endif\n",
            descriptor.spelling,
            usize::from(advertised)
        )
        .unwrap();
        if descriptor.syntax == SyntaxForm::OrdinaryCall {
            assert_eq!(named_builtin(descriptor.spelling), advertised);
        }
        for suffix in ["x", "lll", "_bad"] {
            let spelling = format!("{}{suffix}", descriptor.spelling);
            assert!(lookup(&spelling).is_none());
            write!(
                source,
                "#if __has_builtin({spelling})\n#error suffix\n#endif\n"
            )
            .unwrap();
        }
    }
    assert_eq!(ids.len(), 350);
    source.push_str("int following;\n");
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for gnu in [false, true] {
            for hosted in [false, true] {
                let configuration = crate::configuration::CompilerConfiguration::default()
                    .with_gnu_extensions(gnu)
                    .with_hosted(hosted)
                    .with_target(target);
                with_configuration(&source, configuration, |context, unit| {
                    assert_eq!(
                        unit.errors,
                        0,
                        "{target:?} {gnu} {hosted}: {:?}",
                        context.take_pending_errors()
                    );
                });
            }
        }
    }
}

#[test]
fn m4_every_ordinary_spelling_yields_to_a_user_function_pointer() {
    let mut source = String::from("int identity(int n){return n;} void f(void){\n");
    for descriptor in crate::builtins::test_descriptors() {
        if descriptor.syntax == crate::builtins::SyntaxForm::OrdinaryCall {
            writeln!(
                source,
                "{{int (*{})(int)=identity;(void){}(1);}}",
                descriptor.spelling, descriptor.spelling
            )
            .unwrap();
        }
    }
    source.push_str("}\n");
    for gnu in [false, true] {
        with_configuration(
            &source,
            crate::configuration::CompilerConfiguration::default().with_gnu_extensions(gnu),
            |context, unit| {
                assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
                for info in unit.expressions {
                    if matches!(info.expression.kind, ExpressionType::Call { .. }) {
                        assert!(matches!(
                            unit.call_facts(info.expression).unwrap().callee,
                            Callee::Indirect(_)
                        ));
                        assert!(unit.builtin_call_facts(info.expression).is_none());
                    }
                }
                assert!(
                    unit.bindings
                        .iter()
                        .all(|binding| binding.origin == BindingOrigin::User)
                );
            },
        );
    }
}

#[test]
fn m4_checked_instances_retain_normalized_values_types_storage_and_effects() {
    use crate::builtins::{
        ArgumentEvaluation as E,
        BuiltinId as I,
        BuiltinOperation as B,
    };
    let source = "struct S{unsigned bit:3;}; void f(void *p,int n,...){
        __builtin_va_list a,b;struct S s={0};
        (__builtin_va_start)(a,n);(__builtin_va_copy)(b,a);
        (void)__builtin_va_arg(b,int);(__builtin_va_end)(b);
        (void)__builtin_object_size(p,((unsigned __int128)1<<127)+2);
        (void)__builtin_assume_aligned(p,((unsigned __int128)1<<64)+16,n);
        (void)__builtin_expect_with_probability(n,1,1.0L+0x1p-60L);
        (void)__builtin_add_overflow_p(n,1,s.bit);
        (void)__builtin_fabsf(1.0f);(void)__builtin_fabsl(1.0L);
        __builtin_prefetch(p,1,3); }";
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        with_configuration(
            source,
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(
                    unit.errors,
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                );
                let instances = unit
                    .expressions
                    .iter()
                    .filter_map(|info| unit.builtin_call_facts(info.expression))
                    .collect::<Vec<_>>();
                assert_eq!(instances.len(), 11);
                let get = |id| {
                    *instances
                        .iter()
                        .find(|instance| instance.descriptor == id)
                        .unwrap()
                };
                for instance in &instances {
                    assert_eq!(
                        instance.operation,
                        instance.descriptor.descriptor().operation
                    );
                    assert_eq!(instance.effects, instance.descriptor.descriptor().effects);
                }
                let start = get(I::VaStart);
                assert_eq!(start.operands[0].evaluation, E::StorageAddress);
                assert!(start.operands[0].storage_binding.is_some());
                assert_eq!(start.operands[1].evaluation, E::Unevaluated);
                let arg = get(I::VaArg);
                assert_eq!(arg.operands[1].evaluation, E::Type);
                assert!(arg.operands[1].expression.is_none());
                assert_eq!(arg.operands[1].ty, arg.result_type);
                let object = get(I::ObjectSize);
                assert_eq!(object.operands[1].integer.unwrap().to_u64(), Some(2));
                assert_eq!(object.operands[1].integer.unwrap().bits, 32);
                assert!(
                    object
                        .operands
                        .iter()
                        .all(|operand| operand.evaluation == E::Unevaluated)
                );
                assert_eq!(object.integer.unwrap().to_u64(), Some(0));
                assert_eq!(
                    get(I::AssumeAligned).operands[1].integer.unwrap().to_u64(),
                    Some(16)
                );
                assert_eq!(
                    get(I::ExpectWithProbability).operands[2]
                        .floating
                        .unwrap()
                        .real,
                    crate::float_parsing::LongDouble::from_double(1.0)
                );
                let overflow = get(I::AddOverflowP);
                assert_eq!(overflow.operands[2].evaluation, E::EffectsOnly);
                assert_eq!(overflow.operands[2].bit_field, Some(3));
                assert!(overflow.operation_type.is_some());
                assert_eq!(get(I::LibraryFabsf).operation, B::Library);
                assert_eq!(
                    unit.types.nodes[get(I::LibraryFabsf).result_type.index],
                    TypeKind::Scalar(Scalar::Float)
                );
                assert_eq!(
                    unit.types.nodes[get(I::LibraryFabsl).result_type.index],
                    TypeKind::Scalar(Scalar::LongDouble)
                );
            },
        );
    }
}

#[test]
fn m4_adjusted_sysv_varargs_parameters_retain_pointer_evaluation() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_configuration(
            "void f(__builtin_va_list a){__builtin_va_end(a);}",
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
                let instance = unit
                    .expressions
                    .iter()
                    .find_map(|info| unit.builtin_call_facts(info.expression))
                    .unwrap();
                assert_eq!(
                    instance.operands[0].evaluation,
                    if target == Target::LinuxGnu {
                        crate::builtins::ArgumentEvaluation::AdjustedPointer
                    } else {
                        crate::builtins::ArgumentEvaluation::StorageAddress
                    }
                );
            },
        );
    }
}

#[test]
fn m4_binding_origins_keep_implicit_and_m6_opaque_calls_distinct() {
    with_configuration(
        "int f(void){(void)__atomic_always_lock_free(4,0);
        (void)__builtin_classify_type(0);(void)__builtin_M4_unknown();return external();}",
        crate::configuration::CompilerConfiguration::default().with_gnu_extensions(true),
        |context, unit| {
            assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
            for (spelling, origin) in [
                (
                    "__atomic_always_lock_free",
                    BindingOrigin::Intrinsic(crate::builtins::BuiltinId::AtomicGnuAlwaysLockFree),
                ),
                ("__builtin_classify_type", BindingOrigin::LegacyIntrinsic),
                ("__builtin_M4_unknown", BindingOrigin::OpaqueIntrinsic),
                ("external", BindingOrigin::ImplicitOrdinary),
            ] {
                let binding = unit
                    .bindings
                    .iter()
                    .find(|binding| context.string_cache.at(binding.name.name) == spelling)
                    .unwrap();
                assert_eq!(binding.origin, origin);
            }
        },
    );
}

#[test]
fn m4_scalar_extension_policy_uses_descriptor_feature() {
    for gnu in [false, true] {
        for policy in [
            crate::configuration::ExtensionPolicy::Allow,
            crate::configuration::ExtensionPolicy::Warn,
            crate::configuration::ExtensionPolicy::Deny,
        ] {
            with_configuration(
                "int f(unsigned n){return __builtin_clz(n);}",
                crate::configuration::CompilerConfiguration::new(CStandard::C99, policy)
                    .with_gnu_extensions(gnu),
                |context, unit| {
                    let errors = context.take_pending_errors();
                    assert_eq!(unit.errors, 0);
                    for error in &errors {
                        assert_eq!(
                            crate::translation_phases::GetSeverity::severity(error),
                            if policy == crate::configuration::ExtensionPolicy::Deny {
                                crate::translation_phases::ErrorSeverity::Error
                            } else {
                                crate::translation_phases::ErrorSeverity::Warning
                            }
                        );
                    }
                    assert_eq!(
                        errors.len(),
                        usize::from(policy != crate::configuration::ExtensionPolicy::Allow),
                        "{gnu:?} {policy:?}: {errors:?}"
                    );
                },
            );
        }
    }
}

#[test]
fn m4_registry_preserves_baseline_semantic_types_on_every_target() {
    for (target, expected) in [
        (
            Target::LinuxGnu,
            include_str!("../../../../tests/fixtures/semantic/registry-contracts-linux-gnu.types"),
        ),
        (
            Target::LinuxMusl,
            include_str!("../../../../tests/fixtures/semantic/registry-contracts-linux-musl.types"),
        ),
        (
            Target::WindowsGnu,
            include_str!(
                "../../../../tests/fixtures/semantic/registry-contracts-windows-gnu.types"
            ),
        ),
        (
            Target::WindowsMsvc,
            include_str!(
                "../../../../tests/fixtures/semantic/registry-contracts-windows-msvc.types"
            ),
        ),
    ] {
        let tu = Bump::new();
        let configuration = crate::configuration::CompilerConfiguration::new(
            CStandard::C17,
            crate::configuration::ExtensionPolicy::Allow,
        )
        .with_gnu_extensions(true)
        .with_target(target);
        let mut context = Context::with_configuration(&tu, configuration);
        let syntax = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<registry>"),
            include_str!("../../../../tests/fixtures/semantic/registry-contracts.c"),
            crate::headers::HeaderSearch::default(),
        );
        let unit = crate::pipeline::analyze_translation_unit(&mut context, &syntax);
        assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
        let arena = Bump::new();
        assert_eq!(unit.inspect(&context, &arena), expected, "{target:?}");
    }
}
