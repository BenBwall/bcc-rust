//! GNU statement-expression entry restrictions and permitted exits.
//! <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
use super::*;

#[test]
fn jumps_into_statement_expressions_are_semantic_errors() {
    for (source, expected) in [
        (
            "int main(void){goto a;return ({a:;0;});}",
            SemanticErrorKind::JumpIntoStatementExpression,
        ),
        (
            "int main(void){switch(0){return ({case 0:;0;});}}",
            SemanticErrorKind::SwitchIntoStatementExpression,
        ),
        (
            "int f(void){return 7;} int main(void){goto a;return f()+({a:;0;});}",
            SemanticErrorKind::JumpIntoStatementExpression,
        ),
        (
            "int main(void){switch(0){return ({default:;0;});}}",
            SemanticErrorKind::SwitchIntoStatementExpression,
        ),
        (
            "int main(void){({goto a;0;});return ({a:;0;});}",
            SemanticErrorKind::JumpIntoStatementExpression,
        ),
        (
            "int main(void){return ({goto a;({a:;0;});0;});}",
            SemanticErrorKind::JumpIntoStatementExpression,
        ),
    ] {
        with_source(source, |context, unit| {
            assert!(!unit.lowerable(context));
            let errors = context.take_pending_errors();
            assert!(errors.iter().any(|e| matches!(e, TranslationError::Semantic(SemanticError {kind,..}) if *kind==expected)), "{source}: {errors:?}");
        });
    }
}

#[test]
fn internal_and_outgoing_statement_expression_jumps_are_legal() {
    for source in [
        "int main(void){int i=0;int n=({goto a;i=100;a:i++;i;});return n!=1;}",
        "int main(void){int i=0;({i++;goto out;i=100;});return 1;out:return i!=1;}",
        "int main(void){return ({({goto out;0;});out:;0;});}",
        "int main(void){return ({int i=0;switch(0){case 0:i=7;break;}i!=7;});}",
    ] {
        with_source(source, |context, unit| {
            assert_eq!(
                unit.errors,
                0,
                "{source}: {:?}",
                context.take_pending_errors()
            );
        });
    }
}
