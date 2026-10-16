//! Hosted prototype inference and explicit GNU library aliases.
//! C99: §5.1.2.1p1, p. 11; PDF p. 23.
use super::*;
use crate::configuration::{
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn implicit_library_prototypes_require_hosted_mode() {
    for hosted in [false, true] {
        let configuration =
            CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Allow).with_hosted(hosted);
        with_configuration(
            "int f(void){return strlen(7);} int strlen(int x){return x+1;}",
            configuration,
            |context, unit| {
                let binding = unit
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == "strlen")
                    .unwrap();
                let TypeKind::Function {
                    result,
                    parameters,
                    prototype,
                    ..
                } = unit.types.nodes[binding.ty.index]
                else {
                    panic!("function declaration");
                };
                assert_eq!(prototype, hosted);
                assert_eq!(parameters.len(), usize::from(hosted));
                assert_eq!(
                    unit.types.nodes[result.index],
                    TypeKind::Scalar(if hosted {
                        unit.types.target.size_t
                    } else {
                        Scalar::Int
                    })
                );
                let errors = context.take_pending_errors();
                assert_eq!(unit.errors > 0, hosted, "{errors:?}");
                assert_eq!(
                    errors.iter().any(|e| matches!(
                        e,
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::ImplicitLibraryFunction(_),
                            ..
                        })
                    )),
                    hosted
                );
                assert_eq!(
                    errors.iter().any(|e| matches!(
                        e,
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::PointerConstraint(
                                crate::configuration::DiagnosticGroup::IntConversion
                            ),
                            ..
                        })
                    )),
                    hosted
                );
            },
        );
        // Explicit reserved aliases still exist under -ffreestanding, as in
        // GCC Library Builtins; only implicit ordinary names need hosting.
        with_configuration(
            "int f(void){return __builtin_strlen(7);}",
            configuration,
            |context, unit| {
                assert!(unit.errors > 0, "{:?}", context.take_pending_errors());
            },
        );
    }
}
