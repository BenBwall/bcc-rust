//! The zig cc personality: zig 0.17.1's `zig cc`, which is Clang 22.1.8's
//! driver behind a command line zig reads first (`src/main.zig`, the `cc`
//! loop). zig takes for itself the target (`-target`, in its own query
//! syntax), the processor (`-mcpu=`, and `-march=` and `-mtune=` read the
//! same way), the optimization mode the `-O` options choose, the
//! sanitizers it runs (`-fsanitize=undefined`), position-independent code,
//! frame pointers, stack protection and debug information, and passes the
//! rest to Clang behind its own defaults (`addCCArgs` in
//! `src/Compilation.zig`).
//!
//! This translates exactly those: it reads zig's options, reports zig's
//! own errors in zig's words, and hands every other option to the Clang
//! personality ([`super::gnu`]) together with [`Defaults`], the options zig
//! gives Clang first: the mode's macros and checks, the target's library
//! macros, and Clang 22.1.8's identity.
//!
//! Driver conventions are host CLI policy; the macros zig defines are
//! reserved identifiers (C99 §7.1.3 paragraph 1, p. 166; PDF p. 178).

use std::{
    ffi::OsString,
    fmt::Write as _,
    path::{
        Path,
        PathBuf,
    },
};

use super::{
    Invocation,
    MainError,
    Plan,
    gnu,
    known::{
        Compiler,
        knows,
    },
};
use crate::target::Target;

/// The Clang release zig 0.17.1 bundles, which its `--version`,
/// `-dumpversion` and `__clang_version__` give.
pub(super) const CLANG_VERSION: &str = "22.1.8";
/// The zig release whose `zig cc` the personality mimics.
const ZIG_VERSION: &str = "0.17.1";

/// Translates a `zig cc` command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let arguments = command_arguments(invocation)?;
    let expanded = gnu::expand_response_files(invocation, &arguments, Compiler::Clang)?;
    let mut translation = Translation::default();
    let mut arguments = expanded.iter();
    while let Some(argument) = arguments.next() {
        translation.argument(argument, &mut arguments)?;
    }
    // zig answers its linker's `--version` as it reads it, and with nothing
    // to build stops there.
    if translation.linker_version && translation.inputs == 0 {
        return Ok(Plan::default());
    }
    let defaults = translation.defaults()?;
    gnu::plan_translated(invocation, &translation.out, gnu::Front::Zig(&defaults))
}

/// The arguments after `zig cc`: an executable named `zig` takes zig's
/// command first, of which bcc has only `cc`.
fn command_arguments(invocation: &Invocation) -> Result<Vec<OsString>, MainError> {
    let named_zig = Path::new(&invocation.program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| {
            let stem = stem.to_ascii_lowercase();
            let stem = stem
                .trim_end_matches(|character: char| character.is_ascii_digit() || character == '.');
            let stem = stem.strip_suffix('-').unwrap_or(stem);
            stem == "zig" || stem.ends_with("-zig")
        });
    if !named_zig {
        return Ok(invocation.arguments.clone());
    }
    match invocation
        .arguments
        .first()
        .and_then(|command| command.to_str())
    {
        | Some("cc") => Ok(invocation.arguments[1..].to_vec()),
        | Some(command) => {
            if ZIG_COMMANDS.contains(&command) {
                fatal(format_args!(
                    "'zig {command}' is not supported: bcc implements only 'zig cc'"
                ));
            } else {
                fatal(format_args!("unknown command: {command}"));
            }
            Err(MainError::Reported(1))
        },
        | None => {
            fatal(format_args!("expected command argument"));
            Err(MainError::Reported(1))
        },
    }
}

/// The commands `zig help` lists besides `cc`, in its order.
const ZIG_COMMANDS: [&str; 29] = [
    "build",
    "fetch",
    "init",
    "build-exe",
    "build-lib",
    "build-obj",
    "test",
    "test-obj",
    "run",
    "ast-check",
    "fmt",
    "reduce",
    "translate-c",
    "ar",
    "c++",
    "dlltool",
    "lib",
    "objcopy",
    "objdump",
    "ranlib",
    "rc",
    "env",
    "help",
    "cache-cat",
    "std",
    "libc",
    "targets",
    "version",
    "zen",
];

/// zig's optimization modes, which `zig cc` chooses from the last `-O`
/// option it knows (`.optimize` in the `cc` loop).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    /// `-O0`, `-Og` or none: `-O0` with `UBSan` and the stack protector.
    #[default]
    Debug,
    /// `-O1` to `-O4` and `-Ofast` with `UBSan`'s checks on, which zig
    /// makes safe after the `cc` loop (`main.zig`): `-O2`,
    /// `_FORTIFY_SOURCE=2` and the stack protector, without `NDEBUG`.
    ReleaseSafe,
    /// `-O1` to `-O4` and `-Ofast`: `-O2` and `NDEBUG`.
    ReleaseFast,
    /// `-Os` and `-Oz`: `-Os` and `NDEBUG`, with neither debug information
    /// nor symbols unless a `-g` option keeps them (`root_strip` in
    /// `Compilation/Config.zig`).
    ReleaseSmall,
}

/// How zig compiles `UBSan`'s checks (`sanitize_c`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum UndefinedChecks {
    Off,
    /// Checks that report through a run-time library.
    Full,
    /// Checks that trap.
    Trap,
}

/// The command line as zig reads it so far.
#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent switches of the command line being translated."
)]
struct Translation {
    /// The options zig passes on to Clang, and the inputs, in order.
    out:             Vec<OsString>,
    /// The `-target` operand.
    query:           Option<String>,
    /// The `-mcpu=`, `-march=` or `-mtune=` operand, the last of them.
    cpu:             Option<String>,
    mode:            Mode,
    /// `-fsanitize=undefined` and its relatives, when given.
    undefined:       Option<UndefinedChecks>,
    /// `-f[no-]sanitize=thread`, when given.
    thread:          Option<bool>,
    /// `-f[no-]sanitize=fuzzer`, when given.
    fuzzer:          bool,
    /// `-f[no-]PIC` and `-f[no-]pic`, when given.
    pic:             Option<bool>,
    /// `-f[no-]PIE` and `-f[no-]pie`, when given.
    pie:             Option<bool>,
    /// `-f[no-]omit-frame-pointer`, when given.
    omit_frame:      Option<bool>,
    /// `-fstack-protector` and its relatives, when given.
    stack_protector: Option<bool>,
    /// `-s`, or a `-g` option that keeps debug information.
    strip:           Option<bool>,
    /// The stage options zig reads: `-c`, `-S`, `-E`, `-M` and `-MM`.
    stops:           bool,
    /// `-fsyntax-only`, which zig passes on and so still compiles.
    syntax_only:     bool,
    shared:          bool,
    /// `-static`, and `-dynamic` taking it back.
    static_link:     bool,
    /// `-nostdlib`, `-nodefaultlibs` or `-nolibc`: no C library.
    no_libc:         bool,
    /// The stem of the first C source, which zig names its module after.
    module:          Option<String>,
    /// Whether any input is a source zig compiles.
    has_sources:     bool,
    /// Whether a query (`--version`, `-dumpmachine`) or `-v` answers
    /// without compiling.
    queries:         bool,
    /// The language `-x` selects, when it is not `none`.
    language:        Option<String>,
    /// How many input files the command line names.
    inputs:          usize,
    /// Whether a linker option asked for the linker's version, which zig
    /// prints at once.
    linker_version:  bool,
}

impl Translation {
    /// Reads one argument and its operand.
    fn argument<'a>(
        &mut self,
        argument: &'a OsString,
        rest: &mut impl Iterator<Item = &'a OsString>,
    ) -> Result<(), MainError> {
        let Some(text) = argument.to_str() else {
            self.input(argument);
            return Ok(());
        };
        // The response files that could be read are expanded already; zig
        // stops at one that cannot.
        if let Some(path) = text.strip_prefix('@') {
            let name = match std::fs::metadata(path).map_err(|error| error.kind()) {
                | Err(std::io::ErrorKind::NotFound) => "FileNotFound",
                | Err(std::io::ErrorKind::PermissionDenied) => "AccessDenied",
                | _ => "Unexpected",
            };
            fatal(format_args!(
                "unable to read response file \"{path}\": {name}"
            ));
            return Err(MainError::Reported(1));
        }
        if text == "-" || !text.starts_with('-') {
            self.input(argument);
            return Ok(());
        }
        if text == "-target" {
            let Some(query) = rest.next() else {
                fatal(format_args!("expected parameter after \"-target\""));
                return Err(MainError::Reported(1));
            };
            self.query = Some(query.to_string_lossy().into_owned());
            return Ok(());
        }
        if let Some(query) = text.strip_prefix("--target=") {
            self.query = Some(query.into());
            return Ok(());
        }
        // zig knows Clang's spelling only with the operand joined.
        if text == "--target" {
            fatal(format_args!("unknown clang option: \"--target\""));
            return Err(MainError::Reported(1));
        }
        for prefix in ["-mcpu=", "-march=", "-mtune="] {
            if let Some(cpu) = text.strip_prefix(prefix) {
                self.cpu = Some(cpu.into());
                return Ok(());
            }
        }
        if let Some(level) = text.strip_prefix("-O") {
            match level {
                | "s" | "z" => self.mode = Mode::ReleaseSmall,
                | "1" | "2" | "3" | "4" | "fast" => self.mode = Mode::ReleaseFast,
                | "g" | "0" => self.mode = Mode::Debug,
                // Clang reads the rest: `-O` is `-O1`, `-O5` warns.
                | _ => self.out.push(argument.clone()),
            }
            return Ok(());
        }
        if let Some((enable, names)) = text
            .strip_prefix("-fsanitize=")
            .map(|names| (true, names))
            .or_else(|| {
                text.strip_prefix("-fno-sanitize=")
                    .map(|names| (false, names))
            })
        {
            self.sanitize(argument, enable, names);
            return Ok(());
        }
        if let Some((enable, names)) = text
            .strip_prefix("-fsanitize-trap=")
            .map(|names| (true, names))
            .or_else(|| {
                text.strip_prefix("-fno-sanitize-trap=")
                    .map(|names| (false, names))
            })
        {
            if names.split(',').any(|name| name == "undefined") {
                self.undefined = Some(match (self.undefined, enable) {
                    | (Some(UndefinedChecks::Trap), false) => UndefinedChecks::Full,
                    | (Some(UndefinedChecks::Full | UndefinedChecks::Off) | None, true) =>
                        UndefinedChecks::Trap,
                    // zig cannot tell yet what the mode would choose, and
                    // takes the checks away (the comment in the `cc` loop).
                    | (None, false) => UndefinedChecks::Off,
                    | (Some(checks), _) => checks,
                });
            } else {
                self.out.push(argument.clone());
            }
            return Ok(());
        }
        match text {
            | "-fPIC" | "-fpic" => self.pic = Some(true),
            | "-fno-PIC" | "-fno-pic" => self.pic = Some(false),
            | "-fPIE" | "-fpie" => self.pie = Some(true),
            | "-fno-PIE" | "-fno-pie" => self.pie = Some(false),
            | "-fomit-frame-pointer" => self.omit_frame = Some(true),
            | "-fno-omit-frame-pointer" => self.omit_frame = Some(false),
            | "-fstack-protector" | "-fstack-protector-strong" | "-fstack-protector-all" =>
                if self.stack_protector.is_none() {
                    self.stack_protector = Some(true);
                },
            | "-fno-stack-protector" => self.stack_protector = Some(false),
            // zig strips the debug information and the symbols.
            | "-s" => {
                self.strip = Some(true);
                self.out.push(argument.clone());
            },
            | "-g" | "-gdwarf32" | "-gdwarf64" => self.strip = Some(false),
            | "-g1" | "-gline-tables-only" => {
                self.strip = Some(false);
                self.out.push("-gline-tables-only".into());
            },
            | "-static" => self.static_link = true,
            | "-dynamic" => self.static_link = false,
            | "-shared" => {
                self.shared = true;
                self.out.push(argument.clone());
            },
            | "-x" => {
                let Some(language) = rest.next() else {
                    fatal(format_args!("expected parameter after \"-x\""));
                    return Err(MainError::Reported(1));
                };
                self.language(&language.to_string_lossy())?;
                self.out.push(argument.clone());
                self.out.push(language.clone());
            },
            | "-flto=" => {
                fatal(format_args!(
                    "invalid -flto mode \"\"; must be \"auto\", \"full\", \"thin\", or \
                     \"jobserver\""
                ));
                return Err(MainError::Reported(1));
            },
            | _ => {
                if text.starts_with("-g") {
                    self.strip = Some(false);
                } else if let Some(language) = text.strip_prefix("-x") {
                    self.language(language)?;
                } else if let Some(mode) = text.strip_prefix("-flto=")
                    && !matches!(mode, "auto" | "full" | "thin" | "jobserver")
                {
                    fatal(format_args!(
                        "invalid -flto mode \"{mode}\"; must be \"auto\", \"full\", \"thin\", or \
                         \"jobserver\""
                    ));
                    return Err(MainError::Reported(1));
                }
                match text {
                    | "-c" | "-S" | "-E" | "-M" | "-MM" => self.stops = true,
                    | "-fsyntax-only" => self.syntax_only = true,
                    | "-nostdlib" | "-nodefaultlibs" | "-nolibc" => self.no_libc = true,
                    | "--version" | "-dumpmachine" | "-dumpversion" | "-v" | "-###" =>
                        self.queries = true,
                    | _ => {},
                }
                if text.starts_with("-print-") || text.starts_with("--print-") {
                    self.queries = true;
                }
                if let Some(options) = text.strip_prefix("-Wl,") {
                    // zig's own linker reads these, and answers `--version`
                    // and `-v` in its own words (`zig ld`), which Meson
                    // recognizes; a first line naming LLD would make Meson
                    // take it for LLD.
                    let kept: Vec<&str> = options
                        .split(',')
                        .filter(|option| {
                            let version = matches!(*option, "--version" | "-v");
                            if version {
                                self.linker_version = true;
                                super::print(
                                    &format!(
                                        "zig ld {ZIG_VERSION} (bcc {}; not zig)\n",
                                        env!("CARGO_PKG_VERSION")
                                    ),
                                    false,
                                );
                            }
                            !version
                        })
                        .collect();
                    if !kept.is_empty() {
                        self.out.push(format!("-Wl,{}", kept.join(",")).into());
                    }
                    return Ok(());
                }
                self.out.push(argument.clone());
            },
        }
        Ok(())
    }

    /// An input: a source zig compiles names the module after its stem.
    fn input(&mut self, argument: &OsString) {
        self.inputs += 1;
        let path = Path::new(argument);
        let source = self.language.is_some()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| matches!(extension, "c" | "h" | "s" | "S" | "sx" | "i"));
        if source {
            self.has_sources = true;
            if self.module.is_none() {
                self.module = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned());
            }
        }
        self.out.push(argument.clone());
    }

    /// `-x LANGUAGE`: zig knows the languages it compiles
    /// (`FileExt.toLang`), and `none`.
    fn language(&mut self, language: &str) -> Result<(), MainError> {
        const LANGUAGES: [&str; 10] = [
            "c",
            "c-header",
            "c++",
            "c++-header",
            "objective-c",
            "objective-c-header",
            "objective-c++",
            "objective-c++-header",
            "assembler",
            "assembler-with-cpp",
        ];
        if language == "none" {
            self.language = None;
        } else if LANGUAGES.contains(&language) {
            self.language = Some(language.into());
        } else {
            fatal(format_args!("language not recognized: \"{language}\""));
            return Err(MainError::Reported(1));
        }
        Ok(())
    }

    /// `-f[no-]sanitize=NAMES`. zig takes the whole option for itself when
    /// it names `undefined`, `thread` or a fuzzer, and leaves it to Clang
    /// otherwise.
    fn sanitize(&mut self, argument: &OsString, enable: bool, names: &str) {
        let mut recognized = false;
        for name in names.split(',') {
            match name {
                | "undefined" => {
                    self.undefined = Some(if enable {
                        UndefinedChecks::Full
                    } else {
                        UndefinedChecks::Off
                    });
                    recognized = true;
                },
                | "thread" => {
                    self.thread = Some(enable);
                    recognized = true;
                },
                | "fuzzer" | "fuzzer-no-link" => {
                    self.fuzzer = enable;
                    recognized = true;
                },
                | _ => {},
            }
        }
        if !recognized {
            self.out.push(argument.clone());
        }
    }

    /// Resolves the target and zig's defaults for it, once the whole
    /// command line has been read.
    fn defaults(&mut self) -> Result<Defaults, MainError> {
        let query = match self.query.as_deref() {
            | Some(text) => match Query::parse(text) {
                | Ok(query) => query,
                | Err(error) => {
                    error.report(text);
                    return Err(MainError::Reported(1));
                },
            },
            | None => Query::native(),
        };
        let Some(target) = query.target() else {
            fatal(format_args!(
                "unable to compile for target '{}': bcc compiles for x86_64-linux-gnu, \
                 x86_64-linux-musl, x86_64-windows-gnu, x86_64-windows-msvc, wasm32-wasi and \
                 wasm32-freestanding",
                self.query.as_deref().unwrap_or("native")
            ));
            return Err(MainError::Reported(1));
        };
        let wasm = matches!(target, Target::Wasm32Unknown | Target::Wasm32Wasi);
        let windows = matches!(target, Target::WindowsGnu | Target::WindowsMsvc);
        let linux = matches!(target, Target::LinuxGnu | Target::LinuxMusl);
        // The processor: the query's native CPU, or `-mcpu`. The options
        // come first, so that the command line's own `-m` options follow.
        let cpu = self
            .cpu
            .clone()
            .or_else(|| (query.native_cpu && !wasm).then(|| "native".to_owned()));
        let machine = match cpu {
            | Some(cpu) => match processor_options(&cpu, wasm) {
                | Ok(options) => options,
                | Err(()) => return Err(MainError::Reported(1)),
            },
            | None => Vec::new(),
        };
        drop(self.out.splice(0..0, machine));
        // A thread or fuzzer sanitizer that zig took is Clang's again.
        if let Some(enable) = self.thread {
            self.out.push(
                if enable {
                    "-fsanitize=thread"
                } else {
                    "-fno-sanitize=thread"
                }
                .into(),
            );
        }
        if self.fuzzer {
            self.out.push("-fsanitize=fuzzer-no-link".into());
        }
        let links = !self.stops && !self.syntax_only && !self.queries;
        let link_libc = !self.no_libc && !matches!(target, Target::Wasm32Unknown);
        let dynamic = self.shared || (target == Target::LinuxGnu && link_libc && !self.static_link);
        // `requiresPIC`: Windows, and glibc wherever zig links it.
        let requires_pic = windows || (target == Target::LinuxGnu && link_libc);
        let module = self.module.clone().unwrap_or_else(|| "a".into());
        let pie = self.pie.unwrap_or(false);
        // zig settles PIE for the whole compilation before PIC for the
        // module (`Compilation/Config.zig`, then `Module.zig`).
        if pie && self.shared && links {
            fatal(format_args!(
                "dynamic libraries cannot be position independent executables"
            ));
            return Err(MainError::Reported(1));
        }
        if self.pic == Some(false) && self.has_sources {
            if requires_pic {
                fatal(format_args!(
                    "unable to create module \"{module}\": the selected target requires position \
                     independent code"
                ));
                return Err(MainError::Reported(1));
            }
            if pie {
                fatal(format_args!(
                    "unable to create module \"{module}\": making a Position Independent \
                     Executable requires enabling Position Independent Code"
                ));
                return Err(MainError::Reported(1));
            }
            if dynamic {
                fatal(format_args!(
                    "unable to create module \"{module}\": dynamic linking requires enabling \
                     Position Independent Code"
                ));
                return Err(MainError::Reported(1));
            }
        }
        if links && self.static_link && target == Target::LinuxGnu && !query.native_abi {
            fatal(format_args!(
                "libc of the specified target requires dynamic linking"
            ));
            return Err(MainError::Reported(1));
        }
        // A PIE is position-independent code too.
        let pic = self.pic.unwrap_or(requires_pic || pie || dynamic);
        // `UBSan`'s checks make ReleaseFast safe, in `-O1` to `-Ofast` alike.
        let mode = match (self.mode, self.undefined) {
            | (Mode::ReleaseFast, Some(UndefinedChecks::Full | UndefinedChecks::Trap)) =>
                Mode::ReleaseSafe,
            | (mode, _) => mode,
        };
        let debug = mode == Mode::Debug;
        let safe = matches!(mode, Mode::Debug | Mode::ReleaseSafe);
        let mut compile: Vec<OsString> = Vec::new();
        // Clang 22.1.8's identity over the pinned Clang's, which has four
        // macros Clang 22 lacks.
        let (major, rest) = CLANG_VERSION.split_once('.').unwrap_or((CLANG_VERSION, ""));
        let patch = rest.rsplit_once('.').map_or("0", |(_, patch)| patch);
        for (name, value) in [
            ("__clang_major__", major.to_owned()),
            ("__clang_patchlevel__", patch.to_owned()),
            ("__clang_version__", format!("\"{CLANG_VERSION} \"")),
            ("__VERSION__", format!("\"Clang {CLANG_VERSION}\"")),
        ] {
            compile.push(format!("-U{name}").into());
            compile.push(format!("-D{name}={value}").into());
        }
        for name in [
            "__SIG_ATOMIC_MIN__",
            "__SIG_ATOMIC_TYPE__",
            "__WCHAR_MIN__",
            "__WINT_MIN__",
        ] {
            compile.push(format!("-U{name}").into());
        }
        // `addCCArgs` in `Compilation.zig`.
        match mode {
            | Mode::Debug => {},
            | Mode::ReleaseSafe => compile.push("-D_FORTIFY_SOURCE=2".into()),
            | Mode::ReleaseFast | Mode::ReleaseSmall => compile.push("-DNDEBUG".into()),
        }
        if link_libc {
            if let Some(minor) = query.glibc_minor(target) {
                compile.push(format!("-D__GLIBC_MINOR__={minor}").into());
            }
            if target == Target::WindowsGnu {
                compile.push("-D__MSVCRT_VERSION__=0xE00".into());
                compile
                    .push(format!("-D_WIN32_WINNT=0x{:04x}", query.windows_minimum >> 16).into());
                compile.push("-D__CRT__NO_INLINE".into());
            }
        }
        // zig's `-fno-spell-checking` changes nothing, since bcc suggests no
        // corrections, and bcc has no `-Wdate-time` for the `-Werror=date-time`
        // of the release modes (driver-modes.md, "zig cc").
        // UBSan reports through the bundled library, which is glibc's; zig's
        // own library stops at the first report, which `-fno-sanitize-recover`
        // gives Clang's.
        let mut undefined = self.undefined.unwrap_or(if debug {
            UndefinedChecks::Full
        } else {
            UndefinedChecks::Off
        });
        // zig always has its library; a bundled clang built without
        // compiler-rt has none, and then the checks trap, which stops the
        // program at the first one as zig's library does, without its
        // report. Queries and options alone compile nothing.
        let mut runtime = None;
        if undefined == UndefinedChecks::Full && target == Target::LinuxGnu && self.inputs > 0 {
            runtime = bundled_undefined_runtime(target);
            if runtime.is_none() {
                undefined = UndefinedChecks::Trap;
            }
        }
        match undefined {
            | UndefinedChecks::Off => {},
            | _ if target != Target::LinuxGnu => {
                // zig's checks, which bcc writes only for glibc's targets.
                // An explicit request is refused there as Clang's would be.
                if self.undefined.is_some() {
                    self.out.push("-fsanitize=undefined".into());
                }
            },
            | UndefinedChecks::Full => {
                compile.push("-fsanitize=undefined".into());
                compile.push("-fno-sanitize-recover=undefined".into());
            },
            | UndefinedChecks::Trap => {
                compile.push("-fsanitize=undefined".into());
                compile.push("-fsanitize-trap=undefined".into());
            },
        }
        compile.push(match mode {
            | Mode::Debug => "-O0".into(),
            | Mode::ReleaseSafe | Mode::ReleaseFast => "-O2".into(),
            | Mode::ReleaseSmall => "-Os".into(),
        });
        if linux {
            // PIE first: Clang reads `-fno-PIE` as `-fno-PIC` too.
            compile.push(if pie { "-fPIE" } else { "-fno-PIE" }.into());
            compile.push(if pic { "-fPIC" } else { "-fno-PIC" }.into());
            compile.push(
                if self.omit_frame == Some(true) {
                    "-fomit-frame-pointer"
                } else {
                    "-fno-omit-frame-pointer"
                }
                .into(),
            );
            // The checks call the C library's `__stack_chk_fail`.
            if self.stack_protector.unwrap_or(safe && link_libc) {
                compile.push("-fstack-protector-strong".into());
            }
        }
        if target == Target::Wasm32Unknown {
            compile.push("-ffreestanding".into());
        }
        // zig writes DWARF 4 unless asked to strip, as ReleaseSmall is by
        // default; bcc writes it through the LLVM back end.
        let strip = self.strip.unwrap_or(mode == Mode::ReleaseSmall);
        if !strip && !wasm {
            compile.push(
                if target == Target::WindowsMsvc {
                    "-g"
                } else {
                    "-gdwarf-4"
                }
                .into(),
            );
        }
        let mut link = Vec::new();
        if linux && !pie && !self.shared {
            link.push("-no-pie".into());
        }
        // A stripped ReleaseSmall links without symbols too, as `-s` does;
        // an MSVC target's image has no symbol table without debug
        // information.
        if strip && self.strip.is_none() && target != Target::WindowsMsvc {
            link.push("-Wl,-s".into());
        }
        // zig links its UBSan library into a shared library too, which
        // Clang's driver leaves to the executable; bcc links the bundled
        // clang's archive there, its symbols kept local.
        let mut link_after = Vec::new();
        if self.shared
            && let Some(archive) = runtime
        {
            link_after.push(archive.into_os_string());
            link_after.push(format!("-Wl,--exclude-libs,{UNDEFINED_RUNTIME}").into());
        }
        if self.static_link {
            self.out.push("-static".into());
        }
        Ok(Defaults {
            target,
            compile,
            link,
            link_after,
            triple: query.llvm_triple(target),
            machine: query.clang_triple.clone(),
        })
    }
}

/// The bundled clang's `UBSan` library, which its driver links for
/// `-fsanitize=undefined`.
const UNDEFINED_RUNTIME: &str = "libclang_rt.ubsan_standalone.a";

/// The bundled clang's [`UNDEFINED_RUNTIME`] for `target`, when it was built
/// with compiler-rt.
fn bundled_undefined_runtime(target: Target) -> Option<PathBuf> {
    let directory = gnu::bundled_clang_answer("-print-runtime-dir", target)?;
    let archive = Path::new(&directory).join(UNDEFINED_RUNTIME);
    archive.is_file().then_some(archive)
}

/// What zig gives Clang beside the options it passes on.
pub(super) struct Defaults {
    pub(super) target:     Target,
    /// bcc's options before the command line's own, in every step.
    pub(super) compile:    Vec<OsString>,
    /// bcc's options in a step that links.
    pub(super) link:       Vec<OsString>,
    /// bcc's inputs after the command line's own in a step that links.
    pub(super) link_after: Vec<OsString>,
    /// The LLVM triple of zig's target, which `--version` and `-v` print:
    /// `x86_64-unknown-linux5.10.0-gnu2.31.0`.
    pub(super) triple:     String,
    /// What Clang's own driver, which zig runs for `-dumpmachine`, makes of
    /// the `-target` operand.
    pub(super) machine:    String,
}

impl Defaults {
    /// Clang's error for a [`Defaults::machine`] whose environment carries
    /// a version Clang does not read, as zig's glibc version is: `version
    /// '.2.17' in target triple 'x86_64-unknown-linux-gnu.2.17' is invalid`.
    pub(super) fn machine_error(&self) -> Option<String> {
        let environment = self.machine.split('-').nth(3)?;
        let version = environment.strip_prefix("gnu")?;
        version.starts_with('.').then(|| {
            format!(
                "version '{version}' in target triple '{}' is invalid",
                self.machine
            )
        })
    }

    /// `--version`, and what `-v` prints first: Clang's banner with zig's
    /// target, naming bcc.
    pub(super) fn banner(triple: &str) -> String {
        let mut text = String::new();
        _ = writeln!(
            text,
            "clang version {CLANG_VERSION} (bcc {}, a zig cc-compatible C compiler; not zig)",
            env!("CARGO_PKG_VERSION")
        );
        _ = writeln!(text, "Target: {triple}");
        text.push_str("Thread model: posix\n");
        if let Some(directory) = std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(Path::parent)
        {
            _ = writeln!(text, "InstalledDir: {}", directory.display());
        }
        text
    }
}

/// A semantic version as zig's target queries spell it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// `Query.parseVersion`: two or three numbers, the patch 0 if absent.
    fn parse(text: &str) -> Result<Self, VersionError> {
        let mut components = text.split('.');
        let major = components.next().unwrap_or_default();
        let minor = components.next().ok_or(VersionError::Invalid)?;
        let patch = components.next().unwrap_or("0");
        if components.next().is_some() {
            return Err(VersionError::Invalid);
        }
        let number = |component: &str| {
            if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(VersionError::Invalid);
            }
            component.parse::<u64>().map_err(|_| VersionError::Overflow)
        };
        Ok(Self::new(number(major)?, number(minor)?, number(patch)?))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

enum VersionError {
    Invalid,
    Overflow,
}

/// The parts of a zig target query (`std.Target.Query.parse`) that bcc's
/// targets depend on.
struct Query {
    /// The architecture, `None` for `native`.
    architecture:    Option<String>,
    /// The operating system, `None` for `native`.
    os:              Option<String>,
    /// The earliest Linux version the query names.
    linux_minimum:   Option<Version>,
    /// The earliest Windows version, as zig numbers them (`0x0A000000`
    /// for `win10`).
    windows_minimum: u32,
    /// The ABI, `None` when the query has none.
    abi:             Option<String>,
    /// The glibc version after the ABI.
    glibc:           Option<Version>,
    /// Whether zig compiles for the machine's own processor: no
    /// architecture was named.
    native_cpu:      bool,
    /// Whether zig detects the ABI and glibc version of the machine.
    native_abi:      bool,
    /// What Clang's driver makes of the query (`Triple::normalize`).
    clang_triple:    String,
}

/// Why zig refuses a target query: the names of its errors.
enum QueryError {
    UnknownArchitecture(String),
    MissingOperatingSystem,
    UnknownOperatingSystem,
    InvalidOperatingSystemVersion,
    UnknownApplicationBinaryInterface,
    InvalidAbiVersion,
    InvalidVersion,
    Overflow,
    UnexpectedExtraField,
}

impl QueryError {
    /// Prints zig's error for `query` (`parseTargetQueryOrReportFatalError`
    /// in `lib/std/zig.zig`).
    fn report(&self, query: &str) {
        let name = match self {
            | Self::UnknownArchitecture(architecture) => {
                let mut text = String::from("info: available architectures:\n");
                for name in ARCHITECTURES.lines() {
                    _ = writeln!(text, " {name}");
                }
                text.push_str(" native\n\n");
                _ = writeln!(text, "error: unknown architecture: '{architecture}'");
                super::print(&text, true);
                return;
            },
            | Self::MissingOperatingSystem => "MissingOperatingSystem",
            | Self::UnknownOperatingSystem => "UnknownOperatingSystem",
            | Self::InvalidOperatingSystemVersion => "InvalidOperatingSystemVersion",
            | Self::UnknownApplicationBinaryInterface => "UnknownApplicationBinaryInterface",
            | Self::InvalidAbiVersion => "InvalidAbiVersion",
            | Self::InvalidVersion => "InvalidVersion",
            | Self::Overflow => "Overflow",
            | Self::UnexpectedExtraField => "UnexpectedExtraField",
        };
        fatal(format_args!(
            "unable to parse target query '{query}': {name}"
        ));
    }
}

/// zig's architectures (`std.Target.Cpu.Arch`), in its order: the list
/// `zig cc -target bogus-linux` prints, without its indent and `native`.
const ARCHITECTURES: &str = include_str!("zig/architectures.list");
/// zig's operating systems (`std.Target.Os.Tag`).
const OPERATING_SYSTEMS: [&str; 48] = [
    "freestanding",
    "other",
    "contiki",
    "fuchsia",
    "hermit",
    "managarm",
    "haiku",
    "hurd",
    "illumos",
    "linux",
    "plan9",
    "rtems",
    "serenity",
    "dragonfly",
    "freebsd",
    "netbsd",
    "openbsd",
    "driverkit",
    "ios",
    "maccatalyst",
    "macos",
    "tvos",
    "visionos",
    "watchos",
    "windows",
    "uefi",
    "3ds",
    "wiiu",
    "switch",
    "gba",
    "psx",
    "ps3",
    "ps4",
    "ps5",
    "psp",
    "vita",
    "emscripten",
    "wasi",
    "amdhsa",
    "amdpal",
    "cuda",
    "mesa3d",
    "nvcl",
    "opencl",
    "opengl",
    "vulkan",
    "tios",
    "ashetos",
];
/// The operating systems zig gives no versions (`versionRangeTag` is
/// `.none`).
const UNVERSIONED_SYSTEMS: [&str; 15] = [
    "freestanding",
    "other",
    "managarm",
    "haiku",
    "illumos",
    "plan9",
    "serenity",
    "psx",
    "ps3",
    "ps4",
    "ps5",
    "gba",
    "emscripten",
    "mesa3d",
    "ashetos",
];
/// zig's ABIs (`std.Target.Abi`).
const ABIS: [&str; 30] = [
    "none",
    "gnu",
    "gnuabin32",
    "gnuabi64",
    "gnueabi",
    "gnueabihf",
    "gnuf32",
    "gnusf",
    "gnux32",
    "eabi",
    "eabihf",
    "abin32",
    "x32",
    "ilp32",
    "android",
    "androideabi",
    "musl",
    "muslabin32",
    "muslabi64",
    "musleabi",
    "musleabihf",
    "muslf32",
    "muslsf",
    "muslx32",
    "msvc",
    "itanium",
    "simulator",
    "ohos",
    "ohoseabi",
    "call0",
];
/// zig's Windows versions (`std.Target.Os.WindowsVersion`).
const WINDOWS_VERSIONS: [(&str, u32); 28] = [
    ("nt4", 0x0400_0000),
    ("win2k", 0x0500_0000),
    ("xp", 0x0501_0000),
    ("ws2003", 0x0502_0000),
    ("vista", 0x0600_0000),
    ("win7", 0x0601_0000),
    ("win8", 0x0602_0000),
    ("win8_1", 0x0603_0000),
    ("win10", 0x0A00_0000),
    ("win10_th2", 0x0A00_0001),
    ("win10_rs1", 0x0A00_0002),
    ("win10_rs2", 0x0A00_0003),
    ("win10_rs3", 0x0A00_0004),
    ("win10_rs4", 0x0A00_0005),
    ("win10_rs5", 0x0A00_0006),
    ("win10_19h1", 0x0A00_0007),
    ("win10_vb", 0x0A00_0008),
    ("win10_mn", 0x0A00_0009),
    ("win10_fe", 0x0A00_000A),
    ("win10_co", 0x0A00_000B),
    ("win10_ni", 0x0A00_000C),
    ("win10_cu", 0x0A00_000D),
    ("win11_zn", 0x0A00_000E),
    ("win11_ga", 0x0A00_000F),
    ("win11_ge", 0x0A00_0010),
    ("win11_dt", 0x0A00_0011),
    ("win11_br", 0x0A00_0012),
    ("win11_kr", 0x0A00_0013),
];

impl Query {
    /// The query without `-target`: the machine itself.
    fn native() -> Self {
        Self {
            architecture:    None,
            os:              None,
            linux_minimum:   None,
            windows_minimum: 0x0A00_0000,
            abi:             None,
            glibc:           None,
            native_cpu:      true,
            native_abi:      true,
            clang_triple:    "x86_64-unknown-linux-musl".into(),
        }
    }

    /// `std.Target.Query.parse` of `ARCH-OS[.VERSIONS]-ABI[.VERSION]`.
    fn parse(text: &str) -> Result<Self, QueryError> {
        let mut query = Self::native();
        query.clang_triple = normalized_triple(text);
        let mut fields = text.split('-');
        let architecture = fields.next().unwrap_or_default();
        if architecture != "native" {
            if !ARCHITECTURES.lines().any(|name| name == architecture) {
                return Err(QueryError::UnknownArchitecture(architecture.into()));
            }
            query.architecture = Some(architecture.into());
            query.native_cpu = false;
        }
        match fields.next() {
            | Some(os) => query.parse_os(os)?,
            | None if query.architecture.is_some() =>
                return Err(QueryError::MissingOperatingSystem),
            | None => {},
        }
        if let Some(abi_text) = fields.next() {
            let (abi, version) = match abi_text.split_once('.') {
                | Some((abi, version)) => (abi, Some(version)),
                | None => (abi_text, None),
            };
            if !ABIS.contains(&abi) {
                return Err(QueryError::UnknownApplicationBinaryInterface);
            }
            query.abi = Some(abi.into());
            if let Some(version) = version {
                if abi.starts_with("gnu") {
                    query.glibc =
                        Some(Version::parse(version).map_err(|_| QueryError::InvalidAbiVersion)?);
                } else if abi.starts_with("android") {
                    if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(QueryError::InvalidVersion);
                    }
                    if version.parse::<u32>().is_err() {
                        return Err(QueryError::Overflow);
                    }
                } else {
                    return Err(QueryError::InvalidAbiVersion);
                }
            }
        }
        if fields.next().is_some() {
            return Err(QueryError::UnexpectedExtraField);
        }
        query.native_abi = query.abi.is_none() && query.os.is_none();
        Ok(query)
    }

    /// `parseOs`: the operating system and its version range, of which zig
    /// keeps the minimum.
    fn parse_os(&mut self, text: &str) -> Result<(), QueryError> {
        let (name, versions) = match text.split_once('.') {
            | Some((name, versions)) => (name, versions),
            | None => (text, ""),
        };
        if name != "native" {
            if !OPERATING_SYSTEMS.contains(&name) {
                return Err(QueryError::UnknownOperatingSystem);
            }
            self.os = Some(name.into());
        }
        if versions.is_empty() {
            return Ok(());
        }
        let tag = self.os.as_deref().unwrap_or("linux");
        if UNVERSIONED_SYSTEMS.contains(&tag) {
            return Err(QueryError::InvalidOperatingSystemVersion);
        }
        let mut range = versions.split("...");
        let minimum = range.next().unwrap_or_default();
        let maximum = range.next();
        if tag == "windows" {
            for version in std::iter::once(minimum).chain(maximum) {
                let value =
                    windows_version(version).ok_or(QueryError::InvalidOperatingSystemVersion)?;
                if version == minimum {
                    self.windows_minimum = value;
                }
            }
        } else {
            for version in std::iter::once(minimum).chain(maximum) {
                let parsed = Version::parse(version)
                    .map_err(|_| QueryError::InvalidOperatingSystemVersion)?;
                if version == minimum {
                    self.linux_minimum = Some(parsed);
                }
            }
        }
        Ok(())
    }

    /// The bcc target the query selects, if bcc has it.
    fn target(&self) -> Option<Target> {
        let architecture = self.architecture.as_deref().unwrap_or("x86_64");
        let os = self
            .os
            .as_deref()
            .unwrap_or(if cfg!(windows) { "windows" } else { "linux" });
        let abi = self.abi.as_deref();
        match (architecture, os, abi) {
            | ("x86_64", "linux", None) if self.os.is_none() => Some(Target::LinuxGnu),
            | ("x86_64", "linux", Some("gnu")) => Some(Target::LinuxGnu),
            | ("x86_64", "linux", Some("musl") | None) => Some(Target::LinuxMusl),
            | ("x86_64", "windows", Some("gnu") | None) => Some(Target::WindowsGnu),
            | ("x86_64", "windows", Some("msvc")) => Some(Target::WindowsMsvc),
            | ("wasm32", "wasi", Some("musl") | None) => Some(Target::Wasm32Wasi),
            | ("wasm32", "freestanding", Some("none") | None) => Some(Target::Wasm32Unknown),
            | _ => None,
        }
    }

    /// The glibc minor version `__GLIBC_MINOR__` gives: the query's, the
    /// machine's when the ABI is detected, or zig's default, 2.31.
    fn glibc_minor(&self, target: Target) -> Option<u64> {
        if target != Target::LinuxGnu {
            return None;
        }
        let version = self
            .glibc
            .or_else(|| self.native_abi.then(host_glibc).flatten())
            .unwrap_or(DEFAULT_GLIBC);
        Some(version.minor)
    }

    /// zig's LLVM triple for `target`
    /// (`std.zig.llvm.Builder.tripleForTarget`).
    fn llvm_triple(&self, target: Target) -> String {
        match target {
            | Target::LinuxGnu | Target::LinuxMusl => {
                let linux = self
                    .linux_minimum
                    .or_else(|| self.os.is_none().then(host_linux).flatten())
                    .unwrap_or(DEFAULT_LINUX);
                if target == Target::LinuxGnu {
                    let glibc = self
                        .glibc
                        .or_else(|| self.native_abi.then(host_glibc).flatten())
                        .unwrap_or(DEFAULT_GLIBC);
                    format!("x86_64-unknown-linux{linux}-gnu{glibc}")
                } else {
                    format!("x86_64-unknown-linux{linux}-musl")
                }
            },
            | Target::WindowsGnu => "x86_64-unknown-windows-gnu".into(),
            | Target::WindowsMsvc => "x86_64-unknown-windows-msvc".into(),
            | Target::Wasm32Wasi => "wasm32-unknown-wasip1-unknown".into(),
            | Target::Wasm32Unknown => "wasm32-unknown-unknown-unknown".into(),
        }
    }
}

/// zig's earliest Linux and glibc for a Linux query that names none.
const DEFAULT_LINUX: Version = Version::new(5, 10, 0);
const DEFAULT_GLIBC: Version = Version::new(2, 31, 0);

/// A Windows version zig names (`WindowsVersion.parse`): a name, or a number
/// in any base Zig's `parseInt` reads.
fn windows_version(text: &str) -> Option<u32> {
    if let Some((_, value)) = WINDOWS_VERSIONS.iter().find(|(name, _)| *name == text) {
        return Some(*value);
    }
    let (digits, radix) = if let Some(hex) = text.strip_prefix("0x") {
        (hex, 16)
    } else if let Some(octal) = text.strip_prefix("0o") {
        (octal, 8)
    } else if let Some(binary) = text.strip_prefix("0b") {
        (binary, 2)
    } else {
        (text, 10)
    };
    u32::from_str_radix(digits, radix).ok()
}

/// The running Linux kernel's version, as zig reads `uname`: the numbers
/// that start its release.
fn host_linux() -> Option<Version> {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let numbers: String = release
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let mut components = numbers.split('.').map(|number| number.parse::<u64>().ok());
    let major = components.next()??;
    let minor = components.next().flatten().unwrap_or(0);
    let patch = components.next().flatten().unwrap_or(0);
    Some(Version::new(major, minor, patch))
}

/// The machine's glibc version, as zig finds it: the newest `GLIBC_2.N`
/// symbol version that the `libc.so.6` beside the dynamic linker defines
/// (`glibcVerFromSoFile` in `lib/std/zig/system.zig`).
fn host_glibc() -> Option<Version> {
    let linker = std::fs::canonicalize("/lib64/ld-linux-x86-64.so.2").ok();
    let candidates = linker
        .as_deref()
        .and_then(Path::parent)
        .map(|directory| directory.join("libc.so.6"))
        .into_iter()
        .chain(
            [
                "/lib/x86_64-linux-gnu/libc.so.6",
                "/lib64/libc.so.6",
                "/usr/lib64/libc.so.6",
                "/usr/lib/libc.so.6",
            ]
            .map(PathBuf::from),
        );
    for candidate in candidates {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        let mut newest: Option<Version> = None;
        let marker = b"GLIBC_2.";
        let mut at = 0;
        while let Some(found) = bytes[at..]
            .windows(marker.len())
            .position(|window| window == marker)
        {
            let start = at + found + b"GLIBC_".len();
            let end = bytes[start..]
                .iter()
                .position(|byte| !byte.is_ascii_digit() && *byte != b'.')
                .map_or(bytes.len(), |length| start + length);
            if let Ok(text) = std::str::from_utf8(&bytes[start..end])
                && let Ok(version) = Version::parse(text.trim_end_matches('.'))
                && newest.is_none_or(|newest| {
                    (version.major, version.minor, version.patch)
                        > (newest.major, newest.minor, newest.patch)
                })
            {
                newest = Some(version);
            }
            at = end.max(start);
        }
        if newest.is_some() {
            return newest;
        }
    }
    None
}

/// What Clang's driver makes of a target query (`Triple::normalize`), for
/// the queries zig hands it whole: a vendor is added before a known
/// operating system and an operating system before a known environment,
/// and Windows defaults to MSVC.
fn normalized_triple(text: &str) -> String {
    const SYSTEMS: [&str; 22] = [
        "linux",
        "windows",
        "win32",
        "wasi",
        "emscripten",
        "darwin",
        "macos",
        "ios",
        "tvos",
        "watchos",
        "freebsd",
        "netbsd",
        "openbsd",
        "dragonfly",
        "fuchsia",
        "haiku",
        "hurd",
        "illumos",
        "solaris",
        "uefi",
        "cuda",
        "amdhsa",
    ];
    const ENVIRONMENTS: [&str; 9] = [
        "gnu",
        "musl",
        "msvc",
        "android",
        "eabi",
        "itanium",
        "cygnus",
        "simulator",
        "ohos",
    ];
    let mut parts: Vec<&str> = text.split('-').collect();
    let is = |part: &str, names: &[&str]| names.iter().any(|name| part.starts_with(name));
    if parts.len() >= 2 && is(parts[1], &SYSTEMS) {
        parts.insert(1, "unknown");
    }
    if parts.len() >= 3 && !is(parts[2], &SYSTEMS) && is(parts[2], &ENVIRONMENTS) {
        parts.insert(2, "unknown");
    }
    if parts.len() == 3 && parts[2].starts_with("windows") {
        parts.push("msvc");
    }
    parts.join("-")
}

/// The options that select `cpu`: zig's `NAME+FEATURE-FEATURE` for the
/// processor zig names, onto bcc's `-march=` and `-m` options. Reports zig's
/// error for a processor or feature it does not know.
fn processor_options(cpu: &str, wasm: bool) -> Result<Vec<OsString>, ()> {
    let (cpus, features, architecture) = if wasm {
        (WASM32_CPUS, WASM32_FEATURES, "wasm32")
    } else {
        (X86_64_CPUS, X86_64_FEATURES, "x86_64")
    };
    let end = cpu.find(['+', '-']).unwrap_or(cpu.len());
    let name = &cpu[..end];
    let mut options = Vec::new();
    match name {
        | "native" => options.push("-march=native".into()),
        | "baseline" => {},
        | _ if cpus.lines().any(|known| known == name) =>
            if !wasm && name != "x86_64" {
                options.push(format!("-march={}", name.replace('_', "-")).into());
            },
        | _ => {
            let mut text = format!("info: available CPUs for architecture '{architecture}':\n");
            for known in cpus.lines() {
                _ = writeln!(text, " {known}");
            }
            _ = writeln!(text, "\nerror: unknown CPU: '{name}'");
            super::print(&text, true);
            return Err(());
        },
    }
    let mut rest = &cpu[end..];
    while let Some(sign) = rest.chars().next() {
        let after = &rest[1..];
        let length = after.find(['+', '-']).unwrap_or(after.len());
        let feature = &after[..length];
        rest = &after[length..];
        if !features.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(known, _)| known == feature)
        }) {
            let mut text =
                format!("info: available CPU features for architecture '{architecture}':\n");
            for line in features.lines() {
                _ = writeln!(text, " {line}");
            }
            _ = writeln!(text, "\nerror: unknown CPU feature: '{feature}'");
            super::print(&text, true);
            return Err(());
        }
        if wasm {
            continue;
        }
        // zig spells LLVM's feature names with `_` for `-` and `.`; Clang's
        // options follow LLVM's. Features no option names stay LLVM's own.
        let llvm = match feature {
            | "sse4_1" | "sse4_2" | "avx10_1" | "avx10_2" => feature.replace('_', "."),
            | _ => feature.replace('_', "-"),
        };
        let option = if sign == '+' {
            format!("-m{llvm}")
        } else {
            format!("-mno-{llvm}")
        };
        if knows(Compiler::Clang, &option) {
            options.push(option.into());
        }
    }
    Ok(options)
}

/// zig's processors and features for the architectures bcc compiles for
/// (`std.Target.x86` and `std.Target.wasm`), as `zig cc` lists them for an
/// unknown `-mcpu=bogus` or `-mcpu=baseline+bogus`, without their indent.
const X86_64_CPUS: &str = include_str!("zig/x86_64_cpus.list");
const X86_64_FEATURES: &str = include_str!("zig/x86_64_features.list");
const WASM32_CPUS: &str = include_str!("zig/wasm32_cpus.list");
const WASM32_FEATURES: &str = include_str!("zig/wasm32_features.list");

/// Prints `error: MESSAGE`, as zig words the errors that stop it
/// (`std.process.fatal`).
fn fatal(message: std::fmt::Arguments<'_>) {
    super::print(&format!("error: {message}\n"), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_queries_select_targets_as_zig_reads_them() {
        let target = |text: &str| Query::parse(text).ok().and_then(|query| query.target());
        assert_eq!(target("x86_64-linux-gnu"), Some(Target::LinuxGnu));
        assert_eq!(target("x86_64-linux-gnu.2.17"), Some(Target::LinuxGnu));
        assert_eq!(
            target("x86_64-linux.6.1...6.5-gnu.2.28"),
            Some(Target::LinuxGnu)
        );
        assert_eq!(target("x86_64-linux"), Some(Target::LinuxMusl));
        assert_eq!(target("x86_64-linux-musl"), Some(Target::LinuxMusl));
        // A native operating system is the machine's, as zig detects it:
        // glibc on Linux, MinGW on Windows.
        let native = if cfg!(windows) {
            Target::WindowsGnu
        } else {
            Target::LinuxGnu
        };
        assert_eq!(target("x86_64-native"), Some(native));
        assert_eq!(target("native"), Some(native));
        assert_eq!(target("x86_64-windows"), Some(Target::WindowsGnu));
        assert_eq!(target("x86_64-windows.win7-gnu"), Some(Target::WindowsGnu));
        assert_eq!(target("x86_64-windows-msvc"), Some(Target::WindowsMsvc));
        assert_eq!(target("wasm32-wasi"), Some(Target::Wasm32Wasi));
        assert_eq!(target("wasm32-freestanding"), Some(Target::Wasm32Unknown));
        assert_eq!(target("aarch64-linux-gnu"), None);
        assert_eq!(target("x86_64-macos"), None);
        assert_eq!(target("x86_64-linux-gnux32"), None);
    }

    #[test]
    fn malformed_queries_fail_with_zigs_error_names() {
        let error = |text: &str| match Query::parse(text) {
            | Ok(_) => "ok",
            | Err(QueryError::UnknownArchitecture(_)) => "UnknownArchitecture",
            | Err(QueryError::MissingOperatingSystem) => "MissingOperatingSystem",
            | Err(QueryError::UnknownOperatingSystem) => "UnknownOperatingSystem",
            | Err(QueryError::InvalidOperatingSystemVersion) => "InvalidOperatingSystemVersion",
            | Err(QueryError::UnknownApplicationBinaryInterface) =>
                "UnknownApplicationBinaryInterface",
            | Err(QueryError::InvalidAbiVersion) => "InvalidAbiVersion",
            | Err(QueryError::InvalidVersion) => "InvalidVersion",
            | Err(QueryError::Overflow) => "Overflow",
            | Err(QueryError::UnexpectedExtraField) => "UnexpectedExtraField",
        };
        // Each measured against zig 0.17.1.
        assert_eq!(error("x86_64-unknown-linux-gnu"), "UnknownOperatingSystem");
        assert_eq!(error("bogus-linux-gnu"), "UnknownArchitecture");
        assert_eq!(error("x86_64"), "MissingOperatingSystem");
        assert_eq!(
            error("x86_64-linux-bogus"),
            "UnknownApplicationBinaryInterface"
        );
        assert_eq!(error("x86_64-linux-gnu.abc"), "InvalidAbiVersion");
        assert_eq!(error("x86_64-linux-gnu.2"), "InvalidAbiVersion");
        assert_eq!(error("x86_64-linux.6-gnu"), "InvalidOperatingSystemVersion");
        assert_eq!(
            error("x86_64-windows.bogus-gnu"),
            "InvalidOperatingSystemVersion"
        );
        assert_eq!(
            error("x86_64-freestanding.1.2-none"),
            "InvalidOperatingSystemVersion"
        );
        assert_eq!(error("x86_64-linux-gnu-extra"), "UnexpectedExtraField");
        assert_eq!(error("wasm32-wasip1"), "UnknownOperatingSystem");
        assert_eq!(error("x86_64-linux-gnu.2.17.1"), "ok");
    }

    #[test]
    fn triples_follow_zig_and_clang() {
        let query = |text: &str| Query::parse(text).ok().unwrap_or_else(Query::native);
        let triple = |text: &str| {
            let query = query(text);
            query.target().map(|target| query.llvm_triple(target))
        };
        assert_eq!(
            triple("x86_64-linux-gnu").as_deref(),
            Some("x86_64-unknown-linux5.10.0-gnu2.31.0")
        );
        assert_eq!(
            triple("x86_64-linux.6.1-gnu.2.17").as_deref(),
            Some("x86_64-unknown-linux6.1.0-gnu2.17.0")
        );
        assert_eq!(
            triple("x86_64-linux").as_deref(),
            Some("x86_64-unknown-linux5.10.0-musl")
        );
        assert_eq!(
            triple("wasm32-wasi").as_deref(),
            Some("wasm32-unknown-wasip1-unknown")
        );
        // What Clang's driver makes of the same queries.
        assert_eq!(
            normalized_triple("x86_64-linux-gnu.2.17"),
            "x86_64-unknown-linux-gnu.2.17"
        );
        assert_eq!(
            normalized_triple("x86_64-windows"),
            "x86_64-unknown-windows-msvc"
        );
        assert_eq!(
            normalized_triple("x86_64-native-gnu"),
            "x86_64-native-unknown-gnu"
        );
        assert_eq!(
            normalized_triple("native-linux-gnu"),
            "native-unknown-linux-gnu"
        );
        assert_eq!(
            normalized_triple("wasm32-freestanding"),
            "wasm32-freestanding"
        );
        assert_eq!(
            normalized_triple("wasm32-wasi-musl"),
            "wasm32-unknown-wasi-musl"
        );
        assert_eq!(normalized_triple("x86_64-linux"), "x86_64-unknown-linux");
    }
}
