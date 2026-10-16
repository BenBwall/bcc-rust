//! Clang's `-Winitializer-overrides` (#68): an initializer for a subobject
//! that an earlier one in the same initializer already initialized, as a
//! whole, in part, or as another member of a union (C99 §6.7.8p19).

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    DiagnosticGroup,
    WarningOption,
};

/// A diagnostic's kind, line and column, and those of the source it
/// refers to.
type Positioned = (SemanticErrorKind, (u32, u32), Option<(u32, u32)>);

const SOURCE: &str = r#"struct P { int x, y; };
struct G { int a; int b[3]; struct P p; };
union U { int i; float f; struct P p; };
struct P q;
struct G g1 = { .b = {1}, .b[2] = 5 };
struct G g2 = { .b[0] = 1, .b = {4} };
struct G g3 = { .p = {1,2}, .p.x = 3 };
void f(void) { struct G g4 = { .p = q, .p.x = 3 }; struct G g5 = { .p.x = 1, .p = q }; (void)g4; (void)g5; }
union U u1 = { .i = 1, .f = 2 };
union U u2 = { .p.x = 1, .p.y = 2 };
union U u3 = { .p.x = 1, .i = 2 };
int a1[3] = { [1] = 1, 2, [2] = 3 };
struct G g6 = { .b = {1,2,3}, .b = {} };
struct G g7 = { .b = {}, .b = {1} };
struct G g8 = { 1, {2}, .b[1] = 3, .b[1] = 4 };
char s1[4] = { [0] = 'a', [0 ... 2] = 'b' };
struct S2 { char t[4]; } s2 = { .t = "ab", .t[1] = 'c' };
struct S2 s3 = { .t[1] = 'c', .t = "ab" };
struct G g9 = { .p.x = 1, .p = {2} };
struct G g10 = { .b = {1,2,3}, .a = 1, 4 };
int a2[2][2] = { [0][1] = 1, { 2 }, [0] = { 3 } };
union U u4 = { .p.x = 1, .i = 2, .p.y = 3 };
int a3[8] = { [2 ... 5] = 1, [3] = 2, [7] = 3 };"#;

/// Each diagnostic of `source` under `configuration`: its kind, line and
/// column, and the line and column of the earlier initializer it names.
fn overrides(source: &str, configuration: CompilerConfiguration) -> Vec<Positioned> {
    let mut result = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            let error = match error {
                | TranslationError::Semantic(error) => error,
                // GNU flexible array initialization under the default policy.
                | TranslationError::Extension(_) => continue,
                | error => panic!("unexpected {error:?}"),
            };
            let at = |source: SourceVectors| {
                let position = crate::translation_phases::GetPosition::position(&source, context);
                (position.line, position.column)
            };
            result.push((error.kind, at(error.source_vectors), error.previous.map(at)));
        }
    });
    result
}

#[test]
fn overridden_initializers_warn_where_clang_does() {
    use SemanticErrorKind::{
        InitializerOverride as Whole,
        PartialInitializerOverride as Partial,
    };
    // Every position is Clang 23's for the same source, warning and note.
    assert_eq!(
        overrides(SOURCE, CompilerConfiguration::default()),
        [
            (Whole, (6, 33), Some((6, 19))),
            (Whole, (7, 36), Some((7, 23))),
            (Partial, (8, 42), Some((8, 37))),
            (Whole, (8, 83), Some((8, 70))),
            (Whole, (9, 24), Some((9, 21))),
            (Whole, (11, 26), Some((11, 18))),
            (Whole, (12, 33), Some((12, 24))),
            (Whole, (13, 36), Some((13, 22))),
            (Whole, (14, 31), Some((14, 22))),
            (Whole, (15, 44), Some((15, 33))),
            (Whole, (16, 39), Some((16, 22))),
            (Partial, (17, 46), Some((17, 38))),
            (Whole, (18, 36), Some((18, 20))),
            (Whole, (19, 32), Some((19, 19))),
            (Whole, (20, 40), Some((20, 24))),
            (Whole, (21, 43), Some((21, 21))),
            (Whole, (22, 26), Some((22, 18))),
            (Whole, (22, 34), Some((22, 31))),
            (Whole, (23, 36), Some((23, 27))),
        ]
    );
}

#[test]
fn initializer_overrides_follow_their_warning_group() {
    let silenced = CompilerConfiguration::default().with_warning_option(
        DiagnosticGroup::InitializerOverrides,
        WarningOption::Enable(false),
    );
    assert_eq!(overrides(SOURCE, silenced), []);
    // An error by -Werror=initializer-overrides, a warning still counts
    // for nothing else: the analysis of the initializers is unchanged.
    let errors = CompilerConfiguration::default().with_warning_option(
        DiagnosticGroup::InitializerOverrides,
        WarningOption::Error(true),
    );
    with_configuration(
        "struct G { int a, b; } g = { .a = 1, .a = 2 };",
        errors,
        |context, _| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1);
            assert_eq!(
                crate::translation_phases::GetSeverity::severity(&errors[0]),
                crate::translation_phases::ErrorSeverity::Error,
                "{errors:?}"
            );
        },
    );
}

#[test]
fn a_long_range_records_its_last_element_without_inventing_warnings() {
    // A range longer than the recorded limit writes only its last element:
    // the override of element 70000 is reported as Clang reports it, that
    // of element 3 inside the range (Clang: 1:44, note 1:25) is not, and
    // nothing is reported where Clang reports nothing.
    let source = "int a[100000] = { [3] = 1, [0 ... 70000] = 2, [70000] = 3, [80000] = 4 };";
    assert_eq!(
        overrides(source, CompilerConfiguration::default()),
        [(
            SemanticErrorKind::InitializerOverride,
            (1, 57),
            Some((1, 44))
        )]
    );
}

#[test]
fn a_whole_expression_stays_partly_overridden_by_every_later_write() {
    use SemanticErrorKind::{
        InitializerOverride as Whole,
        PartialInitializerOverride as Partial,
    };
    // Clang 23 reports each write inside `p` at its designator, noting the
    // expression `p` each time, and the repeated `.p.x` also overrides `3`.
    let source = "struct P { int x, y; };
struct S { struct P p; };
void f(struct P p) {
  struct S s = { .p = p, .p.x = 3, .p.y = 4 };
  struct S t = { .p = p, .p.x = 3, .p.x = 4, .p.y = 5 };
  (void)s; (void)t;
}";
    assert_eq!(
        overrides(source, CompilerConfiguration::default()),
        [
            (Partial, (4, 28), Some((4, 23))),
            (Partial, (4, 38), Some((4, 23))),
            (Partial, (5, 28), Some((5, 23))),
            (Partial, (5, 38), Some((5, 23))),
            (Whole, (5, 43), Some((5, 33))),
            (Partial, (5, 48), Some((5, 23))),
        ]
    );
}

#[test]
fn every_element_of_a_range_keeps_what_its_initializer_wrote() {
    use SemanticErrorKind::{
        InitializerOverride as Whole,
        PartialInitializerOverride as Partial,
    };
    // Clang 23: a braced range initializer writes `x` and `y` of every
    // element, also in a flexible array member, and a range expression
    // covers every element whole. Element 1 keeps its own history after
    // element 0 is written again.
    let source = "struct P { int x, y; };
struct P a[2] = { [0 ... 1] = { 1, 2 }, [0].x = 3, [0].y = 4 };
struct F { int n; struct P v[]; };
struct F b = { .v = { [0 ... 1] = { 1, 2 }, [0].x = 3, [0].y = 4 } };
void f(struct P p) {
  struct P c[2] = { [0 ... 1] = p, [0].x = 3, [0].y = 4 };
  (void)c;
}
struct P d[3] = { [0 ... 2] = { 1, 2 }, [0].x = 3, [2].x = 5 };";
    assert_eq!(
        overrides(source, CompilerConfiguration::default()),
        [
            (Whole, (2, 49), Some((2, 33))),
            (Whole, (2, 60), Some((2, 36))),
            (Whole, (4, 53), Some((4, 37))),
            (Whole, (4, 64), Some((4, 40))),
            (Partial, (6, 39), Some((6, 33))),
            (Partial, (6, 50), Some((6, 33))),
            (Whole, (9, 49), Some((9, 33))),
            (Whole, (9, 60), Some((9, 33))),
        ]
    );
}
