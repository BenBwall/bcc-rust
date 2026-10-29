//! The TCC personality. It reads the command line as TCC's
//! `tcc_parse_args` (`libtcc.c`) does: options are looked up in TCC's table
//! in its order, by the longest leading match the table allows (`-run` and
//! `-runFILE`, `-ofile`, `-xc`), `@FILE` reads more arguments from a file,
//! and `-Wp,` splits its operand into options. `-c`, `-E`, `-r`, `-run` and
//! `-shared` choose what is built, the last one winning with TCC's warning;
//! `-run FILE ARGS...` compiles FILE, runs it with ARGS and exits with its
//! status. TCC compiles in C99 with GNU extensions, accepts Microsoft's
//! anonymous structure members (`-fms-extensions` is on by default), keeps
//! assignments between pointers and integers or incompatible pointers
//! warnings, and ignores the `-W`, `-f` and `-m` options it does not know,
//! as it ignores `-pedantic`, `-pipe`, `-s` and its other no-op options
//! (warning only under `-Wunsupported`); any other unknown option is an
//! error (`invalid option -- '-q'`).
//!
//! Each choice translates to bcc's own options: an object per source for
//! `-c`, named as TCC names it (`a.c` → `a.o`; `a.out` for standard input),
//! one bcc step for a link, `-E` or `-M`, and for `-run` a build in a
//! temporary directory and a run of the program. Bounds checking (`-b`),
//! backtraces (`-bt`) and `-bench` statistics are accepted with a warning:
//! bcc has none of them. `docs/design/driver-modes.md` lists the
//! remaining differences.

use std::{
    ffi::{
        OsStr,
        OsString,
    },
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
    Step,
    TemporaryDirectory,
    host_target,
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
        },
    },
    target::Target,
};

/// Translates a TCC command line.
pub(super) fn plan(invocation: &Invocation) -> Result<Plan, MainError> {
    let target = invocation
        .target
        .unwrap_or_else(|| host_target(invocation.personality));
    let mut parser = Parser {
        invocation,
        target,
        state: State::default(),
    };
    let query = parser.parse(invocation.arguments.clone(), true)?;
    let state = parser.state;
    let mut plan = Plan::default();
    // `tcc.c`, `main`: help, or the version for `-v`, before anything else.
    match &query {
        | Some(Query::Help) => {
            let mut text = help(&invocation.name);
            if state.verbose > 0 {
                text.push_str(&more_help(&invocation.name));
            }
            return Ok(Plan::print(text));
        },
        | Some(Query::MoreHelp) => return Ok(Plan::print(more_help(&invocation.name))),
        | Some(Query::Print(text)) => return Ok(Plan::print(text.clone())),
        | Some(Query::Archive(arguments)) => {
            plan.steps.push(Step::Run {
                program:   version::program_name("ar").into(),
                arguments: arguments.clone(),
                stdin:     None,
            });
            return Ok(plan);
        },
        | _ if state.verbose > 0 => plan.steps.push(Step::Print {
            text:   version::banner(invocation.personality, &invocation.name, target),
            stderr: false,
        }),
        | _ => {},
    }
    if let Some(Query::SearchDirectories) = query {
        plan.steps.push(Step::Print {
            text:   search_directories(target),
            stderr: false,
        });
        return Ok(plan);
    }
    if query.is_some() {
        return Ok(plan);
    }
    if state.inputs.is_empty() {
        invocation.error(format_args!("no input files"));
        return Err(MainError::Reported(1));
    }
    if state.action == Some(Action::Object) {
        if state.inputs.iter().any(|input| input.library) {
            invocation.error(format_args!("cannot specify libraries with -c"));
            return Err(MainError::Reported(1));
        }
        let files = state.inputs.iter().filter(|input| !input.name.is_empty());
        if files.count() > 1 && state.output.is_some() {
            invocation.error(format_args!(
                "cannot specify output file with -c many files"
            ));
            return Err(MainError::Reported(1));
        }
    }
    state.steps(invocation, target, plan)
}

/// What a command line asks for instead of a compilation.
enum Query {
    /// `-h`, or no arguments at all.
    Help,
    /// `-hh`.
    MoreHelp,
    /// `-print-search-dirs`, or `-vv` alone.
    SearchDirectories,
    /// `-v` alone, which prints the version; the banner is printed by
    /// [`plan`].
    Version,
    /// `-dumpversion` and `-dumpmachine`, which print and stop at once.
    Print(String),
    /// `-ar ARGS...`, TCC's archiver, which bcc runs as `llvm-ar`.
    Archive(Vec<OsString>),
}

/// What the command line builds: TCC's output type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    /// `-c`: an object per input.
    Object,
    /// `-r`: one object linked from every input.
    Relocatable,
    /// `-E`.
    Preprocess,
    /// `-shared`.
    Shared,
    /// `-run`.
    Run,
}

/// The type `-x` gives the inputs after it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Language {
    /// By extension (`-xn`, or no `-x`).
    #[default]
    None,
    /// `-xc`.
    C,
    /// `-xa`: assembly, preprocessed.
    Assembly,
}

/// An input, in command-line order with the linker options.
struct Input {
    /// What bcc receives: the file name, or a linker option.
    word:     OsString,
    /// The name TCC reports and names outputs after; empty for a linker
    /// option.
    name:     OsString,
    /// The `-x` language in effect for a file.
    language: Language,
    /// `-lNAME`, which `-c` refuses.
    library:  bool,
}

impl Input {
    /// Whether bcc compiles the input, rather than passing it to the
    /// linker.
    fn compiled(&self) -> bool {
        self.language != Language::None
            || self.name == "-"
            || kind(Path::new(&self.name)) != InputKind::Link
    }
}

/// The command line read so far, as TCC's `TCCState` keeps it.
#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "TCC's state holds independent switches, one per option."
)]
struct State {
    inputs:            Vec<Input>,
    /// Compiler options, in order.
    options:           Vec<OsString>,
    action:            Option<Action>,
    output:            Option<OsString>,
    language:          Language,
    /// Whether an input could not be opened, after which TCC reads no
    /// more.
    failed:            bool,
    /// `-v` counts.
    verbose:           usize,
    /// Whether no input and no option but `-v`, `-B` and `-m64` was read,
    /// which leaves TCC with nothing to do but print.
    empty:             bool,
    /// `-run`'s joined options (`-run -lm` from a `#!` line).
    run:               Option<String>,
    /// The program's arguments after `-run FILE`.
    run_arguments:     Vec<OsString>,
    /// `-rstdin FILE`.
    run_stdin:         Option<OsString>,
    standard_c11:      bool,
    /// `-Wunsupported`: warn about the options TCC ignores.
    warn_unsupported:  bool,
    /// `-Werror`.
    warnings_error:    bool,
    /// `-P` (1) or `-P1` (2).
    line_markers:      u32,
    /// `-dM`.
    dump_macros:       bool,
    /// `-M` or `-MM`: rules only.
    just_dependencies: bool,
    /// `-M`, `-MD` (every file) or `-MM`, `-MMD` (no system headers), and
    /// whether any was given.
    dependencies:      Option<bool>,
    dependency_file:   Option<OsString>,
    phony:             bool,
}

/// Reads a command line into a [`State`].
struct Parser<'a> {
    invocation: &'a Invocation,
    target:     Target,
    state:      State,
}

/// How an option in TCC's table takes its value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operand {
    /// None: the argument must be the option alone.
    None,
    /// Joined, possibly empty (`TCC_OPTION_NOSEP`).
    Joined,
    /// Joined, or the next argument when nothing is joined.
    JoinedOrNext,
}

/// TCC's options (`tcc_options` in `libtcc.c`), in its order, for x86-64
/// Linux with bounds checking and backtraces: an argument is the first
/// entry it starts with whose operand rule it satisfies. The ignored
/// entries at the end are TCC's.
const OPTIONS: &[(&str, Operand)] = &[
    ("h", Operand::None),
    ("-help", Operand::None),
    ("?", Operand::None),
    ("hh", Operand::None),
    ("v", Operand::Joined),
    ("-version", Operand::None),
    ("I", Operand::JoinedOrNext),
    ("D", Operand::JoinedOrNext),
    ("U", Operand::JoinedOrNext),
    ("P", Operand::Joined),
    ("L", Operand::JoinedOrNext),
    ("B", Operand::JoinedOrNext),
    ("l", Operand::JoinedOrNext),
    ("bench", Operand::None),
    ("bt", Operand::Joined),
    ("b", Operand::None),
    ("g", Operand::Joined),
    ("c", Operand::None),
    ("dumpmachine", Operand::None),
    ("dumpversion", Operand::None),
    ("d", Operand::Joined),
    ("static", Operand::None),
    ("std", Operand::Joined),
    ("shared", Operand::None),
    ("soname", Operand::JoinedOrNext),
    ("o", Operand::JoinedOrNext),
    ("pthread", Operand::None),
    ("run", Operand::Joined),
    ("rstdin", Operand::JoinedOrNext),
    ("rdynamic", Operand::None),
    ("r", Operand::None),
    ("Wl,", Operand::Joined),
    ("Wp,", Operand::Joined),
    ("W", Operand::Joined),
    ("O", Operand::Joined),
    ("m", Operand::Joined),
    ("f", Operand::Joined),
    ("isystem", Operand::JoinedOrNext),
    ("include", Operand::JoinedOrNext),
    ("nostdinc", Operand::None),
    ("nostdlib", Operand::None),
    ("print-search-dirs", Operand::None),
    ("w", Operand::None),
    ("E", Operand::None),
    ("M", Operand::None),
    ("MM", Operand::None),
    ("MD", Operand::Joined),
    ("MMD", Operand::Joined),
    ("MF", Operand::JoinedOrNext),
    ("MP", Operand::None),
    ("x", Operand::JoinedOrNext),
    ("ar", Operand::None),
    ("arch", Operand::JoinedOrNext),
    ("C", Operand::None),
    ("-param", Operand::JoinedOrNext),
    ("pedantic", Operand::None),
    ("pie", Operand::None),
    ("no-pie", Operand::None),
    ("pipe", Operand::None),
    ("s", Operand::None),
    ("traditional", Operand::None),
];

impl Parser<'_> {
    /// Reads `arguments` in order, as `tcc_parse_args` does, and returns
    /// the query they make instead of a compilation, if any. `command_line`
    /// is false for `-run`'s joined options, which may not make one.
    fn parse(
        &mut self,
        mut arguments: Vec<OsString>,
        command_line: bool,
    ) -> Result<Option<Query>, MainError> {
        self.state.empty = command_line;
        let mut index = 0;
        while index < arguments.len() {
            let argument = arguments[index].clone();
            let bytes = argument.as_encoded_bytes();
            // `@FILE` is replaced by the arguments it holds.
            if let [b'@', _, ..] = bytes {
                let name = PathBuf::from(&argument.to_string_lossy()[1..]);
                let Ok(text) = std::fs::read(&name) else {
                    self.invocation
                        .error(format_args!("listfile '{}' not found", name.display()));
                    return Err(MainError::Reported(1));
                };
                let inserted = split_arguments(&String::from_utf8_lossy(&text), None);
                drop(arguments.splice(index..=index, inserted));
                continue;
            }
            index += 1;
            if bytes.first() != Some(&b'-') || bytes == b"-" {
                self.add_input(&argument)?;
                self.state.empty = false;
                if self.state.run.is_some() {
                    break;
                }
                continue;
            }
            // `tcc FILES -run -- ARGS`.
            if bytes == b"--" {
                if self.state.run.is_some() {
                    break;
                }
                continue;
            }
            let text = argument.to_string_lossy().into_owned();
            let rest = &text[1..];
            let Some(&(name, operand)) = OPTIONS.iter().find(|(name, operand)| {
                rest.strip_prefix(name)
                    .is_some_and(|joined| *operand != Operand::None || joined.is_empty())
            }) else {
                self.invocation
                    .error(format_args!("invalid option -- '{text}'"));
                return Err(MainError::Reported(1));
            };
            let joined = &rest[name.len()..];
            let value: OsString = if operand == Operand::JoinedOrNext && joined.is_empty() {
                let Some(next) = arguments.get(index) else {
                    self.invocation
                        .error(format_args!("argument to '{text}' is missing"));
                    return Err(MainError::Reported(1));
                };
                index += 1;
                next.clone()
            } else {
                joined.into()
            };
            let value_text = value.to_string_lossy().into_owned();
            match self.option(name, &text, value, &value_text)? {
                | Parsed::Read => self.state.empty = false,
                | Parsed::Continue => {},
                | Parsed::Insert(inserted) => {
                    drop(arguments.splice(index..index, inserted));
                    self.state.empty = false;
                },
                // `-dumpversion` and `-dumpmachine` print and exit at once.
                | Parsed::Query(query @ Query::Print(_)) => return Ok(Some(query)),
                | Parsed::Query(query) => {
                    // `extra_action`: only as the first argument, and not
                    // from `-run`'s options.
                    if !command_line || !self.state.empty {
                        self.invocation
                            .error(format_args!("cannot parse {text} here"));
                        return Err(MainError::Reported(1));
                    }
                    return Ok(Some(match query {
                        | Query::Archive(_) => Query::Archive(arguments.split_off(index)),
                        | query => query,
                    }));
                },
            }
        }
        if !command_line {
            return Ok(None);
        }
        // `tcc -run FILE ARGS...`: the arguments after FILE are the
        // program's; `-run`'s joined options are read after the rest.
        if let Some(options) = self.state.run.clone() {
            self.state.run_arguments = arguments.split_off(index);
            if !options.is_empty() {
                let empty = self.state.empty;
                drop(self.parse(split_arguments(&options, None), false)?);
                self.state.empty = empty;
            }
            return Ok(None);
        }
        Ok(if !self.state.empty {
            None
        } else if self.state.verbose >= 2 {
            Some(Query::SearchDirectories)
        } else if self.state.verbose == 1 {
            Some(Query::Version)
        } else {
            Some(Query::Help)
        })
    }

    /// Reads the option `name` from TCC's table, spelled `text`, with
    /// its `value`.
    fn option(
        &mut self,
        name: &str,
        text: &str,
        value: OsString,
        value_text: &str,
    ) -> Result<Parsed, MainError> {
        let state = &mut self.state;
        match name {
            | "h" | "-help" | "?" => return Ok(Parsed::Query(Query::Help)),
            | "hh" => return Ok(Parsed::Query(Query::MoreHelp)),
            | "print-search-dirs" => return Ok(Parsed::Query(Query::SearchDirectories)),
            | "ar" => return Ok(Parsed::Query(Query::Archive(Vec::new()))),
            | "v" => {
                state.verbose += 1 + value_text.bytes().take_while(|&byte| byte == b'v').count();
                return Ok(Parsed::Continue);
            },
            | "-version" => {
                state.verbose += 1;
                return Ok(Parsed::Continue);
            },
            | "dumpversion" =>
                return Ok(Parsed::Query(Query::Print(format!(
                    "{}\n",
                    version::TCC_VERSION
                )))),
            | "dumpmachine" =>
                return Ok(Parsed::Query(Query::Print(format!(
                    "{}\n",
                    machine(self.target)
                )))),
            | "I" | "D" | "U" | "isystem" | "include" => {
                state.options.push(format!("-{name}").into());
                state.options.push(value);
            },
            | "L" => self.link_option(concat("-L", &value)),
            | "l" =>
                if !state.failed {
                    state.inputs.push(Input {
                        word:     concat("-l", &value),
                        name:     value,
                        language: Language::None,
                        library:  true,
                    });
                },
            | "B" => return Ok(Parsed::Continue),
            | "bench" => self.invocation.warning(format_args!(
                "'-bench' is ignored: bcc gathers no compilation statistics"
            )),
            | "bt" => self.invocation.warning(format_args!(
                "backtraces (-bt) are not supported; ignoring it"
            )),
            | "b" => self.invocation.warning(format_args!(
                "bounds checking (-b) is not supported; ignoring it"
            )),
            // Debug information has no effect on the program.
            | "g" => {},
            | "c" => self.action(Action::Object, name),
            | "r" => self.action(Action::Relocatable, name),
            | "E" => self.action(Action::Preprocess, name),
            | "shared" => self.action(Action::Shared, name),
            | "run" => {
                state.run = Some(value_text.to_owned());
                self.action(Action::Run, name);
            },
            | "rstdin" => state.run_stdin = Some(value),
            | "d" => match value_text.bytes().next() {
                | Some(b'M') => state.dump_macros = true,
                | Some(b'D') => {
                    self.invocation
                        .warning(format_args!("'-dD' is treated as '-dM'"));
                    state.dump_macros = true;
                },
                | Some(b't') => {
                    self.invocation.error(format_args!(
                        "'-dt' is not supported: bcc defines no 'test_...' macros"
                    ));
                    return Err(MainError::Reported(1));
                },
                | Some(digit) if digit.is_ascii_digit() => {},
                | _ => self.unsupported(text),
            },
            | "static" | "rdynamic" | "nostdlib" => self.link_option(format!("-{name}").into()),
            | "soname" => self.link_option(concat("-Wl,-soname,", &value)),
            | "Wl," => self.link_option(concat("-Wl,", &value)),
            | "Wp," => return Ok(Parsed::Insert(split_arguments(value_text, Some(',')))),
            | "pthread" => state.options.push("-pthread".into()),
            | "std" =>
                if matches!(value_text, "=c11" | "=gnu11") {
                    state.standard_c11 = true;
                },
            | "o" => {
                if state.output.is_some() {
                    self.invocation.warning(format_args!("multiple -o option"));
                }
                state.output = Some(value);
            },
            | "nostdinc" => state.options.push("--nostdinc".into()),
            | "w" => state.options.push("-w".into()),
            | "O" => state.options.push(
                // `-O` and `-Os` are TCC's level 1, which defines only
                // `__OPTIMIZE__`.
                match value_text.bytes().next() {
                    | Some(digit @ b'0'..=b'3') => format!("-O{}", char::from(digit)),
                    | Some(b'4'..=b'9') => "-O3".into(),
                    | _ => "-O1".into(),
                }
                .into(),
            ),
            | "P" => state.line_markers = if value_text.starts_with('1') { 2 } else { 1 },
            | "M" | "MM" => {
                state.just_dependencies = true;
                state.dependencies = Some(name == "M");
            },
            | "MD" | "MMD" => {
                state.dependencies = Some(name == "MD");
                // `-MMD,FILE`, as the Linux kernel spells it.
                if let Some(file) = value_text.strip_prefix(',') {
                    state.dependency_file = Some(file.into());
                }
            },
            | "MF" => state.dependency_file = Some(value),
            | "MP" => state.phony = true,
            | "x" =>
                state.language = match value_text.bytes().next() {
                    | Some(b'c') => Language::C,
                    | Some(b'a') => Language::Assembly,
                    | Some(b'n') => Language::None,
                    | Some(b'b') => {
                        self.invocation.error(format_args!(
                            "'-xb' is not supported: bcc cannot add a binary file"
                        ));
                        return Err(MainError::Reported(1));
                    },
                    | _ => {
                        self.invocation
                            .warning(format_args!("unsupported language '{value_text}'"));
                        Language::None
                    },
                },
            | "W" => self.warning_option(text, value_text),
            | "f" => self.flag(text, value_text)?,
            | "m" => {
                if matches!(value_text, "64") {
                    return Ok(Parsed::Continue);
                }
                self.machine_option(text, value_text)?;
            },
            // TCC's ignored options.
            | _ => self.unsupported(text),
        }
        Ok(Parsed::Read)
    }

    /// `-c`, `-r`, `-E`, `-shared` or `-run`: the last one decides, as in
    /// TCC, which warns about each one after the first.
    fn action(&mut self, action: Action, name: &str) {
        if self.state.action.is_some() {
            self.invocation.warning(format_args!(
                "-{name}: overriding compiler action already specified"
            ));
        }
        self.state.action = Some(action);
    }

    /// A linker option, which keeps its place among the inputs.
    fn link_option(&mut self, option: OsString) {
        if !self.state.failed {
            self.state.inputs.push(Input {
                word:     option,
                name:     OsString::new(),
                language: Language::None,
                library:  false,
            });
        }
    }

    /// An option TCC ignores, with a warning under `-Wunsupported`.
    fn unsupported(&self, text: &str) {
        if self.state.warn_unsupported {
            self.invocation
                .warning(format_args!("unsupported option '{text}'"));
        }
    }

    /// `-W`: TCC's warnings (`-Wall`, `-Werror`, `-Wunsupported`,
    /// `-Wwrite-strings`, `-Wimplicit-function-declaration`,
    /// `-Wdiscarded-qualifiers`, each also with `no-`, and `-Werror=` and
    /// `-Wno-error=` of the last two). bcc reads those it has in its own
    /// spelling; the others are ignored, as TCC ignores an unknown one.
    fn warning_option(&mut self, text: &str, value: &str) {
        let (enabled, name) = value
            .strip_prefix("no-")
            .map_or((true, value), |name| (false, name));
        let (error, name) = name
            .strip_prefix("error=")
            .map_or((false, name), |name| (true, name));
        let bcc_name = match name {
            | "error" if !error => {
                self.state.warnings_error = enabled;
                return;
            },
            | "unsupported" if !error => {
                self.state.warn_unsupported = enabled;
                return;
            },
            | "" | "all" | "write-strings" | "implicit-function-declaration" => return,
            | "discarded-qualifiers" => "incompatible-pointer-types-discards-qualifiers",
            | _ => {
                self.unsupported(text);
                return;
            },
        };
        let option = match (error, enabled) {
            | (false, true) => format!("-W{bcc_name}"),
            | (false, false) => format!("-Wno-{bcc_name}"),
            | (true, true) => format!("-Werror={bcc_name}"),
            | (true, false) => format!("-Wno-error={bcc_name}"),
        };
        self.state.options.push(option.into());
    }

    /// `-f`: TCC's flags (`tcc -hh`). Those bcc has are passed on; of the
    /// others, one that would change the program is refused as the GCC
    /// personality refuses it. A flag TCC does not know is ignored, as TCC
    /// ignores it, so the program is the one TCC builds; bcc honours the
    /// few that only bring its code closer to TCC's, which never assumes
    /// that signed or pointer arithmetic does not overflow, never drops a
    /// null pointer check and never replaces library calls.
    fn flag(&mut self, text: &str, value: &str) -> Result<(), MainError> {
        let name = value.strip_prefix("no-").unwrap_or(value);
        let enabled = name.len() == value.len();
        match name {
            | "unsigned-char"
            | "signed-char"
            | "common"
            | "gnu89-inline"
            | "dollars-in-identifiers" => self.state.options.push(text.into()),
            // TCC's `-fms-extensions` is only Microsoft's anonymous
            // structure members.
            | "ms-extensions" => self.state.options.push(
                if enabled {
                    "-fms-anonymous-structs"
                } else {
                    "-fno-ms-anonymous-structs"
                }
                .into(),
            ),
            | "reverse-funcargs" =>
                if enabled {
                    self.invocation.warning(format_args!(
                        "'{text}' is ignored: bcc evaluates function arguments in its own order, \
                         which C leaves unspecified"
                    ));
                },
            | "asynchronous-unwind-tables" => {},
            | "leading-underscore" | "test-coverage" => self.treat(text)?,
            | _ =>
                if implements_language_option(text)
                    && (matches!(
                        value,
                        "wrapv"
                            | "no-strict-overflow"
                            | "wrapv-pointer"
                            | "no-strict-aliasing"
                            | "no-delete-null-pointer-checks"
                            | "no-builtin"
                            | "pic"
                            | "PIC"
                            | "pie"
                            | "PIE"
                            | "no-pic"
                            | "no-pie"
                    ) || value.starts_with("no-builtin-"))
                {
                    self.state.options.push(text.into());
                } else {
                    self.unsupported(text);
                },
        }
        Ok(())
    }

    /// `-m`: TCC's `-mms-bitfields` and `-m[no-]sse`, treated as the GCC
    /// personality treats them (`-mno-sse` and, on Linux, `-mms-bitfields`
    /// are refused), and `-m32`, for which TCC runs its i386 compiler. Any
    /// other is ignored, as in TCC.
    fn machine_option(&mut self, text: &str, value: &str) -> Result<(), MainError> {
        match value {
            | "ms-bitfields" | "no-ms-bitfields" | "sse" | "no-sse" | "32" => self.treat(text),
            | _ => {
                self.unsupported(text);
                Ok(())
            },
        }
    }

    /// An option TCC implements and bcc does not: refused or warned about
    /// as the GCC personality does.
    fn treat(&self, text: &str) -> Result<(), MainError> {
        match treatment(text, self.target) {
            | Treatment::Ignore => Ok(()),
            | Treatment::Warn(why) => {
                self.invocation
                    .warning(format_args!("'{text}' is ignored: {why}"));
                Ok(())
            },
            | Treatment::Reject(why) => {
                self.invocation
                    .error(format_args!("'{text}' is not supported: {why}"));
                Err(MainError::Reported(1))
            },
        }
    }

    /// Adds an input file. TCC reads its inputs in order and stops at the
    /// first it cannot add (`tcc.c`, `main`): the inputs after one it
    /// cannot open, missing or unreadable, are dropped. bcc's job reports
    /// such a C source in TCC's words, after compiling the ones before it;
    /// another such file is reported here, and nothing runs.
    fn add_input(&mut self, input: &OsStr) -> Result<(), MainError> {
        let state = &mut self.state;
        if state.failed {
            return Ok(());
        }
        if input != "-" && std::fs::File::open(input).is_err() {
            state.failed = true;
            let c = match state.language {
                | Language::C => true,
                | Language::Assembly => false,
                | Language::None => kind(Path::new(input)) == InputKind::C,
            };
            if !c {
                self.invocation
                    .error(format_args!("file '{}' not found", input.to_string_lossy()));
                return Err(MainError::Reported(1));
            }
        }
        state.inputs.push(Input {
            word:     input.into(),
            name:     input.into(),
            language: state.language,
            library:  false,
        });
        Ok(())
    }
}

/// What reading one option did.
enum Parsed {
    /// The option was read; TCC now has something to do.
    Read,
    /// The option was read, and TCC would still only print (`-v`, `-B`,
    /// `-m64`).
    Continue,
    /// `-Wp,`: these arguments are read next.
    Insert(Vec<OsString>),
    /// The option asks for something other than a compilation.
    Query(Query),
}

impl State {
    /// The steps that build what the command line asks for.
    fn steps(
        self,
        invocation: &Invocation,
        target: Target,
        mut plan: Plan,
    ) -> Result<Plan, MainError> {
        let mut common: Vec<OsString> = vec![
            invocation.program.clone(),
            "--compiler-identity=tcc".into(),
            "--diagnostic-format=tcc".into(),
            format!("--target={}", target.triple()).into(),
            format!("--driver-name={}", invocation.name).into(),
            if self.standard_c11 {
                "-std=gnu11"
            } else {
                "-std=gnu99"
            }
            .into(),
            // TCC accepts Microsoft's anonymous structure members unless
            // `-fno-ms-extensions` is given.
            "-fms-anonymous-structs".into(),
        ];
        // TCC warns where GCC 14 and Clang stop: an assignment between a
        // pointer and an integer, or between incompatible pointers. Its
        // `-Werror` makes those errors again.
        if self.warnings_error {
            common.push("-Werror".into());
        } else {
            for group in [
                "int-conversion",
                "incompatible-pointer-types",
                "incompatible-function-pointer-types",
            ] {
                common.push(format!("-Wno-error={group}").into());
            }
        }
        common.extend(self.options.iter().cloned());
        let verbose = self.verbose > 0;
        let announce = |plan: &mut Plan, name: &OsStr, arrow: &str| {
            if verbose {
                plan.steps.push(Step::Print {
                    text:   format!("{arrow} {}\n", name.to_string_lossy()),
                    stderr: false,
                });
            }
        };
        let sources = || {
            self.inputs
                .iter()
                .filter(|input| !input.name.is_empty() && !input.library)
        };
        match self.action {
            | Some(Action::Object) if !self.just_dependencies => {
                // TCC compiles each input on its own and stops at the
                // first that fails. An input for the linker goes with the
                // first, for bcc's warning that it is unused (TCC rewrites
                // an object given to `-c`).
                let mut unused: Vec<OsString> = sources()
                    .filter(|input| !input.compiled())
                    .map(|input| input.word.clone())
                    .collect();
                if sources().all(|input| !input.compiled()) {
                    let mut step = common;
                    step.push("-c".into());
                    step.append(&mut unused);
                    plan.steps.push(Step::Bcc(step));
                    return Ok(plan);
                }
                for input in sources().filter(|input| input.compiled()) {
                    let output = self
                        .output
                        .clone()
                        .unwrap_or_else(|| default_output(&input.name, true));
                    announce(&mut plan, &input.name, "->");
                    let mut step = common.clone();
                    step.push("-c".into());
                    self.dependency_options(&mut step, &output);
                    step.extend(words(std::iter::once(input)));
                    step.append(&mut unused);
                    step.push(concat("--output=", &output));
                    plan.steps.push(Step::Bcc(step));
                    announce(&mut plan, &output, "<-");
                }
            },
            | Some(Action::Preprocess) => {
                for input in sources() {
                    announce(&mut plan, &input.name, "->");
                }
                let mut step = common;
                step.push("-E".into());
                if self.line_markers == 1 {
                    step.push("-P".into());
                }
                if self.dump_macros {
                    step.push("-dM".into());
                }
                step.extend(words(sources()));
                if let Some(output) = &self.output {
                    step.push(concat("--output=", output));
                }
                plan.steps.push(Step::Bcc(step));
            },
            | _ if self.just_dependencies => {
                // `-M` and `-MM` write the rules to stdout or the `-MF`
                // file, named after the output: one rule for each input
                // under `-c`, or one for them all.
                let groups: Vec<Vec<&Input>> = if self.action == Some(Action::Object) {
                    sources().map(|input| vec![input]).collect()
                } else {
                    vec![sources().collect()]
                };
                for group in groups {
                    let first = group
                        .first()
                        .map_or_else(OsString::new, |input| input.name.clone());
                    let target = self
                        .output
                        .clone()
                        .unwrap_or_else(|| default_output(&first, true));
                    let mut step = common.clone();
                    step.push(
                        if self.dependencies == Some(true) {
                            "-M"
                        } else {
                            "-MM"
                        }
                        .into(),
                    );
                    step.push("-MF".into());
                    step.push(self.dependency_file.clone().unwrap_or_else(|| "-".into()));
                    step.push("-MT".into());
                    step.push(target);
                    if self.phony {
                        step.push("-MP".into());
                    }
                    step.extend(words(group.into_iter()));
                    plan.steps.push(Step::Bcc(step));
                }
            },
            | Some(Action::Run) => {
                let directory = TemporaryDirectory::create().map_err(|source| {
                    MainError::WriteOutputFileError {
                        path: std::env::temp_dir(),
                        source,
                    }
                })?;
                let program = directory.0.join(
                    if matches!(target, Target::WindowsGnu | Target::WindowsMsvc) {
                        "a.exe"
                    } else {
                        "a.out"
                    },
                );
                for input in sources() {
                    announce(&mut plan, &input.name, "->");
                }
                let mut step = common;
                step.extend(words(self.inputs.iter()));
                step.push("-o".into());
                step.push(program.clone().into_os_string());
                plan.steps.push(Step::Bcc(step));
                plan.steps.push(Step::Run {
                    program,
                    arguments: self.run_arguments,
                    stdin: self.run_stdin.map(PathBuf::from),
                });
                plan.temporary = Some(directory);
            },
            | action => {
                let output = self.output.clone().unwrap_or_else(|| "a.out".into());
                for input in sources() {
                    announce(&mut plan, &input.name, "->");
                }
                let mut step = common;
                match action {
                    | Some(Action::Shared) => step.push("-shared".into()),
                    // `-r` links the objects into one, with no start files
                    // and no libraries.
                    | Some(Action::Relocatable) => {
                        step.push("-no-pie".into());
                        step.push("-nostdlib".into());
                        step.push("-Wl,-r".into());
                    },
                    | _ => {},
                }
                // TCC writes one rule for the whole link; bcc writes one
                // per source, so it is named after the output only when
                // there is one source.
                if sources().count() == 1 {
                    self.dependency_options(&mut step, &output);
                } else if let Some(all) = self.dependencies {
                    step.push(if all { "-MD" } else { "-MMD" }.into());
                    if self.phony {
                        step.push("-MP".into());
                    }
                }
                step.extend(words(self.inputs.iter()));
                step.push(concat("--output=", &output));
                plan.steps.push(Step::Bcc(step));
                announce(&mut plan, &output, "<-");
            },
        }
        Ok(plan)
    }

    /// `-MD` or `-MMD` for a step whose output is `output`: TCC names the
    /// rule after it and the file after it with `.d`, unless `-MF` names
    /// one.
    fn dependency_options(&self, step: &mut Vec<OsString>, output: &OsStr) {
        let Some(all) = self.dependencies else {
            return;
        };
        step.push(if all { "-MD" } else { "-MMD" }.into());
        step.push("-MF".into());
        step.push(self.dependency_file.clone().unwrap_or_else(|| {
            let output = Path::new(output);
            let mut file = output.to_path_buf();
            if output.extension().is_some() {
                _ = file.set_extension("d");
            } else {
                let mut name = file.into_os_string();
                name.push(".d");
                file = name.into();
            }
            file.into_os_string()
        }));
        step.push("-MT".into());
        step.push(output.into());
        if self.phony {
            step.push("-MP".into());
        }
    }
}

/// The words that give bcc `inputs`, with `-x` before each file whose
/// language differs from the one before it.
fn words<'a>(inputs: impl Iterator<Item = &'a Input>) -> Vec<OsString> {
    let mut words = Vec::new();
    let mut current = Language::None;
    for input in inputs {
        if !input.name.is_empty() && !input.library && input.language != current {
            current = input.language;
            words.push("-x".into());
            words.push(
                match current {
                    | Language::None => "none",
                    | Language::C => "c",
                    | Language::Assembly => "assembler-with-cpp",
                }
                .into(),
            );
        }
        words.push(input.word.clone());
    }
    words
}

/// The output TCC names after `input` (`default_outputfile` in `tcc.c`):
/// for an object or rules (`object`), its base name with `.o` for its
/// extension; otherwise, or for standard input or a name without an
/// extension, `a.out`.
fn default_output(input: &OsStr, object: bool) -> OsString {
    let name = if input.is_empty() || input == "-" {
        Path::new("a")
    } else {
        Path::new(Path::new(input).file_name().unwrap_or(input))
    };
    let text = name.as_os_str().to_string_lossy();
    match text.rfind('.') {
        | Some(dot) if object => format!("{}.o", &text[..dot]).into(),
        | _ => "a.out".into(),
    }
}

/// `prefix` and `value` as one argument.
fn concat(prefix: &str, value: &OsStr) -> OsString {
    let mut argument = OsString::from(prefix);
    argument.push(value);
    argument
}

/// Splits `text` into arguments as TCC's `dynarray_split` does: at
/// `separator` (`-Wp,` splits at commas), or, without one, at blanks and
/// control characters, where double quotes group and a backslash escapes a
/// quote or a backslash (an `@FILE`, or `-run`'s joined options). Blanks
/// before each argument are skipped either way.
fn split_arguments(text: &str, separator: Option<char>) -> Vec<OsString> {
    let mut arguments = Vec::new();
    let mut characters = text.chars().peekable();
    loop {
        while characters.next_if(|&c| c <= ' ').is_some() {}
        if characters.peek().is_none() {
            break;
        }
        let mut argument = String::new();
        let mut quoted = false;
        while let Some(c) = characters.next() {
            if let Some(separator) = separator {
                if c == separator {
                    break;
                }
            } else if c == '\\' && matches!(characters.peek(), Some('"' | '\\')) {
                argument.extend(characters.next());
                continue;
            } else if c == '"' {
                quoted = !quoted;
                continue;
            } else if c <= ' ' && !quoted {
                break;
            }
            argument.push(c);
        }
        arguments.push(argument.into());
    }
    arguments
}

/// `-dumpmachine`: TCC's guess at the GNU triple of its target.
const fn machine(target: Target) -> &'static str {
    match target {
        | Target::LinuxGnu => "x86_64-pc-linux-gnu",
        | Target::LinuxMusl => "x86_64-pc-linux-musl",
        | Target::WindowsGnu | Target::WindowsMsvc => "x86_64-pc-mingw32",
        | Target::Wasm32Unknown | Target::Wasm32Wasi => "wasm32-unknown-unknown",
    }
}

/// `-print-search-dirs`, in TCC's form: where bcc is installed, the
/// header directories it searches after its own embedded headers, the
/// libraries' directories and the start files' that the bundled clang
/// links with, and the dynamic linker. bcc has no `libtcc1.a`.
fn search_directories(target: Target) -> String {
    let mut text = String::new();
    let install = std::env::current_exe()
        .ok()
        .and_then(|program| program.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    _ = writeln!(text, "install: {}", install.display());
    text.push_str("include:\n");
    for directory in crate::cli::search::library_directories(target, None) {
        _ = writeln!(text, "  {}", directory.display());
    }
    text.push_str("libraries:\n");
    let clang = version::program_name("clang");
    let query = |argument: &str| {
        std::process::Command::new(&clang)
            .arg(format!("--target={}", target.triple()))
            .arg(argument)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    };
    let libraries = query("-print-search-dirs");
    if let Some(list) = libraries
        .lines()
        .find_map(|line| line.strip_prefix("libraries: ="))
    {
        for directory in std::env::split_paths(list) {
            _ = writeln!(text, "  {}", directory.display());
        }
    }
    let start = query("-print-file-name=crt1.o");
    let start = Path::new(start.trim());
    if start.is_absolute()
        && let Some(directory) = start.parent()
    {
        _ = write!(text, "crt:\n  {}\n", directory.display());
    }
    let interpreter = match target {
        | Target::LinuxGnu => Some("/lib64/ld-linux-x86-64.so.2"),
        | Target::LinuxMusl => Some("/lib/ld-musl-x86_64.so.1"),
        | _ => None,
    };
    if let Some(interpreter) = interpreter {
        _ = write!(text, "elfinterp:\n  {interpreter}\n");
    }
    text
}

/// `-h`, in TCC's layout, for the options this personality reads.
fn help(name: &str) -> String {
    format!(
        "bcc {bcc} - TCC-compatible driver (tcc {tcc}); not TCC
Usage: {name} [options...] [-o outfile] [-c] infile(s)...
       {name} [options...] -run infile (or --) [arguments...]
General options:
  -c           compile only - generate an object file
  -o outfile   set output filename
  -run         run compiled source
  -fflag       set or reset (with 'no-' prefix) 'flag' (see {name} -hh)
  -Wwarning    set or reset (with 'no-' prefix) 'warning' (see {name} -hh)
  -w           disable all warnings
  -v --version show version
  -vv          show search paths
  -h -hh       show this, show more help
  -            use stdin pipe as infile
  @listfile    read arguments from listfile
Preprocessor options:
  -Idir        add include path 'dir'
  -Dsym[=val]  define 'sym' with value 'val'
  -Usym        undefine 'sym'
  -E           preprocess only
  -nostdinc    do not use standard system include paths
Linker options:
  -Ldir        add library path 'dir'
  -llib        link with dynamic or static library 'lib'
  -nostdlib    do not link with standard crt and libraries
  -r           generate (relocatable) object file
  -rdynamic    export all global symbols to dynamic linker
  -shared      generate a shared library/dll
  -soname      set name for shared library to be used at runtime
  -Wl,-opt[=val]  set linker option
Debugger options:
  -g           accepted; bcc writes no debug information
  -b, -bt[N]   accepted with a warning; bcc has no bounds checker or backtraces
Misc. options:
  -std=version define __STDC_VERSION__ according to version (c11/gnu11)
  -x[c|a|n]    specify type of the next infile (C,ASM,NONE)
  -M[M]D       generate make dependency file [ignore system files]
  -M[M]        as above but no other output
  -MF file     specify dependency file name
Tools:
  create library  : {name} -ar [crstvx] lib [files]
See docs/design/driver-modes.md.
",
        bcc = env!("CARGO_PKG_VERSION"),
        tcc = version::TCC_VERSION,
    )
}

/// `-hh`, in TCC's layout.
fn more_help(name: &str) -> String {
    format!(
        "bcc {bcc} - TCC-compatible driver; more options
Special options:
  -P -P1                        with -E: no/alternative #line output
  -dD -dM                       with -E: output #define directives
  -pthread                      same as -D_REENTRANT and -lpthread
  -On                           same as -D__OPTIMIZE__ for n > 0
  -Wp,-opt                      same as -opt
  -include file                 include 'file' above each input file
  -isystem dir                  add 'dir' to system include path
  -static                       link to static libraries (not recommended)
  -dumpversion                  print version
  -dumpmachine                  print target triple
  -print-search-dirs            print search paths
  -rstdin file                  with -run: use 'file' as custom stdin
Ignored options:
  -arch -C --param -pedantic -pie -no-pie -pipe -s -traditional
-W[no-]... warnings:
  error                         stop after warning
  discarded-qualifiers          warn when const is dropped
  unsupported                   warn about ignored options
-f[no-]... flags:
  unsigned-char                 default char is unsigned
  signed-char                   default char is signed
  common                        use common section instead of bss
  ms-extensions                 allow anonymous struct in struct
  dollars-in-identifiers        allow '$' in C symbols
  gnu89-inline                  'extern inline' is like 'static inline'
Predefined macros:
  {name} -E -dM - < /dev/null
",
        bcc = env!("CARGO_PKG_VERSION"),
    )
}
