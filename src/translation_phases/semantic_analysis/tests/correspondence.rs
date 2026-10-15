//! M2 occurrence, parameter, bound and size-query retained-interface controls.
//! C99: §6.7.5.2p5, p. 117; PDF p. 129; §6.9.1p10, p. 142;
//! PDF p. 154; §6.5.3.4p2, p. 80; PDF p. 92.

use super::*;
use crate::{
    target::Target,
    translation_phases::parsing::syntax::StatementType,
};

#[test]
fn m2_parameters_retain_prototype_body_and_promoted_types_on_four_targets() {
    let source = "int prototype(int, int a[static 3]);
        int f(register int n, int a[n]) { { int n=7; (void)n; } return n; }
        int u(int, int named) { return named; }
        int minimum(int a[static 3]) { return a[0]; }
        int old(c,r) char c; float r; { return c+(int)r; }";
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        with_configuration(
            source,
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                let prototype = sema.correspondence.declarations[0].parameter_facts;
                assert_eq!(prototype.len(), 2);
                assert!(prototype[0].prototype_binding.is_none());
                assert!(prototype[1].prototype_binding.is_some());
                assert!(
                    prototype
                        .iter()
                        .all(|parameter| parameter.body_binding.is_none())
                );
                let f = sema.functions[0];
                assert_eq!(f.parameter_facts.len(), 2);
                assert_eq!(f.parameter_aliases.len(), 2);
                for (ordinal, facts) in f.parameter_facts.iter().enumerate() {
                    let body = facts.body_binding.unwrap();
                    let prototype = facts.prototype_binding.unwrap();
                    assert_ne!(prototype, body);
                    assert_eq!(f.parameter_aliases[ordinal], (prototype, body));
                    assert_eq!(facts.declared, sema.bindings[body].ty);
                    assert_eq!(facts.promoted, facts.declared.unqualified());
                    assert_eq!(facts.original, facts.site.unwrap().ty);
                }
                assert_eq!(f.parameter_facts[1].site.unwrap().bounds.len(), 1);
                assert!(
                    sema.runtime_bounds
                        .iter()
                        .all(|bound| bound.conversion == sema.types.target.size_t)
                );
                assert!(matches!(
                    sema.types.kind(f.parameter_facts[1].original),
                    TypeKind::Array(_, ArrayBound::Variable(_))
                ));
                assert!(sema.functions[1].parameter_facts[0].body_binding.is_none());
                assert!(
                    sema.functions[1].parameter_facts[0]
                        .prototype_binding
                        .is_none()
                );
                let old = sema.functions[3].parameter_facts;
                assert!(old.iter().all(|p| p.prototype_binding.is_none()));
                assert!(matches!(
                    sema.types.kind(old[0].declared),
                    TypeKind::Scalar(Scalar::Char)
                ));
                assert!(matches!(
                    sema.types.kind(old[0].promoted),
                    TypeKind::Scalar(Scalar::Int)
                ));
                assert!(matches!(
                    sema.types.kind(old[1].declared),
                    TypeKind::Scalar(Scalar::Float)
                ));
                assert!(matches!(
                    sema.types.kind(old[1].promoted),
                    TypeKind::Scalar(Scalar::Double)
                ));
                assert_eq!(
                    sema.correspondence
                        .parameter_minimums
                        .get(&sema.functions[2].parameter_facts[0].body_binding.unwrap()),
                    Some(&ArrayBound::Constant(3))
                );
            },
        );
    }
}

#[test]
fn m2_copied_declarators_keep_occurrence_identity_and_initializer_association() {
    with_source(
        "int x=1; int f(void){int x=2; {int x=3; (void)x;} return x;}",
        |context, sema| {
            assert_eq!(context.error_count(), 0);
            let StatementType::Compound { items } = sema.functions[0].syntax.body.kind else {
                panic!()
            };
            let BlockItem::Declaration(declaration) = items[0] else {
                panic!()
            };
            let init = declaration.init_declarators[0];
            let facts = *sema.correspondence.declaration(init.declarator).unwrap();
            let mut copied = init.declarator;
            copied.source_vectors = sema.functions[0].syntax.source_vectors;
            assert_eq!(
                sema.correspondence.declaration(copied).unwrap().id,
                facts.id
            );
            assert!(std::ptr::eq(
                facts.initializer.unwrap(),
                init.initializer.unwrap()
            ));
            let occurrences: Vec<_> = sema
                .correspondence
                .declarations
                .iter()
                .filter(|fact| {
                    fact.binding.is_some_and(|binding| {
                        context.string_cache.at(sema.bindings[binding].name.name) == "x"
                    })
                })
                .collect();
            assert_eq!(occurrences.len(), 3);
            assert_ne!(occurrences[0].entity, occurrences[1].entity);
            assert_ne!(occurrences[1].entity, occurrences[2].entity);
        },
    );
}

#[test]
fn m2_bounds_and_size_queries_share_saved_extent_formulas() {
    with_source(
        "int f(int n){typedef int A[n++]; A a; int b[sizeof a/sizeof(int)];
        int (*p)[n]=(int (*)[n])0; (void)p;
        return sizeof(A)+sizeof a+sizeof b+sizeof(int[n++])+sizeof(n++);}",
        |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let bounds = sema.runtime_bounds;
            assert!(
                bounds
                    .iter()
                    .all(|bound| bound.site != BoundEvaluationSite::Unevaluated)
            );
            assert_eq!(bounds[1].dependencies, [BoundId(0)]);
            let queries: Vec<_> = sema
                .expressions
                .iter()
                .filter_map(|info| sema.size_query_site(info.expression))
                .collect();
            assert_eq!(queries.len(), 7);
            assert_eq!(queries[0].formula, queries[3].formula);
            assert_eq!(queries[2].formula, queries[3].formula);
            assert_ne!(queries[3].formula, queries[4].formula);
            let constant = queries.last().unwrap();
            assert!(constant.operand.is_none());
            assert_eq!(constant.site.bounds.len(), 0);
            for fact in sema.type_names {
                assert_eq!(sema.type_name_fact(fact.syntax).unwrap().id, fact.id);
                assert_eq!(fact.size_query.unwrap().site.ty, fact.resolved);
            }
            assert!(!std::mem::needs_drop::<RuntimeBound<'_>>());
            assert!(!std::mem::needs_drop::<ParameterFacts<'_>>());
        },
    );
}

#[test]
fn m2_rejected_declaration_keeps_an_occurrence_without_an_accepted_binding() {
    with_source("int x; double x; int y;", |context, sema| {
        assert!(context.error_count() > 0);
        assert_eq!(sema.correspondence.declarations.len(), 3);
        assert!(sema.correspondence.declarations[0].binding.is_some());
        assert!(sema.correspondence.declarations[1].binding.is_none());
        assert!(sema.correspondence.declarations[1].entity.is_none());
        assert!(sema.correspondence.declarations[2].binding.is_some());
    });
    with_source(
        "int f(int); double f(int x){return x;} int g(void){return 0;}",
        |context, sema| {
            assert!(context.error_count() > 0);
            assert_eq!(sema.correspondence.declarations.len(), 3);
            assert!(sema.correspondence.declarations[0].binding.is_some());
            assert!(sema.correspondence.declarations[1].binding.is_none());
            assert!(sema.correspondence.declarations[1].entity.is_none());
            assert!(sema.correspondence.declarations[2].binding.is_some());
        },
    );
}

#[test]
fn m2_bound_dependencies_include_statement_expression_size_and_stride_uses() {
    with_source(
        "int f(int n){int a[n];int (*p)[n]=&a;int b[({int x=sizeof a; x;})];int c[(p+1)-p];return \
         sizeof b+sizeof c;}",
        |context, sema| {
            assert_eq!(context.error_count(), 0);
            assert_eq!(sema.runtime_bounds[2].dependencies, [BoundId(0)]);
            assert_eq!(sema.runtime_bounds[3].dependencies, [BoundId(1)]);
        },
    );
}
