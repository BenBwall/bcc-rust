//! GNU `transparent_union` (#60): which unions qualify and which arguments
//! a prototyped call converts to them, with Clang's warnings and errors.
//! GNU extension: GCC manual, "Common Type Attributes".
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>

use super::*;
use crate::configuration::CompilerConfiguration;

/// Each semantic diagnostic of `source` with its line and column.
fn diagnostics(source: &str) -> Vec<(SemanticErrorKind, u32, u32)> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    let position =
                        crate::translation_phases::GetPosition::position(&error, context);
                    result.push((error.kind, position.line, position.column));
                }
            }
        },
    );
    result
}

#[test]
fn transparent_unions_qualify_and_convert_arguments_as_in_clang() {
    use SemanticErrorKind::{
        IgnoredTransparentUnion as Ignored,
        InvalidArgumentType as Invalid,
    };
    // Every position Clang accepts, the unions it ignores and the arguments
    // it rejects; the expected positions are Clang's, which also warns twice
    // for the structure.
    let source = "typedef union { int *ip; const long *lp; } __attribute__((transparent_union)) \
                  a_t;
union __attribute__((transparent_union)) u2 { int *ip; long *lp; };
union u3 { int *ip; long *lp; } __attribute__((__transparent_union__));
typedef union { int *ip; long *lp; } t4 __attribute__((transparent_union));
union u5 { int *ip; long *lp; };
typedef union u5 __attribute__((transparent_union)) t5;
union u6 { int i; char c; } __attribute__((transparent_union));
union u7 { double d; long l; } __attribute__((transparent_union));
union u8 { int i; long long l; } __attribute__((transparent_union));
union u9 { short s; int i; } __attribute__((transparent_union));
int f1(a_t); int f2(union u2); int f3(union u3); int f4(t4); int f5(t5); int f5b(union u5);
int f6(union u6); int f8(union u8);
void g(int *ip, long *lp, const long *clp, void *vp, const int *cip, char *cp, a_t a) {
  f1(ip); f1(clp); f1(lp); f1(vp); f1(0); f1(a);
  f2(ip); f2(lp); f2(vp); f2(0);
  f3(ip); f4(ip); f5(ip); f5b(ip);
  f6(1); f6('c'); f6(2.5);
  f1(cip);
  f2(cp);
  f6(ip);
}
struct __attribute__((transparent_union)) not_union { int *p; };
union incomplete;
typedef union incomplete __attribute__((transparent_union)) incomplete_t;
typedef int *not_union_t __attribute__((transparent_union));";
    assert_eq!(
        diagnostics(source),
        [
            (Ignored, 7, 24),
            (Ignored, 8, 19),
            (Ignored, 9, 29),
            (Ignored, 10, 25),
            (Invalid, 17, 6),
            (Invalid, 17, 13),
            (Invalid, 17, 22),
            (Invalid, 18, 6),
            (Invalid, 19, 6),
            (Invalid, 20, 6),
            (Ignored, 22, 23),
            (Ignored, 24, 41),
            (Ignored, 25, 41),
        ]
    );
}

#[test]
fn a_transparent_union_argument_records_its_member_and_union_conversions() {
    with_configuration(
        "typedef union { long l; int *p; } __attribute__((transparent_union)) u_t;
         int f(u_t); int g(int *p) { return f(p) + f(0) + f(1.5); }",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, s| {
            assert!(context.take_pending_errors().is_empty());
            let union = s
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "u_t")
                .unwrap()
                .ty;
            let members: Vec<_> = s
                .conversions
                .iter()
                .filter(|c| c.kind == ConversionKind::TransparentUnion)
                .map(|c| {
                    assert_eq!(c.ty.index, union.index);
                    let all = s.expression_conversions(c.expression);
                    let member = all[all.len() - 2];
                    assert_eq!(member.kind, ConversionKind::Assignment);
                    s.types.kind(member.ty)
                })
                .collect();
            // `p` selects the pointer; `0` and `1.5` are assignable to the
            // first member, which comes first.
            assert_eq!(members.len(), 3);
            assert!(matches!(members[0], TypeKind::Pointer(_)));
            assert!(matches!(members[1], TypeKind::Scalar(_)));
            assert!(matches!(members[2], TypeKind::Scalar(_)));
        },
    );
}

/// The attribute catalog identifies `transparent_union`, so the standard
/// syntax's `gnu::` prefix names it as Clang accepts it, without an unknown-
/// or ignored-attribute warning.
#[test]
fn prefixed_transparent_union_is_the_same_rule() {
    with_configuration(
        "union [[gnu::transparent_union]] u { int *ip; long *lp; };
         int f(union u); int g(int *p) { return f(p); }",
        CompilerConfiguration::new(CStandard::C23, crate::configuration::ExtensionPolicy::Warn)
            .with_gnu_extensions(true),
        |context, s| {
            assert!(context.take_pending_errors().is_empty());
            assert_eq!(
                s.conversions
                    .iter()
                    .filter(|c| c.kind == ConversionKind::TransparentUnion)
                    .count(),
                1
            );
        },
    );
}
