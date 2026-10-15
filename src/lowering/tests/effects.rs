//! Effects-only lowering fails with provenance when a recipe is missing.
use super::*;

#[test]
fn m5_missing_effect_summaries_and_recipes_report_provenance() {
    for remove_summaries in [false, true] {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let syntax = parse_translation_unit(
            &mut context,
            Path::new("<m5>"),
            tu.alloc_str(
                "int f(void);int main(void){int n=0;return __builtin_add_overflow_p(1,2,n&&f());}",
            ),
            HeaderSearch::default(),
        );
        let mut sema = analyze_translation_unit(&mut context, &syntax);
        assert!(sema.lowerable(&context));
        if remove_summaries {
            sema.effects.summaries = &[];
        } else {
            sema.effects.recipes.clear();
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
        assert!(
            errors.errors.iter().any(|error| matches!(
                error.kind,
                LoweringErrorKind::MissingFact(_)
            ) && error.source.is_some()),
            "{errors:?}"
        );
    }
}
