//! The warning (`-W`), code-generation and language (`-f`) and machine
//! (`-m`) option names GCC and Clang know, frozen from GCC 15.2's `--help`
//! and the pinned Clang's `--autocomplete` by
//! `scripts/update_driver_options.py`. A personality accepts and ignores a
//! known option bcc does not implement, and diagnoses an unknown one as its
//! compiler does. GCC 15 knows a few options GCC 14 did not; accepting
//! them is harmless.

/// Which compiler's names to look in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Compiler {
    Gcc,
    Clang,
}

/// Whether `compiler` knows `option`, a `-W`, `-f` or `-m` option, in its
/// positive or `no-` form, with or without a value: `-Wno-shadow` is
/// known when `-Wshadow` is, and `-Wformat=2` when `-Wformat=` is.
pub(super) fn knows(compiler: Compiler, option: &str) -> bool {
    let Some(kind) = option.get(..2) else {
        return false;
    };
    let list = match (compiler, kind) {
        | (Compiler::Gcc, "-W") => include_str!("options/gcc-warnings.list"),
        | (Compiler::Gcc, "-f") => include_str!("options/gcc-flags.list"),
        | (Compiler::Gcc, "-m") => include_str!("options/gcc-machine.list"),
        | (Compiler::Clang, "-W") => include_str!("options/clang-warnings.list"),
        | (Compiler::Clang, "-f") => include_str!("options/clang-flags.list"),
        | (Compiler::Clang, "-m") => include_str!("options/clang-machine.list"),
        | _ => return false,
    };
    let positive = option
        .strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix("no-"))
        .map(|rest| (kind, rest));
    let names = [Some((kind, &option[2..])), positive];
    names.into_iter().flatten().any(|(kind, rest)| {
        let (name, value) = match rest.split_once('=') {
            | Some((name, _)) => (name, true),
            | None => (rest, false),
        };
        list.lines()
            .filter(|line| !line.starts_with('#'))
            .any(|line| {
                let Some(entry) = line.strip_prefix(kind) else {
                    return false;
                };
                // `-Wformat=` takes a value; `-Wformat` may stand alone. A
                // joined value such as `-mtune=native` matches its `-mtune=`
                // entry.
                match entry.strip_suffix('=') {
                    | Some(entry) => entry == name,
                    | None => entry == name && !value,
                }
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_options_match_their_forms() {
        assert!(knows(Compiler::Gcc, "-Wshadow"));
        assert!(knows(Compiler::Gcc, "-Wno-shadow"));
        assert!(knows(Compiler::Gcc, "-Wformat=2"));
        assert!(knows(Compiler::Gcc, "-fstack-protector-strong"));
        assert!(knows(Compiler::Gcc, "-fno-strict-aliasing"));
        assert!(knows(Compiler::Gcc, "-march=native"));
        assert!(!knows(Compiler::Gcc, "-Wnot-a-gcc-warning"));
        assert!(!knows(Compiler::Gcc, "-fnot-a-gcc-flag"));
        assert!(knows(Compiler::Clang, "-Wshadow"));
        assert!(knows(Compiler::Clang, "-fcolor-diagnostics"));
        assert!(!knows(Compiler::Clang, "-fnot-a-clang-flag"));
    }
}
