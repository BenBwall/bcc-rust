//! The warning (`-W`), code-generation and language (`-f`) and machine
//! (`-m`) option names GCC and Clang know, frozen from GCC 15.2's `--help`
//! and the pinned Clang's `--autocomplete` by
//! `scripts/update_driver_options.py`. A personality accepts and ignores a
//! known option bcc does not implement, and diagnoses an unknown one as its
//! compiler does. GCC 15 knows a few options GCC 14 did not; accepting
//! them is harmless.
//!
//! The `cl` options clang-cl knows, frozen from the pinned Clang's
//! `Options.td` in `options/clang-cl.list`, tell an option from a path: like
//! clang-cl, the `cl` and clang-cl personalities read an argument that
//! starts with `/` and names none of them as an input file.

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
                // `-Wformat=` and `-fexcess-precision=[fast|standard|16]`
                // take a value; `-Wformat` may stand alone. A joined value
                // such as `-mtune=native` matches its `-mtune=` entry.
                // `-fbuiltin-`, `-fdump-` and `-fdisable-[tree|rtl|ipa]-`
                // name a family by its prefix.
                let entry = entry.split_once('[').map_or(entry, |(head, _)| head);
                if let Some((entry, _)) = entry.split_once('=') {
                    entry == name && value
                } else if entry.ends_with('-') {
                    rest.starts_with(entry)
                } else {
                    entry == name && !value
                }
            })
    })
}

/// How clang-cl reads an argument that is one of its `cl` options.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClOption {
    /// The argument is the whole option: a flag, or an option with its
    /// value joined (`/Fofoo.obj`, `/Fo:foo.obj`).
    Whole,
    /// The bare name of an option whose value is the next argument (`/Fo:`
    /// in `/Fo: foo.obj`, `/imsvc` in `/imsvc /usr/include`).
    TakesNext,
}

/// Whether clang-cl reads `option`, an argument without its `/` or `-`, as
/// one of its `cl` options, as its option parser (LLVM's `OptTable`)
/// matches them: a flag by its whole name, an option with a value by the
/// name the argument starts with. clang-cl reads an argument that starts
/// with `/` and matches none as an input file, so `/tmp/a.c` is a source
/// while `/Users/a.c` is `/U` with `sers/a.c` and `/opt/a.c` is `/o`.
pub(super) fn clang_cl_option(option: &str) -> Option<ClOption> {
    let mut known = None;
    for (kind, name) in clang_cl_options() {
        if name == option && kind == "separate" {
            return Some(ClOption::TakesNext);
        }
        if name == option || (kind != "flag" && option.starts_with(name)) {
            known = Some(ClOption::Whole);
        }
    }
    known
}

/// The `cl` option clang-cl suggests for an input `argument` that does not
/// exist (`no such file or directory: '/Brepo'; did you mean '/Brepro'?`),
/// as LLVM's `OptTable::findNearest` chooses it: the closest spelling, with
/// either prefix, of an option at least four characters long, if it is one
/// edit away. An option that ends in `:` or `=` is compared with the
/// argument up to that delimiter, one edit further when the argument gives
/// no value. clang-cl also considers the Clang options it takes, which
/// start with `-` only; they are not frozen here.
pub(super) fn nearest_clang_cl_option(argument: &str) -> Option<String> {
    let mut nearest: Option<(usize, String)> = None;
    for (_, name) in clang_cl_options() {
        if name.len() < 4 {
            continue;
        }
        let delimiter = name.ends_with([':', '=']).then(|| &name[name.len() - 1..]);
        let (compared, value) = match delimiter
            .and_then(|delimiter| Some((delimiter, argument.split_once(delimiter)?)))
        {
            | Some((delimiter, (head, value))) => (format!("{head}{delimiter}"), value),
            | None => (argument.to_owned(), ""),
        };
        for prefix in ["/", "-"] {
            let candidate = format!("{prefix}{name}");
            let mut distance = edit_distance(&candidate, &compared);
            if delimiter.is_some() && value.is_empty() {
                distance += 1;
            }
            if nearest.as_ref().is_none_or(|(best, _)| distance < *best) {
                nearest = Some((distance, format!("{candidate}{value}")));
            }
        }
    }
    nearest
        .filter(|(distance, _)| *distance <= 1)
        .map(|(_, option)| option)
}

/// The `cl` options clang-cl knows, as (kind, name): a `flag`, a `joined`
/// option, or a `separate` one, whose value is joined or the next argument.
fn clang_cl_options() -> impl Iterator<Item = (&'static str, &'static str)> {
    include_str!("options/clang-cl.list")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once(' '))
}

/// The Levenshtein distance between `a` and `b`, in bytes, as LLVM's
/// `StringRef::edit_distance` counts it with replacements allowed.
fn edit_distance(a: &str, b: &str) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &x) in a.as_bytes().iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &y) in b.as_bytes().iter().enumerate() {
            let substitution = diagonal + usize::from(x != y);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[b.len()]
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
        assert!(knows(Compiler::Gcc, "-fcf-protection=full"));
        assert!(knows(Compiler::Gcc, "-fexcess-precision=standard"));
        assert!(knows(Compiler::Gcc, "-fno-builtin-memcpy"));
        assert!(knows(Compiler::Gcc, "-fdump-tree-all"));
        assert!(!knows(Compiler::Gcc, "-Wnot-a-gcc-warning"));
        assert!(!knows(Compiler::Gcc, "-fnot-a-gcc-flag"));
        assert!(knows(Compiler::Clang, "-Wshadow"));
        assert!(knows(Compiler::Clang, "-fcolor-diagnostics"));
        assert!(!knows(Compiler::Clang, "-fnot-a-clang-flag"));
    }

    #[test]
    fn clang_cl_tells_its_options_from_paths() {
        assert_eq!(clang_cl_option("nologo"), Some(ClOption::Whole));
        assert_eq!(clang_cl_option("GR-"), Some(ClOption::Whole));
        assert_eq!(clang_cl_option("Fo/tmp/a.obj"), Some(ClOption::Whole));
        assert_eq!(clang_cl_option("Fo:"), Some(ClOption::TakesNext));
        assert_eq!(clang_cl_option("imsvc"), Some(ClOption::TakesNext));
        // A flag matches only whole; a path that starts with an option
        // taking a joined value is that option, as in clang-cl.
        assert_eq!(clang_cl_option("nologox"), None);
        assert_eq!(clang_cl_option("tmp/a.c"), None);
        assert_eq!(clang_cl_option("home/user/a.c"), None);
        assert_eq!(clang_cl_option("Users/a.c"), Some(ClOption::Whole));
        assert_eq!(clang_cl_option("opt/a.c"), Some(ClOption::Whole));
    }

    #[test]
    fn clang_cl_suggests_an_option_one_edit_away() {
        assert_eq!(
            nearest_clang_cl_option("/Brepo").as_deref(),
            Some("/Brepro")
        );
        assert_eq!(
            nearest_clang_cl_option("/nolog").as_deref(),
            Some("/nologo")
        );
        // An option ending in a delimiter keeps the argument's value.
        assert_eq!(
            nearest_clang_cl_option("/sourc-charset:utf-8").as_deref(),
            Some("/source-charset:utf-8")
        );
        assert_eq!(nearest_clang_cl_option("/source-charse"), None);
        assert_eq!(nearest_clang_cl_option("/Zq"), None);
        assert_eq!(nearest_clang_cl_option("/tmp/missing.c"), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
    }
}
