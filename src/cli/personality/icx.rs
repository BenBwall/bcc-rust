//! The icx personality: Intel oneAPI DPC++/C++ Compiler 2026.2.0's `icx`
//! (also `icx-cc`), Clang 23.1.1's driver behind a wrapper of Intel's own
//! (`bin/icx`). The wrapper reads Intel's options first: it drops the
//! classic `icc` options icx no longer has with `command line warning
//! #10430`, turns `-x` processor codes, `-qopenmp`, `-ipo`, `-fp-model`
//! and the rest into Clang's, and Intel's driver gives Clang its defaults:
//! `-O2`, or `-O0` when a `-g` option is given without an `-O` option, the
//! `fast` floating-point model, code at a fixed address, and Intel's
//! identity.
//!
//! This translates exactly those, and hands the result to the Clang
//! personality ([`super::gnu`]) together with [`Defaults`]. The clang-cl
//! personality ([`super::msvc`]) gives `icx-cl` the same identity
//! ([`identity`]), and its own banner ([`cl_logo`]).
//!
//! Driver conventions are host CLI policy; the macros icx defines are
//! reserved identifiers (C99 §7.1.3 paragraph 1, p. 166; PDF p. 178).

use std::{
    ffi::OsString,
    fmt::Write as _,
    iter::Peekable,
    path::Path,
    slice,
};

use super::{
    Invocation,
    MainError,
    Plan,
    gnu,
    known::Compiler,
};
use crate::target::Target;

/// The oneAPI release whose `icx` the personality mimics.
pub(super) const ICX_VERSION: &str = "2026.2.0";
/// Its build, which `--version` and `__VERSION__` give with the release.
const ICX_BUILD: &str = "20261001";
/// The Clang release icx 2026.2.0 is built from, which `-dumpversion` and
/// `__clang_version__` give.
pub(super) const CLANG_VERSION: &str = "23.1.1";

/// Translates an `icx` command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let expanded = gnu::expand_response_files(invocation, &invocation.arguments, Compiler::Clang)?;
    let mut translation = Translation::default();
    let mut arguments = expanded.iter().peekable();
    while let Some(argument) = arguments.next() {
        translation.argument(argument, &mut arguments);
    }
    // The wrapper reports the classic options before Clang's driver runs.
    if !translation.classic.is_empty() && !translation.classic_disabled {
        let mut text = format!(
            "{}: command line warning #10430: Unsupported command line options encountered\nThese \
             options as listed are not supported.\nFor more information, use \
             '-qnextgen-diag'.\noption list: \n",
            invocation.name
        );
        for option in &translation.classic {
            _ = writeln!(text, "\t{option}");
        }
        super::print(&text, true);
    }
    for message in &translation.wrapper_errors {
        invocation.error(format_args!("{message}"));
    }
    let defaults = translation.defaults();
    gnu::plan_translated(invocation, &translation.out, gnu::Front::Icx(&defaults))
}

/// What Intel's driver gives Clang besides the options it passed on.
pub(super) struct Defaults {
    /// bcc's options before the command line's own, in every step: Intel's
    /// identity, the optimization level, the floating-point model and code
    /// at a fixed address.
    pub(super) compile:  Vec<OsString>,
    /// bcc's options in a step that links.
    pub(super) link:     Vec<OsString>,
    /// The errors the wrapper reported already, which fail the command.
    pub(super) reported: usize,
    /// The errors Clang's driver reports about Intel's options after
    /// reading the command line (`-ax`), after its own.
    pub(super) errors:   Vec<String>,
    /// The debug option whose remark Clang's driver gives when no `-O`
    /// option chose a level: `-g` turns optimization off.
    pub(super) remark:   Option<String>,
}

impl Defaults {
    /// The remark Intel's Clang driver gives for a debug option without an
    /// `-O` option, which turns optimization off as `level` would.
    pub(super) fn remark_text(option: &str, level: &str) -> String {
        format!(
            "note that use of '{option}' without any optimization-level option will turn off most \
             compiler optimizations similar to use of '{level}'; use \
             '-Rno-debug-disables-optimization' to disable this remark \
             [-Rdebug-disables-optimization]"
        )
    }
}

/// `--version`, and what `-v` and `-###` print first: Intel's banner with
/// the target, naming bcc. Clang's driver adds the configuration file
/// Intel installs beside it (`icx.cfg`), which bcc has not.
pub(super) fn banner(target: Target) -> String {
    let mut text = String::new();
    _ = writeln!(
        text,
        "Intel(R) oneAPI DPC++/C++ Compiler {ICX_VERSION} ({ICX_VERSION}.{ICX_BUILD}) (bcc {}, an \
         icx-compatible C compiler; not icx)",
        env!("CARGO_PKG_VERSION")
    );
    _ = writeln!(text, "Target: {}", target.triple());
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

/// The logo `icx-cl` prints to stderr before it does anything else, as
/// `cl` does, naming bcc. Meson recognizes `icx-cl` by its first line.
pub(super) fn cl_logo() -> String {
    format!(
        "Intel(R) oneAPI DPC++/C++ Compiler for applications running on Intel(R) 64, Version \
         {ICX_VERSION} Build {ICX_BUILD}\nbcc {}, an icx-cl-compatible C compiler; not an Intel \
         product.\n\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// The options that make bcc's Clang identity Intel's on `target`, as
/// `icx -dM -E` and `icx-cl` list them beside Clang 23.1.1's macros:
/// `__INTEL_LLVM_COMPILER` and `__INTEL_CLANG_COMPILER` as `VVVVMMUU`, the
/// DPC++ release, Intel's version strings, and the few target macros
/// Intel's Clang adds or changes. Intel's Clang reports no finite math
/// even under `-ffast-math`.
pub(super) fn identity(target: Target) -> Vec<OsString> {
    let mut options = Vec::new();
    let quoted = |text: &str| format!("\"{text}\"");
    let replaced = [
        ("__clang_patchlevel__", "1".to_owned()),
        (
            "__clang_version__",
            quoted(&format!("{CLANG_VERSION} (icx {ICX_VERSION}.{ICX_BUILD})")),
        ),
        (
            "__VERSION__",
            quoted(&format!(
                "Intel(R) oneAPI DPC++/C++ Compiler {ICX_VERSION} ({ICX_VERSION}.{ICX_BUILD})"
            )),
        ),
        ("__FINITE_MATH_ONLY__", "0".to_owned()),
    ];
    for (name, value) in replaced {
        options.push(format!("-U{name}").into());
        options.push(format!("-D{name}={value}").into());
    }
    let added = [
        ("__INTEL_LLVM_COMPILER", "20260200"),
        ("__INTEL_CLANG_COMPILER", "20260200"),
        ("__DPCPP__", "1"),
        ("__dpcpp_major__", "2026"),
        ("__dpcpp_minor__", "2"),
        ("__dpcpp_patchlevel__", "0"),
        ("__AMX_SUPPORTED__", "1"),
        ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_16", "1"),
        ("__I__", "1j"),
        (
            "__LONG_DOUBLE_SIZE__",
            if target == Target::WindowsMsvc {
                "64"
            } else {
                "80"
            },
        ),
    ];
    for (name, value) in added {
        options.push(format!("-D{name}={value}").into());
    }
    options
}

/// What Intel's driver gives clang-cl under `icx-cl`, as the command line
/// before `/link` decides it.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each is an independent default the command line overrides."
)]
pub(super) struct ClDefaults {
    /// Whether an `/O` option chose the level.
    optimization: bool,
    /// The last debug option (`/Zi`), which turns optimization off without
    /// an `/O` option.
    debug:        Option<String>,
    /// `/fp:fast`, the default, rather than `/fp:precise` or `/fp:strict`.
    fast:         bool,
    /// Whether the last of `/fp:` and `/Qftz[-]` flushes subnormals, which
    /// an optimized `main` then sets the processor to do, as under `icx`
    /// ([`Denormals`]).
    flush:        bool,
    /// `-Rno-debug-disables-optimization` was not given.
    remark:       bool,
}

impl ClDefaults {
    /// Reads the options that decide the defaults from `arguments`.
    pub(super) fn read(arguments: &[OsString]) -> Self {
        let mut defaults = Self {
            optimization: false,
            debug:        None,
            fast:         true,
            flush:        true,
            remark:       true,
        };
        for argument in arguments {
            let Some(text) = argument.to_str() else {
                continue;
            };
            let Some(option) = text.strip_prefix('/').or_else(|| text.strip_prefix('-')) else {
                continue;
            };
            match option {
                | "link" => break,
                | "Zi" | "Z7" | "ZI" => defaults.debug = Some(text.into()),
                | "fp:fast" => (defaults.fast, defaults.flush) = (true, true),
                | "fp:precise" | "fp:strict" | "fp:except" =>
                    (defaults.fast, defaults.flush) = (false, false),
                | "Qftz" => defaults.flush = true,
                | "Qftz-" => defaults.flush = false,
                | "Rno-debug-disables-optimization" => defaults.remark = false,
                | "Rdebug-disables-optimization" => defaults.remark = true,
                | _ =>
                    if option.starts_with('O') {
                        defaults.optimization = true;
                    },
            }
        }
        defaults
    }

    /// Adds Intel's identity and defaults to bcc's options `common`, and
    /// gives the remark about a debug option that turns optimization off.
    pub(super) fn apply(&self, name: &str, common: &mut Vec<OsString>, target: Target) {
        common.extend(identity(target));
        common.extend(["-D_ISO_VOLATILE=1", "-D_UCRT_NOISY_NAN=1"].map(OsString::from));
        if !self.optimization {
            if let Some(option) = &self.debug {
                common.push("-O0".into());
                if self.remark {
                    super::print(
                        &format!("{name}: remark: {}\n", Defaults::remark_text(option, "/Od")),
                        true,
                    );
                }
            } else {
                common.push("-O2".into());
            }
        }
        if self.flush {
            common.push("--flush-denormals-in-main".into());
        }
        // The floating-point model's macros, which `cl` defines as
        // `/fp:` chooses (`/fp:strict` is refused).
        common.push("-D_M_FP_CONTRACT=1".into());
        if self.fast {
            common.extend(
                [
                    "-ffast-math",
                    "-fno-finite-math-only",
                    "-D_M_FP_FAST=1",
                    "-D__NO_MATH_ERRNO__=1",
                ]
                .map(OsString::from),
            );
        } else {
            common.push("-D_M_FP_PRECISE=1".into());
        }
    }
}

/// What `icx-cl` does with Intel's option `/QOPTION`: bcc's options for it,
/// or why bcc refuses it, or `None` for a name Intel does not have, which
/// clang-cl reads as a path.
pub(super) fn cl_option(option: &str) -> Option<Result<&'static [&'static str], &'static str>> {
    const CHANGES: &str = "bcc does not implement it, and ignoring it could change the program";
    if let Some(code) = option.strip_prefix('x') {
        return processor(code).map(Ok);
    }
    let name = option
        .split_once([':', '='])
        .map_or(option, |(name, _)| name);
    let name = name.strip_suffix('-').unwrap_or(name);
    Some(match name {
        | "openmp" | "iopenmp" | "openmp-stubs" => Err(CHANGES),
        | "openmp-simd" => Ok(&["-D__INTEL_HOST_OPENMP=1"]),
        | "mkl" | "tbb" | "ipp" | "daal" => Err("bcc has none of Intel's libraries"),
        | "ipo" | "ftz" | "unroll" | "vec" | "fma" | "traceback" => Ok(&[]),
        | _ if ["ax", "opt-", "diag-", "imf-", "fp-speculation", "inline-"]
            .iter()
            .any(|prefix| name.starts_with(prefix)) =>
            Ok(&[]),
        | _ => return None,
    })
}

/// The command line as Intel's wrapper reads it so far.
#[derive(Default)]
struct Translation {
    /// The options passed on to Clang's driver, and the inputs, in order.
    out:              Vec<OsString>,
    /// The classic options dropped, as spelled (`-wd10441`, `-tcheck x`).
    classic:          Vec<String>,
    /// `-diag-disable=10430`, which keeps the warning about them quiet.
    classic_disabled: bool,
    /// The wrapper's own errors, reported at once.
    wrapper_errors:   Vec<String>,
    /// The errors Clang's driver reports about Intel's options.
    errors:           Vec<String>,
    /// Whether an `-O` option chose the level.
    optimization:     bool,
    /// The last debug option, which turns optimization off without an `-O`
    /// option.
    debug:            Option<String>,
    /// `-R[no-]debug-disables-optimization`.
    remark:           Option<bool>,
    /// The floating-point options' state, which decides the flush in
    /// `main` and the link's `crtfastmath.o`.
    denormals:        Denormals,
}

/// What icx's floating-point options decide about subnormal numbers, each
/// as the last option that sets it. Intel's Clang makes an optimized
/// `main` set MXCSR's FTZ and DAZ bits under its default `fast` model,
/// `-ftz` or `-ffast-math`, and not under `-no-ftz` or a value-safe model.
/// Its driver links `crtfastmath.o`, whose constructor sets them for the
/// whole process, only where Clang's would for the options on the command
/// line, not for the default model, and after any `-Ofast` at all.
#[derive(Default)]
struct Denormals {
    /// `-no-ftz`, `-fno-fast-math`, `-fp-model=precise` or another option
    /// that keeps subnormals came after any that flushes them.
    kept:    bool,
    /// The last of Clang's options that link `crtfastmath.o` and those that
    /// do not asks for it.
    runtime: bool,
    /// Whether the link takes `crtfastmath.o` whatever those say: after any
    /// `-Ofast`, but never after `-mno-daz-ftz`.
    forced:  Option<bool>,
}

impl Denormals {
    /// Reads `option`, one of Clang's options or Intel's `-fp-model=MODEL`
    /// as spelled.
    fn read(&mut self, option: &str) {
        let (kept, runtime) = match option {
            | "-ftz" => (Some(false), None),
            | "-no-ftz" | "-fdenormal-fp-math=ieee" => (Some(true), None),
            | "-ffast-math"
            | "-funsafe-math-optimizations"
            | "-ffp-model=fast"
            | "-ffp-model=aggressive"
            | "-fp-model=fast" => (Some(false), Some(true)),
            | "-fp-model=fast=1" | "-fp-model=fast=2" => (Some(false), Some(false)),
            | "-fno-fast-math"
            | "-fno-unsafe-math-optimizations"
            | "-ffp-model=precise"
            | "-ffp-model=strict"
            | "-fp-model=precise"
            | "-fp-model=strict"
            | "-fp-model=consistent" => (Some(true), Some(false)),
            // `-fast` is `-Ofast` and more.
            | "-Ofast" | "-fast" => {
                if self.forced.is_none() {
                    self.forced = Some(true);
                }
                (Some(false), None)
            },
            | "-mno-daz-ftz" => {
                self.forced = Some(false);
                (None, None)
            },
            | _ => (None, None),
        };
        if let Some(kept) = kept {
            self.kept = kept;
        }
        if let Some(runtime) = runtime {
            self.runtime = runtime;
        }
    }

    /// bcc's options for what was read.
    fn options(&self) -> impl Iterator<Item = OsString> {
        let flush = (!self.kept).then_some("--flush-denormals-in-main");
        let runtime = if self.forced.unwrap_or(self.runtime) {
            "--link-fast-math-runtime"
        } else {
            "--no-fast-math-runtime"
        };
        flush.into_iter().chain([runtime]).map(OsString::from)
    }
}

impl Translation {
    /// Reads one argument and its operand.
    fn argument(&mut self, argument: &OsString, rest: &mut Arguments<'_>) {
        let Some(text) = argument.to_str() else {
            self.out.push(argument.clone());
            return;
        };
        if text == "-" || !text.starts_with('-') {
            self.out.push(argument.clone());
            return;
        }
        if let Some(spelling) = classic_option(text, rest) {
            self.classic.push(spelling);
            return;
        }
        self.denormals.read(text);
        // Clang's options that take the next argument keep it.
        if SEPARATE_OPERANDS.contains(&text) {
            self.out.push(argument.clone());
            if let Some(operand) = rest.next() {
                self.out.push(operand.clone());
            }
            return;
        }
        if let Some(model) = text.strip_prefix("-fp-model=") {
            self.fp_model(model);
            return;
        }
        if text == "-fp-model" {
            let model = rest
                .next()
                .map(|model| model.to_string_lossy().into_owned());
            let model = model.as_deref().unwrap_or_default();
            self.denormals.read(&format!("-fp-model={model}"));
            self.fp_model(model);
            return;
        }
        if let Some(code) = text.strip_prefix("-x")
            && let Some(options) = processor(code)
        {
            self.out.extend(options.iter().map(OsString::from));
            return;
        }
        if let Some(codes) = text.strip_prefix("-ax") {
            self.check_processors(codes.strip_prefix('=').unwrap_or(codes), "ax");
            return;
        }
        if let Some(codes) = text.strip_prefix("-mauto-arch") {
            self.check_processors(codes.strip_prefix('=').unwrap_or(codes), "mauto-arch");
            return;
        }
        if let Some(value) = text.strip_prefix("-debug") {
            self.debug_option(value, rest);
            return;
        }
        if let Some(kind) = text.strip_prefix("-diag-") {
            self.diagnostic_option(kind, rest);
            return;
        }
        match intel_option(text) {
            | Some(Intel::Ignore) => {},
            | Some(Intel::IgnoreWithOperand) => drop(rest.next()),
            | Some(Intel::Pass(options)) => {
                // `-fast` chooses the level.
                self.optimization |= options.iter().any(|option| option.starts_with("-O"));
                self.out.extend(options.iter().map(OsString::from));
            },
            | Some(Intel::Keep) => self.out.push(argument.clone()),
            | Some(Intel::Refuse(why)) => self
                .errors
                .push(format!("'{text}' is not supported: {why}")),
            | Some(Intel::Error(message)) => self.wrapper_errors.push(message.into()),
            | Some(Intel::Remark(enabled)) => self.remark = Some(enabled),
            | None => {
                if text.starts_with("-O") {
                    self.optimization = true;
                } else if text.starts_with("-g") {
                    self.debug = Some(text.into());
                }
                self.out.push(argument.clone());
            },
        }
    }

    /// `-fp-model=MODEL`: Intel's models onto Clang's `-ffp-model=` and the
    /// options it does not cover. Clang's driver refuses another model, as
    /// `-ffp-model=` names it: icx 2026.2.0 no longer has the classic
    /// compiler's `source`, `double`, `extended` and `except`.
    fn fp_model(&mut self, model: &str) {
        let options: &[&str] = match model {
            | "fast" | "fast=1" => &["-ffp-model=fast"],
            | "fast=2" => &["-ffp-model=aggressive"],
            | "precise" => &["-ffp-model=precise"],
            // Value-safe and never contracting.
            | "consistent" => &["-ffp-model=precise", "-ffp-contract=off"],
            | _ => {
                self.out.push(format!("-ffp-model={model}").into());
                return;
            },
        };
        self.out.extend(options.iter().map(OsString::from));
    }

    /// `-axCODE,CODE` and `-mauto-arch=CODE`: further code paths for the
    /// processors named, beside the baseline, which bcc does not write;
    /// Clang's driver checks each name.
    fn check_processors(&mut self, codes: &str, option: &str) {
        for code in codes.split(',') {
            if processor(code).is_none() {
                self.errors
                    .push(format!("unknown target CPU '{code}' for option '{option}'"));
            }
        }
    }

    /// `-debug[=KIND]` or `-debug KIND`: debug information, or none for
    /// `none`, which turns optimization off as `-g` does.
    fn debug_option(&mut self, value: &str, rest: &mut Arguments<'_>) {
        let kind = match value.strip_prefix('=') {
            | Some(kind) => Some(kind.to_owned()),
            // Intel's driver takes the next argument as the kind when it is
            // not an option.
            | None if value.is_empty() => rest
                .next_if(|next| !next.to_string_lossy().starts_with('-'))
                .map(|kind| kind.to_string_lossy().into_owned()),
            | None => {
                self.out.push(format!("-debug{value}").into());
                return;
            },
        };
        self.debug = Some("-debug".into());
        match kind.as_deref() {
            | Some("none") => self.out.push("-g0".into()),
            | Some(kind) if !DEBUG_KINDS.contains(&kind) => self
                .errors
                .push(format!("unsupported argument '{kind}' to option '-debug='")),
            | _ => self.out.push("-g".into()),
        }
    }

    /// `-diag-disable=IDS`, `-diag-enable=`, `-diag-warning=`,
    /// `-diag-error=` and their forms with the operand after a space: the
    /// classic diagnostics they name are Intel's, which bcc has not, except
    /// `10430`, the warning about the classic options.
    fn diagnostic_option(&mut self, kind: &str, rest: &mut Arguments<'_>) {
        let (name, ids) = match kind.split_once(['=', ':']) {
            | Some((name, ids)) => (name, ids.to_owned()),
            | None if matches!(kind, "disable" | "enable" | "warning" | "error" | "remark") => (
                kind,
                rest.next()
                    .map_or_else(String::new, |ids| ids.to_string_lossy().into_owned()),
            ),
            // `-diag-dump` and the like.
            | None => return,
        };
        if name == "disable" && ids.split(',').any(|id| id == "10430") {
            self.classic_disabled = true;
        }
    }

    /// Intel's defaults for what the command line leaves open.
    fn defaults(&self) -> Defaults {
        let mut compile = identity(Target::LinuxGnu);
        // `-O2`, or `-O0` after a debug option without an `-O` option, with a
        // remark unless `-Rno-debug-disables-optimization` asks for none.
        let debug_only = self.debug.is_some() && !self.optimization;
        compile.push(if debug_only { "-O0" } else { "-O2" }.into());
        // The `fast` model: Clang's `-ffp-model=fast`, whose options the
        // command line's own override without Clang's warning about an
        // overridden model.
        compile.push("-ffast-math".into());
        compile.push("-fno-finite-math-only".into());
        // Code at a fixed address, linked so (Intel's Linux toolchain is not
        // position-independent by default).
        compile.push("-fno-pic".into());
        // Subnormals as the floating-point options leave them, not as the
        // default model would link them.
        compile.extend(self.denormals.options());
        Defaults {
            compile,
            link: vec!["--no-pie".into()],
            reported: self.wrapper_errors.len(),
            errors: self.errors.clone(),
            remark: if debug_only && self.remark != Some(false) {
                self.debug.clone()
            } else {
                None
            },
        }
    }
}

/// The command line after the argument being read.
type Arguments<'a> = Peekable<slice::Iter<'a, OsString>>;

/// The values of `-debug` that ask for debug information (Intel's `-debug`
/// reference).
const DEBUG_KINDS: [&str; 12] = [
    "full",
    "all",
    "minimal",
    "extended",
    "emit-column",
    "emit_column",
    "expr-source-pos",
    "inline-debug-info",
    "parallel",
    "pubnames",
    "semantic-stepping",
    "variable-locations",
];

/// Clang's options that take their operand as the next argument, which
/// the wrapper passes on with them.
const SEPARATE_OPERANDS: [&str; 30] = [
    "-o",
    "-I",
    "-D",
    "-U",
    "-include",
    "-isystem",
    "-iquote",
    "-idirafter",
    "-x",
    "-l",
    "-L",
    "-MF",
    "-MT",
    "-MQ",
    "-Xlinker",
    "-target",
    "--sysroot",
    "-isysroot",
    "-Xpreprocessor",
    "-Xassembler",
    "-Xclang",
    "-Xbcc",
    "-T",
    "-u",
    "-z",
    "-e",
    "--entry",
    "--param",
    "-B",
    "-imacros",
];

/// What the wrapper does with one of Intel's own options.
enum Intel {
    /// Drops it: it tunes the code or asks for reports bcc does not write.
    Ignore,
    /// Drops it and the argument after it.
    IgnoreWithOperand,
    /// Passes these options to Clang's driver instead.
    Pass(&'static [&'static str]),
    /// Passes it on to the Clang personality, which knows it under icx.
    Keep,
    /// Refuses it, as an option that would change the program in a way bcc
    /// cannot follow, for this reason.
    Refuse(&'static str),
    /// Refuses it in Intel's words.
    Error(&'static str),
    /// `-R[no-]debug-disables-optimization`.
    Remark(bool),
}

/// What the wrapper and Intel's Clang do with `option`, one of Intel's own
/// (`icx --autocomplete` beside Clang 23's, and the wrapper's `-q`, `-diag-`
/// and processor options), or `None` for Clang's.
fn intel_option(option: &str) -> Option<Intel> {
    const CHANGES: &str = "bcc does not implement it, and ignoring it could change the program";
    let name = option.split_once('=').map_or(option, |(name, _)| name);
    Some(match name {
        // OpenMP, which bcc refuses as it does `-fopenmp`; calling
        // conventions, data layouts and precisions bcc does not follow; and
        // header search from the environment Intel's driver adds.
        | "-qopenmp"
        | "-fiopenmp"
        | "-qopenmp-stubs"
        | "-fiopenmp-offload"
        | "-regcall"
        | "-fstack-limit-register"
        | "-pc32"
        | "-pc64"
        | "-iexternal-env"
        | "-isystem-env"
        | "-ivfsoverlay-lib"
        | "-fheader-search" => Intel::Refuse(CHANGES),
        | "-fintel-long-double-size" if option != "-fintel-long-double-size=80" =>
            Intel::Refuse(CHANGES),
        | "-falign-stack" if option != "-falign-stack=maintain-16-byte" => Intel::Refuse(CHANGES),
        // OpenMP's SIMD subset, which needs no run-time library.
        | "-qopenmp-simd" | "-fiopenmp-simd" =>
            Intel::Pass(&["-fopenmp-simd", "-D__INTEL_HOST_OPENMP=1"]),
        | "-qno-openmp" | "-fno-iopenmp" => Intel::Pass(&["-fno-openmp"]),
        // SYCL, offloading and FPGAs, which bcc does not compile.
        | "-fsycl" | "-fhls" | "--dpcpp" =>
            Intel::Refuse("bcc compiles C, not SYCL or device code"),
        | "-fintelfpga" => Intel::Error(
            "option '-fintelfpga' is not supported and has been removed from the compiler. Please \
             see the compiler documentation for more details",
        ),
        // Intel's libraries, which bcc does not have.
        | "-qmkl" | "-mkl" | "-qmkl-ilp64" | "-qmkl-sycl-impl" | "-qtbb" | "-tbb" | "-qipp"
        | "-ipp" | "-qipp-link" | "-ipp-link" | "-qdaal" | "-daal" | "-qactypes" | "-fortlib" =>
            Intel::Refuse("bcc has none of Intel's libraries"),
        // Header search Intel's driver adds, with its operand.
        | "-iexternal" | "-pch-use" => Intel::IgnoreWithOperand,
        // Onto Clang's options.
        | "-ipo" => Intel::Pass(&["-flto"]),
        | "-no-ipo" => Intel::Pass(&["-fno-lto"]),
        | "-ansi-alias" => Intel::Pass(&["-fstrict-aliasing"]),
        | "-no-ansi-alias" => Intel::Pass(&["-fno-strict-aliasing"]),
        | "-fma" => Intel::Pass(&["-ffp-contract=fast"]),
        | "-no-fma" => Intel::Pass(&["-ffp-contract=off"]),
        | "-strict-ansi" => Intel::Pass(&["-ansi"]),
        | "-fstack-security-check" => Intel::Pass(&["-fstack-protector-strong"]),
        | "-use-msasm" => Intel::Pass(&["-fasm-blocks"]),
        | "-EP" => Intel::Pass(&["-E", "-P"]),
        | "-#" | "-#x" | "-dryrun" => Intel::Pass(&["-###"]),
        | "-fhelp" => Intel::Pass(&["--help"]),
        | "-fast" => Intel::Pass(&["-Ofast", "-march=native", "-flto", "-static"]),
        // Which of Intel's libraries the link takes, which bcc's never does.
        | "-static-intel" | "-shared-intel" | "-no-intel-lib" => Intel::Keep,
        | "-Rdebug-disables-optimization" => Intel::Remark(true),
        | "-Rno-debug-disables-optimization" => Intel::Remark(false),
        // The list of classic options, which bcc does not print.
        | "-qnextgen-diag" => Intel::Ignore,
        // Tuning, reports and debug-information variants bcc does not
        // write, and switches that only matter with OpenMP or SYCL.
        | _ if IGNORED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix)) =>
            Intel::Ignore,
        | _ if IGNORED.contains(&name) => Intel::Ignore,
        | _ if is_intel_warning(option) => Intel::Ignore,
        | _ => return None,
    })
}

/// Intel's option families that only tune the code, report on it, or
/// configure OpenMP and SYCL (whose main switches bcc refuses).
const IGNORED_PREFIXES: [&str; 21] = [
    "-qopt-",
    "-qno-opt-",
    "-fimf-",
    "-fvec-",
    "-fno-vec-",
    "-floop-",
    "-fno-loop-",
    "-fsycl-",
    "-fno-sycl-",
    "-fopenmp-",
    "-fno-openmp-",
    "-fiopenmp-",
    "-fno-iopenmp-",
    "-qopenmp-",
    "-foffload-",
    "-fno-offload-",
    "-Xsycl-",
    "-Xopenmp-target-",
    "-Xspirv-",
    "-Xdevice-post-link",
    "-i_",
];

/// Intel's other options that bcc can drop.
const IGNORED: [&str; 64] = [
    "-unroll",
    "-inline-level",
    "-inline-forceinline",
    "-nolib-inline",
    "-vec",
    "-no-vec",
    "-vec-threshold",
    "-vecabi",
    "-scalar-rep",
    "-no-scalar-rep",
    "-global-hoist",
    "-no-global-hoist",
    "-ftz",
    "-no-ftz",
    "-fno-alias",
    "-fargument-noalias",
    "-fhonor-nan-compares",
    "-fno-honor-nan-compares",
    "-fdisable-fp-invariant-reassociation",
    "-ffp-accuracy",
    "-ffp-builtin-accuracy",
    "-fp-speculation",
    "-faltmathlib",
    "-fuse-complex-intrinsics",
    "-fno-use-complex-intrinsics",
    "-fintel-pragma-prefetch",
    "-fno-intel-pragma-prefetch",
    "-fuse-unaligned-vector-move",
    "-fno-use-unaligned-vector-move",
    "-mintrinsic-promote",
    "-fcodegen-with-llc",
    "-flegacy-pass-manager",
    "-fno-legacy-pass-manager",
    "-fno-experimental-new-pass-manager",
    "-mllvm-lto",
    "-fmaintain-32-byte-stack-align",
    "-fno-maintain-32-byte-stack-align",
    "-mstackrealignext",
    "-fintel-long-double-size",
    "-falign-stack",
    "-pc80",
    "-gintel-sti",
    "-gintel-opencl-builtin-types",
    "-gno-intel-opencl-builtin-types",
    "-fsystem-debug",
    "-fno-system-debug",
    "-fmerge-debug-strings",
    "-fno-merge-debug-strings",
    "-femit-class-debug-always",
    "-fno-emit-class-debug-always",
    "-fms-debug-info-file-type",
    "-fms-debug-info-obj-file",
    "-fms-debug-info-pdb-file",
    "-mdebug-line-version",
    "-fprofile-dwo-dir",
    "-parallel-source-info",
    "-no-parallel-source-info",
    "-traceback",
    "-sox",
    "-no-sox",
    "-qoverride-limits",
    "-qno-actypes",
    "-qunknown-option-as-warning",
    "-qno-unknown-option-as-warning",
];

/// Whether `option` is one of the warning options Intel's Clang adds to
/// Clang's (`-Wrecommended-option`, `-Wno-debug-disables-optimization`),
/// which bcc has none of.
fn is_intel_warning(option: &str) -> bool {
    let Some(name) = option
        .strip_prefix("-Werror=")
        .or_else(|| option.strip_prefix("-Wno-error="))
        .or_else(|| option.strip_prefix("-Wno-"))
        .or_else(|| option.strip_prefix("-W"))
    else {
        return false;
    };
    include_str!("icx/warnings.list")
        .lines()
        .any(|line| line == name)
}

/// Whether icx's wrapper reports `text`, a whole argument, as one of
/// Intel's classic options (`command line warning #10430`) and drops it.
pub(super) fn is_classic_option(text: &str) -> bool {
    classic_option(text, &mut [].iter().peekable()).is_some()
}

/// The classic option `text` names, as the wrapper lists it, reading the
/// operand of one that takes the next argument: icx 2026.2.0's
/// `-qnextgen-diag` table.
fn classic_option(text: &str, rest: &mut Arguments<'_>) -> Option<String> {
    for entry in include_str!("icx/classic.list").lines() {
        if entry.starts_with('#') {
            continue;
        }
        if let Some(name) = entry.strip_suffix(" <string>") {
            if text == name {
                return Some(match rest.next() {
                    | Some(operand) => format!("{text} {}", operand.to_string_lossy()),
                    | None => text.into(),
                });
            }
            continue;
        }
        let (name, value) = if let Some(name) = entry.strip_suffix("=<val>") {
            (name, Some(true))
        } else if let Some(name) = entry.strip_suffix("=<string>") {
            (name, Some(false))
        } else {
            (entry, None)
        };
        let matched = match (value, text.strip_prefix(name)) {
            | (None, Some(rest)) => rest.is_empty(),
            | (Some(numeric), Some(rest)) => {
                let rest = rest.strip_prefix('=').unwrap_or(rest);
                rest.bytes()
                    .next()
                    .is_some_and(|first| !numeric || first.is_ascii_digit())
            },
            | (_, None) => false,
        };
        if matched {
            return Some(text.into());
        }
    }
    None
}

/// The options bcc reads for the processor an Intel `-x` code names, in
/// any case, which icx maps onto Clang's processors: the newest x86-64
/// level the processor has, with the SSE options below the second level
/// and the AVX, F16C and carry-less multiplication options bcc has above
/// it. `Host` is the machine's. icx refuses the codes of processors it
/// dropped (`KNL`, `MIC-AVX512`) and `SSE2`, as languages it does not know.
fn processor(code: &str) -> Option<&'static [&'static str]> {
    const V2: &[&str] = &["-march=x86-64-v2"];
    const V3: &[&str] = &["-march=x86-64-v3", "-mpclmul"];
    let code = code.to_ascii_uppercase();
    Some(match code.as_str() {
        | "HOST" => &["-march=native"],
        | "SSE3" => &["-msse3"],
        | "SSSE3" | "ATOM_SSSE3" => &["-mssse3"],
        | "SSE4.1" => &["-msse4.1"],
        | "SSE4.2" | "ATOM_SSE4.2" | "SILVERMONT" | "GOLDMONT" | "GOLDMONT-PLUS" | "TREMONT" => V2,
        | "AVX" | "SANDYBRIDGE" => &["-march=x86-64-v2", "-mavx", "-mpclmul"],
        | "CORE-AVX-I" | "IVYBRIDGE" => &["-march=x86-64-v2", "-mavx", "-mf16c", "-mpclmul"],
        | "CORE-AVX2" | "HASWELL" | "BROADWELL" | "SKYLAKE" | "KABYLAKE" | "COFFEELAKE"
        | "AMBERLAKE" | "WHISKEYLAKE" | "ALDERLAKE" | "RAPTORLAKE" | "METEORLAKE"
        | "ARROWLAKE" | "ARROWLAKE-S" | "LUNARLAKE" | "PANTHERLAKE" | "WILDCATLAKE"
        | "NOVALAKE" | "SIERRAFOREST" | "GRANDRIDGE" | "CLEARWATERFOREST" | "CORE-AVX512"
        | "COMMON-AVX512" | "SKYLAKE-AVX512" | "CASCADELAKE" | "COOPERLAKE" | "CANNONLAKE"
        | "ICELAKE" | "ICELAKE-CLIENT" | "ICELAKE-SERVER" | "TIGERLAKE" | "ROCKETLAKE"
        | "SAPPHIRERAPIDS" | "EMERALDRAPIDS" | "GRANITERAPIDS" | "GRANITERAPIDS-D"
        | "DIAMONDRAPIDS" | "AVX10.1" | "AVX10.2" => V3,
        | _ => return None,
    })
}
