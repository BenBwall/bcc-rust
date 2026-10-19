//! Required checked instances and normalized operands fail with provenance.
use super::*;

#[test]
fn m4_missing_checked_instances_and_operands_report_provenance() {
    for source in [
        "int main(void){int n=1;return __builtin_clz(n);}",
        "int f(int n,...){__builtin_va_list a;__builtin_va_start(a,n);int \
         r=__builtin_va_arg(a,int);__builtin_va_end(a);return r;}int main(void){return f(1,3)!=3;}",
    ] {
        for remove_operands in [false, true] {
            let tu = Bump::new();
            let mut context = Context::new(&tu);
            let syntax = parse_translation_unit(
                &mut context,
                Path::new("<m4>"),
                tu.alloc_str(source),
                HeaderSearch::default(),
            );
            let mut sema = analyze_translation_unit(&mut context, &syntax);
            assert!(sema.lowerable(&context));
            for (_, call) in &mut sema.resolved.calls {
                if let crate::translation_phases::semantic_analysis::Callee::Builtin {
                    instance,
                    ..
                } = &mut call.callee
                {
                    if source.contains("__builtin_va_arg")
                        && !instance.is_some_and(|facts| {
                            facts.descriptor == crate::builtins::BuiltinId::VaArg
                        })
                    {
                        continue;
                    }
                    if remove_operands {
                        if let Some(facts) = *instance {
                            *instance = Some(tu.alloc(
                                crate::translation_phases::semantic_analysis::BuiltinCallFacts {
                                    operands: &[],
                                    ..*facts
                                },
                            ));
                        }
                    } else {
                        *instance = None;
                    }
                }
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
}
