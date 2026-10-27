//! The GCC and Clang personalities. bcc's own command line already takes
//! most GCC options in GCC's spelling, so these pass those through, answer
//! the driver queries, drop the options the compiler knows that cannot
//! change the program, refuse those that would (`unimplemented.rs`),
//! diagnose unknown ones as the compiler does, and add the
//! compiler's defaults: its identity and diagnostic format, the host's
//! target and system headers, and `-o a.out` when the command links.

use std::{
    ffi::{
        OsStr,
        OsString,
    },
    fmt::Write as _,
};

use super::{
    Invocation,
    MainError,
    Personality,
    Plan,
    Step,
    host_target,
    known::{
        Compiler,
        knows,
    },
    target_named,
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

/// Translates a GCC or Clang command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let personality = invocation.personality;
    let compiler = if personality == Personality::Gcc {
        Compiler::Gcc
    } else {
        Compiler::Clang
    };
    let mut state = State {
        invocation,
        compiler,
        out: Vec::new(),
        errors: 0,
        target: invocation.target,
        sysroot: None,
        stage: None,
        output: false,
        inputs: 0,
        verbose: false,
        dry_run: false,
        machine: Vec::new(),
    };
    let mut arguments = invocation.arguments.iter().map(OsString::as_os_str);
    while let Some(argument) = arguments.next() {
        if let Some(query) = state.argument(argument, &mut arguments) {
            return Ok(query);
        }
    }
    let target = state.target.unwrap_or_else(|| host_target(personality));
    state.machine_options(target);
    if state.errors != 0 {
        return Err(MainError::Reported(1));
    }
    let mut bcc = vec![invocation.program.clone()];
    bcc.push(
        match personality {
            | Personality::Gcc => "--compiler-identity=gcc",
            | _ => "--compiler-identity=clang",
        }
        .into(),
    );
    bcc.push(
        match personality {
            | Personality::Gcc => "--diagnostic-format=gcc",
            | _ => "--diagnostic-format=clang",
        }
        .into(),
    );
    bcc.push(format!("--target={}", target.triple()).into());
    if invocation.name.ends_with("c89") {
        bcc.push("-std=c89".into());
    } else if invocation.name.ends_with("c99") {
        bcc.push("-std=c99".into());
    }
    // bcc searches the host's C library when it compiles for the host, as
    // these compilers do, and the selected root's otherwise, also for the
    // link; `-nostdinc` keeps the root for the link alone.
    if let Some(sysroot) = &state.sysroot {
        let mut option = OsString::from("--sysroot=");
        option.push(sysroot);
        bcc.push(option);
    }
    let links = state.stage.is_none();
    if !links {
        // GCC ignores what links when it stops before linking.
        state
            .out
            .retain(|option| !matches!(option.to_str(), Some("-shared" | "-pie" | "-no-pie")));
    }
    bcc.append(&mut state.out);
    if links && !state.output && state.inputs != 0 {
        bcc.push("-o".into());
        bcc.push(
            if matches!(target, Target::WindowsGnu | Target::WindowsMsvc) {
                "a.exe"
            } else {
                "a.out"
            }
            .into(),
        );
    }
    let mut plan = Plan::default();
    if state.verbose || state.dry_run {
        let mut text = version::verbose(personality, target);
        if state.dry_run {
            text.push_str(" \"bcc\"");
            for argument in &bcc[1..] {
                _ = write!(text, " \"{}\"", argument.to_string_lossy());
            }
            text.push('\n');
        }
        plan.steps.push(Step::Print { text, stderr: true });
        if state.dry_run || state.inputs == 0 {
            return Ok(plan);
        }
    }
    if state.inputs == 0 {
        invocation.error(format_args!(
            "{}no input files",
            if personality == Personality::Gcc {
                "fatal error: "
            } else {
                ""
            }
        ));
        return Err(MainError::Reported(1));
    }
    plan.steps.push(Step::Bcc(bcc));
    Ok(plan)
}

/// The translation so far.
struct State<'a> {
    invocation: &'a Invocation,
    compiler:   Compiler,
    /// bcc's options and inputs, in order.
    out:        Vec<OsString>,
    errors:     usize,
    target:     Option<Target>,
    sysroot:    Option<OsString>,
    /// The option that stops before linking: `-c`, `-S`, `-E`, `-M`, `-MM`
    /// or `-fsyntax-only`.
    stage:      Option<&'static str>,
    output:     bool,
    inputs:     usize,
    verbose:    bool,
    dry_run:    bool,
    /// The known `-m` options bcc does not implement, judged once the
    /// target is known, since `--target` may come after them.
    machine:    Vec<String>,
}

/// Options bcc reads in GCC's spelling, which take their operand as the next
/// argument when it is not joined.
const OPERAND_OPTIONS: [&str; 15] = [
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
];

/// Options bcc reads in GCC's spelling and that take no operand.
const PLAIN_OPTIONS: [&str; 27] = [
    "-w",
    "-pedantic",
    "-pedantic-errors",
    "-Wpedantic",
    "-shared",
    "-pie",
    "-no-pie",
    "-static",
    "-rdynamic",
    "-pthread",
    "-nostdlib",
    "-nostartfiles",
    "-nodefaultlibs",
    "-static-libgcc",
    "-shared-libgcc",
    "-dM",
    "-P",
    "-MD",
    "-MMD",
    "-MP",
    "-nostdlibinc",
    "-nobuiltininc",
    "-ffreestanding",
    "-fhosted",
    "-fgnu89-inline",
    "-fno-gnu89-inline",
    "-fms-extensions",
];

/// GCC options that need no translation and cannot change the program bcc
/// builds: driver conveniences and reports.
/// `-g*` (debug information, which bcc does not emit) and the `-f` and `-m`
/// options in `unimplemented.rs` are ignored too.
const IGNORED_OPTIONS: [&str; 5] = ["-pipe", "-save-temps", "-Q", "-H", "-time"];

impl State<'_> {
    /// Translates one argument, reading its operand from `rest`. Returns
    /// the plan of a query that ends the command line, such as
    /// `--version`.
    fn argument<'r>(
        &mut self,
        argument: &'r OsStr,
        rest: &mut impl Iterator<Item = &'r OsStr>,
    ) -> Option<Plan> {
        let Some(text) = argument.to_str() else {
            self.input(argument);
            return None;
        };
        if text == "-" || !text.starts_with('-') {
            self.input(argument);
            return None;
        }
        let personality = self.invocation.personality;
        let target = || self.target.unwrap_or_else(|| host_target(personality));
        match text {
            | "--version" =>
                return Some(Plan::print(version::banner(
                    personality,
                    &self.invocation.name,
                    target(),
                ))),
            | "-dumpversion" =>
                return Some(Plan::print(format!(
                    "{}\n",
                    version::dump_version(personality)
                ))),
            | "-dumpfullversion" =>
                return Some(Plan::print(format!("{}\n", version::GCC_VERSION))),
            | "-dumpmachine" | "-print-target-triple" | "-print-effective-triple" => {
                let machine = if personality == Personality::Gcc {
                    version::gcc_machine(target())
                } else {
                    target().triple()
                };
                return Some(Plan::print(format!("{machine}\n")));
            },
            | "-print-multiarch" =>
                return Some(Plan::print(format!("{}\n", version::gcc_machine(target())))),
            | "-print-search-dirs" =>
                return Some(Plan::print(format!(
                    "install: {}\nprograms: ={}\nlibraries: =/usr/local/lib:/usr/lib/{}:/usr/lib\n",
                    version::program_name("bcc"),
                    version::program_name("ld"),
                    version::gcc_machine(target())
                ))),
            | "-print-libgcc-file-name" => return Some(Plan::print("libgcc.a\n".into())),
            | "-print-sysroot" =>
                return Some(Plan::print(format!(
                    "{}\n",
                    self.sysroot
                        .as_deref()
                        .map_or_else(|| "".into(), OsStr::to_string_lossy)
                ))),
            | "--help" | "-help" =>
                return Some(Plan::print(help(personality, &self.invocation.name))),
            | "-v" => self.verbose = true,
            | "-###" => self.dry_run = true,
            | "-c" | "-S" | "-E" | "-M" | "-MM" => {
                self.stage = Some(match text {
                    | "-c" => "-c",
                    | "-S" => "-S",
                    | "-E" => "-E",
                    | "-M" => "-M",
                    | _ => "-MM",
                });
                self.out.push(text.into());
            },
            | "-fsyntax-only" => {
                self.stage = Some("-fsyntax-only");
                self.out.push("--syntax-only".into());
            },
            | "-ansi" => self.out.push("-std=c89".into()),
            | "-nostdinc" => self.out.push("--nostdinc".into()),
            | "-s" => self.out.push("-Wl,-s".into()),
            | "-Ofast" => self.unsupported(
                text,
                "it implies -ffast-math; bcc keeps IEEE semantics and does not define \
                 __FAST_MATH__",
            ),
            | "-C" | "-CC" => self.invocation.warning(format_args!(
                "'{text}' is not supported: bcc does not keep comments in preprocessed output"
            )),
            | "-pg" | "-p" => self.invocation.warning(format_args!(
                "'{text}' is not supported: bcc does not instrument programs for profiling"
            )),
            | "-m32" | "-mx32" | "-m16" => self.unsupported(text, "bcc compiles for x86-64 only"),
            | "-traditional" | "-traditional-cpp" =>
                self.unsupported(text, "bcc has no traditional preprocessor"),
            | _ =>
                if let Some((option, operand)) = self.operand(text, rest) {
                    self.with_operand(option, operand);
                } else {
                    self.flag(text);
                },
        }
        None
    }

    /// An input file, or `-` for stdin.
    fn input(&mut self, argument: &OsStr) {
        self.inputs += 1;
        self.out.push(argument.into());
    }

    /// The option and its operand, joined or the next argument, of an
    /// option that takes one.
    fn operand<'r>(
        &mut self,
        text: &'r str,
        rest: &mut impl Iterator<Item = &'r OsStr>,
    ) -> Option<(&'static str, &'r OsStr)> {
        const OTHER: [&str; 16] = [
            "-target",
            "--target",
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
        let option = OPERAND_OPTIONS
            .into_iter()
            .chain(OTHER)
            .filter(|option| text.starts_with(option))
            .max_by_key(|option| option.len())?;
        let joined = &text[option.len()..];
        // Only options whose operand may be joined take a longer spelling.
        let joined = match joined.strip_prefix('=') {
            | Some(value) if option.starts_with("--") => value,
            | _ if joined.is_empty() => "",
            | _ if option.len() > 2
                && !matches!(option, "-include" | "-isystem" | "-iquote" | "-idirafter")
                && !option.starts_with("-M") =>
                return None,
            | _ => joined,
        };
        if joined.is_empty() {
            match rest.next() {
                | Some(operand) => Some((option, operand)),
                | None => {
                    self.error(format_args!("missing argument to '{option}'"));
                    None
                },
            }
        } else {
            Some((option, OsStr::new(joined)))
        }
    }

    /// Translates an option with its operand.
    fn with_operand(&mut self, option: &'static str, operand: &OsStr) {
        let lossy = operand.to_string_lossy();
        match option {
            | "-target" | "--target" => match target_named(&lossy) {
                | Some(target) => self.target = Some(target),
                | None => self.error(format_args!(
                    "unsupported target '{lossy}'; bcc supports x86_64-unknown-linux-gnu, \
                     x86_64-unknown-linux-musl, x86_64-w64-windows-gnu and x86_64-pc-windows-msvc"
                )),
            },
            | "--sysroot" | "-isysroot" => self.sysroot = Some(operand.into()),
            | "-Xpreprocessor" => {
                let mut none = std::iter::empty();
                drop(self.argument(operand, &mut none));
            },
            | "-Xbcc" => self.out.push(operand.into()),
            | "-Xassembler" => {
                self.out.push("-Xassembler".into());
                self.out.push(operand.into());
            },
            // Tuning parameters of GCC's optimizers.
            | "--param" => {},
            | "-B" => self.invocation.warning(format_args!(
                "'-B {lossy}' is ignored: bcc uses its bundled assembler and linker"
            )),
            | "-Xclang" => self.unsupported(
                &format!("-Xclang {lossy}"),
                "bcc has no Clang frontend options",
            ),
            | "-imacros" => self.unsupported(option, "bcc does not implement -imacros"),
            | "-T" | "-u" | "-z" | "-e" | "--entry" => self.out.push(
                format!(
                    "-Wl,{},{lossy}",
                    match option {
                        | "--entry" => "-e",
                        | other => other,
                    }
                )
                .into(),
            ),
            | _ => {
                if option == "-o" {
                    self.output = true;
                }
                self.out.push(option.into());
                self.out.push(operand.into());
            },
        }
    }

    /// Translates an option without an operand.
    fn flag(&mut self, text: &str) {
        if PLAIN_OPTIONS.contains(&text)
            || text.starts_with("-std=")
            || text.starts_with("-O")
            || text.starts_with("-Wl,")
            || text.starts_with("-fdiagnostics-format=")
            || implements_language_option(text)
        {
            self.out.push(text.into());
            return;
        }
        if IGNORED_OPTIONS.contains(&text)
            || text.starts_with("-g")
            || text.starts_with("-save-temps")
        {
            return;
        }
        if text.starts_with("-Wa,") {
            // bcc passes assembler options to its assembly inputs.
            self.out.push(text.into());
            return;
        }
        if text.starts_with("-specs=") {
            self.invocation.warning(format_args!(
                "'{text}' is ignored: bcc does not read GCC spec files"
            ));
            return;
        }
        if let Some(options) = text.strip_prefix("-Wp,") {
            // Preprocessor options, comma-separated; `-MD` and `-MMD` take
            // their file here, as GCC passes them to its preprocessor.
            let mut pieces = options.split(',').map(OsStr::new);
            while let Some(piece) = pieces.next() {
                if let Some(plan) = self.argument(piece, &mut pieces) {
                    drop(plan);
                }
            }
            return;
        }
        // GCC's loop distribution into library calls, which LLVM's loop
        // idioms are; bcc stops them by not treating those functions as
        // builtins.
        if text == "-fno-tree-loop-distribute-patterns" {
            for name in ["memset", "memcpy", "memmove"] {
                self.out.push(format!("-fno-builtin-{name}").into());
            }
            return;
        }
        // MinGW-w64's driver options, which the lists of a Linux GCC lack.
        if matches!(
            text,
            "-mwindows" | "-mconsole" | "-mthreads" | "-municode" | "-mdll"
        ) {
            self.machine.push(text.into());
            return;
        }
        let known = knows(self.compiler, text);
        if text.starts_with("-W") {
            // GCC ignores an unknown `-Wno-` option; Clang warns about an
            // unknown warning option, which bcc's own parser does.
            if known || (self.compiler == Compiler::Gcc && text.starts_with("-Wno-")) {
                return;
            }
            if self.compiler == Compiler::Clang {
                self.out.push(text.into());
                return;
            }
        } else if known && text.starts_with("-m") {
            self.machine.push(text.into());
            return;
        } else if known && text.starts_with("-f") {
            let target = self
                .target
                .unwrap_or_else(|| host_target(self.invocation.personality));
            self.treat(text, target);
            return;
        }
        match self.compiler {
            | Compiler::Gcc =>
                self.error(format_args!("unrecognized command-line option '{text}'")),
            | Compiler::Clang => self.error(format_args!("unknown argument: '{text}'")),
        }
    }

    /// Applies [`treatment`] to the deferred `-m` options for `target`.
    /// `-mwindows` on MinGW-w64 links a GUI program, as GCC's driver does.
    fn machine_options(&mut self, target: Target) {
        for option in std::mem::take(&mut self.machine) {
            if option == "-mwindows" && target == Target::WindowsGnu {
                self.out.push("-Wl,--subsystem,windows".into());
            } else {
                self.treat(&option, target);
            }
        }
    }

    /// Ignores, warns about or refuses a known option bcc does not
    /// implement, as [`treatment`] decides.
    fn treat(&mut self, option: &str, target: Target) {
        match treatment(option, target) {
            | Treatment::Ignore => {},
            | Treatment::Warn(why) => self
                .invocation
                .warning(format_args!("'{option}' is ignored: {why}")),
            | Treatment::Reject(why) => self.unsupported(option, why),
        }
    }

    /// An option the compiler knows that would change the program in a way
    /// bcc cannot follow.
    fn unsupported(&mut self, option: &str, why: &str) {
        self.error(format_args!("'{option}' is not supported: {why}"));
    }

    fn error(&mut self, message: std::fmt::Arguments<'_>) {
        self.errors += 1;
        self.invocation.error(message);
    }
}

/// `--help`.
fn help(personality: Personality, name: &str) -> String {
    let compiler = if personality == Personality::Gcc {
        "GCC"
    } else {
        "Clang"
    };
    format!(
        "Usage: {name} [options] file...\nbcc's {compiler}-compatible driver: it reads \
         {compiler}'s command line and runs bcc.\n  -c, -S, -E, -o FILE, -x LANGUAGE   stages and \
         output\n  -I, -isystem, -iquote, -idirafter DIR, -D, -U, -include   preprocessing\n  \
         -std=STD, -ansi, -pedantic, -O*, -W*, -w, -f*   language, optimization and warnings\n  \
         -l, -L, -Wl, -Xlinker, -shared, -static, -pie   linking\n  -M, -MM, -MD, -MMD, -MF, -MT, \
         -MQ, -MP   dependencies\n  -Xbcc OPTION   pass OPTION to bcc itself\nOptions {compiler} \
         knows that bcc does not implement are ignored when they cannot change the program and \
         refused otherwise; see docs/design/driver-modes.md.\n"
    )
}
