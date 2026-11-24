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
    assert_eq!(named("zig-cc"), Some((Personality::ZigCc, None)));
    assert_eq!(named("zigcc"), Some((Personality::ZigCc, None)));
    assert_eq!(named("zig"), Some((Personality::ZigCc, None)));
    assert_eq!(named("bcc-zig-cc"), Some((Personality::ZigCc, None)));
    assert_eq!(named("zig-0.17"), Some((Personality::ZigCc, None)));
    let host_icx = if cfg!(windows) {
        Personality::IcxCl
    } else {
        Personality::Icx
    };
    assert_eq!(named("icx"), Some((host_icx, None)));
    assert_eq!(named("icx-2026"), Some((host_icx, None)));
    assert_eq!(named("bcc-icx"), Some((host_icx, None)));
    assert_eq!(named("icx-cc"), Some((Personality::Icx, None)));
    assert_eq!(named("icx-cl"), Some((Personality::IcxCl, None)));
    assert_eq!(named("ICX-CL.EXE"), Some((Personality::IcxCl, None)));
    assert_eq!(named("bcc-icx-cl"), Some((Personality::IcxCl, None)));
    // Only a whole word names icx: these are `cl` and GCC, as before icx.
    assert_eq!(named("myicx-cl"), Some((Personality::Msvc, None)));
    assert_eq!(named("gicx-cc"), Some((Personality::Gcc, None)));
    assert_eq!(named("icc"), None);
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
    let mut arguments: Vec<OsString> = ["bcc-rust", "--zig-cc-mode"].map(Into::into).into();
    assert_eq!(select(&mut arguments), Some(Personality::ZigCc));
    let mut arguments: Vec<OsString> = ["bcc-rust", "--icx-mode"].map(Into::into).into();
    assert_eq!(select(&mut arguments), Some(Personality::Icx));
    let mut arguments: Vec<OsString> = ["bcc-rust", "--driver-mode=icx-cl"].map(Into::into).into();
    assert_eq!(select(&mut arguments), Some(Personality::IcxCl));
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
            | Step::Bcc(arguments) | Step::Link(arguments) => Some(
                arguments
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
            ),
            | _ => None,
        })
        .collect()
}

/// clang-cl's `-shared` links a DLL under the executable's name, and each
/// spelling of `-o` names the output clang-cl's driver names: Clang's `-o`
/// as it is, whatever `/Fo` and `/Fe` say, and `cl`'s `/o` as the last of
/// it and `/Fo` or `/Fe` does, except under `/E` (#452).
#[test]
fn clang_cl_plans_the_outputs_clang_cl_names() {
    // clang-cl looks for its inputs.
    let directory = std::env::temp_dir().join(format!("bcc-cl-outputs-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("a.c");
    std::fs::write(&source, "int x;\n").unwrap();
    let source = source.to_str().unwrap();
    let last = |arguments: &[&str]| {
        let arguments: Vec<&str> = arguments
            .iter()
            .map(|&argument| if argument == "a.c" { source } else { argument })
            .collect();
        planned(Personality::ClangCl, &arguments)
            .pop()
            .expect("a step")
    };
    let output = |step: &[String]| {
        step.iter()
            .position(|argument| argument == "-o")
            .map(|index| step[index + 1].clone())
    };
    let link = last(&["a.c", "/clang:-shared"]);
    assert!(
        link.iter().any(|argument| argument == "-shared"),
        "{link:?}"
    );
    assert_eq!(output(&link).as_deref(), Some("a.exe"));
    let compile = last(&["/c", "a.c", "/clang:-shared"]);
    assert!(!compile.iter().any(|argument| argument == "-shared"));
    for (arguments, expected) in [
        (&["/c", "a.c", "-o", "x.obj"][..], Some("x.obj")),
        (&["/c", "a.c", "-ox.obj"], Some("x.obj")),
        (&["/c", "a.c", "/o", "x"], Some("x.obj")),
        (&["/c", "a.c", "-objectx"], Some("bjectx.obj")),
        // clang-cl's `/openmp-`, the default, is no `-o penmp-`.
        (&["/c", "a.c", "-openmp-"], Some("a.obj")),
        (&["/c", "a.c", "-o", "x.obj", "/Foy.obj"], Some("y.obj")),
        (&["/c", "a.c", "/Foy.obj", "-o", "x.obj"], Some("x.obj")),
        (&["/c", "a.c", "/clang:-o", "/clang:x"], Some("x")),
        (&["/c", "a.c", "/clang:-ox.obj", "/Foy.obj"], Some("x.obj")),
        (&["/c", "a.c", "/clang:--output=x.obj"], Some("x.obj")),
        (&["a.c", "-o", "x"], Some("x.exe")),
        (&["a.c", "/Fey.exe", "-o", "x.exe"], Some("x.exe")),
        (&["a.c", "-o", "x.exe", "/Fey.exe"], Some("y.exe")),
        (&["a.c", "/clang:-o", "/clang:x"], Some("x")),
        (&["a.c", "/clang:-ox.exe", "/Fey.exe"], Some("x.exe")),
        (&["a.c", "-openmp-"], Some("a.exe")),
        (&["/E", "a.c", "/clang:-o", "/clang:x.i"], Some("x.i")),
        (&["/EP", "a.c", "/clang:-ox.i"], Some("x.i")),
        (&["/P", "a.c", "/clang:-ox.i"], Some("x.i")),
        (&["/P", "a.c", "/clang:-o", "/clang:-"], None),
        (&["/E", "a.c", "-o", "x.i"], None),
        (&["/P", "a.c", "-o", "x.i"], Some("a.i")),
        (&["/P", "a.c", "-openmp-"], Some("a.i")),
        // clang-cl's `/P` writes its file beside `/E`.
        (&["/P", "/E", "a.c"], Some("a.i")),
    ] {
        assert_eq!(
            output(&last(arguments)).as_deref(),
            expected,
            "{arguments:?}"
        );
    }
    drop(std::fs::remove_dir_all(&directory));
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
    // A command line of file names alone still selects C17 and the
    // diagnostics `cl` writes without `/diagnostics:`, `file(line):`.
    let alone = planned(Personality::Msvc, &["a.c"]);
    assert!(
        alone[0].iter().any(|argument| argument == "-std=c17"),
        "{alone:?}"
    );
    assert!(
        alone[0]
            .iter()
            .any(|argument| argument == "--diagnostic-format=msvc-classic"),
        "{alone:?}"
    );
}

/// clang-cl reads `/O`'s letters in order and sets the level from the last
/// `1`, `2`, `x` or `d` on the command line alone (`TranslateOptArg` in
/// Clang's `MSVC.cpp`), so `/O2s` asks for size and `/Os2` for speed; `-O`
/// is the same option, and `-O3` none of Clang's. `cl` reads combined
/// letters as the options they name (Microsoft Learn, "/O options
/// (Optimize code)": "/Odi is the same as /Od /Oi").
#[test]
fn combined_optimization_letters_read_as_each_driver_reads_them() {
    let directory = std::env::temp_dir().join(format!("bcc-cl-letters-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("a.c");
    std::fs::write(&source, "int x;\n").unwrap();
    let source = source.to_str().unwrap();
    let optimization = |personality: Personality, options: &[&str]| {
        let mut arguments = vec!["/nologo", "/c"];
        arguments.extend(options);
        arguments.push(source);
        let step = planned(personality, &arguments).pop().expect("a step");
        step.into_iter()
            .filter(|argument| {
                matches!(
                    argument.as_str(),
                    "-O0" | "-Os" | "-O2" | "-O3" | "-fbuiltin" | "-fno-builtin" | "-fno-inline"
                )
            })
            .collect::<Vec<_>>()
    };
    for (options, expected) in [
        (&["/O2s"][..], &["-fbuiltin", "-O2", "-Os"][..]),
        (&["-O2s"], &["-fbuiltin", "-O2", "-Os"]),
        (&["/Os2"], &["-Os", "-fbuiltin", "-O2"]),
        (&["/Oxs"], &["-fbuiltin", "-O2", "-Os"]),
        // Only the last level letter sets a level, wherever it stands.
        (&["/O1", "/O2"], &["-fbuiltin", "-O2"]),
        (&["/O12", "/Os"], &["-fbuiltin", "-O2", "-Os"]),
        (&["/Od", "/Ob0"], &["-O0", "-fno-inline"]),
        (&["/O2", "/Ob0"], &["-fbuiltin", "-O2", "-fno-inline"]),
        // The last of `/Ob0` and `/clang:-finline` decides, and `/Ob2` is
        // `-finline-functions`, which a `-fno-inline` overrides.
        (&["/O2", "/Ob0", "/clang:-finline"], &["-fbuiltin", "-O2"]),
        (
            &["/O2", "/Ob0", "/Ob2"],
            &["-fbuiltin", "-O2", "-fno-inline"],
        ),
        (
            &["/O2", "/clang:-fno-inline-functions"],
            &["-fbuiltin", "-O2", "-fno-inline"],
        ),
        (
            &["/O2", "/clang:-fno-inline-functions", "/Ob2"],
            &["-fbuiltin", "-O2", "-fno-inline"],
        ),
        (&["/Oi-2"], &["-fno-builtin", "-fbuiltin", "-O2"]),
        (&["/Ob12"], &["-fbuiltin", "-O2"]),
        // A letter after `b` is its value, so `x` sets nothing here.
        (&["/Obx"], &[]),
        (&["-O3"], &[]),
        (&["-Ofast"], &["-Os", "-O2"]),
    ] {
        assert_eq!(
            optimization(Personality::ClangCl, options),
            expected,
            "{options:?}"
        );
    }
    // icx-cl's `/O3` is a level, which the last level letter overrides.
    for (options, expected) in [
        (&["/O3"][..], &["-O3"][..]),
        (&["-O3"], &["-O3"]),
        (&["/O3", "/O2"], &["-fbuiltin", "-O2"]),
        (&["/O2", "/O3"], &["-O3"]),
        (&["/O3", "/Os"], &["-O3", "-Os"]),
    ] {
        assert_eq!(
            optimization(Personality::IcxCl, options),
            expected,
            "{options:?}"
        );
    }
    for (options, expected) in [
        (&["/O2s"][..], &["-O2", "-Os"][..]),
        (&["-Oxs"], &["-O2", "-Os"]),
        (&["/Odi"], &["-O0"]),
        (&["/Ob2i-t"], &["-O2"]),
        (&["/O2b9"], &[]),
    ] {
        assert_eq!(
            optimization(Personality::Msvc, options),
            expected,
            "{options:?}"
        );
    }
    drop(std::fs::remove_dir_all(&directory));
}
