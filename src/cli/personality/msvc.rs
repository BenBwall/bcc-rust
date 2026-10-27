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
//! macro and trigraph options.

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

use super::{
    Invocation,
    MainError,
    Personality,
    Plan,
    Step,
    host_target,
    version,
};

/// Clang options that clang-cl takes only after `/clang:`; given bare, it
/// ignores them with a warning, as for an unknown `/` option.
const CLANG_ONLY_OPTIONS: [&str; 4] = [
    "-fsigned-char",
    "-fno-signed-char",
    "-funsigned-char",
    "-fno-unsigned-char",
];

/// Translates a `cl.exe` or clang-cl command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let clang_cl = invocation.personality == Personality::ClangCl;
    let mut state = State::default();
    // clang-cl reads its `/clang:` options after the others, wherever they
    // stand, so they win over `cl`'s (`/clang:-fsigned-char` over `/J`).
    let mut clang_options: Vec<OsString> = Vec::new();
    let mut arguments = invocation.arguments.iter().map(OsString::as_os_str);
    while let Some(argument) = arguments.next() {
        if state.linking {
            state.link.push(link_option(argument));
            continue;
        }
        let Some(text) = argument.to_str() else {
            state.sources.push(Source::new(argument, false));
            continue;
        };
        // clang-cl reads `-` as standard input.
        let Some(option) = text
            .strip_prefix('/')
            .or_else(|| text.strip_prefix('-'))
            .filter(|_| !(clang_cl && text == "-"))
        else {
            state.sources.push(Source::new(argument, false));
            continue;
        };
        if option == "?" || option == "help" {
            return Ok(Plan::print(help(&invocation.name)));
        }
        if clang_cl && let Some(clang) = option.strip_prefix("clang:") {
            // `/clang:OPTION` gives clang-cl a Clang option; bcc reads most.
            clang_options.push(clang.into());
            continue;
        }
        if !state.option(invocation, option, &mut arguments) {
            if clang_cl && text.starts_with('-') && !CLANG_ONLY_OPTIONS.contains(&text) {
                // clang-cl also takes Clang's options; bcc reads most of them.
                state.bcc.push(argument.into());
            } else if clang_cl {
                invocation.warning(format_args!(
                    "unknown argument ignored in clang-cl: '{text}'"
                ));
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
    state.check_inputs(invocation);
    let c_count = state.sources.iter().filter(|source| source.is_c()).count();
    state.check_output_names(invocation, c_count);
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
        "-fms-extensions".into(),
        // MSVC's default C mode is close to C17 without its version macro;
        // clang-cl claims C17, and so does bcc.
        format!("-std={}", state.standard).into(),
        // `cl` replaces trigraphs only under `/Zc:trigraphs`.
        "-fno-trigraphs".into(),
    ];
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
    if !state.ignore_include_variable
        && let Some(include) = std::env::var_os("INCLUDE")
    {
        for directory in std::env::split_paths(&include) {
            if !directory.as_os_str().is_empty() {
                common.push("--isystem".into());
                common.push(directory.into_os_string());
            }
        }
    }
    common.append(&mut state.bcc);
    common.append(&mut clang_options);
    // `cl` compiles every source even after one fails, then skips the link;
    // clang-cl stops at the first failure.
    let mut plan = Plan {
        keep_going: !clang_cl,
        ..Plan::default()
    };
    if !state.nologo && !clang_cl {
        plan.steps.push(Step::Print {
            text:   version::banner(invocation.personality, &invocation.name, target),
            stderr: true,
        });
    }
    let c_sources: Vec<&Source> = state
        .sources
        .iter()
        .filter(|source| source.is_c())
        .collect();
    if c_sources.is_empty() && state.sources.is_empty() {
        if !state.nologo || clang_cl {
            super::print(
                &format!(
                    "usage: {} [ option... ] filename... [ /link linkoption... ]\n",
                    invocation.name
                ),
                true,
            );
        }
        return Ok(plan);
    }
    if let Some(source) = state.sources.iter().find(|source| source.cpp) {
        invocation.error(format_args!(
            "'{}': bcc compiles C only; C++ sources are not supported",
            Path::new(&source.path).display()
        ));
        return Err(MainError::Reported(error_status(invocation.personality)));
    }
    // Preprocessing: `/E` and `/EP` write every source to stdout, `/P` each
    // to its `.i` file.
    if let Some(mode) = state.preprocess {
        let outputs: Vec<PathBuf> = c_sources
            .iter()
            .filter(|_| mode.to_file)
            .map(|source| named(state.preprocessed.as_deref(), &source.path, "i"))
            .collect();
        check_outputs(invocation, &state, &outputs)?;
        for (index, source) in c_sources.iter().enumerate() {
            let mut step = common.clone();
            step.push("-E".into());
            if mode.no_line_markers {
                step.push("-P".into());
            }
            if mode.to_file {
                step.push("-o".into());
                step.push(outputs[index].clone().into());
            }
            step.extend(source.arguments());
            if !clang_cl {
                plan.steps.push(Step::Print {
                    text:   format!("{}\n", Path::new(&source.path).display()),
                    stderr: true,
                });
            }
            plan.steps.push(Step::Bcc(step));
        }
        return Ok(plan);
    }
    // `/Zs`: each source is only checked.
    if state.syntax_only {
        for source in &c_sources {
            let mut step = common.clone();
            step.push("--syntax-only".into());
            step.extend(source.arguments());
            if !clang_cl {
                plan.steps.push(Step::Print {
                    text:   format!("{}\n", Path::new(&source.path).display()),
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
    let extension = if state.dll { "dll" } else { "exe" };
    let executable = named(state.executable.as_deref(), &first, extension);
    let mut outputs = Vec::new();
    for source in &c_sources {
        outputs.push(named(state.object.as_deref(), &source.path, "obj"));
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
        let object = named(state.object.as_deref(), &source.path, "obj");
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
        if !clang_cl {
            plan.steps.push(Step::Print {
                text:   format!("{}\n", Path::new(&source.path).display()),
                stderr: false,
            });
        }
        plan.steps.push(Step::Bcc(step));
        objects.push(object);
    }
    if state.compile_only {
        return Ok(plan);
    }
    let mut link = common;
    if state.dll {
        link.push("-shared".into());
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
            .map(|source| source.path.clone()),
    );
    link.append(&mut state.link);
    link.push("-o".into());
    link.push(executable.into());
    plan.steps.push(Step::Link(link));
    Ok(plan)
}

/// The status `cl` exits with after any error, its own or a compiler's;
/// clang-cl exits with 1, as Clang does.
pub(super) const ERROR_STATUS: i32 = 2;

/// The status the personality exits with after an error it reports itself.
fn error_status(personality: Personality) -> i32 {
    if personality == Personality::ClangCl {
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
                if invocation.personality == Personality::ClangCl {
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

    /// The source on bcc's command line, as C.
    fn arguments(&self) -> [OsString; 4] {
        ["-x".into(), "c".into(), self.path.clone(), "-xnone".into()]
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
    preprocessed:            Option<PathBuf>,
    /// `/FA` or `/Fa`, with `/Fa`'s name.
    listing:                 bool,
    listing_name:            Option<PathBuf>,
    dll:                     bool,
    /// `/LDd`, which selects the static debug runtime unless `/MD` or
    /// `/MDd` names one.
    debug_dll:               bool,
    /// The last of `/MT`, `/MTd`, `/MD` and `/MDd`.
    runtime:                 Option<Runtime>,
    /// Microsoft extensions: `/Ze` (the default) or `/Za`, the last wins.
    extensions:              bool,
    nologo:                  bool,
    ignore_include_variable: bool,
    standard:                &'static str,
    diagnostics:             &'static str,
    errors:                  usize,
}

impl Default for State {
    /// `cl`'s defaults: C17 (as clang-cl maps its default C mode), column
    /// diagnostics, Microsoft extensions, and the static runtime.
    fn default() -> Self {
        Self {
            bcc:                     Vec::new(),
            sources:                 Vec::new(),
            link:                    Vec::new(),
            linking:                 false,
            all_c:                   false,
            compile_only:            false,
            syntax_only:             false,
            preprocess:              None,
            object:                  None,
            executable:              None,
            preprocessed:            None,
            listing:                 false,
            listing_name:            None,
            dll:                     false,
            debug_dll:               false,
            runtime:                 None,
            extensions:              true,
            nologo:                  false,
            ignore_include_variable: false,
            standard:                "c17",
            diagnostics:             "msvc",
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
        let path = |value: &str| Some(PathBuf::from(value.strip_prefix(':').unwrap_or(value)));
        match option {
            | "c" => self.compile_only = true,
            | "nologo" => self.nologo = true,
            | "E" =>
                self.preprocess = Some(Preprocess {
                    to_file:         false,
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
            | "W0" | "w" => self.bcc.push("-w".into()),
            | "W4" | "Wall" => {
                self.bcc.push("-Wall".into());
                self.bcc.push("-Wextra".into());
            },
            | "WX" => self.bcc.push("-Werror".into()),
            | "WX-" => self.bcc.push("-Wno-error".into()),
            | "std:c11" => self.standard = "c11",
            | "std:c17" => self.standard = "c17",
            | "std:clatest" => self.standard = "c23",
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
            | "Za" => self.extensions = false,
            | "Ze" => self.extensions = true,
            | "Zp" => self.bcc.push("-fpack-struct=1".into()),
            | "Zp1" | "Zp2" | "Zp4" | "Zp8" | "Zp16" => self
                .bcc
                .push(format!("-fpack-struct={}", &option[2..]).into()),
            | "Zc:trigraphs" => self.bcc.push("-ftrigraphs".into()),
            | "Zc:trigraphs-" => self.bcc.push("-fno-trigraphs".into()),
            // An unsigned plain `char`, which also defines `_CHAR_UNSIGNED`.
            | "J" => self.bcc.push("-funsigned-char".into()),
            | "Zs" => self.syntax_only = true,
            | "diagnostics:classic" => self.diagnostics = "msvc-classic",
            | "diagnostics:column" => self.diagnostics = "msvc",
            | "diagnostics:caret" => self.diagnostics = "msvc-caret",
            | "link" => self.linking = true,
            | "FA" | "FAs" | "FAc" | "FAcs" | "FAu" => self.listing = true,
            | "showIncludes" => self.bcc.push("--show-includes".into()),
            // Options that cannot change the program bcc builds: UTF-8
            // source and execution character sets (bcc's only ones),
            // calling conventions that x64 ignores, function-level linking,
            // string pooling, whole-program optimization, frame pointers
            // and intrinsics (optimizations bcc may or may not make), debug
            // information and tool files, warning levels, and parallel
            // builds.
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
            | "Zi"
            | "Z7"
            | "ZI"
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
            | "permissive"
            | "permissive-"
            | "Qpar"
            | "Qpar-"
            | "W1"
            | "W2"
            | "W3"
            | "arch:SSE2"
            | "fp:precise"
            | "fp:fast"
            | "fp:contract"
            | "fp:except-"
            | "volatile:iso"
            | "Zc:preprocessor"
            | "Zc:__STDC__"
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
            | _ =>
                if let Some(value) = option.strip_prefix("Fe") {
                    self.executable = path(value);
                } else if let Some(value) = option.strip_prefix("Fo") {
                    self.object = path(value);
                } else if let Some(value) = option.strip_prefix("Fi") {
                    self.preprocessed = path(value);
                } else if let Some(value) = option.strip_prefix("Fa") {
                    self.listing = true;
                    self.listing_name = path(value).filter(|path| !path.as_os_str().is_empty());
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
                        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
                        .unwrap_or_default()
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

    /// Diagnoses the inputs as the compiler does before it compiles any.
    /// clang-cl's driver refuses a missing source and compiles none, while
    /// `cl` reports it when it reaches it (fatal error C1083, in bcc's job),
    /// and compiles the others. `cl` warns that a file whose extension it
    /// does not know is assumed to be an object (warning D9024), and both
    /// say that `/c` leaves the linker's inputs unused (`cl`'s warning
    /// D9027). clang-cl preprocesses standard input (`-`) as C, but
    /// compiles it only as `/Tc` or `/Tp` names its language.
    fn check_inputs(&mut self, invocation: &Invocation) {
        let clang_cl = invocation.personality == Personality::ClangCl;
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
            if clang_cl
                && compiled
                && source.path != "-"
                && std::fs::metadata(&source.path).is_err()
            {
                invocation.error(format_args!("no such file or directory: '{path}'"));
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
                if invocation.personality == Personality::ClangCl {
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
        if invocation.personality == Personality::ClangCl {
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

/// Accepts hardening or a run-time check bcc does not implement, with a
/// warning.
fn ignored(invocation: &Invocation, option: &str, why: &str) {
    if invocation.personality == Personality::ClangCl {
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

/// The output name for `source`: the `/Fo`, `/Fe`, `/Fi` or `/Fa` file,
/// or the source's name with `extension` in the working directory or in
/// the option's directory (a name ending in a path separator). A file name
/// names one output; [`State::check_output_names`] refuses it for several
/// sources, as `cl` does.
fn named(option: Option<&Path>, source: &OsStr, extension: &str) -> PathBuf {
    let stem = Path::new(Path::new(source).file_name().unwrap_or(source)).with_extension(extension);
    match option {
        | Some(option) if is_directory(option) => option.join(stem),
        | Some(option) if option.extension().is_none() => option.with_extension(extension),
        | Some(option) => option.into(),
        // Standard input's `-.obj`, which bcc's command line must not read
        // as an option.
        | None if stem.as_os_str().to_string_lossy().starts_with('-') => Path::new(".").join(stem),
        | None => stem,
    }
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
