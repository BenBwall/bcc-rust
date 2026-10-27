use std::{
    ffi::OsString,
    path::Path,
};

use super::*;

#[test]
fn executable_names_select_personalities() {
    let named = |name: &str| Personality::from_program(Path::new(name));
    assert_eq!(named("gcc"), Some((Personality::Gcc, None)));
    assert_eq!(named("/usr/bin/cc"), Some((Personality::Gcc, None)));
    assert_eq!(named("gcc-14"), Some((Personality::Gcc, None)));
    assert_eq!(named("bcc-gcc"), Some((Personality::Gcc, None)));
    assert_eq!(named("clang-18.1"), Some((Personality::Clang, None)));
    assert_eq!(named("CL.EXE"), Some((Personality::Msvc, None)));
    assert_eq!(named("clang-cl"), Some((Personality::ClangCl, None)));
    assert_eq!(named("bcc-clang-cl"), Some((Personality::ClangCl, None)));
    assert_eq!(named("tcc"), Some((Personality::Tcc, None)));
    assert_eq!(
        named("x86_64-w64-mingw32-gcc"),
        Some((Personality::Gcc, Some(Target::WindowsGnu)))
    );
    assert_eq!(
        named("x86_64-linux-musl-gcc"),
        Some((Personality::Gcc, Some(Target::LinuxMusl)))
    );
    assert_eq!(named("bcc-rust"), None);
    assert_eq!(named("bcc"), None);
}

#[test]
fn mode_flags_select_personalities_anywhere() {
    let mut arguments: Vec<OsString> = ["bcc-rust", "-c", "--msvc-mode", "x.c"]
        .map(Into::into)
        .into();
    assert_eq!(select(&mut arguments), Some(Personality::Msvc));
    assert_eq!(arguments, ["bcc-rust", "-c", "x.c"].map(OsString::from));
    let mut arguments: Vec<OsString> = ["bcc-rust", "--driver-mode=clang-cl"]
        .map(Into::into)
        .into();
    assert_eq!(select(&mut arguments), Some(Personality::ClangCl));
    let mut arguments: Vec<OsString> = ["bcc-rust", "x.c"].map(Into::into).into();
    assert_eq!(select(&mut arguments), None);
}

/// The bcc command lines `personality` plans for `arguments`.
fn planned(personality: Personality, arguments: &[&str]) -> Vec<Vec<String>> {
    let mut command_line: Vec<OsString> = vec!["bcc".into()];
    command_line.extend(arguments.iter().map(OsString::from));
    let invocation = Invocation::new(personality, command_line);
    let Ok(plan) = msvc::plan(&invocation) else {
        panic!("the command line is valid")
    };
    plan.steps
        .into_iter()
        .filter_map(|step| match step {
            | Step::Bcc(arguments) => Some(
                arguments
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
            ),
            | _ => None,
        })
        .collect()
}

#[test]
fn cl_links_the_runtime_library_it_selects() {
    let link = |options: &[&str]| {
        let mut arguments = vec!["/nologo"];
        arguments.extend(options);
        arguments.push("a.c");
        planned(Personality::Msvc, &arguments)
            .pop()
            .expect("a link step")
    };
    // Clang's driver links libcmt, the static runtime, by default.
    assert!(
        !link(&[])
            .iter()
            .any(|argument| argument.contains("defaultlib"))
    );
    assert!(
        !link(&["/MD", "/MT"])
            .iter()
            .any(|argument| argument.contains("defaultlib"))
    );
    for (options, library) in [
        (&["/MD"][..], "msvcrt"),
        (&["/MT", "/MDd"], "msvcrtd"),
        (&["/MTd"], "libcmtd"),
        (&["/LDd"], "libcmtd"),
    ] {
        let step = link(options);
        assert!(
            step.iter()
                .any(|argument| argument == "-Wl,-nodefaultlib:libcmt"),
            "{options:?}: {step:?}"
        );
        assert!(
            step.iter()
                .any(|argument| *argument == format!("-Wl,-defaultlib:{library}")),
            "{options:?}: {step:?}"
        );
    }
    // `/Zs` plans only checks: no object, no link.
    let checks = planned(Personality::Msvc, &["/nologo", "/Zs", "a.c", "b.c"]);
    assert_eq!(checks.len(), 2);
    for step in checks {
        assert!(
            step.iter().any(|argument| argument == "--syntax-only"),
            "{step:?}"
        );
        assert!(
            !step
                .iter()
                .any(|argument| argument == "-c" || argument == "-o"),
            "{step:?}"
        );
    }
    // A command line of file names alone still selects C17 and the column
    // diagnostics.
    let alone = planned(Personality::Msvc, &["a.c"]);
    assert!(
        alone[0].iter().any(|argument| argument == "-std=c17"),
        "{alone:?}"
    );
    assert!(
        alone[0]
            .iter()
            .any(|argument| argument == "--diagnostic-format=msvc"),
        "{alone:?}"
    );
}
