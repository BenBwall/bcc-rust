//! The TCC personality. TCC takes GCC's short options (`-c`, `-o`, `-E`,
//! `-I`, `-D`, `-U`, `-l`, `-L`, `-Wl,`, `-shared`, `-static`), compiles in
//! C99 with GNU extensions by default, ignores the `-W`, `-f` and `-m`
//! options it does not know (bcc refuses those GCC knows that would change
//! the program, as the GCC personality does), and rejects any other unknown
//! option
//! (`invalid option`). `-run FILE ARGS...` compiles FILE and runs it with
//! ARGS; bcc builds the program in a temporary directory and runs it there,
//! exiting with its status. Bounds checking (`-b`) and backtraces (`-bt`)
//! are accepted with a warning: bcc has neither.

use std::ffi::{
    OsStr,
    OsString,
};

use super::{
    Invocation,
    MainError,
    Plan,
    Step,
    TemporaryDirectory,
    host_target,
    known::{
        Compiler,
        knows,
    },
    unimplemented::{
        Treatment,
        treatment,
    },
    version,
};
use crate::{
    cli::arguments::implements_language_option,
    target::Target,
};

/// Translates a TCC command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let target = invocation
        .target
        .unwrap_or_else(|| host_target(invocation.personality));
    let mut out: Vec<OsString> = Vec::new();
    let mut standard = "gnu99";
    let mut stage = false;
    let mut output = false;
    let mut inputs = 0;
    let mut run: Option<Vec<OsString>> = None;
    let mut arguments = invocation.arguments.iter().map(OsString::as_os_str);
    while let Some(argument) = arguments.next() {
        let Some(text) = argument
            .to_str()
            .filter(|text| text.starts_with('-') && *text != "-")
        else {
            inputs += 1;
            out.push(argument.into());
            continue;
        };
        let operand = |arguments: &mut dyn Iterator<Item = &OsStr>, joined: &str| {
            if joined.is_empty() {
                arguments.next().map(OsString::from)
            } else {
                Some(OsString::from(joined))
            }
        };
        match text {
            | "-v" | "--version" if invocation.arguments.len() == 1 =>
                return Ok(Plan::print(version::banner(
                    invocation.personality,
                    &invocation.name,
                    target,
                ))),
            | "-v" | "-vv" | "-vvv" => super::print(
                &version::banner(invocation.personality, &invocation.name, target),
                true,
            ),
            | "-dumpversion" => return Ok(Plan::print(format!("{}\n", version::TCC_VERSION))),
            | "-print-search-dirs" =>
                return Ok(Plan::print(format!(
                    "install: {}\ninclude:\n  /usr/local/include\n  /usr/include\nlibraries:\n  \
                     /usr/lib\n",
                    version::program_name("tcc")
                ))),
            | "-h" | "-hh" | "--help" => return Ok(Plan::print(help(&invocation.name))),
            | "-run" => {
                let Some(file) = arguments.next() else {
                    invocation.error(format_args!("argument to '-run' is missing"));
                    return Err(MainError::Reported(1));
                };
                inputs += 1;
                out.push(file.into());
                run = Some(arguments.by_ref().map(OsString::from).collect());
            },
            | "-c" | "-E" => {
                stage = true;
                out.push(text.into());
            },
            | "-b" | "-g" | "-w" | "-shared" | "-static" | "-rdynamic" | "-nostdlib" | "-P"
            | "-dM" | "-pthread" | "-M" | "-MD" =>
                if text == "-b" {
                    invocation.warning(format_args!(
                        "bounds checking (-b) is not supported; ignoring it"
                    ));
                } else if text != "-g" {
                    if text == "-M" {
                        stage = true;
                    }
                    out.push(text.into());
                },
            | "-nostdinc" => out.push("--nostdinc".into()),
            | "-dD" => {
                invocation.warning(format_args!("'-dD' is treated as '-dM'"));
                out.push("-dM".into());
            },
            | "-funsigned-char" => {
                invocation.error(format_args!(
                    "'-funsigned-char' is not supported: plain char is always signed in bcc"
                ));
                return Err(MainError::Reported(1));
            },
            | _ => {
                if let Some(level) = text.strip_prefix("-bt") {
                    let _ = level;
                    invocation.warning(format_args!(
                        "backtraces (-bt) are not supported; ignoring it"
                    ));
                } else if let Some(value) = text.strip_prefix("-std=") {
                    standard = if matches!(value, "c11" | "gnu11") {
                        "gnu11"
                    } else {
                        "gnu99"
                    };
                } else if text.starts_with("-O")
                    || text.starts_with("-Wl,")
                    || text.starts_with("-D")
                    || text.starts_with("-U")
                    || text.starts_with("-I")
                    || text.starts_with("-l")
                    || text.starts_with("-L")
                    || text.starts_with("-o")
                    || text.starts_with("-MF")
                    || text.starts_with("-x")
                    || text == "-include"
                    || text == "-isystem"
                {
                    let (option, joined) = if text == "-include" || text == "-isystem" {
                        (text, "")
                    } else {
                        text.split_at(if text.starts_with("-MF") || text.starts_with("-Wl,") {
                            text.len().min(3)
                        } else {
                            2
                        })
                    };
                    if text.starts_with("-O") || text.starts_with("-Wl,") {
                        out.push(text.into());
                    } else {
                        let Some(value) = operand(&mut arguments, joined) else {
                            invocation.error(format_args!("argument to '{option}' is missing"));
                            return Err(MainError::Reported(1));
                        };
                        if option == "-o" {
                            output = true;
                        }
                        out.push(option.into());
                        out.push(value);
                    }
                } else if text.starts_with("-B") || text == "-soname" {
                    if text == "-B" || text == "-soname" {
                        _ = arguments.next();
                    }
                } else if text.starts_with("-W") || text.starts_with("-f") || text.starts_with("-m")
                {
                    // TCC ignores what it does not know of these. One GCC
                    // knows is treated as the GCC personality treats it, so
                    // none that bcc lacks silently changes the program.
                    if implements_language_option(text) {
                        out.push(text.into());
                    } else if !text.starts_with("-W") && knows(Compiler::Gcc, text) {
                        match treatment(text) {
                            | Treatment::Ignore => {},
                            | Treatment::Warn(why) =>
                                invocation.warning(format_args!("'{text}' is ignored: {why}")),
                            | Treatment::Reject(why) => {
                                invocation.error(format_args!("'{text}' is not supported: {why}"));
                                return Err(MainError::Reported(1));
                            },
                        }
                    }
                } else {
                    invocation.error(format_args!("invalid option -- '{text}'"));
                    return Err(MainError::Reported(1));
                }
            },
        }
    }
    if inputs == 0 {
        invocation.error(format_args!("no input files"));
        return Err(MainError::Reported(1));
    }
    let mut bcc: Vec<OsString> = vec![
        invocation.program.clone(),
        "--compiler-identity=tcc".into(),
        "--diagnostic-format=tcc".into(),
        format!("--target={}", target.triple()).into(),
        format!("-std={standard}").into(),
    ];
    bcc.append(&mut out);
    let mut plan = Plan::default();
    if let Some(arguments) = run {
        let directory =
            TemporaryDirectory::create().map_err(|source| MainError::WriteOutputFileError {
                path: std::env::temp_dir(),
                source,
            })?;
        let program = directory.0.join(
            if matches!(target, Target::WindowsGnu | Target::WindowsMsvc) {
                "a.exe"
            } else {
                "a.out"
            },
        );
        bcc.push("-o".into());
        bcc.push(program.clone().into_os_string());
        plan.steps.push(Step::Bcc(bcc));
        plan.steps.push(Step::Run { program, arguments });
        plan.temporary = Some(directory);
        return Ok(plan);
    }
    if !stage && !output {
        bcc.push("-o".into());
        bcc.push("a.out".into());
    }
    plan.steps.push(Step::Bcc(bcc));
    Ok(plan)
}

/// `-h`.
fn help(name: &str) -> String {
    format!(
        "Usage: {name} [options...] [-o outfile] [-c] infile(s)...\n       {name} [options...] \
         -run infile [arguments...]\nbcc's TCC-compatible driver: it reads TCC's command line and \
         runs bcc.\n  -c -o -E -I -D -U -l -L -Wl, -shared -static -std=c99|c11 -v -run\nSee \
         docs/design/driver-modes.md.\n"
    )
}
