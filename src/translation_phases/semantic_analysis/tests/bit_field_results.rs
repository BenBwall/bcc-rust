//! Value-producing operators retain narrow bit-field integer promotions.
//! C99: §6.3.1.1p2, pp. 42-43; PDF pp. 54-55.

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

#[test]
fn bit_field_value_results_promote_before_arithmetic() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for value in ["(0,b.f)", "(b.f=308)", "(b.f+=1)", "++b.f", "(one?b.f:b.f)"] {
            with_configuration(
                &format!(
                    "struct B {{ unsigned f:17; }} b; int one; void f(void) {{ +({value}); }}"
                ),
                CompilerConfiguration::default().with_target(target),
                |context, sema| {
                    assert_eq!(context.error_count(), 0, "{value}");
                    let info = sema.expressions.last().unwrap();
                    assert_eq!(
                        sema.types.nodes[info.ty.index],
                        TypeKind::Scalar(Scalar::Int),
                        "{target:?}: {value}"
                    );
                },
            );
        }
    }
}

#[test]
fn extended_bit_field_value_results_promote_by_width() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for ty in ["unsigned long", "unsigned long long", "unsigned __int128"] {
            with_configuration(
                &format!("struct S {{ {ty} a:17; }} s; void f(void) {{ +(0,s.a); }}"),
                CompilerConfiguration::default().with_target(target),
                |context, sema| {
                    assert_eq!(context.error_count(), 0);
                    let info = sema.expressions.last().unwrap();
                    assert_eq!(
                        sema.types.nodes[info.ty.index],
                        TypeKind::Scalar(Scalar::Int),
                        "{target:?}: {ty}"
                    );
                },
            );
        }
    }
}
