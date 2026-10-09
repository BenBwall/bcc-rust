//! Frozen target-description macros from the repository's pinned Clang.
//! C99: implementation-defined limits §5.2.4.2, pp. 21-27; PDF pp. 33-39;
//! reserved names §7.1.3p1, p. 166; PDF p. 178. Feature exclusions and the
//! regeneration procedure are documented in language-standards.md.

use super::Target;
use crate::util::bump::Bump;

impl Target {
    /// Ordinary target definitions, read before the user's translation unit.
    /// C99: reserved implementation names §7.1.3p1, p. 166; PDF p. 178;
    /// implementation-defined limits §5.2.4.2, pp. 21-27; PDF pp. 33-39.
    pub(crate) fn predefined_macros(self, arena: &Bump, gnu: bool) -> &str {
        let source = match (self, gnu) {
            | (Self::LinuxGnu, false) => include_str!("x86_64-unknown-linux-gnu-c11.h"),
            | (Self::LinuxGnu, true) => include_str!("x86_64-unknown-linux-gnu-gnu17.h"),
            | (Self::LinuxMusl, false) => include_str!("x86_64-unknown-linux-musl-c11.h"),
            | (Self::LinuxMusl, true) => include_str!("x86_64-unknown-linux-musl-gnu17.h"),
            | (Self::WindowsGnu, false) => include_str!("x86_64-w64-windows-gnu-c11.h"),
            | (Self::WindowsGnu, true) => include_str!("x86_64-w64-windows-gnu-gnu17.h"),
            | (Self::WindowsMsvc, false) => include_str!("x86_64-pc-windows-msvc-c11.h"),
            | (Self::WindowsMsvc, true) => include_str!("x86_64-pc-windows-msvc-gnu17.h"),
        };
        arena.alloc_str(source)
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Macro oracle tests invoke Clang and compare owned test buffers."
)]
mod tests {
    use super::*;

    #[test]
    fn previous_linux_macro_contract_is_preserved() {
        let arena = Bump::new();
        let actual = Target::LinuxGnu.predefined_macros(&arena, true);
        for line in include_str!("../../tests/fixtures/freestanding/target-macros.h").lines() {
            assert!(
                actual.lines().any(|a| a.trim_end() == line.trim_end()),
                "{line}"
            );
        }
    }

    #[test]
    fn every_target_definition_equals_live_clang_minus_documented_exclusions() {
        use std::{
            collections::BTreeSet,
            process::Command,
        };
        let excluded = include_str!("macro-exclusions.tsv");
        let documentation = include_str!("../../language-standards.md");
        for line in excluded.lines() {
            let (name, _) = line.split_once('\t').unwrap();
            assert!(
                documentation.contains(&format!("`{name}`")),
                "undocumented exclusion: {name}"
            );
        }
        let clang = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        });
        for target in [
            Target::LinuxGnu,
            Target::LinuxMusl,
            Target::WindowsGnu,
            Target::WindowsMsvc,
        ] {
            for gnu in [false, true] {
                let mode = if gnu { "gnu17" } else { "c11" };
                let output = Command::new(&clang)
                    .args([
                        format!("--target={}", target.triple()).as_str(),
                        format!("-std={mode}").as_str(),
                        "-dM",
                        "-E",
                        "-x",
                        "c",
                        if cfg!(windows) { "NUL" } else { "/dev/null" },
                    ])
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{output:?}");
                let reference = String::from_utf8(output.stdout).unwrap();
                let expected: BTreeSet<_> = reference
                    .lines()
                    .filter(|line| {
                        let name = line
                            .strip_prefix("#define ")
                            .unwrap()
                            .split([' ', '('])
                            .next()
                            .unwrap();
                        !excluded
                            .lines()
                            .any(|entry| entry.split_once('\t').unwrap().0 == name)
                    })
                    .map(str::trim_end)
                    .collect();
                let arena = Bump::new();
                let actual: BTreeSet<_> = target
                    .predefined_macros(&arena, gnu)
                    .lines()
                    .map(str::trim_end)
                    .collect();
                assert_eq!(actual, expected, "{} {mode}", target.triple());
            }
        }
    }
}
