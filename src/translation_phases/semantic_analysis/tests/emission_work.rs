//! Growing-input guards for query scans and condition-reference omission.

use super::*;

#[test]
fn nested_query_scans_have_linear_work() {
    crate::test_support::on_compilation_stack(|| {
        for depth in [64, 1024, 8192] {
            let source = format!(
                "int g;int main(void){{return {}g{};}}",
                "__builtin_constant_p(".repeat(depth),
                "+1)".repeat(depth)
            );
            with_analyzer(&source, |analyzer| {
                // Exercise the cached syntax walk even when a known query ICE
                // lets value classification bypass it. Every enclosing operand
                // shares descendants with the preceding query.
                for i in 0..analyzer.expressions.len() {
                    if let ExpressionType::Call { arguments, .. } =
                        analyzer.expressions[i].expression.kind
                        && let Some(&operand) = arguments.first()
                    {
                        _ = analyzer.reads_array_literal(operand);
                    }
                }
                let steps = analyzer.functions.emission.array_scan_steps;
                let expressions = analyzer.expressions.len();
                assert!(steps >= depth);
                assert!(
                    steps <= 5 * expressions,
                    "{steps} scans for {expressions} expressions"
                );
            });
        }
    });
}

#[test]
fn nested_condition_reference_elision_has_linear_work() {
    crate::test_support::on_compilation_stack(|| {
        for depth in [64, 1024, 8192] {
            let source = format!(
                "static int h(void){{return 1;}}int main(void){{return {}0{};}}",
                "(".repeat(depth),
                "&&h())".repeat(depth)
            );
            with_analyzer(&source, |analyzer| {
                let steps = analyzer.functions.condition_reference_steps;
                let uses = analyzer.reference_count();
                assert!(steps > 0, "the input must exercise reference elision");
                assert!(
                    steps <= 16 * uses,
                    "{steps} reference steps for {uses} uses"
                );
            });
        }
    });
}

fn with_analyzer(source: &str, run: impl FnOnce(&mut Analyzer<'_, '_, '_>)) {
    let tu = Bump::new();
    let scratch = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = crate::pipeline::parse_translation_unit(
        &mut context,
        Path::new("<input>"),
        tu.alloc_str(source),
        crate::headers::HeaderSearch::default(),
    );
    let mut analyzer = Analyzer::new(&mut context, &scratch);
    for &root in syntax.external_declarations().iter().rev() {
        analyzer.work.push(Work::Root(root));
    }
    while let Some(work) = analyzer.work.pop() {
        analyzer.step(work);
    }
    analyzer.finish_translation_unit();
    assert_eq!(
        analyzer.context.error_count(),
        0,
        "{:?}",
        analyzer.context.take_pending_errors()
    );
    run(&mut analyzer);
}
