//! Missing M2 facts are internal errors with the written node's provenance.
use super::*;

#[test]
fn m2_missing_occurrences_bounds_and_formulas_fail_with_provenance() {
    for family in 0..5 {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let syntax = parse_translation_unit(
            &mut context,
            Path::new("<m2>"),
            tu.alloc_str(
                "int global=7;int f(int unused){return 0;}int main(void){int n=2,a[n];return \
                 sizeof a!=8;}",
            ),
            HeaderSearch::default(),
        );
        let mut sema = analyze_translation_unit(&mut context, &syntax);
        assert!(sema.lowerable(&context));
        match family {
            | 0 => sema.correspondence.declarations = &[],
            | 1 => sema.runtime_bounds = &[],
            | 2 => sema.sizes.formulas = &[],
            | 3 => {
                let binding = sema.correspondence.declarations[0].binding.unwrap();
                let id = sema
                    .correspondence
                    .entity_of(binding, sema.bindings[binding]);
                sema.entities.clear_initializer(id);
            },
            | _ => {
                let functions = tu.alloc_slice_copy(sema.functions);
                functions[0].parameter_facts = &[];
                sema.functions = functions;
            },
        }
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
        ) && error.source.is_some()));
    }
}
