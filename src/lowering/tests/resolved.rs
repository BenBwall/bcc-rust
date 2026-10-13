//! Missing M3 facts are internal errors with source provenance.
use super::*;

#[test]
fn m3_missing_designations_calls_members_and_tails_report_provenance() {
    for family in 0..5 {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let syntax = parse_translation_unit(
            &mut context,
            Path::new("<m3>"),
            tu.alloc_str(
                "int f(int x){return x;}struct S{int x;};int main(void){int n=3;int \
                 *p=&(n);struct S s={4};return f(s.x)+({L:*p;});}",
            ),
            HeaderSearch::default(),
        );
        let mut sema = analyze_translation_unit(&mut context, &syntax);
        assert!(sema.lowerable(&context));
        match family {
            | 0 => sema.resolved.designations = &[],
            | 1 => sema.resolved.calls.clear(),
            | 2 => sema.resolved.members.clear(),
            | 3 => sema.resolved.statement_expressions.clear(),
            | _ =>
                for (_, call) in &mut sema.resolved.calls {
                    call.arguments = &[];
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
        assert!(
            errors.errors.iter().any(|error| matches!(
                error.kind,
                LoweringErrorKind::MissingFact(_)
            ) && error.source.is_some()),
            "family {family}: {errors:?}"
        );
    }
}

#[test]
fn m3_missing_static_member_facts_report_provenance() {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<m3>"),
        tu.alloc_str("struct S{int x;};struct S s;int *p=&s.x;int main(void){return 0;}"),
        HeaderSearch::default(),
    );
    let mut sema = analyze_translation_unit(&mut context, &syntax);
    assert!(sema.lowerable(&context));
    sema.resolved.members.clear();
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
