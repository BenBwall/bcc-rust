//! The `cl.exe` and clang-cl personalities. `cl` options start with `/` or
//! `-`, are case-sensitive, and mostly take their value joined (`/Fefoo.exe`,
//! `/Ifoo`; `/I`, `/D`, `/U` and `/FI` also take the next argument). `cl`
//! compiles each source to an object named after it (`a.c` to `a.obj`),
//! keeps the objects, and links an executable named after the first source
//! unless `/c` stops it; `/link` passes everything after it to the linker.
//!
//! The personality translates the options into bcc's, one compile step per
//! source and a link step, for the `x86_64-pc-windows-msvc` target with all
//! of bcc's Microsoft extensions, MSVC's predefined macros, MSVC's default
//! C mode and its diagnostic format. Like `cl`, it prints its banner to
//! stderr unless `/nologo` is given, the name of each source it compiles,
//! and warns about an option it does not know (warning D9002) instead of
//! failing. Of the options `cl` knows that bcc does not implement, it
//! ignores those that cannot change the program, warns about hardening, and
//! refuses the rest with a `Command line error`; `/Zp`, `/Za`, the runtime
//! options and `/Zc:trigraphs` map onto bcc's packing, Microsoft extension,
//! macro and trigraph options. clang-cl knows every option it takes, so its
//! own options none of these translate are read in `own_options`, which
//! warns as clang-cl does about those it leaves unused and refuses those
//! that would change what a step writes. The command line is read with the `CL`
//! and `_CL_` environment variables around it and each command file (`@file`)
//! in its place (`command_files`). The warning level and `/wd`, `/we` and
//! `/w1`-`/w4` control the warnings bcc reports with `cl`'s code.
//!
//! An argument that starts with `/` is an option only if clang-cl knows it
//! (`known::clang_cl_option`), which is decided before any option is
//! translated: clang-cl reads any other as a path, on every host, so
//! `/tmp/a.c` is a source. `cl` on Windows, where paths do not start with
//! `/`, warns about it as an unknown option; elsewhere the `cl` personality
//! reads it as clang-cl does, except for the few `cl` options clang-cl does
//! not know (`/Zf`, `/analyze`), a file inside a directory, which is an
//! input whatever option its name starts with, and a name without a
//! directory that is no file, which is an option (`cl_slash_argument`).
//! Both read every argument after
//! `--` as an input there, for a path that clang-cl would read as an option
//! (`/Users/a.c` is `/U`).

use std::{
    ffi::{
        OsStr,
        OsString,
    },
    path::{
        Path,
        PathBuf,
    },
};

use crate::configuration::CL_CODED_WARNINGS;

mod command_files;
mod optimization;
mod own_options;

use command_files::CommandFileError;

use super::{
    Invocation,
    MainError,
    Personality,
    Plan,
    Step,
    host_target,
    icx,
    inlining::ClangInlining,
    known::{
        self,
        ClOption,
        ClangIgnored,
        ClangOption,
        ClangStep,
        ClangVisibility,
    },
    unimplemented::{
        Treatment,
        treatment,
    },
    version,
};
use crate::{
    backend::llvm::operand,
    cli::{
        arguments::implements_language_option,
        warning_options,
    },
    target::{
        UNKNOWN_VECTOR_ABI,
        X86Features,
    },
};

/// Translates a `cl.exe` or clang-cl command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let clang_cl = invocation.personality.is_clang_cl();
    // Whether an argument that starts with `/` and names no option is a
    // path: always in clang-cl, and in `cl` off Windows.
    let slash_paths = clang_cl || !cfg!(windows);
    // After `--`, every argument is an input.
    let mut inputs_only = false;
    let mut state = State {
        // `cl` writes `file(line):` unless `/diagnostics:column` asks for
        // the column; clang-cl writes `file(line,col):` and the snippet, as
        // `/diagnostics:caret` asks.
        diagnostics: if clang_cl {
            "msvc-caret"
        } else {
            "msvc-classic"
        },
        ..State::default()
    };
    // clang-cl reads its `/clang:` options after the others, wherever they
    // stand, so they win over `cl`'s (`/clang:-fsigned-char` over `/J`).
    let mut clang_arguments: Vec<&str> = Vec::new();
    let mut clang_options: Vec<OsString> = Vec::new();
    // clang-cl's `-m` and `-march=` options in order, for how vectors travel.
    let mut x86_options: Vec<&str> = Vec::new();
    // `CL`, the command line with its command files, then `_CL_`.
    let expanded = match command_files::expand(&invocation.arguments, clang_cl) {
        | Ok(arguments) => arguments,
        | Err(CommandFileError::Unreadable(file)) => {
            state.command_line_error(
                invocation,
                Some("D8022"),
                format_args!("cannot open '{}'", file.to_string_lossy()),
            );
            return Err(MainError::Reported(ERROR_STATUS));
        },
        | Err(CommandFileError::Recursive(file)) => {
            // LLVM's own words, before the driver exists to name itself.
            super::print(
                &format!("recursive expansion of: '{}'\n", file.display()),
                true,
            );
            return Err(MainError::Reported(1));
        },
    };
    // `cl` prints its banner before anything it says about the command
    // line, unless `/nologo` stands anywhere before `/link`. Meson reads the
    // banner `cl /?` prints to recognize `cl` and its version.
    let nologo = expanded
        .iter()
        .take_while(|argument| !matches!(argument.to_str(), Some("/link" | "-link" | "--")))
        .any(|argument| matches!(argument.to_str(), Some("/nologo" | "-nologo")));
    if !clang_cl && !nologo {
        super::print(
            &version::banner(
                invocation.personality,
                &invocation.name,
                crate::target::Target::WindowsMsvc,
            ),
            true,
        );
    }
    // `icx-cl` prints its logo first, `/nologo` or not.
    let intel =
        (invocation.personality == Personality::IcxCl).then(|| icx::ClDefaults::read(&expanded));
    if intel.is_some() {
        super::print(&icx::cl_logo(), true);
    }
    // clang-cl reads its warning options before it parses the others.
    let unknown_argument = unknown_argument_severity(&expanded);
    state.unused_severity = unused_argument_severity(&expanded);
    // The `/O` letter that sets clang-cl's level, wherever it stands.
    state.level_letter =
        optimization::clang_cl_level_letter(optimization_options(&expanded), intel.is_some());
    let mut arguments = expanded.iter().map(OsString::as_os_str);
    while let Some(argument) = arguments.next() {
        if state.linking {
            state.link.push(link_option(argument));
            continue;
        }
        let Some(text) = argument.to_str().filter(|_| !inputs_only) else {
            state.sources.push(Source::new(argument, false));
            continue;
        };
        if slash_paths && text == "--" {
            inputs_only = true;
            continue;
        }
        // clang-cl reads `-` as standard input.
        let Some(option) = text
            .strip_prefix('/')
            .or_else(|| text.strip_prefix('-'))
            .filter(|_| !(clang_cl && text == "-"))
        else {
            state.sources.push(Source::new(argument, false));
            continue;
        };
        if intel.is_some() && state.intel_option(invocation, option, &mut arguments) {
            continue;
        }
        // Whether the argument is an option at all is decided first, so no
        // handler below that matches a prefix reads a path.
        let known = known::clang_cl_option(option);
        let slash = if slash_paths && text.starts_with('/') && !clang_cl {
            cl_slash_argument(text)
        } else {
            SlashArgument::AsClangCl
        };
        if slash == SlashArgument::Input
            || (slash == SlashArgument::AsClangCl
                && slash_paths
                && text.starts_with('/')
                && known.is_none()
                && (clang_cl || !is_cl_only_option(option)))
        {
            // clang-cl's option parser reads an argument that starts with
            // `/` and matches no option as an input file.
            state.sources.push(Source::new(argument, false));
            continue;
        }
        if matches!(option, "?" | "help" | "HELP") {
            return Ok(Plan::print(help(&invocation.name)));
        }
        if clang_cl && let Some(plan) = clang_cl_query(invocation, text) {
            return Ok(plan);
        }
        // clang-cl's `-v` and `-###` print its version even without an
        // input; with one they reach bcc's driver.
        state.verbose |= clang_cl && matches!(text, "-v" | "-###");
        if clang_cl && let Some(clang) = option.strip_prefix("clang:") {
            // `/clang:OPTION` gives clang-cl a Clang option, read below.
            clang_arguments.push(clang);
            continue;
        }
        // clang-cl's `/openmp` flags, which `/o` below would otherwise
        // read: the last one wins, `/openmp-` is the default, and bcc
        // implements no OpenMP for the others, which define `_OPENMP` and
        // link its runtime (refused after the loop).
        if clang_cl && matches!(option, "openmp" | "openmp-" | "openmp:experimental") {
            state.openmp = (option != "openmp-").then(|| text.to_owned());
            continue;
        }
        // clang-cl's `/o`, also `-o`, joined (`-ox.obj`, and `-objectx` too)
        // or separate. The Clang option `-object-file-name` is a longer
        // match.
        if clang_cl
            && let Some(value) = option.strip_prefix('o')
            && !option.starts_with("object-file-name")
        {
            state.output_option(invocation, text, value, &mut arguments);
            continue;
        }
        if clang_cl && let Some(directory) = option.strip_prefix("imsvc") {
            state.imsvc(invocation, text, directory, &mut arguments);
            continue;
        }
        if let Some(letters) = option.strip_prefix('O') {
            let index = expanded.len() - arguments.len() - 1;
            state.optimization_option(invocation, index, text, letters);
            continue;
        }
        if known.is_some() && state.own_option(invocation, text, option, &mut arguments) {
            continue;
        }
        let remaining = arguments.len();
        if state.option(invocation, option, &mut arguments) {
            // The bare name of an option clang-cl reads with the next
            // argument as its value (`/Fp: x.pch`) takes it, also where bcc
            // ignores the option.
            if known == Some(ClOption::TakesNext) && arguments.len() == remaining {
                _ = arguments.next();
            }
        } else if clang_cl
            && text.starts_with('-')
            && known.is_none()
            && let Some(unknown) = ignored_option(text, intel.is_some())
        {
            // An option clang-cl's driver ignores (`-ffreestanding`), which
            // `/clang:` gives its front end.
            if state.unknown_argument(invocation, unknown_argument, text, &unknown) {
                return Err(MainError::Reported(1));
            }
        } else if clang_cl && text.starts_with('-') {
            // clang-cl also takes Clang's options.
            if state.bare_clang_option(
                invocation,
                text,
                &mut arguments,
                unknown_argument,
                &mut x86_options,
            ) {
                return Err(MainError::Reported(1));
            }
        } else {
            // An option clang-cl knows takes its value from the next
            // argument after its bare name (`/vctoolsdir /opt/vc`), which
            // is then no input.
            let mut text = text.to_owned();
            if known == Some(ClOption::TakesNext)
                && let Some(value) = arguments.next()
            {
                text.push(' ');
                text.push_str(&value.to_string_lossy());
            }
            if clang_cl {
                // Every option clang-cl knows is read above; one a newer
                // option table adds is refused until bcc reads it.
                state.command_line_error(
                    invocation,
                    None,
                    format_args!("'{text}' is not supported: bcc does not implement it"),
                );
            } else {
                super::print(
                    &format!(
                        "{} : Command line warning D9002 : ignoring unknown option '{text}'\n",
                        invocation.name
                    ),
                    true,
                );
            }
        }
    }
    state.check_openmp(invocation);
    if state.read_clang_arguments(
        invocation,
        &clang_arguments,
        unknown_argument,
        &mut x86_options,
        &mut clang_options,
    ) {
        return Err(MainError::Reported(1));
    }
    state.read_clang_options_by_step(invocation, &mut clang_options);
    let debug_information = state
        .bcc
        .iter()
        .chain(&clang_options)
        .any(|option| is_debug_information(&option.to_string_lossy()));
    state.check_own_options(invocation, debug_information);
    if state.all_c {
        for source in &mut state.sources {
            let extension = Path::new(&source.path)
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_ascii_lowercase);
            if !matches!(
                extension.as_deref(),
                Some("obj" | "o" | "lib" | "a" | "res" | "def")
            ) {
                source.c = true;
                source.cpp = false;
            }
        }
    }
    if state.compile_only
        && let Some(object) = state.object_from_o.take()
    {
        state.object = Some(object);
    }
    state.check_inputs(invocation);
    if state.warn_ignored(invocation, &expanded) {
        return Err(MainError::Reported(1));
    }
    let c_count = state.sources.iter().filter(|source| source.is_c()).count();
    state.check_output_names(invocation, c_count);
    if state.report_unused(invocation) {
        return Err(MainError::Reported(1));
    }
    if state.errors != 0 {
        return Err(MainError::Reported(error_status(invocation.personality)));
    }
    let target = invocation
        .target
        .unwrap_or_else(|| host_target(invocation.personality));
    let runtime = state.runtime.unwrap_or(if state.debug_dll {
        Runtime::StaticDebug
    } else {
        Runtime::Static
    });
    let mut common: Vec<OsString> = vec![
        invocation.program.clone(),
        if clang_cl {
            "--compiler-identity=clang-cl"
        } else {
            "--compiler-identity=msvc"
        }
        .into(),
        format!("--diagnostic-format={}", state.diagnostics).into(),
        format!("--target={}", target.triple()).into(),
        format!("--driver-name={}", invocation.name).into(),
        "-fms-extensions".into(),
        // MSVC's default C mode is close to C17 without its version macro;
        // clang-cl claims C17, and so does bcc.
        format!("-std={}", state.standard).into(),
        // `cl` replaces trigraphs only under `/Zc:trigraphs`.
        "-fno-trigraphs".into(),
    ];
    // `cl` makes a tentative definition a communal symbol, so two units
    // that both declare `int x;` link; clang-cl does not, as Clang 11 and
    // later do not. `cl` never colors its output.
    if !clang_cl {
        common.push("-fcommon".into());
        common.push("-fno-color-diagnostics".into());
    }
    // `cl` claims the C11 features it lacks only under `/std:`, where it
    // defines `__STDC_VERSION__`: atomics, complex types and VLAs, which
    // bcc has. `<threads.h>` comes with the Visual C++ headers, so
    // `__STDC_NO_THREADS__` stays undefined.
    if !clang_cl {
        if state.standard_given {
            common.extend(
                [
                    "-D__STDC_NO_ATOMICS__=1",
                    "-D__STDC_NO_COMPLEX__=1",
                    "-D__STDC_NO_VLA__=1",
                ]
                .map(OsString::from),
            );
        }
        common.push("-U__STDC_NO_THREADS__".into());
        if state.stdc {
            common.push("-D__STDC__=1".into());
        }
    }
    if !state.extensions {
        // `/Za`: ISO C without Microsoft's anonymous tagged members, and
        // without `_MSC_EXTENSIONS`; the `__`-prefixed keywords stay.
        common.push("-fno-ms-anonymous-structs".into());
        common.push("-U_MSC_EXTENSIONS".into());
    }
    // The runtime library `/MD`, `/MT` and their debug forms select, the
    // last one given winning: `_DLL` for the DLL runtime, `_DEBUG` for the
    // debug one (`_MT` is always defined).
    if matches!(runtime, Runtime::Dll | Runtime::DllDebug) {
        common.push("-D_DLL".into());
    }
    if matches!(runtime, Runtime::StaticDebug | Runtime::DllDebug) {
        common.push("-D_DEBUG".into());
    }
    common.append(&mut state.system_directories());
    // Intel's defaults come before the command line's own options.
    if let Some(intel) = &intel {
        intel.apply(&invocation.name, &mut common, target);
    }
    common.append(&mut state.bcc);
    if X86Features::from_options(&x86_options.iter().copied(), true).unknown_vector_abi()
        == Some(32)
    {
        common.push(UNKNOWN_VECTOR_ABI.into());
    }
    if !clang_cl {
        common.extend(state.coded_warning_options());
        common.push(format!("--cl-warning-level={}", state.warning_level).into());
        for &(code, setting) in &state.warning_settings {
            let value = match setting {
                | WarningSetting::Disabled => "d".to_owned(),
                | WarningSetting::Error => "e".to_owned(),
                | WarningSetting::Level(level) => level.to_string(),
                | WarningSetting::Once => "o".to_owned(),
            };
            common.push(format!("--cl-warning={code}:{value}").into());
        }
    }
    common.append(&mut clang_options);
    if let Some(option) = state.inlining.option() {
        common.push(option.into());
    }
    // `cl` compiles every source even after one fails, then skips the link;
    // clang-cl stops at the first failure.
    let mut plan = Plan {
        keep_going: !clang_cl,
        ..Plan::default()
    };
    // clang-cl's `-v` and `-###` print its version without inputs, and
    // `-###` runs nothing with them either.
    if clang_cl && state.verbose && (state.sources.is_empty() || state.dry_run) {
        plan.steps.push(Step::Print {
            text:   version::verbose(invocation.personality, target),
            stderr: true,
        });
        return Ok(plan);
    }
    if state.sources.is_empty() {
        // clang-cl refuses a command line without inputs, as Clang does.
        // `cl` alone prints its usage to stdout after its banner; with
        // options and no file it stops with error D8003.
        if clang_cl {
            invocation.error(format_args!("no input files"));
            return Err(MainError::Reported(error_status(invocation.personality)));
        }
        if expanded.is_empty() {
            super::print(
                &format!(
                    "usage: {} [ option... ] filename... [ /link linkoption... ]\n",
                    invocation.name
                ),
                false,
            );
            return Ok(plan);
        }
        state.command_line_error(
            invocation,
            Some("D8003"),
            format_args!("missing source filename"),
        );
        return Err(MainError::Reported(ERROR_STATUS));
    }
    let c_sources: Vec<&Source> = state
        .sources
        .iter()
        .filter(|source| source.is_c())
        .collect();
    if let Some(source) = state.sources.iter().find(|source| source.cpp) {
        invocation.error(format_args!(
            "'{}': bcc compiles C only; C++ sources are not supported",
            Path::new(&source.path).display()
        ));
        return Err(MainError::Reported(error_status(invocation.personality)));
    }
    // Clang's `-o` names the one output of a step that does not link, which
    // clang-cl's driver refuses for several (`Driver::BuildJobs`).
    if state.clang_output.is_some()
        && (state.compile_only || state.preprocess.is_some())
        && c_sources.len() > 1
    {
        invocation.error(format_args!(
            "cannot specify -o when generating multiple output files"
        ));
        return Err(MainError::Reported(error_status(invocation.personality)));
    }
    if let Some(mode) = state.preprocess {
        preprocessing_steps(invocation, &state, mode, &common, &c_sources, &mut plan)?;
        return Ok(plan);
    }
    // `cl` writes the diagnostics of a compile, as its notes and the names
    // of its sources, to stdout, and of `/E`, `/EP` and `/P` to stderr.
    if !clang_cl {
        common.push("--diagnostics-to-stdout".into());
    }
    // `/Zs`: each source is only checked.
    if state.syntax_only {
        for source in &c_sources {
            let mut step = common.clone();
            step.push("--syntax-only".into());
            step.extend(source.arguments());
            if !clang_cl {
                plan.steps.push(Step::Print {
                    text:   format!("{}\n", source.echoed()),
                    stderr: false,
                });
            }
            plan.steps.push(Step::Bcc(step));
        }
        return Ok(plan);
    }
    // Compiling: each source to its object, kept beside the executable.
    let first = c_sources
        .first()
        .map(|source| source.path.clone())
        .or_else(|| state.sources.first().map(|source| source.path.clone()))
        .expect("an input");
    // Clang's `-o` names the object under `/c` and otherwise the executable,
    // whatever `/Fo` and `/Fe` say, as it is: `-o x` is no `x.obj`.
    let clang_output = state.clang_output.as_deref().map(operand);
    let object_name = |source: &Source| match &clang_output {
        | Some(output) if state.compile_only => output.clone().into_owned(),
        | _ => named(state.object.as_deref(), &source.path, "obj"),
    };
    let extension = if state.dll { "dll" } else { "exe" };
    let executable = match &clang_output {
        | Some(output) if !state.compile_only => output.clone().into_owned(),
        | _ => named(state.executable.as_deref(), &first, extension),
    };
    let mut outputs = Vec::new();
    for source in &c_sources {
        outputs.push(object_name(source));
        if state.listing {
            outputs.push(named(state.listing_name.as_deref(), &source.path, "asm"));
        }
    }
    if !state.compile_only {
        outputs.push(executable.clone());
    }
    check_outputs(invocation, &state, &outputs)?;
    let mut objects = Vec::new();
    for source in &c_sources {
        let object = object_name(source);
        let mut step = common.clone();
        step.push("-c".into());
        step.push("-o".into());
        step.push(object.clone().into());
        if state.listing {
            let mut assembly = common.clone();
            assembly.push("-S".into());
            assembly.push("-o".into());
            assembly.push(named(state.listing_name.as_deref(), &source.path, "asm").into());
            assembly.extend(source.arguments());
            plan.steps.push(Step::Bcc(assembly));
        }
        step.extend(source.arguments());
        // `cl` names each source it compiles, and clang-cl does under
        // `/showFilenames`.
        if !clang_cl || state.show_filenames {
            plan.steps.push(Step::Print {
                text:   format!("{}\n", source.echoed()),
                stderr: false,
            });
        }
        plan.steps.push(Step::Bcc(step));
        objects.push(object);
    }
    // `cl` compiles several sources in one batch and then says that it
    // generates their code, also when one of them was missing.
    if !clang_cl && c_sources.len() > 1 {
        plan.steps.push(Step::Print {
            text:   "Generating Code...\n".into(),
            stderr: false,
        });
    }
    if state.compile_only {
        return Ok(plan);
    }
    // Intel's driver runs `llvm-lib`, which writes the archive format
    // Microsoft's linker reads; `llvm-ar` writes the same in COFF format.
    if state.archive {
        let mut arguments: Vec<OsString> = vec!["--format=coff".into(), "rcs".into()];
        arguments.push(executable.into());
        arguments.extend(objects.into_iter().map(OsString::from));
        arguments.extend(
            state
                .sources
                .iter()
                .filter(|source| !source.is_c())
                .map(|source| source.path.clone()),
        );
        plan.steps.push(Step::Run {
            program: PathBuf::from(version::program_name("ar")),
            name: None,
            arguments,
            stdin: None,
        });
        return Ok(plan);
    }
    let mut link = common;
    // `/LD` makes a DLL, and so does Clang's `-shared`, under the
    // executable's name.
    if state.dll || state.shared {
        link.push("-shared".into());
    }
    // clang-cl's `/Brepro` asks the linker for an image without a time
    // stamp; bcc's objects carry none in any case.
    if state.reproducible {
        link.push("-Wl,-Brepro".into());
    }
    // Clang's driver links the static runtime, `libcmt`, by default.
    if let Some(library) = runtime.library() {
        link.push("-Wl,-nodefaultlib:libcmt".into());
        link.push(format!("-Wl,-defaultlib:{library}").into());
    }
    link.extend(objects.into_iter().map(OsString::from));
    link.extend(
        state
            .sources
            .iter()
            .filter(|source| !source.is_c())
            .map(|source| operand(Path::new(&source.path)).into_owned().into()),
    );
    link.append(&mut state.link);
    link.push("-o".into());
    link.push(executable.into());
    plan.steps.push(Step::Link(link));
    Ok(plan)
}

/// Plans preprocessing each of `sources` with bcc's options `common`, as
/// `mode` asks: `/E` and `/EP` write every source to stdout, `/P` each to
/// its `.i` file, and Clang's `-o` to its file (`-` is stdout).
fn preprocessing_steps(
    invocation: &Invocation,
    state: &State,
    mode: Preprocess,
    common: &[OsString],
    sources: &[&Source],
    plan: &mut Plan,
) -> Result<(), MainError> {
    let outputs: Vec<Option<PathBuf>> = sources
        .iter()
        .map(|source| match &state.clang_output {
            | Some(output) if output.as_os_str() == "-" => None,
            | Some(output) => Some(operand(output).into_owned()),
            | None => mode
                .to_file
                .then(|| named(state.preprocessed.as_deref(), &source.path, "i")),
        })
        .collect();
    let files: Vec<PathBuf> = outputs.iter().flatten().cloned().collect();
    check_outputs(invocation, state, &files)?;
    for (source, output) in sources.iter().zip(outputs) {
        let mut step = common.to_vec();
        step.push("-E".into());
        if mode.no_line_markers {
            step.push("-P".into());
        }
        if let Some(output) = output {
            step.push("-o".into());
            step.push(output.into());
        }
        step.extend(source.arguments());
        if !invocation.personality.is_clang_cl() {
            plan.steps.push(Step::Print {
                text:   format!("{}\n", source.echoed()),
                stderr: true,
            });
        }
        plan.steps.push(Step::Bcc(step));
    }
    Ok(())
}

/// What clang-cl's driver answers itself, without inputs, for one of its
/// query options: `--version` (to stdout, with the target), the target
/// triple, and the directories it searches for programs and its resource
/// libraries, which are the bundled LLVM's that bcc's driver runs.
fn clang_cl_query(invocation: &Invocation, text: &str) -> Option<Plan> {
    let target = || {
        invocation
            .target
            .unwrap_or_else(|| host_target(invocation.personality))
    };
    match text {
        | "--version" => Some(Plan::print(version::banner(
            invocation.personality,
            &invocation.name,
            target(),
        ))),
        | "-print-target-triple" | "-print-effective-triple" =>
            Some(Plan::print(format!("{}\n", target().triple()))),
        | "-print-search-dirs" => {
            let linker = PathBuf::from(version::program_name("lld-link"));
            let programs = linker.parent().unwrap_or_else(|| Path::new(""));
            let major = version::CLANG_VERSION
                .split('.')
                .next()
                .unwrap_or(version::CLANG_VERSION);
            let libraries = programs
                .parent()
                .unwrap_or(programs)
                .join("lib/clang")
                .join(major);
            Some(Plan::print(format!(
                "programs: ={}\nlibraries: ={}\n",
                programs.display(),
                libraries.display()
            )))
        },
        | _ => None,
    }
}

/// What bcc reads for a Clang option clang-cl hands its front end, bare or
/// after `/clang:`: most as they are. Clang's front end warns about the
/// warning options it does not take, `-Wl,`, `-Wa,` and `-Wp,` among them,
/// which clang-cl's driver does not have; it takes the others, of which bcc
/// reads those it implements. The groups of the driver's own warnings
/// (`-Wno-unknown-argument`) are taken and have no effect there.
/// `--no-warnings` is `-w`.
fn front_end_option(text: &str) -> Option<OsString> {
    if text == "--no-warnings" {
        Some("-w".into())
    } else if !text.starts_with("-W") {
        Some(text.into())
    } else if !warning_options::clang_takes(text) {
        Some(format!("--unknown-warning-option={text}").into())
    } else {
        implements_language_option(text).then(|| text.into())
    }
}

/// The GCC or Clang option `text` that clang-cl's driver, or icx-cl's with
/// `intel`, ignores as unknown ([`known::cl_unknown_option`]). icx-cl's
/// wrapper drops an option of Intel's classic table with its own warning
/// (#10430), which bcc does not give, before clang-cl's driver could ignore
/// it; such an option clang-cl ignores stays ignored.
fn ignored_option(text: &str, intel: bool) -> Option<known::ClUnknown> {
    known::cl_unknown_option(text, intel).or_else(|| {
        (intel && icx::is_classic_option(text))
            .then(|| known::cl_unknown_option(text, false))
            .flatten()
    })
}

/// How a warning of Clang's driver that is on by default comes out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DriverSeverity {
    Ignored,
    Warning,
    Error,
    /// An error that stops the driver, which reports nothing after it.
    Fatal,
}

/// How clang-cl reports an argument it ignores as unknown, a warning in the
/// group `-Wunknown-argument`, after the warning options anywhere on the
/// command line, which its driver reads first (`clang_main`), as Clang's
/// `ProcessWarningOptions` maps them in order: `/w`, `/W0` and `-w` silence
/// every warning; `/WX` and `-Werror` make it an error unless
/// `-Wno-error=unknown-argument` says otherwise, and `-Wfatal-errors` makes
/// an error fatal; `-Wno-unknown-argument` and `-Wno-everything` turn it
/// off, and `-Wunknown-argument` on again, though never down from an error
/// `-Werror=unknown-argument` or `-Wfatal-errors=unknown-argument` asked
/// for. clang-cl hands a `/clang:` option to its front end alone, so none
/// counts here.
fn unknown_argument_severity(arguments: &[OsString]) -> DriverSeverity {
    driver_warning_severity(arguments, "unknown-argument")
}

/// How clang-cl reports an argument no job uses, a warning in the group
/// `-Wunused-command-line-argument`: as [`driver_warning_severity`] says,
/// but never under `-Qunused-arguments`, bare or after `/clang:`, which turns
/// it off (`Driver::BuildJobs`).
fn unused_argument_severity(arguments: &[OsString]) -> DriverSeverity {
    let quiet = arguments
        .iter()
        .take_while(|argument| !matches!(argument.to_str(), Some("/link" | "-link" | "--")))
        .any(|argument| {
            matches!(
                argument.to_str(),
                Some(
                    "-Qunused-arguments"
                        | "/clang:-Qunused-arguments"
                        | "-clang:-Qunused-arguments"
                )
            )
        });
    if quiet {
        DriverSeverity::Ignored
    } else {
        driver_warning_severity(arguments, "unused-command-line-argument")
    }
}

/// Whether `option`, one of bcc's, asks for debug information, as clang-cl's
/// `/Zi`, `/Z7`, `/ZI` and Clang's `-g` options do: any `-g` option but
/// `-g0` and `-ggdb0`.
fn is_debug_information(option: &str) -> bool {
    option.starts_with("--debug-level=") && option != "--debug-level=0"
        || option.starts_with("-g") && !matches!(option, "-g0" | "-ggdb0")
}

/// The letters of clang-cl's `/O` options in `arguments` before `/link`,
/// with each argument's index: every argument `/O` or `-O` starts, but for
/// the value of an option that takes the next argument.
fn optimization_options(arguments: &[OsString]) -> impl Iterator<Item = (usize, &str)> {
    let mut value = false;
    arguments
        .iter()
        .map(|argument| argument.to_str().unwrap_or_default())
        .enumerate()
        .take_while(|(_, argument)| !matches!(*argument, "/link" | "-link" | "--"))
        .filter_map(move |(index, argument)| {
            if std::mem::take(&mut value) {
                return None;
            }
            let option = argument
                .strip_prefix('/')
                .or_else(|| argument.strip_prefix('-'))?;
            value = known::clang_cl_option(option) == Some(ClOption::TakesNext);
            option.strip_prefix('O').map(|letters| (index, letters))
        })
}

/// How clang-cl reports a warning of its driver's in `group` that is on by
/// default, as [`unknown_argument_severity`] says for `-Wunknown-argument`.
fn driver_warning_severity(arguments: &[OsString], group: &str) -> DriverSeverity {
    let (mut ignore_all, mut all_errors, mut all_fatal) = (false, false, false);
    let mut mapping = DriverSeverity::Warning;
    let (mut no_error, mut no_fatal) = (false, false);
    let mut arguments = arguments.iter().filter_map(|argument| argument.to_str());
    while let Some(argument) = arguments.next() {
        if matches!(argument, "/link" | "-link" | "--") {
            break;
        }
        let Some(option) = argument
            .strip_prefix('/')
            .or_else(|| argument.strip_prefix('-'))
        else {
            continue;
        };
        if known::clang_cl_option(option) == Some(ClOption::TakesNext) {
            _ = arguments.next();
            continue;
        }
        let group = |prefix: &str| {
            argument
                .strip_prefix(prefix)
                .is_some_and(|name| name == group)
        };
        // `cl`'s options start with `/` or `-`, Clang's with `-`.
        match (option, argument.starts_with('-')) {
            | ("w" | "W0", _) | ("-no-warnings", true) => ignore_all = true,
            | ("WX", _) | ("Werror", true) => all_errors = true,
            | ("WX-", _) | ("Wno-error", true) => all_errors = false,
            | ("Wfatal-errors", true) => all_fatal = true,
            | ("Wno-fatal-errors", true) => all_fatal = false,
            | ("Wno-everything", true) => mapping = DriverSeverity::Ignored,
            | _ if group("-W") => {
                if !matches!(mapping, DriverSeverity::Error | DriverSeverity::Fatal) {
                    mapping = DriverSeverity::Warning;
                }
            },
            | _ if group("-Wno-") => mapping = DriverSeverity::Ignored,
            | _ if group("-Werror=") => mapping = DriverSeverity::Error,
            | _ if group("-Wno-error=") => {
                no_error = true;
                if matches!(mapping, DriverSeverity::Error | DriverSeverity::Fatal) {
                    mapping = DriverSeverity::Warning;
                }
            },
            | _ if group("-Wfatal-errors=") => mapping = DriverSeverity::Fatal,
            | _ if group("-Wno-fatal-errors=") => {
                no_fatal = true;
                if mapping == DriverSeverity::Fatal {
                    mapping = DriverSeverity::Error;
                }
            },
            | _ => {},
        }
    }
    match mapping {
        | DriverSeverity::Ignored => DriverSeverity::Ignored,
        | _ if ignore_all => DriverSeverity::Ignored,
        | DriverSeverity::Warning if !all_errors || no_error => DriverSeverity::Warning,
        | DriverSeverity::Warning | DriverSeverity::Error if all_fatal && !no_fatal =>
            DriverSeverity::Fatal,
        | DriverSeverity::Warning => DriverSeverity::Error,
        | severity => severity,
    }
}

/// Whether `option` is one of `cl`'s own options that clang-cl does not
/// know, which the `cl` personality keeps reading as options where paths
/// start with `/` (clang-cl reads `/Zf` as a file).
fn is_cl_only_option(option: &str) -> bool {
    matches!(option, "Qpar-" | "Qspectre-" | "Zf" | "analyze") || option.starts_with("analyze:")
}

/// How the `cl` personality reads an argument that starts with `/` where
/// paths do: see [`cl_slash_argument`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlashArgument {
    /// A file to compile or link, whatever option its name starts with.
    Input,
    /// An option, known or not: `cl.exe` warns about an unknown one (D9002).
    Option,
    /// As clang-cl reads it ([`known::clang_cl_option`]).
    AsClangCl,
}

/// How the `cl` personality reads `text`, an argument that starts with `/`,
/// off Windows. `cl.exe` reads every such argument as an option, and build
/// systems that drive it pass absolute paths bare: `CMake` gives clang-cl a
/// `--` before its sources but not `cl`, and `check_c_compiler_flag(/bogus)`
/// expects `cl`'s warning D9002. So a file that exists inside a directory is
/// an input even where clang-cl would read an option (`/opt/a.c` as `/o`,
/// `/Users/a.c` as `/U`), and a name with no directory that is no file is
/// an option, which D9002 reports if `cl` does not know it. Any other
/// argument is read as clang-cl reads it.
fn cl_slash_argument(text: &str) -> SlashArgument {
    let in_directory = text[1..].contains('/');
    let path = Path::new(text);
    if in_directory && path.is_file() {
        SlashArgument::Input
    } else if !in_directory && !path.exists() {
        SlashArgument::Option
    } else {
        SlashArgument::AsClangCl
    }
}

/// The status `cl` exits with after any error, its own or a compiler's;
/// clang-cl exits with 1, as Clang does.
pub(super) const ERROR_STATUS: i32 = 2;

/// The status the personality exits with after an error it reports itself.
fn error_status(personality: Personality) -> i32 {
    if personality.is_clang_cl() {
        1
    } else {
        ERROR_STATUS
    }
}

/// Refuses an output of any step that is one of the command line's input
/// files, by file identity (device and inode on Unix), so a symbolic or hard
/// link counts: each step checks only its own `-o` against its own inputs,
/// and `/Fe`, or an object that lands on another input, would otherwise
/// replace a source. An input that cannot be read is left for its step to
/// report.
fn check_outputs(
    invocation: &Invocation,
    state: &State,
    outputs: &[PathBuf],
) -> Result<(), MainError> {
    let inputs = state
        .sources
        .iter()
        .map(|source| source.path.as_os_str())
        .chain(
            state
                .link
                .iter()
                .map(OsString::as_os_str)
                .filter(|argument| !argument.to_string_lossy().starts_with("-Wl,")),
        );
    for input in inputs {
        let input = Path::new(input);
        if std::fs::metadata(input).is_err() {
            continue;
        }
        for output in outputs {
            if crate::cli::output_file::is_input(input, output).unwrap_or(false) {
                let message = format_args!(
                    "output file '{}' is the same as input file '{}'",
                    output.display(),
                    input.display()
                );
                if invocation.personality.is_clang_cl() {
                    invocation.error(message);
                } else {
                    super::print(
                        &format!("{} : Command line error : {message}\n", invocation.name),
                        true,
                    );
                }
                return Err(MainError::Reported(error_status(invocation.personality)));
            }
        }
    }
    Ok(())
}

/// A `cl` input.
#[derive(Clone)]
struct Source {
    path: OsString,
    /// `/Tc` or `/TC` makes it C whatever its extension.
    c:    bool,
    /// A C++ source, which bcc cannot compile.
    cpp:  bool,
}

impl Source {
    fn new(path: &OsStr, c: bool) -> Self {
        let extension = Path::new(path)
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);
        let cpp = !c && matches!(extension.as_deref(), Some("cpp" | "cxx" | "cc" | "c++"));
        Self {
            path: path.into(),
            c: c || extension.as_deref() == Some("c"),
            cpp,
        }
    }

    const fn is_c(&self) -> bool {
        self.c
    }

    /// The name `cl` prints as it compiles the source: its file name,
    /// after the last `/` or `\\` (`sub\\s.c` prints `s.c`).
    fn echoed(&self) -> String {
        let path = self.path.to_string_lossy();
        path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned()
    }

    /// The source on bcc's command line, as C; a name after `--` that
    /// starts with `-` gets `./` before it, but standard input stays `-`.
    fn arguments(&self) -> [OsString; 4] {
        let path = if self.path == "-" {
            self.path.clone()
        } else {
            operand(Path::new(&self.path)).into_owned().into()
        };
        ["-x".into(), "c".into(), path, "-xnone".into()]
    }
}

/// How `/E`, `/EP` and `/P` preprocess.
#[derive(Clone, Copy, Default)]
struct Preprocess {
    to_file:         bool,
    no_line_markers: bool,
}

/// The translation so far.
#[expect(
    clippy::struct_excessive_bools,
    reason = "The switches of a command line are independent."
)]
struct State {
    /// bcc's options shared by every step.
    bcc:                     Vec<OsString>,
    /// clang-cl's last `/openmp` flag when it asks for OpenMP, which bcc
    /// refuses; `/openmp-` after it clears it.
    openmp:                  Option<String>,
    /// The arguments clang-cl's driver hands to no job, as it shows them,
    /// which it reports after everything else (`own_options`).
    unused:                  Vec<String>,
    /// How clang-cl reports [`State::unused`]
    /// ([`unused_argument_severity`]).
    unused_severity:         DriverSeverity,
    /// The warnings in clang-cl's group `-Woption-ignored` about its own
    /// options, given for each source it compiles (`own_options`).
    option_ignored:          Vec<String>,
    /// clang-cl's `/Brepro`, the last of it and `/Brepro-`.
    reproducible:            bool,
    /// clang-cl's `/showFilenames`, the last of it and `/showFilenames-`,
    /// and both as given.
    show_filenames:          bool,
    filename_options:        Vec<String>,
    /// clang-cl's last `/vlen=256` or `/vlen=512`, unless `/vlen` follows.
    vector_length:           Option<String>,
    /// clang-cl's `/JMC`, unless `/JMC-` follows.
    just_my_code:            bool,
    /// The options of code generation bcc does not implement, as given,
    /// and why, which only a step that compiles refuses (`own_options`).
    code_options:            Vec<(String, &'static str)>,
    /// clang-cl's `/C`.
    keep_comments:           Option<String>,
    /// The letter of a clang-cl `/O` option that sets the level: the index
    /// of its argument and its offset there (`optimization`).
    level_letter:            Option<(usize, usize)>,
    /// clang-cl's inlining options: `/Ob` and `/clang:`'s Clang ones.
    inlining:                ClangInlining,
    sources:                 Vec<Source>,
    /// Linker options after `/link`, as bcc's link options.
    link:                    Vec<OsString>,
    linking:                 bool,
    /// `/TC`: every later source is C.
    all_c:                   bool,
    compile_only:            bool,
    /// `/Zs`.
    syntax_only:             bool,
    preprocess:              Option<Preprocess>,
    /// `/Fo`, `/Fe`, `/Fi`: a file, or a directory ending in a separator.
    object:                  Option<PathBuf>,
    executable:              Option<PathBuf>,
    /// `icx-cl -fuse-ld=llvm-lib`: archive the objects instead of linking.
    archive:                 bool,
    /// clang-cl's `/o` after any `/Fo`, which names the object under `/c`.
    object_from_o:           Option<PathBuf>,
    /// Clang's `-o`, which names the output of whatever step runs.
    clang_output:            Option<PathBuf>,
    preprocessed:            Option<PathBuf>,
    /// `/FA` or `/Fa`, with `/Fa`'s name.
    listing:                 bool,
    listing_name:            Option<PathBuf>,
    dll:                     bool,
    /// Clang's `-shared`, with which a link makes a DLL.
    shared:                  bool,
    /// `/LDd`, which selects the static debug runtime unless `/MD` or
    /// `/MDd` names one.
    debug_dll:               bool,
    /// The last of `/MT`, `/MTd`, `/MD` and `/MDd`.
    runtime:                 Option<Runtime>,
    /// Microsoft extensions: `/Ze` (the default) or `/Za`, the last wins.
    extensions:              bool,
    /// `/Za` or `/Zc:__STDC__`, under which `cl` defines `__STDC__`.
    stdc:                    bool,
    nologo:                  bool,
    /// clang-cl's `-v` or `-###`, which print its version without inputs.
    verbose:                 bool,
    /// clang-cl's `-###`, which prints its version, and the commands it
    /// would run, even with inputs, and runs none.
    dry_run:                 bool,
    /// The `/clang:` options Clang ignores with a warning for each source it
    /// compiles: GCC optimization flags and retired flags, in order.
    ignored:                 Vec<(ClangIgnored, String)>,
    /// The Clang options, each with the values its bare name takes, that
    /// bcc reads differently in different kinds of step, which only the
    /// whole command line decides ([`State::read_clang_options_by_step`]).
    clang_by_step:           Vec<Vec<String>>,
    ignore_include_variable: bool,
    /// clang-cl's `/imsvc` directories, in order.
    imsvc:                   Vec<OsString>,
    standard:                &'static str,
    /// `/std:` named the standard, as `/std:c11` does, rather than `cl`'s
    /// default dialect.
    standard_given:          bool,
    diagnostics:             &'static str,
    /// The warning level `/W0`-`/W4` set; `cl`'s own is 1.
    warning_level:           u8,
    /// The last level option was `/Wall`, which also shows the warnings
    /// that are off by default.
    all_warnings:            bool,
    /// Where in [`State::bcc`] the options of the last level stand, and how
    /// many there are.
    warning_level_options:   (usize, usize),
    /// `/WX`, the last of it and `/WX-`.
    warnings_error:          bool,
    /// What `/wd`, `/we`, `/wo` and `/w1`-`/w4` said of each warning, the
    /// last option for a warning winning.
    warning_settings:        Vec<(u16, WarningSetting)>,
    /// The last value and spelling of each switch whose classes
    /// [`switch_class`] names, and of `/D` or `/U` for each macro (class
    /// `D`), for `cl`'s warning D9025.
    switches:                Vec<(String, String, String)>,
    errors:                  usize,
}

impl Default for State {
    /// `cl`'s defaults: C17 (as clang-cl maps its default C mode), column
    /// diagnostics with the snippet (clang-cl's; `plan` gives `cl` its
    /// classic form),
    /// Microsoft extensions, and the static runtime.
    fn default() -> Self {
        Self {
            bcc:                     Vec::new(),
            openmp:                  None,
            unused:                  Vec::new(),
            unused_severity:         DriverSeverity::Warning,
            option_ignored:          Vec::new(),
            reproducible:            false,
            show_filenames:          false,
            filename_options:        Vec::new(),
            vector_length:           None,
            just_my_code:            false,
            code_options:            Vec::new(),
            keep_comments:           None,
            level_letter:            None,
            inlining:                ClangInlining::default(),
            sources:                 Vec::new(),
            link:                    Vec::new(),
            linking:                 false,
            all_c:                   false,
            compile_only:            false,
            syntax_only:             false,
            preprocess:              None,
            object:                  None,
            executable:              None,
            archive:                 false,
            object_from_o:           None,
            clang_output:            None,
            preprocessed:            None,
            listing:                 false,
            listing_name:            None,
            dll:                     false,
            shared:                  false,
            debug_dll:               false,
            runtime:                 None,
            extensions:              true,
            stdc:                    false,
            nologo:                  false,
            verbose:                 false,
            dry_run:                 false,
            ignored:                 Vec::new(),
            clang_by_step:           Vec::new(),
            ignore_include_variable: false,
            imsvc:                   Vec::new(),
            standard:                "c17",
            standard_given:          false,
            diagnostics:             "msvc-caret",
            warning_level:           1,
            all_warnings:            false,
            warning_level_options:   (0, 0),
            warnings_error:          false,
            warning_settings:        Vec::new(),
            switches:                Vec::new(),
            errors:                  0,
        }
    }
}

/// The C runtime library `/MT`, `/MTd`, `/MD` and `/MDd` select.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Runtime {
    Static,
    StaticDebug,
    Dll,
    DllDebug,
}

impl Runtime {
    /// The default library the link takes instead of `libcmt`, the static
    /// runtime Clang's driver links.
    const fn library(self) -> Option<&'static str> {
        match self {
            | Self::Static => None,
            | Self::StaticDebug => Some("libcmtd"),
            | Self::Dll => Some("msvcrt"),
            | Self::DllDebug => Some("msvcrtd"),
        }
    }
}

impl State {
    /// The options for the system directories the driver adds before the
    /// command line's options: clang-cl's `/imsvc` directories, then those
    /// of `%INCLUDE%` unless `/X` leaves them out, both ahead of bcc's
    /// built-in headers. clang-cl's driver adds them after its resource
    /// headers instead (`MSVCToolChain::AddClangSystemIncludeArgs`), but
    /// bcc's intrinsic headers do not yet coexist with the MSVC headers,
    /// which include `<intrin.h>` (driver-modes.md, "clang-cl"). The command
    /// line's `/I` directories are still searched first, so a header found
    /// here is a system header, whose warnings are not shown.
    fn system_directories(&mut self) -> Vec<OsString> {
        let mut directories = std::mem::take(&mut self.imsvc);
        if !self.ignore_include_variable
            && let Some(include) = std::env::var_os("INCLUDE")
        {
            directories.extend(
                std::env::split_paths(&include)
                    .filter(|directory| !directory.as_os_str().is_empty())
                    .map(PathBuf::into_os_string),
            );
        }
        directories
            .into_iter()
            .flat_map(|directory| ["--isystem".into(), directory])
            .collect()
    }

    /// clang-cl's `/imsvc DIR`, `-imsvc DIR` or `/imsvcDIR` (`argument`),
    /// which `CMake` gives it for a `SYSTEM` include directory: `directory`,
    /// or the next of `rest` when that is empty
    /// ([`Self::system_directories`]).
    fn imsvc<'r>(
        &mut self,
        invocation: &Invocation,
        argument: &str,
        directory: &str,
        rest: &mut impl Iterator<Item = &'r OsStr>,
    ) {
        if !directory.is_empty() {
            self.imsvc.push(directory.into());
        } else if let Some(directory) = rest.next() {
            self.imsvc.push(directory.to_os_string());
        } else {
            invocation.error(format_args!(
                "argument to '{argument}' is missing (expected 1 value)"
            ));
            self.errors += 1;
        }
    }

    /// Translates the option `option` (without its `/` or `-`), reading a
    /// separate operand from `rest`. Returns whether it is a `cl` option.
    fn option<'r>(
        &mut self,
        invocation: &Invocation,
        option: &'r str,
        rest: &mut impl Iterator<Item = &'r OsStr>,
    ) -> bool {
        // The options whose operand may be the next argument.
        for (name, bcc) in [
            ("I", "-I"),
            ("D", "-D"),
            ("U", "-U"),
            ("FI", "-include"),
            ("external:I", "-isystem"),
        ] {
            if let Some(value) = option.strip_prefix(name) {
                let value = if value.is_empty() {
                    match rest.next() {
                        | Some(value) => value.to_string_lossy().into_owned(),
                        | None => {
                            invocation.error(format_args!("missing argument to '/{name}'"));
                            self.errors += 1;
                            return true;
                        },
                    }
                } else {
                    value.into()
                };
                if name == "D" || name == "U" {
                    self.define_or_undefine(invocation, name, &value);
                }
                // `cl /DNAME#VALUE` defines NAME as VALUE.
                let value = if name == "D" {
                    value.replacen('#', "=", 1)
                } else {
                    value
                };
                self.bcc.push(bcc.into());
                self.bcc.push(value.into());
                return true;
            }
        }
        // An empty name is the default one, as in `cl` and clang-cl: `/Fo`
        // alone names the object after the source, also after `/Fofoo.obj`.
        let path = |value: &str| {
            Some(PathBuf::from(value.strip_prefix(':').unwrap_or(value)))
                .filter(|path| !path.as_os_str().is_empty())
        };
        if let Some((class, value)) = switch_class(option) {
            self.set_switch(invocation, class, value, format!("/{option}"));
        }
        match option {
            | "c" => self.compile_only = true,
            | "nologo" => self.nologo = true,
            // clang-cl's `/P` writes its file beside `/E` too.
            | "E" =>
                self.preprocess = Some(Preprocess {
                    to_file:         invocation.personality.is_clang_cl()
                        && self.preprocess.is_some_and(|p| p.to_file),
                    no_line_markers: self.preprocess.is_some_and(|p| p.no_line_markers),
                }),
            | "EP" => {
                let to_file = self.preprocess.is_some_and(|p| p.to_file);
                self.preprocess = Some(Preprocess {
                    to_file,
                    no_line_markers: true,
                });
            },
            | "P" => {
                let no_line_markers = self.preprocess.is_some_and(|p| p.no_line_markers);
                self.preprocess = Some(Preprocess {
                    to_file: true,
                    no_line_markers,
                });
            },
            | "TC" => self.all_c = true,
            | "TP" => self.unsupported(invocation, option, "bcc compiles C only"),
            | "X" => self.ignore_include_variable = true,
            | "O1" | "Os" => self.bcc.push("-Os".into()),
            | "O2" | "Ox" | "Ot" => self.bcc.push("-O2".into()),
            | "Od" => self.bcc.push("-O0".into()),
            | "W0" | "w" => self.set_warning_level(invocation, 0, &["-w"]),
            // clang-cl maps `/W1` to `/W3` to `-Wall` (Clang's
            // `CLCompatOptions.td`); `cl`'s levels name coded warnings only.
            | "W1" | "W2" | "W3" => self.set_warning_level(
                invocation,
                option.as_bytes()[1] - b'0',
                if invocation.personality.is_clang_cl() {
                    &["-Wall"]
                } else {
                    &[]
                },
            ),
            | "W4" => self.set_warning_level(invocation, 4, &["-Wall", "-Wextra"]),
            // clang-cl's `/Wall` is `-Weverything`; `cl`'s is its level 4
            // and more.
            | "Wall" => {
                self.set_warning_level(
                    invocation,
                    4,
                    if invocation.personality.is_clang_cl() {
                        &["-Weverything"]
                    } else {
                        &["-Wall", "-Wextra"]
                    },
                );
                self.all_warnings = true;
            },
            | "WX" => {
                self.warnings_error = true;
                self.bcc.push("-Werror".into());
            },
            | "WX-" => {
                self.warnings_error = false;
                self.bcc.push("-Wno-error".into());
            },
            | "std:c11" | "std:c17" | "std:clatest" => {
                self.standard = match option {
                    | "std:c11" => "c11",
                    | "std:c17" => "c17",
                    | _ => "c23",
                };
                self.standard_given = true;
            },
            | "MT" => self.runtime = Some(Runtime::Static),
            | "MTd" => self.runtime = Some(Runtime::StaticDebug),
            | "MD" => self.runtime = Some(Runtime::Dll),
            | "MDd" => self.runtime = Some(Runtime::DllDebug),
            // `/LD` builds a DLL; the runtime stays `/MT`'s unless `/MD`
            // names another.
            | "LD" | "LDd" => {
                self.dll = true;
                self.debug_dll = option == "LDd";
            },
            | "Za" => {
                self.extensions = false;
                self.stdc = true;
            },
            | "Ze" => self.extensions = true,
            | "Zc:__STDC__" => self.stdc = true,
            | "Zp" => self.bcc.push("-fpack-struct=1".into()),
            | "Zp1" | "Zp2" | "Zp4" | "Zp8" | "Zp16" => self
                .bcc
                .push(format!("-fpack-struct={}", &option[2..]).into()),
            | "Zc:trigraphs" => self.bcc.push("-ftrigraphs".into()),
            | "Zc:trigraphs-" => self.bcc.push("-fno-trigraphs".into()),
            // An unsigned plain `char`, which also defines `_CHAR_UNSIGNED`.
            | "J" => self.bcc.push("-funsigned-char".into()),
            | "Zs" => self.syntax_only = true,
            // clang-cl's `/fp:fast` is Clang's `-ffast-math`, which gives up
            // the math functions' `errno` among the rest, and `/fp:precise`
            // is `-ffp-model=precise`, which restores Clang's default
            // semantics, the target's `errno` among them, as the Clang
            // personality reads it (Clang's `CLCompatOptions.td` and
            // `RenderFloatingPointOptions`). bcc keeps the rest of IEC 60559
            // semantics, and `cl` keeps `errno` under both.
            | "fp:fast" | "fp:precise" if invocation.personality == Personality::ClangCl =>
                self.bcc.push(
                    if option == "fp:fast" {
                        "-fno-math-errno"
                    } else {
                        "-fno-fast-math"
                    }
                    .into(),
                ),
            | "diagnostics:classic" => self.diagnostics = "msvc-classic",
            | "diagnostics:column" => self.diagnostics = "msvc",
            | "diagnostics:caret" => self.diagnostics = "msvc-caret",
            | "link" => self.linking = true,
            | "FA" | "FAs" | "FAc" | "FAcs" | "FAu" => self.listing = true,
            | "showIncludes" => self.bcc.push("--show-includes".into()),
            // Debug information in the object, as clang-cl treats all three.
            | "Zi" | "Z7" | "ZI" => self.bcc.push("--debug-level=2".into()),
            // Options that cannot change the program bcc builds: UTF-8
            // source and execution character sets (bcc's only ones),
            // calling conventions that x64 ignores, function-level linking,
            // string pooling, whole-program optimization, frame pointers
            // and intrinsics (optimizations bcc may or may not make), tool
            // files, warning levels, and parallel builds.
            | "utf-8"
            | "source-charset:utf-8"
            | "execution-charset:utf-8"
            | "validate-charset"
            | "validate-charset-"
            | "Gd"
            | "Gz"
            | "Gr"
            | "GS-"
            | "Gy"
            | "Gy-"
            | "GF"
            | "GF-"
            | "GL"
            | "GL-"
            | "Gw"
            | "Gw-"
            | "sdl-"
            | "guard:cf-"
            | "guard:ehcont-"
            | "Qspectre-"
            | "FS"
            | "MP"
            | "Zf"
            | "Oi"
            | "Oi-"
            | "Oy"
            | "Oy-"
            | "Og"
            | "Ob0"
            | "Ob1"
            | "Ob2"
            | "Ob3"
            | "bigobj"
            // C has no run-time type information, no default-library names in
            // bcc's objects, and no precompiled header to turn off.
            | "GR"
            | "GR-"
            | "Zl"
            | "Y-"
            | "permissive"
            | "permissive-"
            | "Qpar"
            | "Qpar-"
            | "arch:SSE2"
            | "fp:precise"
            | "fp:fast"
            | "fp:contract"
            | "fp:except-"
            | "volatile:iso"
            | "Zc:preprocessor"
            | "analyze-" => {},
            // Hardening and run-time checks a correct program does not
            // depend on.
            | "GS" => ignored(
                invocation,
                option,
                "bcc does not emit buffer security checks",
            ),
            | "sdl" => ignored(invocation, option, "bcc does not add SDL checks"),
            | "guard:cf" | "guard:ehcont" =>
                ignored(invocation, option, "bcc does not emit Control Flow Guard"),
            | "Gv" => self.unsupported(
                invocation,
                option,
                "bcc's default calling convention is the x64 one, not __vectorcall",
            ),
            | "fp:strict" | "fp:except" => self.unsupported(
                invocation,
                option,
                "bcc does not preserve floating-point exception flags or rounding modes",
            ),
            | "volatile:ms" => self.unsupported(
                invocation,
                option,
                "bcc gives volatile accesses ISO semantics, not acquire and release",
            ),
            | "Zc:preprocessor-" => self.unsupported(
                invocation,
                option,
                "bcc has only the conforming preprocessor",
            ),
            // `/Fe:`, `/Fo:` and `/Fi:` alone take the name from the next
            // argument.
            | "Fe:" | "Fo:" | "Fi:" => {
                let Some(value) = rest.next() else {
                    invocation.error(format_args!("missing argument to '/{option}'"));
                    self.errors += 1;
                    return true;
                };
                let value_text = value.to_string_lossy();
                self.set_switch(
                    invocation,
                    &option[..2],
                    &value_text,
                    format!("/{option}{value_text}"),
                );
                let value = Some(PathBuf::from(value));
                match option {
                    | "Fe:" => self.executable = value,
                    | "Fo:" => (self.object, self.object_from_o) = (value, None),
                    | _ => self.preprocessed = value,
                }
            },
            | _ =>
                if let Some(value) = option.strip_prefix("Fe") {
                    self.executable = path(value);
                } else if let Some(value) = option.strip_prefix("Fo") {
                    self.object = path(value);
                    self.object_from_o = None;
                } else if let Some(value) = option.strip_prefix("Fi") {
                    self.preprocessed = path(value);
                } else if let Some(value) = option.strip_prefix("Fa") {
                    self.listing = true;
                    self.listing_name = path(value).filter(|path| !path.as_os_str().is_empty());
                } else if let Some((code, setting)) = warning_setting(option) {
                    self.warning_settings.retain(|&(other, _)| other != code);
                    self.warning_settings.push((code, setting));
                } else if let Some(file) = option.strip_prefix("Tc") {
                    let file = if file.is_empty() {
                        rest.next().map_or_else(OsString::new, OsString::from)
                    } else {
                        file.into()
                    };
                    self.sources.push(Source::new(&file, true));
                } else if option.starts_with("Tp") || option.starts_with("std:c++") {
                    self.unsupported(invocation, option, "bcc compiles C only");
                } else if option.starts_with("source-charset:")
                    || option.starts_with("execution-charset:")
                {
                    self.unsupported(invocation, option, "bcc reads and writes UTF-8 only");
                } else if option.starts_with("Zp") {
                    self.command_line_error(
                        invocation,
                        Some("D8021"),
                        format_args!("invalid numeric argument '/{option}'"),
                    );
                } else if let Some(variable) = option.strip_prefix("external:env:") {
                    // The directories in VARIABLE are system directories.
                    for directory in std::env::var_os(variable)
                        .map_or_default(|value| std::env::split_paths(&value).collect::<Vec<_>>())
                    {
                        if !directory.as_os_str().is_empty() {
                            self.bcc.push("-isystem".into());
                            self.bcc.push(directory.into_os_string());
                        }
                    }
                } else if option.starts_with("RTC") && option != "RTC-" {
                    ignored(invocation, option, "bcc does not emit run-time checks");
                } else if option.starts_with("Qspectre") {
                    ignored(invocation, option, "bcc does not emit Spectre mitigations");
                } else if option.starts_with("arch:") {
                    self.unsupported(
                        invocation,
                        option,
                        "bcc generates baseline x86-64 (SSE2) code and does not define the \
                         extension's macros",
                    );
                } else if [
                    "wd",
                    "we",
                    "wo",
                    "w1",
                    "w2",
                    "w3",
                    "w4",
                    "Wv",
                    "Zc:",
                    "EH",
                    "Fd",
                    "Fp",
                    "favor:",
                    "analyze",
                    "external:",
                    "experimental:",
                    "errorReport:",
                    "constexpr:",
                    "Gs",
                    "RTC-",
                ]
                .iter()
                .any(|prefix| option.starts_with(prefix))
                {
                    // Warning numbers and levels, the conformance switches
                    // for C++ and for what bcc already does (`/Zc:inline`,
                    // `/Zc:wchar_t`), C++ exception handling, tool files,
                    // tuning, analysis, the stack-probe threshold, and the
                    // `/external:` warning levels: none changes a C
                    // program bcc builds.
                } else if option.starts_with("d1") || option.starts_with("d2") {
                    ignored(
                        invocation,
                        option,
                        "bcc has no undocumented compiler switches",
                    );
                } else {
                    return false;
                },
        }
        true
    }

    /// Refuses clang-cl's last `/openmp` flag if it asks for OpenMP, which
    /// bcc does not implement; an earlier one that `/openmp-` overrides is
    /// fine, as clang-cl's last flag wins.
    fn check_openmp(&mut self, invocation: &Invocation) {
        if let Some(text) = self.openmp.take() {
            self.command_line_error(
                invocation,
                None,
                format_args!("'{text}' is not supported: bcc implements no OpenMP"),
            );
        }
    }

    /// Diagnoses the inputs as the compiler does before it compiles any.
    /// clang-cl's driver refuses a missing source and compiles none, while
    /// `cl` reports it when it reaches it (fatal error C1083, in bcc's job),
    /// and compiles the others. `cl` warns that a file whose extension it
    /// does not know is assumed to be an object (warning D9024), and both
    /// say that `/c` leaves the linker's inputs unused (`cl`'s warning
    /// D9027). clang-cl preprocesses standard input (`-`) as C, but
    /// compiles it only as `/Tc` or `/Tp` names its language. It leaves a
    /// missing input for the linker to the linker, unless its name starts
    /// with `/`, as a mistyped option's does, and suggests the option one
    /// edit away from a missing input's name.
    fn check_inputs(&mut self, invocation: &Invocation) {
        let clang_cl = invocation.personality.is_clang_cl();
        let links = !self.compile_only && self.preprocess.is_none() && !self.syntax_only;
        let preprocess = self.preprocess.is_some();
        let mut missing = 0;
        let mut errors = 0;
        self.sources.retain_mut(|source| {
            let path = Path::new(&source.path).display();
            if clang_cl && source.path == "-" && !source.is_c() && !source.cpp {
                if preprocess {
                    source.c = true;
                    return true;
                }
                invocation.error(format_args!(
                    "use /Tc or /Tp to set input type for standard input"
                ));
                errors += 1;
                return false;
            }
            let compiled = source.is_c() || source.cpp;
            let slash = source.path.to_string_lossy().starts_with('/');
            if clang_cl
                && (compiled || slash)
                && source.path != "-"
                && std::fs::metadata(&source.path).is_err()
            {
                match known::nearest_clang_cl_option(&source.path.to_string_lossy()) {
                    | Some(option) => invocation.error(format_args!(
                        "no such file or directory: '{path}'; did you mean '{option}'?"
                    )),
                    | None => invocation.error(format_args!("no such file or directory: '{path}'")),
                }
                missing += 1;
                return false;
            }
            if compiled {
                return true;
            }
            let extension = Path::new(&source.path)
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_ascii_lowercase);
            // The extensions Microsoft documents for D9024, and `.res`,
            // which `cl` passes to the linker too.
            if !clang_cl
                && !matches!(
                    extension.as_deref(),
                    Some("obj" | "lib" | "def" | "exp" | "res")
                )
            {
                super::print(
                    &format!(
                        "{} : Command line warning D9024 : unrecognized source file type \
                         '{path}', object file assumed\n",
                        invocation.name
                    ),
                    true,
                );
            }
            if clang_cl && !links {
                invocation.warning(format_args!(
                    "{path}: 'linker' input unused [-Wunused-command-line-argument]"
                ));
            } else if !clang_cl && self.compile_only {
                super::print(
                    &format!(
                        "{} : Command line warning D9027 : source file '{path}' ignored\n",
                        invocation.name
                    ),
                    true,
                );
            }
            true
        });
        self.errors += errors + missing;
        if missing != 0 && self.sources.is_empty() {
            invocation.error(format_args!("no input files"));
        }
    }

    /// Refuses `/Fo`, `/Fa` or `/Fi` naming a file for several sources, as
    /// `cl` does (error D8036): only a directory, ending in a separator,
    /// names several outputs.
    fn check_output_names(&mut self, invocation: &Invocation, sources: usize) {
        if sources < 2 {
            return;
        }
        let compiling = self.preprocess.is_none() && !self.syntax_only;
        let to_file = self.preprocess.is_some_and(|mode| mode.to_file);
        let options = [
            ("Fo", self.object.clone().filter(|_| compiling)),
            ("Fa", self.listing_name.clone().filter(|_| compiling)),
            ("Fi", self.preprocessed.clone().filter(|_| to_file)),
        ];
        for (name, path) in options {
            if let Some(path) = path
                && !is_directory(&path)
            {
                let option = format!("/{name}{}", path.display());
                if invocation.personality.is_clang_cl() {
                    invocation.error(format_args!(
                        "cannot specify '{option}' when compiling multiple source files"
                    ));
                    self.errors += 1;
                } else {
                    self.command_line_error(
                        invocation,
                        Some("D8036"),
                        format_args!("'{option}' not allowed with multiple source files"),
                    );
                }
            }
        }
    }

    /// Refuses an option `cl` knows that bcc does not implement and that
    /// would change the program.
    fn unsupported(&mut self, invocation: &Invocation, option: &str, why: &str) {
        self.command_line_error(
            invocation,
            None,
            format_args!("'/{option}' is not supported: {why}"),
        );
    }

    /// Prints a command-line error as the compiler does: `cl : Command line
    /// error D8036 : ...`, with Microsoft's code where it documents one and
    /// none where bcc would have to invent it, or clang-cl's `clang-cl:
    /// error: ...`.
    fn command_line_error(
        &mut self,
        invocation: &Invocation,
        code: Option<&str>,
        message: std::fmt::Arguments<'_>,
    ) {
        self.errors += 1;
        if invocation.personality.is_clang_cl() {
            invocation.error(message);
        } else {
            super::print(
                &format!(
                    "{} : Command line error{} : {message}\n",
                    invocation.name,
                    code.map_or_else(String::new, |code| format!(" {code}"))
                ),
                true,
            );
        }
    }
}

impl State {
    /// Reads clang-cl's `/clang:` options `arguments`, which its driver
    /// parses after all the others as a command line of their own, with
    /// Clang's options visible (`Driver::BuildCompilation`): an empty one is
    /// nothing, one that does not start with `-` (or is `-`) an input; an
    /// option's bare name takes its values from the next ones, and the
    /// driver refuses one left without them before it warns, in order,
    /// about the options Clang does not take, which it ignores as unknown
    /// (`-ffoo`) at the severity of the main command line's warning
    /// options. bcc reads the other options as [`State::clang_option`]
    /// says, into `out`. Returns whether a fatal error stopped the command
    /// line.
    fn read_clang_arguments<'a>(
        &mut self,
        invocation: &Invocation,
        arguments: &[&'a str],
        severity: DriverSeverity,
        x86_options: &mut Vec<&'a str>,
        out: &mut Vec<OsString>,
    ) -> bool {
        let mut options = Vec::new();
        let mut rest = arguments;
        while let Some((&argument, after)) = rest.split_first() {
            let count = match known::clang_option(argument, ClangVisibility::All) {
                | Some(ClangOption::TakesNext(count)) => count,
                | _ => 0,
            };
            if after.len() < count {
                self.missing_values(invocation, argument, count);
                break;
            }
            options.push(&rest[..=count]);
            rest = &after[count..];
        }
        for words in options {
            let text = words[0];
            if text.is_empty() {
                continue;
            }
            if text == "-" || !text.starts_with('-') {
                self.sources.push(Source::new(OsStr::new(text), false));
                continue;
            }
            // Clang's inlining options, which its driver reads together
            // with `/Ob`'s.
            if words.len() == 1 && self.inlining.read(text) {
                continue;
            }
            // The options Clang takes and ignores, and the aliases it
            // defines without a name of their own.
            if words.len() == 1
                && let Some(ignored) = known::clang_ignored(text)
            {
                match ignored {
                    | ClangIgnored::Optimization | ClangIgnored::Deprecated =>
                        self.ignored.push((ignored, text.to_owned())),
                    | ClangIgnored::Silent => {},
                    | ClangIgnored::Alias(option) =>
                        self.clang_option(invocation, &[option], true, x86_options, out),
                }
                continue;
            }
            if known::clang_option(text, ClangVisibility::All).is_none() {
                // Clang's option table suggests a near option as clang-cl's
                // does (`-fbuiltin-`; did you mean `-fbuiltin`?).
                let unknown = known::cl_unknown_option(text, false)
                    .unwrap_or(known::ClUnknown { nearest: None });
                if self.unknown_argument(invocation, severity, text, &unknown) {
                    return true;
                }
                continue;
            }
            self.clang_option(invocation, words, true, x86_options, out);
        }
        false
    }

    /// Reads `text`, a Clang option clang-cl takes bare, with the values its
    /// bare name takes from the next `arguments`, into bcc's options
    /// ([`State::clang_option`]); refuses one left without them, and ignores
    /// one clang-cl's mode does not show (`-specs`, `--config`), which takes
    /// no values then, with its warning at `severity`. Returns whether a
    /// fatal error stopped the command line.
    fn bare_clang_option<'a>(
        &mut self,
        invocation: &Invocation,
        text: &'a str,
        arguments: &mut impl Iterator<Item = &'a OsStr>,
        severity: DriverSeverity,
        x86_options: &mut Vec<&'a str>,
    ) -> bool {
        let mut words = vec![text];
        // clang-cl reads a configuration file of options, which bcc does
        // not.
        if text.starts_with("--config=") {
            self.command_line_error(
                invocation,
                None,
                format_args!("'{text}' is not supported: bcc reads no configuration files"),
            );
            return false;
        }
        match known::clang_option(text, ClangVisibility::ClangCl) {
            // A `cl` flag of the same name wins, and takes no value: `-u`
            // is `/u`.
            | Some(ClangOption::TakesNext(_)) if known::is_clang_cl_flag(&text[1..]) => {},
            | Some(ClangOption::TakesNext(count)) => {
                words.extend(arguments.by_ref().take(count).filter_map(OsStr::to_str));
                if words.len() <= count {
                    self.missing_values(invocation, text, count);
                    return false;
                }
            },
            | Some(ClangOption::Whole) => {},
            | None => {
                let unknown = known::ClUnknown { nearest: None };
                return self.unknown_argument(invocation, severity, text, &unknown);
            },
        }
        let mut bcc = std::mem::take(&mut self.bcc);
        self.clang_option(invocation, &words, false, x86_options, &mut bcc);
        self.bcc = bcc;
        false
    }

    /// Reads `words`, a Clang option clang-cl takes and its values, bare or
    /// (`given`) after `/clang:`, into `out`. `-fsyntax-only` is `/Zs`,
    /// `-c` and `-E` are `/c` and `/E`, `-###` prints the version, and the
    /// warning options go to bcc's front end ([`front_end_option`]), but
    /// for the driver's own `-Wl,`, `-Wa,` and `-Wp,` after `/clang:`.
    /// bcc reads any other option it implements as Clang does
    /// ([`bcc_reading`]), except a bare `-f` or `-m` option it does not
    /// implement; it drops one the step leaves unused, with clang-cl's
    /// warning where clang-cl gives one, and treats the rest as its Clang
    /// personality treats an option it does not implement, so none reaches
    /// bcc's command line unread. An option bcc reads differently in
    /// different kinds of step waits for the whole command line
    /// ([`State::read_clang_options_by_step`]).
    fn clang_option<'a>(
        &mut self,
        invocation: &Invocation,
        words: &[&'a str],
        given: bool,
        x86_options: &mut Vec<&'a str>,
        out: &mut Vec<OsString>,
    ) {
        let text = words[0];
        if words.len() == 1 {
            if text.starts_with("-m") {
                x86_options.push(text);
            }
            match text {
                | "-fsyntax-only" => {
                    self.syntax_only = true;
                    return;
                },
                // Clang's stages that are also `cl`'s, `/c` and `/E`.
                | "-c" | "-E" => {
                    _ = self.option(invocation, &text[1..], &mut std::iter::empty());
                    return;
                },
                // clang-cl prints its version and the commands it would run,
                // and runs none; bcc prints the version.
                | "-###" => {
                    self.verbose = true;
                    self.dry_run = true;
                    return;
                },
                // clang-cl's default, which bcc's Microsoft mode already
                // follows, and the switch that silences the warnings about
                // unused arguments, read with the warning options
                // (`unused_argument_severity`).
                | "-fms-compatibility" | "-Qunused-arguments" => return,
                | "--no-warnings" => {
                    out.extend(front_end_option(text));
                    return;
                },
                | _ if text.starts_with("-W")
                    && !(given && ["-Wl,", "-Wa,", "-Wp,"].iter().any(|w| text.starts_with(w))) =>
                {
                    out.extend(front_end_option(text));
                    return;
                },
                | _ => {},
            }
        }
        let flag = words.len() == 1 && (text.starts_with("-f") || text.starts_with("-m"));
        let reading = if flag && !given {
            if implements_language_option(text) {
                BccReading::Read
            } else {
                BccReading::Refused
            }
        } else {
            let readings = ClangStep::ALL.map(|step| bcc_reading(words, step));
            if readings.iter().any(|&reading| reading != readings[0]) {
                self.clang_by_step
                    .push(words.iter().map(|&word| word.to_owned()).collect());
                return;
            }
            readings[0]
        };
        self.apply_clang_reading(invocation, words, reading, out);
    }

    /// Reads the Clang options whose reading depends on the kind of step
    /// ([`State::clang_by_step`]), now that the whole command line has
    /// decided it, into `out`.
    fn read_clang_options_by_step(&mut self, invocation: &Invocation, out: &mut Vec<OsString>) {
        let step = self.clang_step();
        for words in std::mem::take(&mut self.clang_by_step) {
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            self.apply_clang_reading(invocation, &words, bcc_reading(&words, step), out);
        }
    }

    /// The kind of step the command line asks for, as clang-cl's driver
    /// takes the last phase: preprocessing beats only checking, which beats
    /// only compiling, and otherwise it links.
    const fn clang_step(&self) -> ClangStep {
        if self.preprocess.is_some() {
            ClangStep::Preprocess
        } else if self.syntax_only {
            ClangStep::Syntax
        } else if self.compile_only {
            ClangStep::Compile
        } else {
            ClangStep::Link
        }
    }

    /// Takes `words`, a Clang option and its values, as `reading` says,
    /// into `out` where bcc reads it.
    fn apply_clang_reading(
        &mut self,
        invocation: &Invocation,
        words: &[&str],
        reading: BccReading,
        out: &mut Vec<OsString>,
    ) {
        let text = words[0];
        let shown = words.join(" ");
        match reading {
            | BccReading::Read => {
                // bcc's command line takes `-include`'s value apart.
                match known::clang_spelling(text) {
                    | Some(("-include", Some(file))) if words.len() == 1 && !file.is_empty() =>
                        out.extend(["-include", file].map(OsString::from)),
                    | _ => out.extend(words.iter().copied().map(OsString::from)),
                }
                return;
            },
            // Clang's driver warns about an option no job it runs reads
            // (`-shared` without a link), after everything else
            // ([`State::report_unused`]), and says nothing of one its
            // compiler takes and ignores (`-dM` without `-E`).
            | BccReading::Unused => {
                self.unused.push(shown);
                return;
            },
            | BccReading::Inert => return,
            // A linker option clang-cl's driver hands no step but a link.
            | BccReading::LinkerInputUnused => {
                invocation.warning(format_args!(
                    "{text}: 'linker' input unused [-Wunused-command-line-argument]"
                ));
                return;
            },
            | BccReading::Shared => {
                self.shared = true;
                return;
            },
            | BccReading::Output => {
                let value = match words {
                    | [_, value] => *value,
                    | _ => known::clang_spelling(text)
                        .and_then(|(_, value)| value)
                        .unwrap_or_default(),
                };
                if value.is_empty() {
                    self.command_line_error(
                        invocation,
                        None,
                        format_args!("'{shown}' is not supported: it names no file"),
                    );
                } else if value == "-" && self.clang_step() != ClangStep::Preprocess {
                    self.command_line_error(
                        invocation,
                        None,
                        format_args!(
                            "'{shown}' is not supported: bcc writes no object or executable to \
                             standard output"
                        ),
                    );
                } else {
                    self.clang_output = Some(value.into());
                }
                return;
            },
            | BccReading::Refused => {},
        }
        match treatment(text, crate::target::Target::WindowsMsvc) {
            // This target is x86-64, so an x86 option is never for the
            // wrong one.
            | Treatment::Ignore | Treatment::WrongTarget => {},
            | Treatment::Warn(why) =>
                invocation.warning(format_args!("'{shown}' is ignored: {why}")),
            | Treatment::Reject(why) => self.command_line_error(
                invocation,
                None,
                format_args!("'{shown}' is not supported: {why}"),
            ),
        }
    }

    /// Reads clang-cl's `/o` (`text`), with its joined `value` or the next of
    /// `arguments`, which names the object under `/c`, where it and `/Fo`
    /// override each other, and otherwise the executable (or icx-cl's
    /// library), where it and `/Fe` do; `/E` and `/P` ignore it
    /// (`GetNamedOutputPath`).
    fn output_option<'a>(
        &mut self,
        invocation: &Invocation,
        text: &str,
        value: &'a str,
        arguments: &mut impl Iterator<Item = &'a OsStr>,
    ) {
        let output = if value.is_empty() {
            arguments.next()
        } else {
            Some(OsStr::new(value))
        };
        match output {
            | Some(output) => {
                self.executable = Some(output.into());
                self.object_from_o = Some(output.into());
            },
            | None => self.missing_values(invocation, text, 1),
        }
    }

    /// Refuses `option`, whose bare name takes `count` values from the next
    /// arguments, where too few follow, in Clang's words: `argument to
    /// '-Xclang' is missing (expected 1 value)`.
    fn missing_values(&mut self, invocation: &Invocation, option: &str, count: usize) {
        let plural = if count == 1 { "" } else { "s" };
        self.command_line_error(
            invocation,
            None,
            format_args!("argument to '{option}' is missing (expected {count} value{plural})"),
        );
    }
}

/// How bcc's own command line takes a Clang option clang-cl hands it
/// ([`bcc_reading`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BccReading {
    /// bcc reads it, as Clang does.
    Read,
    /// clang-cl's driver warns that it is unused in the step (`-shared`
    /// beside `/c`); bcc warns as it does and drops it.
    Unused,
    /// It does nothing in the step, and clang-cl says nothing of it (`-dM`
    /// without `-E`, `-save-temps`).
    Inert,
    /// `-Wl,` in a step that does not link: clang-cl's driver warns that
    /// the linker input is unused; bcc warns as it does and drops it.
    LinkerInputUnused,
    /// `-shared` in a link, which makes a DLL.
    Shared,
    /// `-o` or `--output`, which names the output of the step.
    Output,
    /// bcc does not implement it.
    Refused,
}

/// How bcc's own command line takes `words`, a Clang option and the values
/// its bare name takes, in a step of kind `step`, by the name Clang's
/// option table gives it, so that its value is the same joined
/// (`-includeinc.h`, `-isysrootdir`) or separate. bcc reads the options it
/// implements as Clang does: the preprocessor's (`-D`, `-I`, `-include`,
/// `-isystem`, the dependency options), the language, debug and
/// optimization options, and its linker options. `-o` names the step's
/// output, and `-shared` makes a link's executable a DLL. `-dM` and `-P`
/// do nothing without `-E`, nor `-dD` and `-dI`, which bcc does not
/// implement, and `-iprefix` without the `-iwithprefix` that uses it, which
/// bcc refuses; `-save-temps` and `-save-stats` change no program, and
/// clang-cl's link uses neither `-nostdlib` nor `-nostartfiles`. An option
/// clang-cl calls unused in the kind of step, as the pinned Clang's table
/// records it for exactly that kind and spelling (`-sectcreate a b c` beside
/// `/c`), is dropped with its warning, as is `-Wl,` without a link; bcc refuses
/// the rest as its Clang personality refuses an option it does not implement
/// (`-imacros`, `-isysroot`, `-working-directory`, `-S`, `-M`, `-Wp,`), never
/// reading them as anything else or dropping one that could change what the
/// step writes.
fn bcc_reading(words: &[&str], step: ClangStep) -> BccReading {
    use BccReading::{
        Inert,
        LinkerInputUnused,
        Output,
        Read,
        Refused,
        Shared,
        Unused,
    };
    let Some((&text, values)) = words.split_first() else {
        return Refused;
    };
    let preprocess = step == ClangStep::Preprocess;
    if text.starts_with("-Wl,") {
        return if step == ClangStep::Link {
            Refused
        } else {
            LinkerInputUnused
        };
    }
    // The options whose value bcc's command line checks.
    let valid = |valid: bool| if valid { Read } else { Refused };
    if values.is_empty() {
        if let Some(level) = text.strip_prefix("-O") {
            let digits = level.strip_prefix('-').unwrap_or(level);
            return valid(
                matches!(level, "" | "s" | "z" | "g")
                    || (!digits.is_empty()
                        && digits.bytes().all(|byte| byte.is_ascii_digit())
                        && level.parse::<i32>().is_ok()),
            );
        }
        if let Some(model) = text.strip_prefix("-ftls-model=") {
            return valid(crate::target::TlsModel::parse(model).is_some());
        }
        if let Some(format) = text.strip_prefix("-fdiagnostics-format=") {
            return valid(
                <crate::cli::arguments::Format as clap::ValueEnum>::from_str(format, false).is_ok(),
            );
        }
        if text.starts_with("-f") || text.starts_with("-m") {
            return valid(
                implements_language_option(text)
                    || matches!(text, "-fintegrated-as" | "-fno-integrated-as"),
            );
        }
    }
    // Clang's name for the option, and its value; a joined name given alone
    // has none.
    let (name, value) = match values.first() {
        | Some(&value) => (text, Some(value)),
        | None => match known::clang_spelling(text) {
            | Some((name, Some(""))) => (name, None),
            | Some(spelling) => spelling,
            | None => (text, None),
        },
    };
    let separate = !values.is_empty();
    match (name, value) {
        // clang-cl's link uses neither, and Clang writes no dependencies
        // where it only checks.
        | ("-nostdlib" | "-nostartfiles", None) if step == ClangStep::Link => Inert,
        | ("-MD" | "-MMD", None) if step == ClangStep::Syntax => Refused,
        | (
            "-D"
            | "-U"
            | "-I"
            | "-include"
            | "-isystem"
            | "-iquote"
            | "-idirafter"
            | "-MF"
            | "-MT"
            | "-MQ"
            | "-x"
            | "-B"
            | "-l"
            | "-Xlinker"
            | "-Xassembler"
            | "--include-directory="
            | "--language="
            | "--sysroot=",
            Some(_),
        )
        | (
            "-H" | "-MD" | "-MMD" | "-MP" | "-help" | "--help" | "-v" | "--verbose" | "--version"
            | "-w" | "-pedantic" | "-pedantic-errors" | "-pg" | "-trigraphs" | "-nostdinc"
            | "-nostdlibinc" | "-nobuiltininc" | "-nostdlib" | "-nodefaultlibs" | "-pthread" | "-r"
            | "-static",
            None,
        ) => Read,
        // bcc's command line joins these long options' values with `=`.
        | ("--include-directory" | "--language" | "--sysroot", Some(_)) if separate => Read,
        | ("-o" | "--output=" | "--output", Some(_)) =>
            if step == ClangStep::Syntax {
                Inert
            } else {
                Output
            },
        | ("-std=", Some(standard)) =>
            valid(crate::configuration::LanguageMode::parse(standard).is_some()),
        | ("--target=", Some(target)) => valid(crate::target::Target::parse(target).is_some()),
        | (_, None) if bcc_debug_option(text) => Read,
        | ("-dM" | "-P", None) =>
            if preprocess {
                Read
            } else {
                Inert
            },
        | ("-dD" | "-dI", None) =>
            if preprocess {
                Refused
            } else {
                Inert
            },
        | ("-shared" | "--shared", None) if step == ClangStep::Link => Shared,
        | ("-save-temps" | "-save-temps=" | "-save-stats", None)
        | ("-iprefix" | "-save-temps=", Some(_))
        | ("-save-stats=", Some("cwd" | "obj")) => Inert,
        | _ if known::clang_unused(words, step) => Unused,
        | _ => Refused,
    }
}

/// Whether bcc's command line reads `text` as the debug option Clang
/// spells so: `-g` and `-ggdb` with a level up to 3 or none, a DWARF
/// version, and the column and line-table options.
fn bcc_debug_option(text: &str) -> bool {
    let level = text
        .strip_prefix("-ggdb")
        .or_else(|| text.strip_prefix("-g"));
    matches!(level, Some("" | "0" | "1" | "2" | "3"))
        || matches!(
            text,
            "-gdwarf"
                | "-gdwarf-2"
                | "-gdwarf-3"
                | "-gdwarf-4"
                | "-gdwarf-5"
                | "-gdwarf-6"
                | "-gcolumn-info"
                | "-gno-column-info"
                | "-gline-tables-only"
                | "-gmlt"
        )
}

impl State {
    /// Reports `argument`, a GCC or Clang option clang-cl's driver ignores,
    /// with its warning in the group `-Wunknown-argument` at `severity`, and
    /// the option it suggests: `unknown argument ignored in clang-cl:
    /// '-fsigned-char' [-Wunknown-argument]`, or `... in clang-cl
    /// '-fbuiltin-'; did you mean '-fbuiltin'? [...]`. An error fails the
    /// command line, and a fatal one stops it: returns whether it did.
    fn unknown_argument(
        &mut self,
        invocation: &Invocation,
        severity: DriverSeverity,
        argument: &str,
        unknown: &known::ClUnknown,
    ) -> bool {
        let message = match &unknown.nearest {
            | Some(nearest) => format!(
                "unknown argument ignored in clang-cl '{argument}'; did you mean '{nearest}'?"
            ),
            | None => format!("unknown argument ignored in clang-cl: '{argument}'"),
        };
        self.driver_warning(invocation, severity, "unknown-argument", &message)
    }

    /// Reports `message`, a warning of clang-cl's driver in `group`, at
    /// `severity`: as a warning, an error that fails the command line, or a
    /// fatal error that stops it, which returns true.
    fn driver_warning(
        &mut self,
        invocation: &Invocation,
        severity: DriverSeverity,
        group: &str,
        message: &str,
    ) -> bool {
        match severity {
            | DriverSeverity::Ignored => {},
            | DriverSeverity::Warning => invocation.warning(format_args!("{message} [-W{group}]")),
            | DriverSeverity::Error => {
                self.errors += 1;
                invocation.error(format_args!("{message} [-Werror,-W{group}]"));
            },
            | DriverSeverity::Fatal => {
                super::print(
                    &format!("{}: fatal error: {message} [-W{group}]\n", invocation.name),
                    true,
                );
                return true;
            },
        }
        false
    }

    /// Warns as clang-cl's driver does about the `/clang:` options Clang
    /// takes and ignores, for each C source it compiles
    /// (`Clang::ConstructJob`): first the GCC optimization flags
    /// (`optimization flag '-fgcse' is not supported`), then the retired
    /// flags (`the flag '-mmpx' has been deprecated and will be ignored`),
    /// each in order, at the severity the warning options give their
    /// groups, after the warnings about clang-cl's own options it ignores
    /// ([`State::option_ignored`]). Returns whether a fatal error stopped
    /// the command line.
    fn warn_ignored(&mut self, invocation: &Invocation, arguments: &[OsString]) -> bool {
        let ignored = std::mem::take(&mut self.ignored);
        let option_ignored = std::mem::take(&mut self.option_ignored);
        if ignored.is_empty() && option_ignored.is_empty() {
            return false;
        }
        let sources = self
            .sources
            .iter()
            .filter(|source| {
                source.is_c() && (source.path == "-" || Path::new(&source.path).exists())
            })
            .count();
        let severity = driver_warning_severity(arguments, "option-ignored");
        for _ in 0..sources {
            for message in &option_ignored {
                if self.driver_warning(invocation, severity, "option-ignored", message) {
                    return true;
                }
            }
            for (kind, group) in [
                (ClangIgnored::Optimization, "ignored-optimization-argument"),
                (ClangIgnored::Deprecated, "unused-command-line-argument"),
            ] {
                let severity = driver_warning_severity(arguments, group);
                for (_, option) in ignored.iter().filter(|(other, _)| *other == kind) {
                    let message = if kind == ClangIgnored::Optimization {
                        format!("optimization flag '{option}' is not supported")
                    } else {
                        format!("the flag '{option}' has been deprecated and will be ignored")
                    };
                    if self.driver_warning(invocation, severity, group, &message) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl State {
    /// Reads one of Intel's own options, which clang-cl would refuse or
    /// read as paths: the remark switch (read already), the dependency
    /// options `CMake` gives `icx-cl` (`-QMD -QMT TARGET -QMF FILE`), options
    /// for the linker (`/Qoption,link,/machine:x64`), and the other `/Q`
    /// options. Returns whether `option` is one.
    fn intel_option<'r>(
        &mut self,
        invocation: &Invocation,
        option: &str,
        arguments: &mut impl Iterator<Item = &'r OsStr>,
    ) -> bool {
        if matches!(
            option,
            "Rno-debug-disables-optimization" | "Rdebug-disables-optimization"
        ) {
            return true;
        }
        if let Some(dependency) = option.strip_prefix('Q')
            && matches!(dependency, "MD" | "MMD" | "MT" | "MF" | "MQ")
        {
            self.bcc.push(format!("-{dependency}").into());
            if matches!(dependency, "MT" | "MF" | "MQ")
                && let Some(operand) = arguments.next()
            {
                self.bcc.push(operand.into());
            }
            return true;
        }
        // CMake has `icx-cl` make a static library with
        // `-fuse-ld=llvm-lib -o NAME.lib OBJECTS`, where `-o` (`/o`) names
        // the library.
        if option == "fuse-ld=llvm-lib" {
            self.archive = true;
            return true;
        }
        if let Some(words) = option.strip_prefix("Qoption,link,") {
            self.link
                .extend(words.split(',').map(|word| link_option(OsStr::new(word))));
            return true;
        }
        if let Some(intel) = option.strip_prefix('Q')
            && let Some(options) = icx::cl_option(intel)
        {
            match options {
                | Ok(options) => self.bcc.extend(options.iter().map(OsString::from)),
                | Err(why) => self.unsupported(invocation, option, why),
            }
            return true;
        }
        false
    }

    /// Reads `/O` with `letters`, the argument `text` at `index` of the
    /// command line (`optimization`): clang-cl's letter by letter, with the
    /// level from [`Self::level_letter`], and `cl`'s combined letters as
    /// the options they name.
    fn optimization_option(
        &mut self,
        invocation: &Invocation,
        index: usize,
        text: &str,
        letters: &str,
    ) {
        if invocation.personality.is_clang_cl() {
            let level = self
                .level_letter
                .filter(|&(at, _)| at == index)
                .map(|(_, offset)| offset);
            let intel = invocation.personality == Personality::IcxCl;
            if !optimization::clang_cl_options(
                letters,
                level,
                intel,
                &mut self.inlining,
                &mut self.bcc,
            ) {
                self.unused.push(text.to_owned());
            }
            return;
        }
        // `cl`: each option the letters combine, and a warning about each
        // letter it does not know.
        let deprecated = |option: &str| {
            super::print(
                &format!(
                    "{} : Command line warning D9035 : option '{option}' has been deprecated and \
                     will be removed in a future release\n",
                    invocation.name
                ),
                true,
            );
        };
        let pieces = optimization::cl_letters(letters);
        if pieces.is_empty() {
            deprecated("O");
        }
        for &piece in &pieces {
            if let optimization::ClLetters::Option(letters @ ("g" | "g-")) = piece {
                deprecated(&format!("O{letters}"));
            }
        }
        for piece in pieces {
            match piece {
                | optimization::ClLetters::Option(letters) => {
                    _ = self.option(invocation, &format!("O{letters}"), &mut std::iter::empty());
                },
                | optimization::ClLetters::Unknown(letters) => super::print(
                    &format!(
                        "{} : Command line warning D9002 : ignoring unknown option \
                         '{}O{letters}'\n",
                        invocation.name,
                        &text[..1]
                    ),
                    true,
                ),
            }
        }
    }

    /// Records `spelling`, the switch of `class` with `value`, warning as
    /// `cl` does (D9025) when it overrides an earlier one of the class with a
    /// different value: `overriding '/W3' with '/W4'`. clang-cl does not
    /// warn.
    fn set_switch(&mut self, invocation: &Invocation, class: &str, value: &str, spelling: String) {
        let previous = self
            .switches
            .iter()
            .position(|(other, _, _)| other == class);
        if let Some(index) = previous {
            let (_, old_value, old_spelling) = self.switches.remove(index);
            if old_value != value && invocation.personality == Personality::Msvc {
                super::print(
                    &format!(
                        "{} : Command line warning D9025 : overriding '{old_spelling}' with \
                         '{spelling}'\n",
                        invocation.name
                    ),
                    true,
                );
            }
        }
        self.switches.push((class.into(), value.into(), spelling));
    }

    /// Records `/D` or `/U` (`name`) with its `operand`: `cl` warns (D9025)
    /// when `/U` undefines a macro the last option for it defined, but not
    /// the other way round or for a redefinition, which replaces the earlier
    /// definition without a word, so bcc undefines the macro first.
    fn define_or_undefine(&mut self, invocation: &Invocation, name: &str, operand: &str) {
        let macro_name = operand.split(['=', '#']).next().unwrap_or(operand);
        let class = format!("D {macro_name}");
        let defined_last = self
            .switches
            .iter()
            .any(|(other, value, _)| *other == class && value == "D");
        if name == "U" && defined_last {
            self.set_switch(invocation, &class, "U", format!("/U{operand}"));
        } else {
            if defined_last {
                self.bcc.push("-U".into());
                self.bcc.push(macro_name.into());
            }
            self.switches.retain(|(other, _, _)| *other != class);
            self.switches
                .push((class, name.into(), format!("/{name}{operand}")));
        }
    }
}

/// The class and value of a switch whose last occurrence wins in `cl`,
/// which warns (D9025) when one overrides another of its class: the warning
/// level (`/w` is `/W0`; `/Wall` is no level), the optimization level and
/// the size or speed preference (`/Ox` is `/O2`, and neither class holds
/// the other's options), the runtime, the standard, the debug information,
/// `/WX`, the output names, the packing, and the switches with a `-` form.
/// Microsoft Learn: "Command-Line Warning D9025".
fn switch_class(option: &str) -> Option<(&'static str, &str)> {
    Some(match option {
        | "w" | "W0" => ("W", "0"),
        | "W1" | "W2" | "W3" | "W4" => ("W", &option[1..]),
        | "Od" => ("O", "d"),
        | "O1" => ("O", "1"),
        | "O2" | "Ox" => ("O", "2"),
        | "Os" | "Ot" => ("Os", option),
        | "MT" | "MTd" | "MD" | "MDd" => ("M", option),
        | "Zi" | "Z7" | "ZI" => ("Z7", option),
        | "Zp" => ("Zp", "1"),
        | "Zp1" | "Zp2" | "Zp4" | "Zp8" | "Zp16" => ("Zp", &option[2..]),
        | "TC" | "TP" => ("T", option),
        | _ =>
            if let Some(standard) = option.strip_prefix("std:") {
                ("std", standard)
            } else if let Some(model) = option.strip_prefix("fp:")
                && matches!(model, "precise" | "fast" | "strict")
            {
                ("fp", model)
            } else if let Some(level) = option.strip_prefix("Ob") {
                ("Ob", level)
            } else if let Some(name) = ["Fo", "Fe", "Fa", "Fi"].into_iter().find(|name| {
                option.starts_with(name) && !option[2..].trim_start_matches(':').is_empty()
            }) {
                (name, option[2..].trim_start_matches(':'))
            } else {
                let base = option.strip_suffix('-').unwrap_or(option);
                let class = [
                    "WX",
                    "GL",
                    "sdl",
                    "Oy",
                    "GF",
                    "GR",
                    "Gw",
                    "Gy",
                    "Oi",
                    "GS",
                    "Zc:trigraphs",
                    "permissive",
                ]
                .into_iter()
                .find(|class| *class == base)?;
                (class, option)
            },
    })
}

/// Accepts hardening or a run-time check bcc does not implement, with a
/// warning.
fn ignored(invocation: &Invocation, option: &str, why: &str) {
    if invocation.personality.is_clang_cl() {
        invocation.warning(format_args!("'/{option}' is ignored: {why}"));
    } else {
        super::print(
            &format!(
                "{} : Command line warning : '/{option}' is ignored: {why}\n",
                invocation.name
            ),
            true,
        );
    }
}

/// What `/wd`, `/we`, `/wo` or `/w1`-`/w4` says of one warning.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WarningSetting {
    /// `/wd`: never shown.
    Disabled,
    /// `/we`: an error.
    Error,
    /// `/wo`: shown once; bcc shows it each time.
    Once,
    /// `/w1`-`/w4`: shown from this level.
    Level(u8),
}

/// The warning number and setting of `/wdNNNN`, `/weNNNN`, `/woNNNN` or
/// `/wLNNNN`.
fn warning_setting(option: &str) -> Option<(u16, WarningSetting)> {
    let (setting, number) = match option.as_bytes() {
        | [b'w', b'd', ..] => (WarningSetting::Disabled, &option[2..]),
        | [b'w', b'e', ..] => (WarningSetting::Error, &option[2..]),
        | [b'w', b'o', ..] => (WarningSetting::Once, &option[2..]),
        | [b'w', level @ b'1'..=b'4', ..] => (WarningSetting::Level(level - b'0'), &option[2..]),
        | _ => return None,
    };
    Some((number.parse().ok()?, setting))
}

impl State {
    /// Sets the warning level, with the bcc `options` that mean it. `cl`'s
    /// rightmost level wins, so they replace an earlier level's and
    /// `/W0 /W3` keeps no `-w` (Microsoft Learn: "Order of CL Options").
    /// clang-cl's levels are aliases of Clang options, which add up as
    /// Clang's own do: `/W0 /W3` keeps `-w`, which no later option undoes,
    /// and `/W4 /W1` keeps `-Wextra` (Clang Compiler User's Manual,
    /// "clang-cl"; Clang's `CLCompatOptions.td`).
    fn set_warning_level(&mut self, invocation: &Invocation, level: u8, options: &[&str]) {
        if !invocation.personality.is_clang_cl() {
            let (at, count) = self.warning_level_options;
            drop(self.bcc.drain(at..at + count));
        }
        self.warning_level = level;
        self.all_warnings = false;
        self.warning_level_options = (self.bcc.len(), options.len());
        self.bcc.extend(options.iter().map(OsString::from));
    }

    /// bcc's options for the warnings it reports with `cl`'s code, from
    /// the warning level and the options naming them: a warning above the
    /// level or under `/wd` is off, `/we` makes it an error (at any level),
    /// and C4047
    /// and C4133 are warnings unless `/WX` makes every warning an error.
    fn coded_warning_options(&self) -> Vec<OsString> {
        let mut options = Vec::new();
        // The groups a setting or the level has turned off or made errors,
        // which a code sharing one of them does not turn on again.
        let mut settled: Vec<&str> = Vec::new();
        // A group without a name follows `--cl-warning` alone.
        for (warning, group) in CL_CODED_WARNINGS.iter().flat_map(|warning| {
            warning
                .groups
                .iter()
                .filter_map(move |group| Some((warning, group.name()?)))
        }) {
            let setting = self
                .warning_settings
                .iter()
                .find(|&&(other, _)| other == warning.code)
                .map(|&(_, setting)| setting);
            let level = match setting {
                | Some(WarningSetting::Level(level)) => level,
                | _ => warning.level,
            };
            if setting == Some(WarningSetting::Error) {
                options.push(format!("-Werror={group}").into());
                settled.push(group);
            } else if setting == Some(WarningSetting::Disabled) || level > self.warning_level {
                options.push(format!("-Wno-{group}").into());
                settled.push(group);
            } else {
                // A group bcc leaves off, or one whose parent a setting of
                // an earlier code (C4133) may have turned off.
                if warning.enable && !settled.contains(&group) {
                    options.push(format!("-W{group}").into());
                }
                if !self.warnings_error && matches!(warning.code, 4047 | 4090 | 4113 | 4133) {
                    options.push(format!("-Wno-error={group}").into());
                }
            }
        }
        // `/Wall` shows the warnings that are off by default at their own
        // level, unless a setting, which comes later, says otherwise.
        if self.all_warnings {
            for warning in CL_CODED_WARNINGS.iter().filter(|warning| warning.off) {
                options.push(format!("--cl-warning={}:{}", warning.code, warning.level).into());
            }
        }
        options
    }
}

/// The output name for `source`: the `/Fo`, `/Fe`, `/Fi` or `/Fa` file,
/// or the source's name with `extension` in the working directory or in
/// the option's directory (a name ending in a path separator). A file name
/// names one output; [`State::check_output_names`] refuses it for several
/// sources, as `cl` does.
fn named(option: Option<&Path>, source: &OsStr, extension: &str) -> PathBuf {
    let stem = Path::new(Path::new(source).file_name().unwrap_or(source)).with_extension(extension);
    let name = match option {
        // A backslash separates directories only on Windows; elsewhere the
        // one that ends `obj\` is dropped, so the objects go into `obj`, as
        // `cl`'s do, rather than to `obj\/a.obj`.
        | Some(option) if is_directory(option) => {
            let text = option.as_os_str().to_string_lossy();
            let directory = if cfg!(windows) {
                &*text
            } else {
                text.trim_end_matches('\\')
            };
            Path::new(directory).join(stem)
        },
        | Some(option) if option.extension().is_none() => option.with_extension(extension),
        | Some(option) => option.into(),
        | None => stem,
    };
    // Standard input's `-.obj`, or `/Fo-x.obj`, which bcc's command line
    // must not read as an option.
    operand(&name).into_owned()
}

/// Whether an output option names a directory: it ends in a separator.
fn is_directory(option: &Path) -> bool {
    option.as_os_str().to_string_lossy().ends_with(['/', '\\'])
}

/// A `/link` option as bcc's link option: a library or object file stays an
/// input; an option (`/SUBSYSTEM:CONSOLE`) goes through `-Wl,`.
fn link_option(argument: &OsStr) -> OsString {
    let text = argument.to_string_lossy();
    if text.starts_with('/') || text.starts_with('-') {
        let mut option = OsString::from("-Wl,");
        option.push(argument);
        option
    } else {
        argument.into()
    }
}

/// `/?`.
fn help(name: &str) -> String {
    format!(
        "usage: {name} [ option... ] filename... [ /link linkoption... ]\nbcc's cl.exe-compatible \
         driver: it reads cl's command line and runs bcc.\n  /c /E /EP /P   stages\n  /Fe<file> \
         /Fo<file> /Fa<file> /Fi<file>   output names\n  /I<dir> /D<name>[=value] /U<name> \
         /FI<file> /X   preprocessing\n  /O1 /O2 /Od /Ox /W0-/W4 /Wall /WX /w   optimization and \
         warnings\n  /std:c11 /std:c17 /std:clatest /TC /Tc<file>   language\n  /MD /MT /LD /link \
         ...   runtime and linking\n  /nologo /diagnostics:classic|column|caret\nOptions cl knows \
         that bcc does not implement are ignored when they cannot change the program and refused \
         otherwise; see docs/design/driver-modes.md.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file inside a directory is a source whatever option its name
    /// starts with (`CMake` passes `cl` the absolute path of its probe,
    /// `/opt/cmake/.../CMakeCCompilerABI.c`); a name without a directory
    /// that is no file is an option, as `cl.exe` reads it; the rest is
    /// read as clang-cl reads it.
    #[test]
    fn cl_reads_existing_files_and_unknown_names_as_cl_exe_would() {
        let directory = std::env::temp_dir().join(format!("bcc-cl-slash-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("a.c");
        std::fs::write(&file, "int x;\n").unwrap();
        let file = file.to_str().unwrap();
        if file.starts_with('/') {
            assert_eq!(cl_slash_argument(file), SlashArgument::Input);
            let missing = format!("{}/missing.c", directory.to_str().unwrap());
            assert_eq!(cl_slash_argument(&missing), SlashArgument::AsClangCl);
        }
        assert_eq!(cl_slash_argument("/bogus-flag"), SlashArgument::Option);
        assert_eq!(cl_slash_argument("/nologo"), SlashArgument::Option);
        drop(std::fs::remove_dir_all(&directory));
    }

    /// bcc reads a Clang option it implements by Clang's name for it, with
    /// its value joined or separate, and never by how its own short options
    /// would split it (`-imacros` is no `-i macros`, `-sectcreate` no `-s
    /// ectcreate`).
    #[test]
    fn bcc_reads_clang_options_by_clangs_names() {
        use BccReading::{
            Inert,
            LinkerInputUnused,
            Output,
            Read,
            Refused,
            Shared,
            Unused,
        };
        use ClangStep::{
            Compile,
            Link,
            Preprocess,
            Syntax,
        };
        for (words, step, reading) in [
            (&["-O2"][..], Compile, Read),
            (&["-O"], Compile, Read),
            (&["-O9"], Compile, Read),
            (&["-Ofoo"], Compile, Refused),
            // `-o` names the step's output, which a check has none of.
            (&["-o", "x.obj"], Compile, Output),
            (&["-ox.obj"], Compile, Output),
            (&["-o", "x.i"], Preprocess, Output),
            (&["-o", "x.exe"], Link, Output),
            (&["-o", "x"], Syntax, Inert),
            (&["--output=x.obj"], Compile, Output),
            (&["-D", "X"], Compile, Read),
            (&["-DX=1"], Compile, Read),
            (&["-include", "inc.h"], Compile, Read),
            (&["-includeinc.h"], Compile, Read),
            (&["-isysteminc"], Compile, Read),
            (&["-std=c11"], Compile, Read),
            (&["-xc"], Compile, Read),
            (&["-g2"], Compile, Read),
            (&["-gsplit-dwarf"], Compile, Refused),
            (&["-funsigned-char"], Compile, Read),
            (&["-ffoo"], Compile, Refused),
            (&["-dM"], Preprocess, Read),
            (&["-dM"], Compile, Inert),
            (&["-shared"], Compile, Unused),
            (&["-shared"], Preprocess, Unused),
            (&["-shared"], Link, Shared),
            (&["--shared"], Link, Shared),
            // Each as clang-cl's driver reads it in that kind of step: `/E`
            // prints the definitions and the includes, which bcc does not.
            (&["-dD"], Preprocess, Refused),
            (&["-dD"], Compile, Inert),
            (&["-dI"], Preprocess, Refused),
            (&["-dI"], Link, Inert),
            (&["-dA"], Compile, Refused),
            (&["-dumpmachine"], Compile, Refused),
            (&["-dynamic"], Compile, Refused),
            (&["-S"], Compile, Refused),
            (&["-S"], Preprocess, Unused),
            (&["-M"], Compile, Refused),
            (&["-MM"], Preprocess, Refused),
            // clang-cl's link uses neither; beside `/c` it calls
            // `-nostartfiles` unused, and every step `-L`.
            (&["-nostartfiles"], Compile, Unused),
            (&["-nostartfiles"], Link, Inert),
            (&["-nostdlib"], Link, Inert),
            (&["-L", "x"], Link, Unused),
            (&["-Lx"], Compile, Unused),
            (&["-rdynamic"], Link, Unused),
            // `-d` with a value is unused, `-dD` alone not.
            (&["-dDx"], Compile, Unused),
            (&["-dumpmachinex"], Link, Unused),
            // With `/Zs`, Clang writes the dependencies `bcc --syntax-only`
            // does not.
            (&["-MD"], Compile, Read),
            (&["-MD"], Syntax, Refused),
            (&["-MMD"], Syntax, Refused),
            // clang-cl refuses these.
            (&["-MG"], Compile, Refused),
            (&["-gtoggle"], Compile, Refused),
            (&["-save-stats=x"], Compile, Refused),
            (&["-save-stats=obj"], Compile, Inert),
            // clang-cl compiles nothing for the host and links nothing.
            (&["--cuda-device-only"], Link, Refused),
            (&["--emit-static-lib"], Compile, Unused),
            (&["--emit-static-lib"], Link, Refused),
            (&["-Tx"], Compile, Unused),
            (&["-Wl,-foo"], Compile, LinkerInputUnused),
            (&["-Wl,-foo"], Preprocess, LinkerInputUnused),
            (&["-Wl,-foo"], Link, Refused),
            (&["-Wp,-DX"], Preprocess, Refused),
            (&["-Wa,-bogus"], Compile, Refused),
            (&["-sectcreate", "a", "b", "c"], Compile, Unused),
            (&["-sectcreate"], Compile, Refused),
            (&["-specs=x"], Compile, Unused),
            (&["-save-temps"], Compile, Inert),
            (&["-save-temps="], Compile, Inert),
            (&["-save-stats="], Compile, Refused),
            (&["-iprefix", "x"], Compile, Inert),
            (&["-iprefixx"], Compile, Inert),
            (&["-imacros", "x"], Compile, Refused),
            (&["-imacrosx"], Compile, Refused),
            (&["-isysroot", "x"], Compile, Refused),
            (&["-isysrootx"], Compile, Refused),
            (&["-iwithsysroot", "x"], Compile, Refused),
            (&["-iwithprefix", "x"], Compile, Refused),
            (&["-ivfsoverlay", "x"], Compile, Refused),
            (&["-working-directory", "x"], Compile, Refused),
            (&["-working-directory=x"], Compile, Refused),
            // Clang's table reads it as `-o bjcmt-migrate-all`, as bcc does.
            (&["-objcmt-migrate-all"], Compile, Output),
            (&["-O0"], Compile, Read),
            (&["-Os"], Compile, Read),
            (&["-Ofast"], Compile, Refused),
            (&["-std=bogus"], Compile, Refused),
            (&["--target=x86_64-pc-windows-msvc"], Compile, Read),
            (&["--target=x"], Compile, Refused),
            (&["-ftls-model=local-exec"], Compile, Read),
            (&["-ftls-model=bogus"], Compile, Refused),
            (&["--output", "x.obj"], Compile, Output),
            (&["--outputx.obj"], Compile, Refused),
            (&["-Xclang", "-O2"], Compile, Refused),
        ] {
            assert_eq!(bcc_reading(words, step), reading, "{words:?} {step:?}");
        }
    }
}
