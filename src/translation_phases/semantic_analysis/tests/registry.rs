//! Registry migration contracts and checked-instance coverage.
use std::fmt::Write as _;

use super::*;
use crate::target::Target;

#[test]
fn m4_registry_ids_suffixes_and_preprocessing_support_agree() {
    use crate::builtins::{
        NamingPolicy,
        lookup,
    };
    let mut source = String::new();
    let mut ids = std::collections::HashSet::new();
    for descriptor in crate::builtins::test_descriptors() {
        assert!(ids.insert(descriptor.id));
        assert_eq!(lookup(descriptor.spelling).unwrap().id, descriptor.id);
        assert_eq!(descriptor.id.descriptor().spelling, descriptor.spelling);
        // The default configuration has no `_FloatN` types, so their
        // builtins do not exist.
        let advertised = descriptor.availability.naming == NamingPolicy::Reserved
            && descriptor.availability.feature
                != Some(crate::configuration::Feature::InterchangeFloatTypes);
        write!(
            source,
            "#if __has_builtin({}) != {}\n#error support\n#endif\n",
            descriptor.spelling,
            usize::from(advertised)
        )
        .unwrap();
        for suffix in ["x", "lll", "_bad"] {
            let spelling = format!("{}{suffix}", descriptor.spelling);
            // A suffix can form another exact builtin, such as fma + x = fmax.
            // Every registered spelling is already a positive probe above.
            if crate::builtins::test_descriptors().any(|other| other.spelling == spelling) {
                continue;
            }
            assert!(lookup(&spelling).is_none());
            write!(
                source,
                "#if __has_builtin({spelling})\n#error suffix\n#endif\n"
            )
            .unwrap();
        }
    }
    assert_eq!(ids.len(), 905);
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
                // A parameter's object is known only after inlining: the
                // pointer is evaluated for the IR query, and the mode not.
                let object = get(I::ObjectSize);
                assert_eq!(object.operands[1].integer.unwrap().to_u64(), Some(2));
                assert_eq!(object.operands[1].integer.unwrap().bits, 32);
                assert_eq!(object.operands[0].evaluation, E::Value);
                assert_eq!(object.operands[1].evaluation, E::Unevaluated);
                assert!(object.integer.is_none());
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
fn binding_origins_keep_implicit_registered_and_opaque_calls_distinct() {
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
                (
                    "__builtin_classify_type",
                    BindingOrigin::Intrinsic(crate::builtins::BuiltinId::ClassifyType),
                ),
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

/// A builtin's reserved name is no extension diagnostic under any policy,
/// as GCC and Clang report none.
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
                    assert!(errors.is_empty(), "{gnu:?} {policy:?}: {errors:?}");
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
        assert_eq!(
            unit.inspect(&context, &arena, false),
            expected,
            "{target:?}"
        );
    }
}

#[test]
fn m6_family_descriptors_retain_no_instance_and_keep_their_contracts() {
    use crate::builtins::{
        BuiltinOperation as B,
        ConstantConstraint,
        SignatureType,
        SyntaxForm,
        TypeRule,
    };
    let mut x86 = 0;
    let mut immediates = 0;
    for descriptor in crate::builtins::test_descriptors() {
        let family = matches!(
            descriptor.type_rule,
            TypeRule::X86Signature
                | TypeRule::VectorShuffle
                | TypeRule::GnuShuffle
                | TypeRule::VectorOverload(_)
                | TypeRule::ConvertVector
                | TypeRule::BitCast
                | TypeRule::TypesCompatible
                | TypeRule::ChooseExpr
                | TypeRule::TypeClassify
                | TypeRule::Math128(_)
        );
        assert_eq!(
            family,
            matches!(descriptor.operation, B::Semantic(_) | B::Unsupported(_)),
            "{}",
            descriptor.spelling
        );
        assert_eq!(
            family,
            !descriptor.operation.retains_instance(),
            "{}",
            descriptor.spelling
        );
        if descriptor.type_rule == TypeRule::X86Signature {
            x86 += 1;
            assert!(descriptor.spelling.starts_with("__builtin_ia32_"));
            assert!(descriptor.signature.prototype && !descriptor.signature.variadic);
            let count = descriptor.signature.parameters.len();
            assert_eq!(descriptor.arguments.len(), count);
            assert_eq!(descriptor.arity, (count, count));
            immediates += usize::from(
                descriptor
                    .arguments
                    .iter()
                    .any(|rule| matches!(rule.constraint, ConstantConstraint::Immediate(..))),
            );
        } else if family {
            assert_eq!(descriptor.signature.result, SignatureType::Opaque);
            assert_eq!(descriptor.syntax == SyntaxForm::OrdinaryCall, {
                !matches!(
                    descriptor.type_rule,
                    TypeRule::ConvertVector
                        | TypeRule::BitCast
                        | TypeRule::TypesCompatible
                        | TypeRule::ChooseExpr
                )
            });
        }
    }
    assert_eq!((x86, immediates), (384, 111));
}

/// The generated x86 shard merges header calls that name a scalar spelling;
/// the scalar descriptor must have the signature Clang verified.
#[test]
fn m6_merged_x86_spellings_agree_with_their_scalar_descriptor() {
    use crate::{
        builtins::{
            BuiltinOperation,
            SignatureType as T,
        },
        target::Scalar as S,
    };
    for &(spelling, encoding) in crate::builtins::test_merged_x86 {
        let descriptor = crate::builtins::lookup(spelling).unwrap();
        assert!(matches!(descriptor.operation, BuiltinOperation::Bit(..)));
        let code = |pattern| match pattern {
            | T::Scalar(S::Int) => "i",
            | T::Scalar(S::UnsignedInt) => "I",
            | T::Scalar(S::UnsignedLongLong) => "Q",
            | other => panic!("{spelling}: {other:?}"),
        };
        let signature = descriptor.signature;
        let mut codes = vec![code(signature.result)];
        codes.extend(
            signature
                .parameters
                .iter()
                .map(|&parameter| code(parameter)),
        );
        assert!(signature.prototype && !signature.variadic);
        assert_eq!(codes.join(","), encoding, "{spelling}");
    }
}

/// Every builtin, atomic or keyword spelling a resource header names is
/// registered, so no header call falls back to an opaque binding.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "Header discovery and loading are outside compiler allocation measurements."
)]
fn m6_every_resource_header_builtin_is_registered() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/headers");
    let mut pending = vec![directory];
    let mut checked = 0;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "h") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (start, _) in text.match_indices("__") {
                let rest = &text[start..];
                let end = rest
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                let word = &rest[..end];
                if start > 0
                    && text[..start]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    continue;
                }
                if !["__builtin_", "__atomic_", "__sync_"]
                    .iter()
                    .any(|prefix| word.starts_with(prefix))
                    || word == "__builtin_va_list"
                {
                    continue;
                }
                checked += 1;
                assert!(
                    crate::builtins::lookup(word).is_some(),
                    "{}: {word}",
                    path.display()
                );
            }
        }
    }
    assert!(checked > 400, "{checked}");
}

/// x86 signatures use only fixed-width scalars, so every target projects
/// the same types; vectors are aligned to their size.
#[test]
fn m6_x86_signatures_project_identically_on_every_target() {
    let mut source = String::from("void f(void){\n");
    for descriptor in crate::builtins::test_descriptors() {
        if descriptor.type_rule == crate::builtins::TypeRule::X86Signature {
            writeln!(source, "(void)sizeof({});", descriptor.spelling).unwrap();
        }
    }
    source.push('}');
    let mut expected: Option<Vec<String>> = None;
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        with_configuration(
            &source,
            crate::configuration::CompilerConfiguration::default()
                .with_gnu_extensions(true)
                .with_target(target),
            |context, unit| {
                let mut types = Vec::new();
                for binding in unit.bindings {
                    let name = context.string_cache.at(binding.name.name);
                    if !name.starts_with("__builtin_ia32_") {
                        continue;
                    }
                    let TypeKind::Function {
                        result, parameters, ..
                    } = unit.types.nodes[binding.ty.index]
                    else {
                        panic!("{name}");
                    };
                    for &ty in std::iter::once(&result).chain(parameters) {
                        if let TypeKind::Vector { count, align, .. } = unit.types.nodes[ty.index] {
                            assert_eq!(unit.types.layout(ty).unwrap().size, align);
                            assert!(count.is_power_of_two());
                        }
                    }
                    types.push(format!(
                        "{name} {}",
                        builtin_names::spell(unit, context, binding.ty)
                    ));
                }
                assert_eq!(types.len(), 384);
                match &expected {
                    | Some(expected) => assert_eq!(&types, expected, "{target:?}"),
                    | None => expected = Some(types),
                }
            },
        );
    }
}
