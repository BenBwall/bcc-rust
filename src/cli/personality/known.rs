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
//!
//! The options of the pinned Clang's ignored groups, which its
//! `--autocomplete` does not list, are frozen from its `Options.td` in
//! `options/clang-ignored.list`, with how its driver reports each, beside
//! the aliases `Options.td` defines without a name of their own
//! (`-ftree-vectorize` for `-fvectorize`).
//!
//! The GCC and Clang options clang-cl's driver does not take, which it
//! ignores with the warning `unknown argument ignored in clang-cl`, are
//! frozen from the pinned Clang's clang-cl and from icx-cl 2026.2.0 in
//! `options/clang-cl-unknown.list` and `options/icx-cl-unknown.list`.
//!
//! The options Clang's own driver takes besides its `-W`, `-f` and `-m`
//! flags, and those whose bare name takes the next arguments as values,
//! are frozen from the pinned Clang in `options/clang-driver.list`, with
//! the ones clang-cl shows bare and, for each kind of step it runs, the
//! ones it calls unused there.

/// Which compiler's names to look in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Compiler {
    Gcc,
    Clang,
}

/// Whether `compiler` knows `option`, a `-W`, `-f` or `-m` option, in its
/// positive or `no-` form, with or without a value: `-Wno-shadow` is
/// known when `-Wshadow` is, and `-Wformat=2` when `-Wformat=` is. A
/// `no-` form names a positive one: `-Wno-no-shadow` is not known.
pub(super) fn knows(compiler: Compiler, option: &str) -> bool {
    knows_name(compiler, option, true)
}

/// Whether `compiler` knows `option` as it is spelled, not as the `no-`
/// form of another: GCC's `-Werror=no-shadow` asks for an option
/// `-Wno-shadow`, which it does not have.
pub(super) fn knows_exactly(compiler: Compiler, option: &str) -> bool {
    knows_name(compiler, option, false)
}

fn knows_name(compiler: Compiler, option: &str, negative: bool) -> bool {
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
        .filter(|rest| negative && !rest.starts_with("no-"))
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

/// What the pinned Clang's driver does with an option it takes and
/// `--autocomplete` does not list: one of its ignored groups in
/// `Options.td`, or an unnamed alias.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClangIgnored {
    /// A GCC optimization flag, ignored with the warning `optimization
    /// flag '-fgcse' is not supported` (`-Wignored-optimization-argument`).
    Optimization,
    /// A retired flag, ignored with the warning `the flag '-mmpx' has been
    /// deprecated and will be ignored` (`-Wunused-command-line-argument`).
    Deprecated,
    /// Ignored without a word.
    Silent,
    /// Another spelling of the option named.
    Alias(&'static str),
}

/// What the pinned Clang does with `option` if it is one of its ignored
/// options or unnamed aliases (`options/clang-ignored.list`). An entry
/// that ends in `*` is a joined option, which matches any argument it
/// begins (`-finline-limit=100`); any other matches the whole argument.
pub(super) fn clang_ignored(option: &str) -> Option<ClangIgnored> {
    include_str!("options/clang-ignored.list")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut words = line.split(' ');
            let (kind, pattern) = (words.next()?, words.next()?);
            let matches = match pattern.strip_suffix('*') {
                | Some(prefix) => option.starts_with(prefix),
                | None => option == pattern,
            };
            if !matches {
                return None;
            }
            match kind {
                | "warn" => Some(ClangIgnored::Optimization),
                | "deprecated" => Some(ClangIgnored::Deprecated),
                | "ignore" => Some(ClangIgnored::Silent),
                | "alias" => words.next().map(ClangIgnored::Alias),
                | _ => None,
            }
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

/// Whether `option`, an argument without its `/` or `-`, is the whole name
/// of one of clang-cl's `cl` flags, which its option parser prefers to a
/// Clang option of the same name (`-u` is `/u`, which takes no value).
pub(super) fn is_clang_cl_flag(option: &str) -> bool {
    clang_cl_options().any(|(kind, name)| kind == "flag" && name == option)
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

/// An option clang-cl's driver does not take and ignores with its warning
/// `unknown argument ignored in clang-cl`, as `Driver::ParseArgStrings`
/// reports an argument its option table (LLVM's `OptTable`) matches with
/// no option visible to clang-cl.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct ClUnknown {
    /// The option the warning suggests (`; did you mean '-fbuiltin'?`).
    pub(super) nearest: Option<String>,
}

/// Whether clang-cl, or icx-cl with `intel`, ignores `argument`, one that
/// starts with `-` and is none of its `cl` options, as a GCC or Clang
/// option it does not take. A `flag` entry matches the whole argument and
/// a `joined` one any argument it begins (`-lfoo`, `-std=c11`); the
/// longest matching entry decides, so a `known` one, an option the driver
/// takes, is not covered by a shorter entry (`-l` and
/// `-lifetime-safety-max-cfg-blocks=`). An entry's suggestion is for its
/// spelling alone, or with `+` for its value given after it.
pub(super) fn cl_unknown_option(argument: &str, intel: bool) -> Option<ClUnknown> {
    let list = if intel {
        include_str!("options/icx-cl-unknown.list")
    } else {
        include_str!("options/clang-cl-unknown.list")
    };
    let mut best: Option<(&str, &str, Option<&str>)> = None;
    for line in list.lines().filter(|line| !line.starts_with('#')) {
        let mut words = line.split(' ');
        let (Some(kind), Some(name)) = (words.next(), words.next()) else {
            continue;
        };
        let matches = argument == name || (kind != "flag" && argument.starts_with(name));
        if matches && best.is_none_or(|(_, longest, _)| name.len() > longest.len()) {
            best = Some((kind, name, words.next()));
        }
    }
    let (kind, name, nearest) = best?;
    if kind == "known" {
        return None;
    }
    let value = &argument[name.len()..];
    let nearest = match nearest {
        | Some(nearest) => match nearest.strip_prefix('+') {
            | Some(nearest) => (!value.is_empty()).then(|| format!("{nearest}{value}")),
            | None => value.is_empty().then(|| nearest.to_owned()),
        },
        | None => None,
    };
    Some(ClUnknown { nearest })
}

/// How Clang's own driver reads an argument that starts with `-`, as its
/// option table (LLVM's `OptTable`) matches it, which is how clang-cl reads
/// its `/clang:` options and the Clang options its mode shows bare.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClangOption {
    /// The argument is the whole option: a flag, or an option with its
    /// value joined (`-DX`, `-std=c11`).
    Whole,
    /// The bare name of an option that takes this many values from the
    /// next arguments (`-Xclang`, `-o`, `-sectcreate`).
    TakesNext(usize),
}

/// Which of Clang's options clang-cl's driver sees: every one after
/// `/clang:`, and bare only those its mode shows (`Options.td`'s
/// `CLOption` and `CoreOption`), ignoring the rest as unknown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClangVisibility {
    All,
    ClangCl,
}

/// How the pinned Clang's driver reads `argument`, one that starts with
/// `-`, among the options `visibility` shows, or `None` if it takes no such
/// option and so ignores it with clang-cl's warning `unknown argument
/// ignored in clang-cl`. Its front end reads every `-W` option, warning
/// about those it does not know; the `-f` and `-m` flags are the ones
/// [`knows`] names. The bare name of an option with separate values takes
/// them even where the option also joins one (`-o`, `-D`); an entry that
/// joins a value matches any argument it begins.
pub(super) fn clang_option(argument: &str, visibility: ClangVisibility) -> Option<ClangOption> {
    let mut known = argument.starts_with("-W") || knows(Compiler::Clang, argument);
    for entry in
        clang_driver_entries().filter(|entry| visibility == ClangVisibility::All || entry.clang_cl)
    {
        match entry.kind {
            | ClangKind::Separate(count) if entry.name == argument =>
                return Some(ClangOption::TakesNext(count)),
            | ClangKind::Flag => known |= entry.name == argument,
            | ClangKind::Joined => known |= argument.starts_with(entry.name),
            | ClangKind::Separate(_) => {},
        }
    }
    known.then_some(ClangOption::Whole)
}

/// The option of the pinned Clang's table that `argument` spells, and the
/// value it joins, as its option table matches it: the longest name that
/// is the argument (a flag) or begins it (an option that joins its value,
/// `-include` in `-includeinc.h`, `-std=` in `-std=c11`). `None` for an
/// argument the table names in no such entry, as the `-W`, `-f` and `-m`
/// flags, which it does not list. A listed name that goes on after its `=`
/// (GCC's `-std=c11`) is a value of the shorter option (`-std=`).
pub(super) fn clang_spelling(argument: &str) -> Option<(&'static str, Option<&str>)> {
    clang_driver_entries()
        .filter(|entry| match entry.kind {
            | ClangKind::Flag => entry.name == argument,
            | ClangKind::Joined =>
                argument.starts_with(entry.name)
                    && entry
                        .name
                        .find('=')
                        .is_none_or(|equals| equals + 1 == entry.name.len()),
            | ClangKind::Separate(_) => false,
        })
        .max_by_key(|entry| (entry.name.len(), entry.kind == ClangKind::Flag))
        .map(|entry| match entry.kind {
            | ClangKind::Joined => (entry.name, Some(&argument[entry.name.len()..])),
            | _ => (entry.name, None),
        })
}

/// The kinds of step clang-cl runs, for which `options/clang-driver.list`
/// records the options it calls unused in each: only compiling (`/c`),
/// preprocessing (`/E`, `/EP` and `/P` alike), only checking (`/Zs`), and
/// linking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClangStep {
    Compile,
    Preprocess,
    Syntax,
    Link,
}

impl ClangStep {
    pub(super) const ALL: [Self; 4] = [Self::Compile, Self::Preprocess, Self::Syntax, Self::Link];

    /// The tag `scripts/update_driver_options.py` gives an option clang-cl
    /// calls unused in this kind of step.
    const fn unused_tag(self) -> &'static str {
        match self {
            | Self::Compile => "unused-c",
            | Self::Preprocess => "unused-E",
            | Self::Syntax => "unused-Zs",
            | Self::Link => "unused-link",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Whether clang-cl warns `argument unused during compilation` about
/// `words`, a Clang option and the values its bare name takes, given after
/// `/clang:` to a step of kind `step` (`-shared` beside `/c`, `-sectcreate
/// a b c`), as the pinned Clang's table records it for that kind of step
/// and for the option as spelled: a joined option's tag holds for its name
/// alone and with a value (`-dD` is no unused `-d` with the value `D`).
pub(super) fn clang_unused(words: &[&str], step: ClangStep) -> bool {
    let [argument, values @ ..] = words else {
        return false;
    };
    if values.is_empty() {
        return clang_spelling(argument).is_some_and(|(name, joined)| {
            clang_driver_entries().any(|entry| {
                entry.name == name
                    && entry.unused & step.bit() != 0
                    && (entry.kind == ClangKind::Joined) == joined.is_some()
            })
        });
    }
    clang_driver_entries().any(|entry| {
        entry.name == *argument
            && entry.kind == ClangKind::Separate(values.len())
            && entry.unused & step.bit() != 0
    })
}

/// How an entry of `options/clang-driver.list` takes its value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ClangKind {
    Flag,
    Joined,
    Separate(usize),
}

/// An entry of `options/clang-driver.list`: `KIND NAME [COUNT] [cl]
/// [unused-STEP...]`, the tags `scripts/update_driver_options.py` gives an
/// option clang-cl shows bare and one it calls unused in each kind of step
/// ([`ClangStep`]).
struct ClangEntry {
    kind:     ClangKind,
    name:     &'static str,
    clang_cl: bool,
    /// The [`ClangStep::bit`]s of the kinds of step it is unused in.
    unused:   u8,
}

fn clang_driver_entries() -> impl Iterator<Item = ClangEntry> {
    include_str!("options/clang-driver.list")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut words = line.split(' ');
            let kind = words.next()?;
            let name = words.next()?;
            let mut entry = ClangEntry {
                kind: match kind {
                    | "flag" => ClangKind::Flag,
                    | "joined" => ClangKind::Joined,
                    | "separate" => ClangKind::Separate(1),
                    | _ => return None,
                },
                name,
                clang_cl: false,
                unused: 0,
            };
            for word in words {
                if word == "cl" {
                    entry.clang_cl = true;
                } else if let Some(step) = ClangStep::ALL
                    .into_iter()
                    .find(|step| step.unused_tag() == word)
                {
                    entry.unused |= step.bit();
                } else {
                    entry.kind = ClangKind::Separate(word.parse().ok()?);
                }
            }
            Some(entry)
        })
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
        assert!(knows(Compiler::Clang, "-Wno-shadow"));
        // Clang warns about an unknown `-Wno-` option, as CMake's
        // `check_c_compiler_flag(-Wno-bogus)` expects.
        assert!(!knows(Compiler::Clang, "-Wno-not-a-clang-warning"));
        assert!(knows(Compiler::Clang, "-fcolor-diagnostics"));
        assert!(!knows(Compiler::Clang, "-fnot-a-clang-flag"));
    }

    #[test]
    fn clang_ignored_options_match_their_spellings() {
        assert_eq!(
            clang_ignored("-fno-ipa-cp"),
            Some(ClangIgnored::Optimization)
        );
        assert_eq!(
            clang_ignored("-finline-limit=100"),
            Some(ClangIgnored::Optimization)
        );
        assert_eq!(clang_ignored("-mmpx"), Some(ClangIgnored::Deprecated));
        assert_eq!(
            clang_ignored("-fimplement-inlines"),
            Some(ClangIgnored::Silent)
        );
        // Of an ignored group, but read beside `-fno-inline`.
        assert_eq!(clang_ignored("-finline"), None);
        assert_eq!(
            clang_ignored("-ftree-vectorize"),
            Some(ClangIgnored::Alias("-fvectorize"))
        );
        // A flag matches only whole.
        assert_eq!(clang_ignored("-fno-ipa-cp=1"), None);
        assert_eq!(clang_ignored("-fvectorize"), None);
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
    fn clang_cl_ignores_the_options_its_driver_does_not_take() {
        let plain = |argument| cl_unknown_option(argument, false);
        let nearest = |argument, intel| {
            cl_unknown_option(argument, intel).and_then(|unknown| unknown.nearest)
        };
        for argument in [
            "-fsigned-char",
            "-ffast-math",
            "-ffreestanding",
            "-funsafe-math-optimizations",
            "-ffp-model=fast",
            "-std=c11",
            "-pedantic",
            "-lm",
            "-isystem",
            "-isystem/usr/include",
        ] {
            assert_eq!(nearest(argument, false), None, "{argument}");
            assert!(plain(argument).is_some(), "{argument}");
        }
        // Options clang-cl takes.
        for argument in [
            "-fms-extensions",
            "-fno-builtin",
            "-fmath-errno",
            "-mavx2",
            "-o",
            "-v",
            "-lifetime-safety-max-cfg-blocks=4",
        ] {
            assert_eq!(plain(argument), None, "{argument}");
        }
        assert_eq!(nearest("-fbuiltin-", false).as_deref(), Some("-fbuiltin"));
        assert_eq!(nearest("-fbuiltin-x", false), None);
        assert_eq!(nearest("-mabs=foo", false).as_deref(), Some("-mabi=foo"));
        assert_eq!(nearest("-mabs=", false), None);
        // icx-cl takes `-ffast-math`, and suggests Intel's own options.
        assert_eq!(cl_unknown_option("-ffast-math", true), None);
        assert_eq!(
            nearest("-ffreestanding", true).as_deref(),
            Some("-Qfreestanding")
        );
    }

    #[test]
    fn clang_reads_its_options_values_as_its_option_table_does() {
        for (argument, read) in [
            ("-Xclang", Some(ClangOption::TakesNext(1))),
            ("-o", Some(ClangOption::TakesNext(1))),
            ("-D", Some(ClangOption::TakesNext(1))),
            ("-target", Some(ClangOption::TakesNext(1))),
            ("-mllvm", Some(ClangOption::TakesNext(1))),
            ("-sectcreate", Some(ClangOption::TakesNext(3))),
            ("-DX=1", Some(ClangOption::Whole)),
            ("-ofile.o", Some(ClangOption::Whole)),
            ("-std=c11", Some(ClangOption::Whole)),
            ("-O2", Some(ClangOption::Whole)),
            ("-c", Some(ClangOption::Whole)),
            ("-ffreestanding", Some(ClangOption::Whole)),
            ("-mavx2", Some(ClangOption::Whole)),
            ("-Wno-such-warning", Some(ClangOption::Whole)),
            ("--target=x86_64-linux-gnu", Some(ClangOption::Whole)),
            // Clang takes none of these.
            ("-ffoo", None),
            ("-mfoo", None),
            ("--bogus", None),
            ("-cfoo", None),
            ("-Xclangx", None),
        ] {
            assert_eq!(
                clang_option(argument, ClangVisibility::All),
                read,
                "{argument}"
            );
        }
    }

    /// Bare, clang-cl sees only the Clang options its mode shows: it takes
    /// `-Xclang`'s value from the next argument, and ignores `-specs`,
    /// `-sectcreate` and `--config` as unknown, which then take none.
    #[test]
    fn clang_cl_sees_only_its_modes_options_bare() {
        for (argument, read) in [
            ("-Xclang", Some(ClangOption::TakesNext(1))),
            ("-mllvm", Some(ClangOption::TakesNext(1))),
            ("-o", Some(ClangOption::TakesNext(1))),
            ("-specs", None),
            ("-sectcreate", None),
            ("--config", None),
            ("-include", None),
            ("-H", Some(ClangOption::Whole)),
            ("-ffreestanding", Some(ClangOption::Whole)),
        ] {
            assert_eq!(
                clang_option(argument, ClangVisibility::ClangCl),
                read,
                "{argument}"
            );
        }
    }

    #[test]
    fn clang_spells_an_option_and_its_joined_value_as_its_table_does() {
        for (argument, spelling) in [
            ("-includeinc.h", Some(("-include", Some("inc.h")))),
            ("-include-pchx", Some(("-include-pch", Some("x")))),
            ("-std=c11", Some(("-std=", Some("c11")))),
            ("-O2", Some(("-O", Some("2")))),
            ("-static-libgcc", Some(("-static-libgcc", None))),
            ("-shared", Some(("-shared", None))),
            ("-ffreestanding", None),
        ] {
            assert_eq!(clang_spelling(argument), spelling, "{argument}");
        }
    }

    /// clang-cl calls a linker option unused in a step that does not link,
    /// each as the pinned Clang's table records it for that kind of step
    /// and for the option as spelled.
    #[test]
    fn clang_cl_calls_options_unused_by_the_step() {
        use ClangStep::{
            Compile,
            Link,
            Preprocess,
            Syntax,
        };
        for (words, step, unused) in [
            (&["-sectcreate", "a", "b", "c"][..], Compile, true),
            (&["-whatsloaded"], Compile, true),
            (&["-specs=x"], Compile, true),
            (&["-shared"], Compile, true),
            (&["-shared"], Preprocess, true),
            (&["-shared"], Syntax, true),
            // Linking, `-shared` makes a DLL.
            (&["-shared"], Link, false),
            // `-S` changes the step beside `/c`, and is unused beside `/E`.
            (&["-S"], Compile, false),
            (&["-S"], Preprocess, true),
            // `-Tx` is `-T x`, as Clang renders it.
            (&["-Tx"], Compile, true),
            // `-dD` is its own option, which `-d` does not absorb without a
            // value; `/E` prints the definitions.
            (&["-dD"], Compile, false),
            (&["-dD"], Preprocess, false),
            (&["-dDx"], Compile, true),
            // clang-cl calls the source unused, not the option.
            (&["--cuda-device-only"], Link, false),
            (&["-dI"], Compile, false),
            (&["-dumpmachine"], Compile, false),
            (&["-D", "X"], Compile, false),
            (&["-imacros", "x"], Compile, false),
            (&["-isysroot", "x"], Compile, false),
            (&["-ffreestanding"], Compile, false),
        ] {
            assert_eq!(clang_unused(words, step), unused, "{words:?} {step:?}");
        }
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
