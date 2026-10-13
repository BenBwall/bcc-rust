//! Typedef and explicit member alignment have distinct bit-field roles.

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

#[test]
fn typedef_alignment_does_not_split_bit_field_units() {
    let source = "typedef unsigned U __attribute__((aligned(8)));\nstruct S { U a:3; U b:3; };";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        with_configuration(
            source,
            CompilerConfiguration::default().with_target(target),
            |context, sema| {
                assert_eq!(context.error_count(), 0);
                let tag = sema
                    .types
                    .tags
                    .iter()
                    .find(|tag| tag.kind == TagKind::Struct)
                    .unwrap();
                let size = if target == Target::WindowsGnu { 4 } else { 8 };
                assert_eq!(
                    tag.layout.get(),
                    Some(Layout { size, align: size }),
                    "{target:?}"
                );
                let members = tag.members.get();
                assert_eq!(
                    (members[0].offset, members[0].bit_offset),
                    (0, 0),
                    "{target:?}"
                );
                assert_eq!(
                    (members[1].offset, members[1].bit_offset),
                    (0, 3),
                    "{target:?}"
                );
                assert_eq!(members[0].explicit_alignment, 0);
                assert_eq!(members[1].explicit_alignment, 0);
            },
        );
    }
}
