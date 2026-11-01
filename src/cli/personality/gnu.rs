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
        position: 0,
        ofast: Ofast::Absent,
        ofast_at: 0,
        eval_method: false,
        fp_model: None,
        basic_complex: None,
        overrides: Vec::new(),
        positions: Vec::new(),
        linker_words: Vec::new(),
    };
    let mut arguments = invocation.arguments.iter().map(OsString::as_os_str);
    while let Some(argument) = arguments.next() {
        state.position += 1;
        if let Some(query) = state.argument(argument, &mut arguments) {
            return Ok(query);
        }
    }
    // Clang refuses an evaluation method beside the options that let the
    // operations be rearranged, in its own words and order.
    if state.eval_method {
        let options: Vec<&str> = state
            .out
            .iter()
            .filter_map(|option| option.to_str())
            .collect();
        let [reassociate, reciprocal, approximate] =
            crate::cli::arguments::eval_method_conflicts(options);
        for (conflict, name) in [
            (approximate, "fapprox-func"),
            (reassociate, "mreassociate"),
            (reciprocal, "freciprocal"),
        ] {
            if conflict {
                state.errors += 1;
                super::frontend_error(format_args!(
                    "option 'ffp-eval-method' cannot be used with option '{name}'"
                ));
            }
        }
    }
    if state.ofast != Ofast::Absent {
        state.clang_warning(
            "deprecated-ofast",
            format_args!(
                "argument '-Ofast' is deprecated; use '-O3' to enable only conforming \
                 optimizations, or consult the documentation for the same behavior"
            ),
        );
    }
    // Clang's driver reports the floating-point models a later option
    // overrides, after `-Ofast`'s deprecation; a final `-Ofast` overrides
    // every model but `aggressive`.
    if state.ofast == Ofast::Last
        && let Some(model) = state.fp_model
        && model != "aggressive"
    {
        state.overrides.push((
            "overriding-option",
            format!("overriding '-ffp-model={model}' option with '-Ofast'"),
        ));
    }
    for (group, message) in std::mem::take(&mut state.overrides) {
        state.clang_warning(group, format_args!("{message}"));
    }
    let target = state.target.unwrap_or_else(|| host_target(personality));
    let mmx = state.machine_options(target);
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
    common.push(format!("--driver-name={}", invocation.name).into());
    // Before the command line's `-D` and `-U`, as the compilers' own
    // macros come before them.
    if !mmx {
        common.push("-U__MMX__".into());
    }
    // GCC applies a final `-Ofast`'s `-ffast-math` before every other
    // option, so any floating-point option overrides it; Clang applies it
    // where the `-Ofast` stands (`State::flag`). Both drivers then link
    // `crtfastmath.o` whatever came after.
    if state.ofast == Ofast::Last {
        if compiler == Compiler::Gcc {
            common.push("-ffast-math".into());
        }
        common.push("--link-fast-math-runtime".into());
    }
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
            bcc.push("--default-output".into());
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
    // GCC quotes `-###`'s words only where the shell needs it and `-v`'s
    // not at all; Clang quotes every word.
    let style = |dry_run: bool| {
        use crate::backend::llvm::CommandStyle;
        match (personality, dry_run) {
            | (Personality::Gcc, true) => CommandStyle::GccDryRun,
            | (Personality::Gcc, false) => CommandStyle::GccVerbose,
            | _ => CommandStyle::Clang,
        }
    };
    // `-v` prints each step before running it, and bcc's driver then
    // prints the commands it runs and its header search, as GCC's and
    // Clang's subprocesses do; `-###` prints the steps alone.
    if state.verbose || state.dry_run {
        let mut text = version::verbose(personality, target);
        if state.dry_run {
            for bcc in &steps {
                text.push_str(&super::command_line(bcc, style(true)));
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
    for mut bcc in steps {
        if state.verbose {
            bcc.insert(1, "-v".into());
            plan.steps.push(Step::Print {
                text:   super::command_line(&bcc, style(false)),
                stderr: true,
            });
        }
        plan.steps.push(Step::Bcc(bcc));
    }
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent switches of the command line being translated."
)]
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
    /// The `-w`, `-W` and `-Qunused-arguments` options, in order, which
    /// decide how Clang reports an argument it leaves unused.
    warning_options: Vec<String>,
    verbose:         bool,
    dry_run:         bool,
    /// The known `-m` options bcc does not implement, judged once the
    /// target is known, since `--target` may come after them.
    machine:         Vec<String>,
    /// The position on the command line of the argument being read.
    position:        usize,
    /// Where `-Ofast` stands among the `-O` options.
    ofast:           Ofast,
    /// The index in [`State::out`] of the `-ffast-math` Clang's last
    /// `-Ofast` stands for.
    ofast_at:        usize,
    /// Clang's `-ffp-eval-method=` was given, which it refuses beside
    /// reassociation, reciprocals and approximate functions.
    eval_method:     bool,
    /// The last Clang `-ffp-model=`.
    fp_model:        Option<&'static str>,
    /// The option that last set Clang's complex range to "basic"
    /// (`-ffast-math`), which a later model overrides with a warning.
    basic_complex:   Option<&'static str>,
    /// Clang's warnings about overridden floating-point options, with
    /// their groups, reported once the warning options are known.
    overrides:       Vec<(&'static str, String)>,
    /// The arguments Clang may report as unused, with their positions and
    /// as Clang spells them: the stage options, `-fsyntax-only`, and the
    /// options that only the link uses (`-L/usr/lib`, `-T x.ld`, `-s`).
    positions:       Vec<(usize, Unused)>,
    /// The linker inputs that are options (`-lm`, `-Wl,...`,
    /// `-Xlinker ARG`, `-z now`), as Clang spells them, each with its index
    /// in [`State::out`], which orders them among the input files.
    linker_words:    Vec<(usize, String)>,
}

/// Where `-Ofast` stands among the `-O` options. Both compilers read a
/// last `-Ofast` as `-O3` with `-ffast-math`: GCC's before every other
/// option, so that any floating-point option overrides it wherever it
/// appears, Clang's where the `-Ofast` stands. Clang reports any `-Ofast`
/// as deprecated.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ofast {
    /// No `-Ofast`.
    Absent,
    /// An `-Ofast` that a later `-O` option overrides.
    Earlier,
    /// The last `-O` option is `-Ofast`.
    Last,
}

/// An argument Clang reports as unused when the command line does not use
/// it.
enum Unused {
    /// A stage option (`-c`), unused under `-fsyntax-only`.
    Stage(&'static str),
    /// `-fsyntax-only`, unused under `-E`, `-M` or `-MM`.
    SyntaxOnly,
    /// An option only the link uses.
    Link(String),
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
const PLAIN_OPTIONS: [&str; 29] = [
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
    "-MG",
    "-H",
    "-nostdlibinc",
    "-nobuiltininc",
    "-ffreestanding",
    "-fhosted",
    "-fgnu89-inline",
    "-fno-gnu89-inline",
    "-fms-extensions",
];

/// The [`PLAIN_OPTIONS`] only the link uses, which Clang reports as unused
/// when the command line does not link. It uses `-static`, `-pthread`,
/// `-nostdlib` and `-nodefaultlibs` in every stage.
const LINK_ONLY_OPTIONS: [&str; 7] = [
    "-shared",
    "-pie",
    "-no-pie",
    "-rdynamic",
    "-nostartfiles",
    "-static-libgcc",
    "-shared-libgcc",
];

/// GCC options that need no translation and cannot change the program bcc
/// builds: driver conveniences and reports.
/// `-g*` (debug information, which bcc does not emit) and the `-f` and `-m`
/// options in `unimplemented.rs` are ignored too.
const IGNORED_OPTIONS: [&str; 4] = ["-pipe", "-save-temps", "-Q", "-time"];

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
                let stage = match text {
                    | "-c" => "-c",
                    | "-S" => "-S",
                    | "-E" => "-E",
                    | "-M" => "-M",
                    | _ => "-MM",
                };
                self.stages.push(stage);
                self.positions.push((self.position, Unused::Stage(stage)));
            },
            // Clang's `-Qunused-arguments` reports no argument or input as
            // unused, whatever the warning options say.
            | "-Qunused-arguments" if self.compiler == Compiler::Clang =>
                self.warning_options.push(text.into()),
            | "-fsyntax-only" => {
                self.syntax_only = true;
                self.positions.push((self.position, Unused::SyntaxOnly));
            },
            | "-ansi" => self.out.push("-std=c89".into()),
            | "-nostdinc" => self.out.push("--nostdinc".into()),
            | "-s" => {
                self.link_only(text.into());
                self.out.push("-Wl,-s".into());
            },
            | "-Ofast" => {
                self.ofast = Ofast::Last;
                self.out.push("-O3".into());
                if self.compiler == Compiler::Clang {
                    self.ofast_at = self.out.len();
                    self.out.push("-ffast-math".into());
                }
            },
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
        // `-MG` lists missing headers only in the rules `-M` and `-MM`
        // write instead of the output.
        if self.out.iter().any(|option| option == "-MG")
            && !self
                .stages
                .iter()
                .any(|stage| matches!(*stage, "-M" | "-MM"))
        {
            if clang {
                self.error(format_args!("option '-MG' requires '-M' or '-MM'"));
            } else {
                self.error(format_args!("'-MG' may only be used with '-M' or '-MM'"));
            }
        }
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
        let syntax_only_unused = self.syntax_only && preprocesses;
        if syntax_only_unused {
            self.syntax_only = false;
        }
        if clang {
            self.warn_unused(syntax_only_unused);
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
        if !clang && !self.stages.contains(&"-S") && self.has(Inputs::Assembly) {
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

    /// Warns as Clang does about the arguments a command line that does not
    /// link leaves unused: first, in order, the inputs and linker options
    /// it does not use (`a.o: 'linker' input unused`, `-lm: 'linker' input
    /// unused`), which it drops; then, in order, the other arguments
    /// (`argument unused during compilation: '-c'`): the stage options
    /// under `-fsyntax-only`, `-fsyntax-only` itself when `syntax_only`
    /// (a preprocessing stage overrides it), and the options of the link.
    fn warn_unused(&mut self, syntax_only: bool) {
        let links = self.stages.is_empty() && !self.syntax_only;
        if !links {
            let assembly_unused = self.syntax_only;
            self.warn_unused_inputs(links, |kind| match kind {
                | InputKind::Link => Some("linker"),
                | InputKind::Assembly {
                    preprocessed: false,
                } if assembly_unused => Some("assembler"),
                | _ => None,
            });
        }
        let stages_unused = self.syntax_only;
        let messages: Vec<String> = self
            .positions
            .iter()
            .filter_map(|(_, unused)| match unused {
                | Unused::Stage(stage) if stages_unused => Some((*stage).to_owned()),
                | Unused::SyntaxOnly if syntax_only => Some("-fsyntax-only".to_owned()),
                | Unused::Link(option) if !links => Some(option.clone()),
                | _ => None,
            })
            .collect();
        // Clang reports the arguments it leaves unused only when no error
        // has occurred, a `-Werror` one about an unused input included.
        if self.errors != 0 {
            return;
        }
        for message in messages {
            self.unused(format_args!(
                "argument unused during compilation: '{message}'"
            ));
        }
    }

    /// Warns, in order, as Clang does, that the inputs of the kinds `unused`
    /// names are unused (`a.o: 'linker' input unused`), and drops them, and
    /// unless the command line `links`, that its linker options are.
    fn warn_unused_inputs(
        &mut self,
        links: bool,
        unused: impl Fn(InputKind) -> Option<&'static str>,
    ) {
        let mut names: Vec<(usize, String, &str)> = self
            .inputs
            .iter_mut()
            .filter(|input| !input.dropped)
            .filter_map(|input| {
                let what = unused(input.kind)?;
                input.dropped = true;
                Some((input.at, input.path.to_string_lossy().into_owned(), what))
            })
            .collect();
        if !links {
            names.extend(
                self.linker_words
                    .iter()
                    .map(|(at, word)| (*at, word.clone(), "linker")),
            );
            names.sort_by_key(|(at, ..)| *at);
        }
        for (_, name, what) in names {
            self.unused(format_args!("{name}: '{what}' input unused"));
        }
    }

    /// Reports an argument Clang leaves unused, as its
    /// `-Wunused-command-line-argument` warning: suppressed by `-w`,
    /// `-Qunused-arguments` or `-Wno-unused-command-line-argument`, an
    /// error under `-Werror` or
    /// `-Werror=unused-command-line-argument`.
    fn unused(&mut self, message: std::fmt::Arguments<'_>) {
        if self
            .warning_options
            .iter()
            .any(|option| option == "-Qunused-arguments")
        {
            return;
        }
        self.clang_warning("unused-command-line-argument", message);
    }

    /// Clang's `-ffp-model=MODEL`: `precise` is its default semantics,
    /// `fast` gives up all but NaNs and infinities, and `aggressive` is
    /// `-ffast-math`. A model after a live `-Ofast` loses to it, and `fast`
    /// then leaves the default semantics (Clang 23 keeps only contraction
    /// there). A model after another, or after `-ffast-math`'s "basic"
    /// complex range, draws Clang's override warnings.
    fn fp_model_option(&mut self, model: &'static str) {
        if let Some(previous) = self.fp_model
            && previous != model
        {
            self.overrides.push((
                "overriding-option",
                format!("overriding '-ffp-model={previous}' option with '-ffp-model={model}'"),
            ));
        }
        if let Some(by) = self.basic_complex
            && model != "aggressive"
        {
            let range = if model == "fast" { "promoted" } else { "full" };
            self.overrides.push((
                "overriding-complex-range",
                format!(
                    "'-ffp-model={model}' sets complex range to \"{range}\" overriding the \
                     setting of \"basic\" that was implied by '{by}'"
                ),
            ));
            self.basic_complex = None;
        }
        self.fp_model = Some(model);
        let options: &[&str] = match model {
            | "fast" if self.ofast != Ofast::Last => &["-ffast-math", "-fno-finite-math-only"],
            | "aggressive" => &["-ffast-math"],
            | _ => &["-fno-fast-math"],
        };
        self.out.extend(options.iter().map(Into::into));
    }

    /// Reports a warning of Clang's driver in `group`, which is on by
    /// default: suppressed by `-w` or `-Wno-GROUP`, an error under
    /// `-Werror` or `-Werror=GROUP`. The GCC personality reports none.
    fn clang_warning(&mut self, group: &str, message: std::fmt::Arguments<'_>) {
        // Clang's `-Wdeprecated` holds its `deprecated-…` groups.
        let parent = group.starts_with("deprecated-").then_some("deprecated");
        let names = |name: Option<&str>| name == Some(group) || (name.is_some() && name == parent);
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
            if names(option.strip_prefix("-W")) {
                enabled = true;
            } else if option == "-Wno-everything" || names(option.strip_prefix("-Wno-")) {
                enabled = false;
            } else if option == "-Werror" {
                all_errors = true;
            } else if option == "-Wno-error" {
                all_errors = false;
            } else if names(option.strip_prefix("-Werror=")) {
                enabled = true;
                error = Some(true);
            } else if names(option.strip_prefix("-Wno-error=")) {
                error = Some(false);
            }
        }
        if !enabled {
            return;
        }
        if error.unwrap_or(all_errors) {
            self.error(format_args!("{message} [-Werror,-W{group}]"));
        } else {
            self.invocation
                .warning(format_args!("{message} [-W{group}]"));
        }
    }

    /// Records a linker option Clang reports as an unused linker input
    /// when the command line does not link, before it is added to
    /// [`State::out`].
    fn linker_word(&mut self, spelling: String) {
        self.linker_words.push((self.out.len(), spelling));
    }

    /// Records an option only the link uses, which Clang reports as unused
    /// when the command line does not link.
    fn link_only(&mut self, spelling: String) {
        self.positions.push((self.position, Unused::Link(spelling)));
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
            | "-T" | "-u" | "-z" | "-e" | "--entry" => {
                let option = match option {
                    | "--entry" => "-e",
                    | other => other,
                };
                // Clang reads `-z` and `-e` as linker inputs, and `-T` and
                // `-u` as options of the link.
                if matches!(option, "-T" | "-u") {
                    self.link_only(format!("{option} {lossy}"));
                } else {
                    self.linker_word(format!("{option} {lossy}"));
                }
                self.out.push(format!("-Wl,{option},{lossy}").into());
            },
            | _ => {
                match option {
                    | "-x" => self.language = kind_of_language(&lossy),
                    | "-l" => self.linker_word(format!("-l{lossy}")),
                    | "-Xlinker" => self.linker_word(format!("-Xlinker {lossy}")),
                    | "-L" => self.link_only(format!("-L{lossy}")),
                    | _ => {},
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
        if text.starts_with("-Wl,") {
            self.linker_word(text.into());
        } else if LINK_ONLY_OPTIONS.contains(&text) {
            self.link_only(text.into());
        }
        // Clang knows `-ftrapv` but not its negation.
        if self.compiler == Compiler::Clang && text == "-fno-trapv" {
            self.error(format_args!("unknown argument: '{text}'"));
            return;
        }
        if text.starts_with("-O") && self.ofast == Ofast::Last {
            self.ofast = Ofast::Earlier;
            // A later `-O` option takes back Clang's `-ffast-math` at the
            // `-Ofast`'s place; a level in its slot changes nothing.
            if self.compiler == Compiler::Clang {
                self.out[self.ofast_at] = "-O3".into();
            }
        }
        // Clang's floating-point models: `precise` is its default
        // semantics, `fast` gives up all but NaNs and infinities, and
        // `aggressive` is `-ffast-math`. `strict` asks for rounding math,
        // which bcc does not implement (`treatment`).
        if self.compiler == Compiler::Clang {
            match text {
                | "-ffast-math" => self.basic_complex = Some("-ffast-math"),
                | "-fno-fast-math" => self.basic_complex = None,
                | _ => {},
            }
            let model = match text {
                | "-ffp-model=precise" => Some("precise"),
                | "-ffp-model=fast" => Some("fast"),
                | "-ffp-model=aggressive" => Some("aggressive"),
                | _ => None,
            };
            if let Some(model) = model {
                self.fp_model_option(model);
                return;
            }
            if text.starts_with("-ffp-eval-method=") {
                self.eval_method = true;
            }
        }
        // Clang's own floating-point options, which bcc implements and GCC
        // does not know.
        let clang_only = self.compiler == Compiler::Gcc
            && matches!(
                text,
                "-fhonor-nans"
                    | "-fno-honor-nans"
                    | "-fhonor-infinities"
                    | "-fno-honor-infinities"
                    | "-fapprox-func"
                    | "-fno-approx-func"
            );
        if !clang_only
            && (PLAIN_OPTIONS.contains(&text)
                || text.starts_with("-std=")
                || text.starts_with("-O")
                || text.starts_with("-Wl,")
                || text.starts_with("-fdiagnostics-format=")
                || implements_language_option(text))
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
        // `-Werror=NAME` and `-Wno-error=NAME` name the warning `-WNAME`.
        let known = knows(self.compiler, text)
            || text
                .strip_prefix("-Werror=")
                .or_else(|| text.strip_prefix("-Wno-error="))
                .is_some_and(|name| knows(self.compiler, &format!("-W{name}")));
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
    /// Returns whether `__MMX__` stays defined: the last of `-mmmx` and
    /// `-mno-mmx` decides, as in both compilers.
    fn machine_options(&mut self, target: Target) -> bool {
        let mut mmx = true;
        for option in std::mem::take(&mut self.machine) {
            if option == "-mwindows" && target == Target::WindowsGnu {
                self.out.push("-Wl,--subsystem,windows".into());
            } else {
                if let "-mmmx" | "-mno-mmx" = option.as_str() {
                    mmx = option == "-mmmx";
                }
                self.treat(&option, target);
            }
        }
        mmx
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
