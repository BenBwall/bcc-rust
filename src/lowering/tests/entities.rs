//! M7 retained policy snapshot, checked against the legacy lowering before
//! switching consumers; fixtures keep the symbol and call capability gates.
use super::*;

#[test]
fn m7_linked_and_asm_policy_fixtures_keep_lowering() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for (case, &source) in SOURCES.iter().enumerate() {
            let lowered = lower_for(source, target);
            if case == 7 {
                let errors = lowered.unwrap_err();
                assert!(
                    errors.iter().all(|error| error.contains("assembler label")),
                    "{errors:?}"
                );
            } else {
                drop(lowered.unwrap_or_else(|errors| panic!("case={case} {target:?}: {errors:?}")));
            }
        }
    }
}

const SOURCES: &[&str] = &[
    "int f(); int g(void){return f(3);} int f(int x){return x;}",
    "int f(); int g(void){return f(3);}",
    "int f(void) __asm__(\"g\"); int f(void){return 42;} int main(void){return f();}",
    "extern int a[] __asm__(\"b\"); int *p=a; int b[1]={42}; int main(void){return p[0];}",
    "int main(void){static int a __asm__(\"b\")=42; return a;}",
    "int f(void) __asm__(\"main\"); int f(void){}",
    "int main(void) __asm__(\"other\"); int main(void){}",
    "int f(void) __asm__(\"\\377\"); int f(void){}",
    "extern inline __attribute__((always_inline,gnu_inline)) int f(void){return 7;} int \
     g(void){return f();}",
    "int target(void){return 7;} int a(void) __attribute__((alias(\"target\")));",
    "int f(int) __attribute__((overloadable)); int f(double) __attribute__((overloadable)); int \
     g(void){return f(2);}",
    "const int a[2]={1,2}; const volatile int b=3; struct S{const int x;}; struct S c={4};",
    "int main(void){}",
    "extern inline __attribute__((gnu_inline)) int f(void){return 1;} int f(void){return 42;} int \
     main(void){return f();}",
];

#[test]
fn m7_retained_policy_snapshot() {
    use std::fmt::Write;
    let mut snapshot = String::new();
    writeln!(
        snapshot,
        "record bytes: entity={} function={} symbol={} convention={}",
        size_of::<crate::translation_phases::semantic_analysis::Entity<'_>>(),
        size_of::<crate::translation_phases::semantic_analysis::FunctionPolicy>(),
        size_of::<crate::translation_phases::semantic_analysis::SymbolRequest<'_>>(),
        size_of::<crate::translation_phases::semantic_analysis::ConventionFacts>()
    )
    .unwrap();
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for (case, &source) in SOURCES.iter().enumerate() {
            let tu = Bump::new();
            let mut context = Context::with_configuration(
                &tu,
                crate::configuration::CompilerConfiguration::default().with_target(target),
            );
            let syntax = parse_translation_unit(
                &mut context,
                Path::new("<m7>"),
                tu.alloc_str(&format!("{source}\n")),
                HeaderSearch::default(),
            );
            let sema = analyze_translation_unit(&mut context, &syntax);
            writeln!(
                snapshot,
                "{target:?} case={case} errors={} refusals={}",
                sema.errors,
                sema.entities.refusals.len()
            )
            .unwrap();
            for (binding, data) in sema.bindings.iter().enumerate() {
                let Some(entity) = sema.entity(binding) else {
                    assert!(!data.needs_entity_policy());
                    continue;
                };
                if let Some(request) = entity.symbol {
                    assert_ne!(context.get_source_vectors(request.source), []);
                }
                writeln!(
                    snapshot,
                    "  {binding}: {} identity={} abi={} asm={:?} definition={:?} function={:?} \
                     body={:?} end={:?} open={} twice={} readonly={} occurrences={:?}",
                    context.string_cache.at(data.name.name),
                    context.string_cache.at(entity.identity),
                    context.string_cache.at(entity.abi_name),
                    entity.symbol.map(|r| r.name),
                    entity.definition.map(|d| (d.binding, d.kind)),
                    entity.function,
                    entity.body,
                    entity.end,
                    sema.unprototyped_external(binding).unwrap(),
                    entity.returns_twice,
                    sema.types.readonly_object(data.ty),
                    entity.occurrences
                )
                .unwrap();
            }
        }
    }
    crate::test_support::assert_snapshot("tests/fixtures/semantic/entity-policy.snap", &snapshot);
}

#[test]
fn m7_convention_occurrences_keep_the_existing_refusal_provenance() {
    use crate::translation_phases::semantic_analysis::{
        CallingConvention as C,
        UnsupportedAttribute,
    };
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for attribute in [
            "cdecl",
            "stdcall",
            "pascal",
            "ms_abi",
            "sysv_abi",
            "vectorcall",
            "preserve_all",
        ] {
            let tu = Bump::new();
            let source = tu.alloc_str(&format!(
                "int f(int) __attribute__(({attribute})); int g(void){{return ((int \
                 (__attribute__(({attribute})) *)(int))0)(2);}}\n"
            ));
            let mut context = Context::with_configuration(
                &tu,
                crate::configuration::CompilerConfiguration::default().with_target(target),
            );
            let syntax = parse_translation_unit(
                &mut context,
                Path::new("<convention>"),
                source,
                HeaderSearch::default(),
            );
            let sema = analyze_translation_unit(&mut context, &syntax);
            assert!(sema.lowerable(&context), "{attribute} {target:?}");
            assert_eq!(sema.symbols.conventions.len(), 2, "{attribute} {target:?}");
            for convention in sema.symbols.conventions {
                let refusal = sema.symbols.refusals.iter().find(|r| {
                    r.source == convention.source
                        && r.capability == UnsupportedAttribute::CallingConvention
                });
                assert_eq!(
                    refusal.is_some(),
                    convention.convention == C::Unsupported,
                    "{attribute} {target:?}"
                );
                if let Some(refusal) = refusal {
                    assert_eq!(refusal.owner, convention.owner);
                }
            }
        }
    }
}

#[test]
fn m7_object_predicates_distinguish_readonly_volatile_and_vm() {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<objects>"),
        tu.alloc_str(
            "const int a[2]={1,2}; const volatile int b=3; struct S{const int x;}; struct S \
             c={4}; void f(int n){typedef int Row[n]; Row v; int (*p)[n]; volatile int w;}\n",
        ),
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    assert!(sema.lowerable(&context));
    for (name, readonly, volatile, vm) in [
        ("a", true, false, false),
        ("b", false, true, false),
        ("c", false, false, false),
        ("Row", false, false, true),
        ("v", false, false, true),
        ("p", false, false, true),
        ("w", false, true, false),
    ] {
        let data = sema
            .bindings
            .iter()
            .find(|b| context.string_cache.at(b.name.name) == name)
            .unwrap();
        assert_eq!(sema.types.readonly_object(data.ty), readonly, "{name}");
        assert_eq!(data.ty.is_volatile(), volatile, "{name}");
        assert_eq!(sema.types.variably_modified(data.ty), vm, "{name}");
    }
}

#[test]
fn m7_missing_entity_and_function_policy_fail_with_provenance() {
    for remove_function in [false, true] {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let syntax = parse_translation_unit(
            &mut context,
            Path::new("<m7-missing>"),
            tu.alloc_str("int main(void){}\n"),
            HeaderSearch::default(),
        );
        let mut sema = analyze_translation_unit(&mut context, &syntax);
        assert!(sema.lowerable(&context));
        let function = &sema.functions[0];
        let binding = function.binding.unwrap();
        let source = if remove_function {
            sema.entities.remove_function(binding);
            function.syntax.source_vectors
        } else {
            let id = sema
                .correspondence
                .entity_of(binding, sema.bindings[binding]);
            sema.entities.remove_entity(id);
            sema.bindings[binding].name.source_vectors
        };
        let arena = Bump::new();
        let mut scratch = Bump::new();
        let errors = lower_translation_unit(
            &context,
            &syntax,
            &sema,
            Target::LinuxGnu,
            &arena,
            &mut scratch,
        )
        .unwrap_err();
        assert!(errors.errors.iter().any(|error| matches!(
            error.kind,
            LoweringErrorKind::MissingFact(_)
        ) && error.source == Some(source)));
    }
}

#[test]
fn m7_main_end_policy_follows_c_identifier_across_asm_renaming() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let renamed = lower_for(
            "int main(void) __asm__(\"other\"); int main(void){}",
            target,
        )
        .unwrap();
        assert!(renamed.contains("function @other()"), "{renamed}");
        assert!(renamed.contains("iconst.i32 0"), "{renamed}");
        let alias = lower_for("int f(void) __asm__(\"main\"); int f(void){}", target).unwrap();
        assert!(alias.contains("function @main()"), "{alias}");
        assert!(alias.contains("poison.i32"), "{alias}");
    }
}

#[test]
fn m7_nested_function_does_not_close_unrelated_external_signature() {
    // C99 §6.5.2.2p6, pp. 71-72; PDF pp. 83-84: default promotions
    // apply to the external f(), independently of the GNU nested definition.
    let ir = lowered(
        "int f(); int g(void){return f(42);} inline int outer(void){int f(int x){return x;} \
         return f(1);}",
    );
    assert!(ir.contains("function @f(..."), "{ir}");
}

#[test]
fn m7_automatic_objects_do_not_retain_entity_records() {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<sparse-entities>"),
        tu.alloc_str(
            "int global; int f(int p){int local=p; static int s; return local+s+__func__[0];}\n",
        ),
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    assert!(sema.lowerable(&context));
    for (index, data) in sema.bindings.iter().enumerate() {
        let name = context.string_cache.at(data.name.name);
        if matches!(name, "p" | "local") {
            assert!(sema.entity(index).is_none(), "{name}");
        } else if matches!(name, "global" | "f" | "s" | "__func__") {
            assert!(sema.entity(index).is_some(), "{name}");
        }
    }
}

#[test]
fn m7_review_redeclaration_fixtures_keep_lowering() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for (case, source) in [
            "int main(void){return f(42);} int f(int x){return x;}",
            "int main(void){extern int f(); return f(42);} int f(x) int x; {return x;}",
            "int a=42; int a; int main(void){return a;}",
            "extern int __r(int) __asm__(\"myread\"); extern inline \
             __attribute__((always_inline,gnu_inline)) int myread(int x){return __r(x)+1;} int \
             main(void){return myread(20);} int myread(int x){return x*2;}",
        ]
        .into_iter()
        .enumerate()
        {
            let config = crate::configuration::CompilerConfiguration::default().with_target(target);
            let config = if case == 0 {
                crate::configuration::CompilerConfiguration::new(
                    crate::configuration::CStandard::C89,
                    crate::configuration::ExtensionPolicy::Allow,
                )
                .with_target(target)
            } else {
                config
            };
            drop(
                lower_with(source, config)
                    .unwrap_or_else(|errors| panic!("case={case} {target:?}: {errors:?}")),
            );
        }
    }
}
