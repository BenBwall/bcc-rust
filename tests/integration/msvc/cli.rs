//! `bcc --msvc-mode` against the real `cl.exe` on the same command lines,
//! each run in a fresh copy of the same directory with the same `CL` and
//! `_CL_` environment variables: the exit status, the files each writes,
//! and what each prints. The report shows every command line side by side;
//! the test fails where a case's [`Expect`] says they should agree.

use std::{
    collections::BTreeSet,
    fs,
    path::Path,
};

use super::toolchain::{
    Differences,
    Msvc,
    msvc,
    scratch,
};
use crate::native::run_native;

/// The files each command line starts with.
const FILES: [(&str, &str); 15] = [
    ("a.c", "int b(void);\nint main(void) { return b() - 2; }\n"),
    ("b.c", "int b(void) { return 2; }\n"),
    (
        "pp.c",
        "#include \"h.h\"\n#ifndef VALUE\n#define VALUE 1\n#endif\nint pp = VALUE + H;\n",
    ),
    ("h.h", "#define H 10\n"),
    ("inc.c", "#include \"deep.h\"\nint deep = DEEP;\n"),
    ("inc/deep.h", "#define DEEP 7\n"),
    // `cl /W4` warns about the unused local (C4101), which bcc does not.
    ("unused.c", "int main(void) { int unused; return 0; }\n"),
    // Both warn about the truncating cast (`cl`'s C4311).
    ("cast.c", "int f(int *p) { return (int)p; }\n"),
    ("bad.c", "int main(void) {\n    return missing;\n}\n"),
    (
        "syntax.c",
        "int f(void) {\n    int x = 1\n    int y = 2;\n    x = 3\n    y = x;\n    return x\n}\n",
    ),
    (
        "implicit.c",
        "int main(void) { return g(1); }\nint g(int x) { return x - 1; }\n",
    ),
    (
        "pointers.c",
        "int main(void) {\n    int *p = 0;\n    float *q = 0;\n    char **r = 0;\n    p = q;\n    \
         p = r;\n    return 0;\n}\n",
    ),
    // `cl` reports an initializer's conversion after the statements that
    // follow it in the function, so its warning has a file of its own.
    (
        "initializer.c",
        "int main(void) {\n    int *p = 5;\n    return p != 0;\n}\n",
    ),
    // A command file whose first line passes the rest to the linker.
    (
        "options.rsp",
        "/c a.c /link /SUBSYSTEM:CONSOLE\n/Foresponse.obj\n",
    ),
    ("define.rsp", "/DVALUE=4\n/EP pp.c\n"),
];

/// What a command line must agree on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The exit status and the files written.
    Results,
    /// Those and every line printed.
    Output,
    /// Those and the diagnostics with a code (`x.c(2): error C2065:
    /// ...`), which `cl` writes to stdout and bcc to stderr.
    Diagnostics,
    /// A difference driver-modes.md records; reported, not checked.
    Documented,
}

/// Command lines compared, after `/nologo` unless they start with `-`.
/// `cl` writes compiler diagnostics to stdout and bcc to stderr, and their
/// messages differ, so only lines without diagnostics are compared.
const CASES: &[(&[&str], Expect)] = &[
    (&["/c", "a.c"], Expect::Output),
    (&["/c", "a.c", "/Foout.obj"], Expect::Output),
    (&["/c", "a.c", "b.c", "/Foobj\\"], Expect::Output),
    (&["/c", "a.c", "b.c", "/Foone.obj"], Expect::Output),
    (&["a.c", "b.c"], Expect::Output),
    (&["a.c", "b.c", "/Feprog.exe"], Expect::Output),
    (&["a.c", "b.c", "/link", "/OUT:linked.exe"], Expect::Output),
    // `cl /E` writes `#line` directives and keeps blank lines.
    (&["/E", "pp.c"], Expect::Results),
    (&["/EP", "pp.c"], Expect::Results),
    (&["/P", "pp.c"], Expect::Results),
    (&["/DVALUE=3", "/EP", "pp.c"], Expect::Results),
    (&["/Iinc", "/EP", "inc.c"], Expect::Results),
    (&["/EP", "inc.c"], Expect::Results),
    (&["/W4", "/c", "unused.c"], Expect::Results),
    (&["/W4", "/WX", "/c", "unused.c"], Expect::Documented),
    (&["/c", "cast.c"], Expect::Results),
    (&["/WX", "/c", "cast.c"], Expect::Results),
    (&["/std:c11", "/c", "a.c"], Expect::Output),
    (&["/std:c17", "/c", "a.c"], Expect::Output),
    (&["/O2", "/c", "a.c"], Expect::Output),
    // `cl /Zi` also writes `vc140.pdb`, which is not compared.
    (&["/Zi", "/c", "a.c"], Expect::Output),
    (&["/c", "/bogus", "a.c"], Expect::Output),
    (&["/c", "missing.c"], Expect::Results),
    (&["/c", "a.c", "missing.c"], Expect::Results),
    (&["/c", "bad.c", "missing.c"], Expect::Results),
    (&["/c", "bad.c"], Expect::Results),
    (&["/c", "/diagnostics:column", "bad.c"], Expect::Results),
    (&["/c", "/diagnostics:caret", "bad.c"], Expect::Results),
    (&["/c", "a.c", "h.h"], Expect::Output),
    // bcc refuses a translation unit without a declaration.
    (&["/c", "/Tch.h"], Expect::Documented),
    // LLD writes an import library for a DLL without exports.
    (&["/LD", "b.c"], Expect::Documented),
    (&[], Expect::Output),
    // Help is a complete action; the texts differ.
    (&["/HELP"], Expect::Results),
    // The banners differ by design.
    (&["-c", "a.c"], Expect::Results),
    // `cl`'s codes and words for the common diagnostics, at its default
    // warning level and the others.
    (&["/c", "bad.c"], Expect::Diagnostics),
    (&["/c", "syntax.c"], Expect::Diagnostics),
    (&["/c", "implicit.c"], Expect::Diagnostics),
    (&["/W3", "/c", "implicit.c"], Expect::Diagnostics),
    (&["/W4", "/wd4013", "/c", "implicit.c"], Expect::Diagnostics),
    (&["/w14013", "/c", "implicit.c"], Expect::Diagnostics),
    (&["/we4013", "/c", "implicit.c"], Expect::Diagnostics),
    (&["/WX", "/c", "implicit.c"], Expect::Diagnostics),
    (&["/c", "pointers.c"], Expect::Diagnostics),
    (&["/c", "initializer.c"], Expect::Diagnostics),
    (&["/WX", "/c", "initializer.c"], Expect::Results),
    (&["/wd4047", "/c", "pointers.c"], Expect::Diagnostics),
    (&["/W0", "/c", "pointers.c"], Expect::Diagnostics),
    (&["/WX", "/c", "pointers.c"], Expect::Results),
    (&["/c", "cast.c"], Expect::Diagnostics),
    // Command files.
    (&["@options.rsp"], Expect::Output),
    (&["@define.rsp"], Expect::Results),
    (&["/c", "@missing.rsp", "a.c"], Expect::Output),
];

/// Command lines compared with the `CL` and `_CL_` environment variables
/// set, after `/nologo`.
const ENVIRONMENT_CASES: &[(&[&str], &str, &str)] = &[
    (&["a.c"], "/c /Foenvironment.obj", ""),
    (&["/Fofirst.obj", "a.c"], "/c", "/Folast.obj"),
    (&["/c", "pp.c"], "/DVALUE#2", "/Fodefined.obj"),
];

/// What one compiler did with one command line.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    status:  Option<i32>,
    created: BTreeSet<String>,
    stdout:  String,
    stderr:  String,
}

#[test]
fn command_lines_match_cl() {
    let Some(msvc) = msvc() else {
        eprintln!("skipping: no Visual Studio C compiler on this host");
        return;
    };
    let root = scratch("cli");
    let mut differences = Differences::default();
    for (index, &(case, expect)) in CASES.iter().enumerate() {
        let arguments: Vec<&str> = if case.first().is_some_and(|first| first.starts_with('-')) {
            case.to_vec()
        } else {
            std::iter::once("/nologo")
                .chain(case.iter().copied())
                .collect()
        };
        let cl = run(
            msvc,
            &root.join(format!("{index}-cl")),
            true,
            &arguments,
            &[],
        );
        let bcc = run(
            msvc,
            &root.join(format!("{index}-bcc")),
            false,
            &arguments,
            &[],
        );
        println!("\n==== {}", arguments.join(" "));
        for (name, run) in [("cl", &cl), ("bcc", &bcc)] {
            println!("{name}: status {:?}, created {:?}", run.status, run.created);
            for (stream, text) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
                for line in text.lines() {
                    println!("{name} {stream}| {line}");
                }
            }
        }
        if expect == Expect::Documented {
            continue;
        }
        let label = arguments.join(" ");
        differences.compare(&format!("{label}: status"), &cl.status, &bcc.status);
        differences.compare(&format!("{label}: files"), &cl.created, &bcc.created);
        if expect == Expect::Output {
            differences.compare(&format!("{label}: stdout"), &cl.stdout, &bcc.stdout);
            differences.compare(&format!("{label}: stderr"), &cl.stderr, &bcc.stderr);
        }
        if expect == Expect::Diagnostics {
            differences.compare(
                &format!("{label}: diagnostics"),
                &coded(&cl.stdout),
                &coded(&bcc.stderr),
            );
        }
    }
    for (index, &(case, prepended, appended)) in ENVIRONMENT_CASES.iter().enumerate() {
        let arguments: Vec<&str> = std::iter::once("/nologo")
            .chain(case.iter().copied())
            .collect();
        let variables = [("CL", prepended), ("_CL_", appended)];
        let cl = run(
            msvc,
            &root.join(format!("env{index}-cl")),
            true,
            &arguments,
            &variables,
        );
        let bcc = run(
            msvc,
            &root.join(format!("env{index}-bcc")),
            false,
            &arguments,
            &variables,
        );
        let label = format!("CL={prepended} _CL_={appended} {}", arguments.join(" "));
        println!("\n==== {label}");
        for (name, run) in [("cl", &cl), ("bcc", &bcc)] {
            println!("{name}: status {:?}, created {:?}", run.status, run.created);
        }
        differences.compare(&format!("{label}: status"), &cl.status, &bcc.status);
        differences.compare(&format!("{label}: files"), &cl.created, &bcc.created);
        differences.compare(&format!("{label}: stdout"), &cl.stdout, &bcc.stdout);
    }
    drop(fs::remove_dir_all(root));
    differences.assert_none("command line");
}

/// The diagnostics with a code in `text`, without the directory before each
/// file name: `bad.c(2): error C2065: 'missing': undeclared identifier`.
fn coded(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let (location, rest) = line.split_once("): ")?;
            let (severity, message) = rest.split_once(": ")?;
            let (kind, code) = severity.split_once(' ')?;
            let coded =
                matches!(kind, "error" | "warning") && code.len() == 5 && code.starts_with('C');
            let file = location.rsplit(['\\', '/']).next()?;
            coded.then(|| format!("{file}): {severity}: {message}"))
        })
        .collect()
}

/// Runs `cl` or `bcc --msvc-mode` with `arguments` and the environment
/// `variables` in a fresh `directory` holding [`FILES`] and an empty `obj`.
fn run(
    msvc: &Msvc,
    directory: &Path,
    cl: bool,
    arguments: &[&str],
    variables: &[(&str, &str)],
) -> Run {
    fs::create_dir_all(directory.join("inc")).unwrap();
    fs::create_dir_all(directory.join("obj")).unwrap();
    for (name, text) in FILES {
        fs::write(directory.join(name), text).unwrap();
    }
    let before = listing(directory);
    let mut command = if cl { msvc.cl() } else { msvc.bcc() };
    for &(name, value) in variables {
        _ = if value.is_empty() {
            command.env_remove(name)
        } else {
            command.env(name, value)
        };
    }
    let output = run_native(command.current_dir(directory).args(arguments)).unwrap();
    // `cl /Zi` writes a program database, which bcc does not.
    let created = listing(directory)
        .difference(&before)
        .filter(|name| {
            !Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pdb"))
        })
        .cloned()
        .collect();
    let normalize = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
        let path = directory.display().to_string();
        text.replace(&path, "<dir>")
            .replace(&path.to_lowercase(), "<dir>")
    };
    Run {
        status: output.status.code(),
        created,
        stdout: normalize(&output.stdout),
        stderr: normalize(&output.stderr),
    }
}

/// The files under `directory`, by relative path with `/`.
fn listing(directory: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(relative) = path.strip_prefix(directory) {
                _ = names.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    names
}
