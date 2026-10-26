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
//! failing.

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

/// Translates a `cl.exe` or clang-cl command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let clang_cl = invocation.personality == Personality::ClangCl;
    let mut state = State::default();
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
        let Some(option) = text.strip_prefix('/').or_else(|| text.strip_prefix('-')) else {
            state.sources.push(Source::new(argument, false));
            continue;
        };
        if option == "?" || option == "help" {
            return Ok(Plan::print(help(&invocation.name)));
        }
        if clang_cl && let Some(clang) = option.strip_prefix("clang:") {
            // `/clang:OPTION` gives clang-cl a Clang option; bcc reads most.
            state.bcc.push(clang.into());
            continue;
        }
        if !state.option(invocation, option, &mut arguments) {
            if clang_cl && text.starts_with('-') {
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
    if state.errors != 0 {
        return Err(MainError::Reported(2));
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
    let target = invocation
        .target
        .unwrap_or_else(|| host_target(invocation.personality));
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
    ];
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
    let mut plan = Plan::default();
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
        return Err(MainError::Reported(2));
    }
    // Preprocessing: `/E` and `/EP` write every source to stdout, `/P` each
    // to its `.i` file.
    if let Some(mode) = state.preprocess {
        for source in &c_sources {
            let mut step = common.clone();
            step.push("-E".into());
            if mode.no_line_markers {
                step.push("-P".into());
            }
            if mode.to_file {
                step.push("-o".into());
                step.push(
                    named(
                        state.preprocessed.as_deref(),
                        &source.path,
                        "i",
                        c_sources.len(),
                    )
                    .into(),
                );
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
    // Compiling: each source to its object, kept beside the executable.
    let mut objects = Vec::new();
    for source in &c_sources {
        let object = named(
            state.object.as_deref(),
            &source.path,
            "obj",
            c_sources.len(),
        );
        let mut step = common.clone();
        step.push("-c".into());
        step.push("-o".into());
        step.push(object.clone().into());
        if state.listing {
            let mut assembly = common.clone();
            assembly.push("-S".into());
            assembly.push("-o".into());
            assembly.push(
                named(
                    state.listing_name.as_deref(),
                    &source.path,
                    "asm",
                    c_sources.len(),
                )
                .into(),
            );
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
    let first = c_sources
        .first()
        .map(|source| source.path.clone())
        .or_else(|| state.sources.first().map(|source| source.path.clone()))
        .expect("an input");
    let extension = if state.dll { "dll" } else { "exe" };
    let executable = named(state.executable.as_deref(), &first, extension, 1);
    let mut link = common;
    if state.dll {
        link.push("-shared".into());
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
    plan.steps.push(Step::Bcc(link));
    Ok(plan)
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
#[derive(Default)]
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
    preprocess:              Option<Preprocess>,
    /// `/Fo`, `/Fe`, `/Fi`: a file, or a directory ending in a separator.
    object:                  Option<PathBuf>,
    executable:              Option<PathBuf>,
    preprocessed:            Option<PathBuf>,
    /// `/FA` or `/Fa`, with `/Fa`'s name.
    listing:                 bool,
    listing_name:            Option<PathBuf>,
    dll:                     bool,
    nologo:                  bool,
    ignore_include_variable: bool,
    standard:                &'static str,
    diagnostics:             &'static str,
    errors:                  usize,
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
        if self.standard.is_empty() {
            self.standard = "c17";
            self.diagnostics = "msvc";
        }
        // The options whose operand may be the next argument.
        for (name, bcc) in [("I", "-I"), ("D", "-D"), ("U", "-U"), ("FI", "-include")] {
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
            | "MD" => self.bcc.push("-D_DLL".into()),
            | "MDd" => {
                self.bcc.push("-D_DLL".into());
                self.bcc.push("-D_DEBUG".into());
            },
            | "MTd" => self.bcc.push("-D_DEBUG".into()),
            | "LD" | "LDd" => {
                self.dll = true;
                self.bcc.push("-D_DLL".into());
                if option == "LDd" {
                    self.bcc.push("-D_DEBUG".into());
                }
            },
            | "J" => self.unsupported(invocation, option, "plain char is always signed in bcc"),
            | "Zs" => self.bcc.push("--syntax-only".into()),
            | "diagnostics:classic" => self.diagnostics = "msvc-classic",
            | "diagnostics:column" => self.diagnostics = "msvc",
            | "diagnostics:caret" => self.diagnostics = "msvc-caret",
            | "link" => self.linking = true,
            | "FA" | "FAs" | "FAc" | "FAcs" | "FAu" => self.listing = true,
            | "utf-8"
            | "source-charset:utf-8"
            | "execution-charset:utf-8"
            | "Gd"
            | "Gz"
            | "Gr"
            | "Gv"
            | "GS"
            | "GS-"
            | "Gy"
            | "Gy-"
            | "GF"
            | "GL"
            | "Gw"
            | "sdl"
            | "sdl-"
            | "FS"
            | "MP"
            | "Zi"
            | "Z7"
            | "ZI"
            | "Za"
            | "Ze"
            | "Oi"
            | "Oi-"
            | "Oy"
            | "Oy-"
            | "Og"
            | "bigobj"
            | "permissive-"
            | "Qpar"
            | "W1"
            | "W2"
            | "W3"
            | "MT" => {},
            | "showIncludes" => self.bcc.push("--show-includes".into()),
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
                    "RTC",
                    "Ob",
                    "Fd",
                    "Fp",
                    "arch:",
                    "favor:",
                    "guard:",
                    "Qspectre",
                    "analyze",
                    "external:",
                    "experimental:",
                    "fp:",
                    "volatile:",
                    "Zp",
                    "Gs",
                    "errorReport:",
                    "constexpr:",
                    "d1",
                    "d2",
                ]
                .iter()
                .any(|prefix| option.starts_with(prefix))
                {
                    // Warning numbers, conformance switches, code generation
                    // details and tool files with no effect in bcc.
                } else {
                    return false;
                },
        }
        true
    }

    fn unsupported(&mut self, invocation: &Invocation, option: &str, why: &str) {
        invocation.error(format_args!("'/{option}' is not supported: {why}"));
        self.errors += 1;
    }
}

/// The output name for `source`: the `/Fo`, `/Fe` or `/Fi` file, or the
/// source's name with `extension` in the working directory or in the
/// option's directory (a name ending in a path separator). A file name
/// given for several sources names only a directory's worth of them, as
/// `cl` rejects; bcc uses the source names then.
fn named(option: Option<&Path>, source: &OsStr, extension: &str, sources: usize) -> PathBuf {
    let stem = Path::new(Path::new(source).file_name().unwrap_or(source)).with_extension(extension);
    match option {
        | Some(option) if option.as_os_str().to_string_lossy().ends_with(['/', '\\']) =>
            option.join(stem),
        | Some(option) if sources == 1 =>
            if option.extension().is_none() {
                option.with_extension(extension)
            } else {
                option.into()
            },
        | _ => stem,
    }
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
         that bcc does not implement are accepted and ignored; see docs/design/driver-modes.md.\n"
    )
}
