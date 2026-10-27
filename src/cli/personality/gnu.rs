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
    path::Path,
};

use super::{
    Invocation,
    MainError,
    Personality,
    Plan,
    Step,
    TemporaryDirectory,
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
    cli::{
        arguments::implements_language_option,
        driver::{
            InputKind,
            kind,
            kind_of_language,
        },
    },
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
        stages: Vec::new(),
        syntax_only: false,
        outputs: Vec::new(),
        inputs: Vec::new(),
        language: None,
        warning_options: Vec::new(),
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
    state.check_inputs();
    let jobs = state.jobs();
    if state.errors != 0 {
        return Err(MainError::Reported(1));
    }
    let mut common = vec![invocation.program.clone()];
    common.push(
        match personality {
            | Personality::Gcc => "--compiler-identity=gcc",
            | _ => "--compiler-identity=clang",
        }
        .into(),
    );
    common.push(
        match personality {
            | Personality::Gcc => "--diagnostic-format=gcc",
            | _ => "--diagnostic-format=clang",
        }
        .into(),
    );
    common.push(format!("--target={}", target.triple()).into());
    if invocation.name.ends_with("c89") {
        common.push("-std=c89".into());
    } else if invocation.name.ends_with("c99") {
        common.push("-std=c99".into());
    }
    // bcc searches the host's C library when it compiles for the host, as
    // these compilers do, and the selected root's otherwise, also for the
    // link; `-nostdinc` keeps the root for the link alone.
    if let Some(sysroot) = &state.sysroot {
        let mut option = OsString::from("--sysroot=");
        option.push(sysroot);
        common.push(option);
    }
    let links = state.stages.is_empty() && !state.syntax_only;
    let mut steps = Vec::new();
    let mut temporary = None;
    for job in jobs {
        if job.outputs == Outputs::Discarded {
            let directory = match &temporary {
                | Some(TemporaryDirectory(directory)) => directory.clone(),
                | None => {
                    let created = TemporaryDirectory::create().map_err(|source| {
                        MainError::WriteOutputFileError {
                            path: std::env::temp_dir(),
                            source,
                        }
                    })?;
                    let path = created.0.clone();
                    temporary = Some(created);
                    path
                },
            };
            for (index, input) in state.inputs.iter().enumerate() {
                if input.dropped || !job.inputs.take(input) {
                    continue;
                }
                let mut bcc = common.clone();
                bcc.extend(state.arguments(Inputs::One(input.at), links));
                bcc.extend(job.stages.iter().map(OsString::from));
                let mut option = OsString::from("--output=");
                option.push(directory.join(format!("{index}.o")));
                bcc.push(option);
                steps.push(bcc);
            }
            continue;
        }
        let mut bcc = common.clone();
        bcc.extend(state.arguments(job.inputs, links));
        bcc.extend(job.stages.iter().map(OsString::from));
        if job.outputs == Outputs::Named {
            // `--output=` keeps a name such as `-.o` from reading as an
            // option.
            for output in &state.outputs {
                let mut option = OsString::from("--output=");
                option.push(output);
                bcc.push(option);
            }
        }
        if links && state.outputs.is_empty() && !state.inputs.is_empty() {
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
        steps.push(bcc);
    }
    // Under `-fsyntax-only` the C and assembly inputs form separate jobs,
    // and each runs whether the other fails, as in the compilers.
    let mut plan = Plan {
        keep_going: true,
        temporary,
        ..Plan::default()
    };
    if state.verbose || state.dry_run {
        let mut text = version::verbose(personality, target);
        if state.dry_run {
            for bcc in &steps {
                text.push_str(" \"bcc\"");
                for argument in &bcc[1..] {
                    _ = write!(text, " \"{}\"", argument.to_string_lossy());
                }
                text.push('\n');
            }
        }
        plan.steps.push(Step::Print { text, stderr: true });
        if state.dry_run || state.inputs.is_empty() {
            return Ok(plan);
        }
    }
    if state.inputs.is_empty() {
        if personality == Personality::Gcc {
            invocation.fatal(format_args!("no input files"));
        } else {
            invocation.error(format_args!("no input files"));
        }
        return Err(MainError::Reported(1));
    }
    plan.steps.extend(steps.into_iter().map(Step::Bcc));
    Ok(plan)
}

/// One bcc command line of a plan: the inputs it takes, and the stage
/// options and `-o` it adds to the other options.
struct Job {
    inputs:  Inputs,
    stages:  Vec<&'static str>,
    outputs: Outputs,
}

/// Where a [`Job`] writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outputs {
    /// Where the stage writes, without `-o`.
    Default,
    /// To the `-o` operands.
    Named,
    /// Each input to its own object in a temporary directory, which is
    /// removed: GCC's `-fsyntax-only` still assembles assembly inputs when
    /// it does not stop before, and keeps nothing.
    Discarded,
}

/// Which of the command line's inputs a [`Job`] takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inputs {
    /// Every input.
    All,
    /// The C sources and the files for the linker.
    CAndLink,
    /// The assembly inputs.
    Assembly,
    /// The one input at this index in [`State::out`].
    One(usize),
}

impl Inputs {
    fn take(self, input: &Input) -> bool {
        match self {
            | Self::All => true,
            | Self::CAndLink => matches!(input.kind, InputKind::C | InputKind::Link),
            | Self::Assembly => matches!(input.kind, InputKind::Assembly { .. }),
            | Self::One(at) => input.at == at,
        }
    }
}

/// An input file on the command line.
struct Input {
    path:    OsString,
    kind:    InputKind,
    /// Its index in [`State::out`].
    at:      usize,
    /// Whether no job takes it: Clang refused it as missing, or left it
    /// unused.
    dropped: bool,
}

/// The translation so far.
struct State<'a> {
    invocation:      &'a Invocation,
    compiler:        Compiler,
    /// bcc's options and inputs, in order.
    out:             Vec<OsString>,
    errors:          usize,
    target:          Option<Target>,
    sysroot:         Option<OsString>,
    /// The options that stop before linking, in order: `-c`, `-S`, `-E`,
    /// `-M` and `-MM`.
    stages:          Vec<&'static str>,
    /// `-fsyntax-only`, which the preprocessing stages override and which
    /// overrides the others.
    syntax_only:     bool,
    /// The `-o` operands.
    outputs:         Vec<OsString>,
    /// The input files.
    inputs:          Vec<Input>,
    /// The kind of input the `-x` language in effect makes a file, or
    /// `None` to decide by its extension.
    language:        Option<InputKind>,
    /// The `-w` and `-W` options, in order, which decide how Clang reports
    /// an argument it leaves unused.
    warning_options: Vec<String>,
    verbose:         bool,
    dry_run:         bool,
    /// The known `-m` options bcc does not implement, judged once the
    /// target is known, since `--target` may come after them.
    machine:         Vec<String>,
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
            | "-c" | "-S" | "-E" | "-M" | "-MM" => self.stages.push(match text {
                | "-c" => "-c",
                | "-S" => "-S",
                | "-E" => "-E",
                | "-M" => "-M",
                | _ => "-MM",
            }),
            | "-fsyntax-only" => self.syntax_only = true,
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
        self.inputs.push(Input {
            path:    argument.into(),
            kind:    self.language.unwrap_or_else(|| kind(Path::new(argument))),
            at:      self.out.len(),
            dropped: false,
        });
        self.out.push(argument.into());
    }

    /// bcc's options and the inputs `inputs` takes, in order, without the
    /// link options when the command line does not link.
    fn arguments(&self, inputs: Inputs, links: bool) -> Vec<OsString> {
        self.out
            .iter()
            .enumerate()
            .filter(|(at, argument)| {
                let input = self.inputs.iter().find(|input| input.at == *at);
                match input {
                    | Some(input) => !input.dropped && inputs.take(input),
                    // GCC ignores what links when it stops before linking.
                    | None =>
                        links || !matches!(argument.to_str(), Some("-shared" | "-pie" | "-no-pie")),
                }
            })
            .map(|(_, argument)| argument.clone())
            .collect()
    }

    /// Diagnoses the inputs as the driver does before it runs anything:
    /// Clang's refuses a missing file and runs no job, and both refuse `-o`
    /// for several outputs. GCC leaves a missing file to its compiler,
    /// which reports it and goes on with the others, as bcc's driver does.
    fn check_inputs(&mut self) {
        let clang = self.compiler == Compiler::Clang;
        if clang {
            for input in &mut self.inputs {
                if input.path != "-" && std::fs::metadata(&input.path).is_err() {
                    input.dropped = true;
                    self.errors += 1;
                    self.invocation.error(format_args!(
                        "no such file or directory: '{}'",
                        input.path.to_string_lossy()
                    ));
                }
            }
            if self.inputs.iter().all(|input| input.dropped) && !self.inputs.is_empty() {
                self.error(format_args!("no input files"));
            }
        }
        // Clang counts the outputs of the inputs it kept.
        let compiled = self
            .inputs
            .iter()
            .filter(|input| input.kind != InputKind::Link && !input.dropped)
            .count();
        if self.outputs.is_empty() || compiled < 2 {
            return;
        }
        // GCC refuses `-o` with `-c`, `-S` or `-E` given, even when
        // `-fsyntax-only` leaves nothing to write, but lets each unit's `-M`
        // rules replace the one before; Clang refuses any stage that writes
        // a file per input.
        if clang {
            let preprocesses = self.preprocesses();
            if preprocesses || (!self.syntax_only && !self.stages.is_empty()) {
                self.error(format_args!(
                    "cannot specify -o when generating multiple output files"
                ));
            }
        } else if self
            .stages
            .iter()
            .any(|stage| matches!(*stage, "-c" | "-S" | "-E"))
        {
            self.errors += 1;
            self.invocation.fatal(format_args!(
                "cannot specify '-o' with '-c', '-S' or '-E' with multiple files"
            ));
        }
    }

    /// Whether `-E`, `-M` or `-MM` stops the command line after
    /// preprocessing.
    fn preprocesses(&self) -> bool {
        self.stages
            .iter()
            .any(|stage| matches!(*stage, "-E" | "-M" | "-MM"))
    }

    /// The bcc command lines the stages and inputs need. `-fsyntax-only`
    /// checks the C sources and writes nothing for them, overriding `-c`,
    /// `-S` and `-o` there, while `-E`, `-M` and `-MM` override it. It
    /// leaves assembly to the assembler: GCC still assembles each `-c`
    /// input unless `-S` is given, keeping the objects only for `-c`, and
    /// Clang leaves `.s` files unused (it preprocesses `.S`
    /// files, which bcc's `-E` does not). Clang warns about what it leaves
    /// unused, and so about the linker's inputs whenever it does not link.
    fn jobs(&mut self) -> Vec<Job> {
        let clang = self.compiler == Compiler::Clang;
        let preprocesses = self.preprocesses();
        if self.syntax_only && preprocesses {
            self.unused(format_args!(
                "argument unused during compilation: '-fsyntax-only'"
            ));
            self.syntax_only = false;
        }
        if clang && (self.syntax_only || !self.stages.is_empty()) {
            let syntax_only = self.syntax_only;
            self.warn_unused_inputs(|kind| match kind {
                | InputKind::Link => Some("linker"),
                | InputKind::Assembly {
                    preprocessed: false,
                } if syntax_only => Some("assembler"),
                | _ => None,
            });
        }
        if !self.syntax_only {
            // Clang runs nothing when it left every input unused.
            if !self.inputs.is_empty() && !self.has(Inputs::All) {
                return Vec::new();
            }
            return vec![Job {
                inputs:  Inputs::All,
                stages:  self.stages.clone(),
                outputs: Outputs::Named,
            }];
        }
        let mut jobs = Vec::new();
        if self.has(Inputs::CAndLink) {
            jobs.push(Job {
                inputs:  Inputs::CAndLink,
                stages:  vec!["--syntax-only"],
                outputs: Outputs::Default,
            });
        }
        if clang {
            for stage in self.stages.clone() {
                self.unused(format_args!(
                    "argument unused during compilation: '{stage}'"
                ));
            }
        } else if !self.stages.contains(&"-S") && self.has(Inputs::Assembly) {
            // GCC assembles them unless `-S` stops before the assembler,
            // keeping the objects only for `-c`.
            jobs.push(Job {
                inputs:  Inputs::Assembly,
                stages:  vec!["-c"],
                outputs: if self.stages.contains(&"-c") {
                    Outputs::Named
                } else {
                    Outputs::Discarded
                },
            });
        }
        jobs
    }

    /// Whether any input that is not dropped is one `inputs` takes.
    fn has(&self, inputs: Inputs) -> bool {
        self.inputs
            .iter()
            .any(|input| !input.dropped && inputs.take(input))
    }

    /// Warns, in order, as Clang does, that the inputs of the kinds `unused`
    /// names are unused (`a.o: 'linker' input unused`), and drops them.
    fn warn_unused_inputs(&mut self, unused: impl Fn(InputKind) -> Option<&'static str>) {
        let names: Vec<(OsString, &str)> = self
            .inputs
            .iter_mut()
            .filter(|input| !input.dropped)
            .filter_map(|input| {
                let what = unused(input.kind)?;
                input.dropped = true;
                Some((input.path.clone(), what))
            })
            .collect();
        for (name, what) in names {
            self.unused(format_args!(
                "{}: '{what}' input unused",
                name.to_string_lossy()
            ));
        }
    }

    /// Reports an argument Clang leaves unused, as its
    /// `-Wunused-command-line-argument` warning: suppressed by `-w` or
    /// `-Wno-unused-command-line-argument`, an error under `-Werror` or
    /// `-Werror=unused-command-line-argument`.
    fn unused(&mut self, message: std::fmt::Arguments<'_>) {
        const GROUP: &str = "unused-command-line-argument";
        if self.compiler != Compiler::Clang
            || self.warning_options.iter().any(|option| option == "-w")
        {
            return;
        }
        // The warning is on by default, so `-Weverything`, which turns on
        // only the warnings no option has set, leaves it as it is.
        let mut enabled = true;
        let mut all_errors = false;
        let mut error = None;
        for option in &self.warning_options {
            let option = option.as_str();
            if option.strip_prefix("-W") == Some(GROUP) {
                enabled = true;
            } else if option == "-Wno-everything" || option.strip_prefix("-Wno-") == Some(GROUP) {
                enabled = false;
            } else if option == "-Werror" {
                all_errors = true;
            } else if option == "-Wno-error" {
                all_errors = false;
            } else if option.strip_prefix("-Werror=") == Some(GROUP) {
                enabled = true;
                error = Some(true);
            } else if option.strip_prefix("-Wno-error=") == Some(GROUP) {
                error = Some(false);
            }
        }
        if !enabled {
            return;
        }
        if error.unwrap_or(all_errors) {
            self.error(format_args!("{message} [-Werror,-W{GROUP}]"));
        } else {
            self.invocation
                .warning(format_args!("{message} [-W{GROUP}]"));
        }
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
            | "-o" => self.outputs.push(operand.into()),
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
                if option == "-x" {
                    self.language = kind_of_language(&lossy);
                }
                self.out.push(option.into());
                self.out.push(operand.into());
            },
        }
    }

    /// Translates an option without an operand.
    fn flag(&mut self, text: &str) {
        if text == "-w"
            || (text.starts_with("-W")
                && !["-Wl,", "-Wa,", "-Wp,"]
                    .iter()
                    .any(|prefix| text.starts_with(prefix)))
        {
            self.warning_options.push(text.into());
        }
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
