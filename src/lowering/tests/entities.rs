//! M7 compares retained policy with the unchanged lowering derivations before
//! switching consumers. The shared test entry exercises all existing fixtures.
use super::*;
use crate::translation_phases::semantic_analysis::{
    BodyEmission,
    FunctionEnd,
    TypeKind,
};

pub(super) fn compare_legacy<'tu>(
    context: &Context<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    target: Target,
) {
    if !sema.lowerable(context) {
        return;
    }
    let tables = Bump::new();
    let arena = Bump::new();
    let Ok(mut unit) = Unit::new(context, sema, target, &arena, &tables) else {
        return;
    };
    let errors = unit.prepare_symbol_names();
    assert_eq!(errors.len(), sema.entities.refusals.len());
    for ((index, old), new) in errors.iter().zip(sema.entities.refusals) {
        assert_eq!(old.source, Some(new.source));
        assert_eq!(
            matches!(old.kind, LoweringErrorKind::MissingFact(_)),
            new.missing_occurrence
        );
        assert_eq!(sema.declarator_functions[*index], new.owner);
    }
    for (binding, data) in sema.bindings.iter().enumerate() {
        let fact = sema
            .entity(binding)
            .expect("every binding has entity policy");
        assert_eq!(unit.binding_identity(binding), fact.identity);
        assert!(fact.bindings.contains(&binding));
        let mut ty = data.ty;
        let old_readonly = loop {
            if ty.qualifiers.contains(
                crate::translation_phases::parsing::declaration_syntax::TypeQualifiers::CONST,
            ) {
                break !types::is_volatile(ty);
            }
            match sema.types.kind(ty) {
                | TypeKind::Array(element, _) => ty = element,
                | _ => break false,
            }
        };
        assert_eq!(old_readonly, sema.types.readonly_object(data.ty));
        let name = fact
            .symbol
            .map_or_else(|| context.string_cache.at(fact.abi_name), |r| r.name);
        assert_eq!(unit.symbol_name(binding), name);
        if let Some(request) = fact.symbol {
            assert!(sema.declarators.iter().any(|d| d.kind.iter().any(|direct| matches!(direct, crate::translation_phases::parsing::declaration_syntax::DirectDeclarator::AsmLabel(label) if label.source_vectors == request.source))));
        }
        let returns_twice = sema
            .entity_attributes(binding)
            .is_some_and(|a| a.returns_twice)
            || data.kind == crate::translation_phases::semantic_analysis::BindingKind::Function
                && data.linkage != crate::translation_phases::semantic_analysis::Linkage::None
                && fact.symbol.is_some()
                && crate::builtins::returns_twice(name);
        assert_eq!(fact.returns_twice, returns_twice);
        assert_eq!(
            unit.unprototyped_external(binding),
            sema.unprototyped_external(binding).unwrap()
        );
        let entity_id = sema.correspondence.entity_of(binding, *data);
        let definition = sema
            .definitions
            .iter()
            .filter(|d| {
                sema.correspondence
                    .entity_of(d.binding, sema.bindings[d.binding])
                    == entity_id
            })
            .last();
        assert_eq!(
            fact.definition.map(|d| (d.binding, d.kind)),
            definition.map(|d| (d.binding, d.kind))
        );
        if let Some(definition) = definition {
            assert_eq!(fact.ty, sema.bindings[definition.binding].ty);
            let initializer = sema
                .correspondence
                .declarations
                .iter()
                .filter(|d| d.binding == Some(definition.binding))
                .last()
                .and_then(|d| d.initializer);
            assert_eq!(
                fact.initializer.map(std::ptr::from_ref),
                initializer.map(std::ptr::from_ref)
            );
        }
        let own = sema
            .correspondence
            .declarations
            .iter()
            .filter(|d| d.entity == Some(sema.correspondence.entity_of(binding, *data)))
            .map(|d| d.id)
            .collect::<Vec<_>>();
        assert_eq!(fact.occurrences, own);
        if matches!(sema.types.kind(data.ty), TypeKind::Function { .. }) {
            assert_eq!(
                unit.private_inline(binding),
                sema.function_policy(binding)
                    .is_some_and(|policy| policy.body == BodyEmission::PrivateInline)
            );
        }
    }
    for function in sema.functions {
        if let Some(binding) = function.binding {
            let end = context.string_cache.at(sema.bindings[binding].name.name) == "main";
            assert_eq!(
                sema.function_policy(binding).unwrap().end == FunctionEnd::MainZero,
                end
            );
            assert_eq!(
                matches!(
                    sema.function_policy(binding).unwrap().body,
                    BodyEmission::InlineOnly | BodyEmission::PrivateInline
                ),
                sema.definitions.iter().any(|d| d.binding == binding
                    && d.kind
                        == crate::translation_phases::semantic_analysis::DefinitionKind::Inline)
            );
        }
    }
}

#[test]
fn m7_entity_policy_matches_legacy_on_linked_and_asm_fixtures() {
    let sources = SOURCES;
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for &source in sources {
            // Some fixtures intentionally end at the lowering refusal gate.
            drop(lower_for(source, target));
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
            compare_legacy(&context, &sema, target);
            writeln!(
                snapshot,
                "{target:?} case={case} errors={} refusals={}",
                sema.errors,
                sema.entities.refusals.len()
            )
            .unwrap();
            for (binding, data) in sema.bindings.iter().enumerate() {
                let entity = sema.entity(binding).unwrap();
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
