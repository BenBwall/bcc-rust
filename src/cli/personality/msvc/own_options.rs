//! clang-cl's own `cl` options that [`State::option`] does not translate.
//! clang-cl's driver takes every one of them, so none is an unknown
//! argument: bcc does what clang-cl does with each, or refuses it, and
//! never drops one that could change what a step writes.
//!
//! - **Unused.** clang-cl hands many of them to no job, and its driver warns
//!   about each argument no job claims (`argument unused during compilation:
//!   '/Bt'`, `Driver::BuildJobs`): Microsoft's options for C++/CLI, C++
//!   modules, browse information, timing and dependency reports, and retired
//!   code-generation switches. bcc warns as it does ([`State::report_unused`]).
//!   An option whose value may be the next argument is shown with its value
//!   apart (`/AIdir` as `'/AI dir'`), as the driver renders it.
//! - **Inert.** clang-cl takes them, and each changes nothing bcc writes; the
//!   match in [`State::own_option`] says why for each.
//! - **Applied.** `/FA` with any letters writes the listing, `/Brepro` asks the
//!   linker for a reproducible image (bcc's objects carry no time stamp either
//!   way), `/showFilenames` prints each source's name as it is compiled, and
//!   `/vlen`, and `/JMC` without debug information, warn as clang-cl does where
//!   they do nothing.
//! - **Ignored.** The precompiled header's (`/Yc`, `/Yu`) and `/JMC` with debug
//!   information change only how fast a build runs or how a debugger steps,
//!   never what the program does: bcc warns that it ignores them, as it does
//!   `/RTC1`, and compiles the source as it is.
//! - **Refused.** The rest change clang-cl's object, preprocessed text or link
//!   in ways bcc does not implement, and stop the command line with status 1:
//!   the directories of a Visual Studio installation (`/winsysroot`), whose
//!   headers and libraries clang-cl would search, among them. Those that only
//!   change the code (`/hotpatch`) are refused only where a step compiles.
//!
//! Of these options, the `cl` personality reads only those that `cl.exe`
//! itself takes without a word ([`CL_READ`]); `cl.exe` reports the others
//! itself, as unknown or as ignored.
//!
//! The kinds and names are those of the pinned Clang's option table
//! (`known::clang_cl_entry`); its behavior is clang-cl 23.1.5's
//! (`CLCompatOptions.td`, `MSVC.cpp`, `Clang.cpp`).

use std::ffi::OsStr;

use super::{
    Invocation,
    State,
    known,
    warning_setting,
};

impl State {
    /// Reads `text`, which is `option` with its `/` or `-`, if it is one of
    /// clang-cl's own options [`State::option`] does not translate, taking
    /// the value of one whose bare name takes the next argument from
    /// `rest`; for `cl` (not `clang_cl`), only one it refuses. Returns
    /// whether it was one.
    pub(super) fn own_option<'r>(
        &mut self,
        invocation: &Invocation,
        text: &str,
        option: &'r str,
        rest: &mut impl Iterator<Item = &'r OsStr>,
    ) -> bool {
        let Some((kind, name)) = known::clang_cl_entry(option) else {
            return false;
        };
        let clang_cl = invocation.personality.is_clang_cl();
        if !clang_cl && !CL_READ.contains(&name) {
            return false;
        }
        let value = &option[name.len()..];
        let mut shown = |state: &mut Self| state.shown(invocation, text, kind, name, value, rest);
        match name {
            // Unused: clang-cl's driver hands them to no job.
            | "AI"
            | "Bt"
            | "Bt+"
            | "F"
            | "FR"
            | "FU"
            | "Fm"
            | "Fr"
            | "Fx"
            | "G1"
            | "G2"
            | "GH"
            | "GT"
            | "GZ"
            | "Ge"
            | "Gh"
            | "Gm"
            | "Gm-"
            | "H"
            | "LN"
            | "MP"
            | "QIfist"
            | "Qfast_transcendentals"
            | "Qimprecise_fwaits"
            | "Qpar-report"
            | "Qsafe_fp_loads"
            | "Qvec-report"
            | "V"
            | "WL"
            | "Wp64"
            | "Yd"
            | "Yl"
            | "ZW"
            | "Zg"
            | "await"
            | "await:"
            | "clr"
            | "doc"
            | "dynamicdeopt"
            | "exportHeader"
            | "favor"
            | "fno-sanitize-address-vcasan-lib"
            | "fsanitize-address-use-after-return"
            | "headerName:"
            | "headerUnit"
            | "headerUnit:angle"
            | "headerUnit:quote"
            | "homeparams"
            | "reference"
            | "sourceDependencies"
            | "sourceDependencies:directives"
            | "translateInclude"
            | "u"
            | "vmm"
            | "vms"
            | "vmv" =>
                if let Some(shown) = shown(self) {
                    self.unused.push(shown);
                },
            // The standards bcc does not read, C++'s among them, are
            // `State::option`'s; clang-cl does not use another.
            | "std:" if !is_standard(value) => self.unused.push(text.to_owned()),
            // Inert. Full paths in diagnostics, which clang-cl ignores; C++
            // exception handling (`/GX` is `/EHsc`), which C does not use,
            // as for `/EH`; the vectorizer's permission and the processor to
            // tune for, which bcc's optimizer does not consult, as its
            // Clang personality ignores `-f[no-]vectorize` and `-mtune=`;
            // C++'s member-pointer layouts; the switches clang-cl ignores
            // (`/Zm`, `/Zo`, `/cgthreads`, `/errorReport`); `/Qgather-` and
            // `/Qscatter-`, which keep the code from AVX gathers and
            // scatters, which bcc never emits; and the default `/kernel-`.
            | "FC" | "GX" | "GX-" | "Qgather-" | "Qscatter-" | "Qvec" | "Qvec-" | "Zm" | "Zo"
            | "Zo-" | "cgthreads" | "errorReport" | "kernel-" | "tune:" | "vmb" | "vmg"
            // The hash of each file in the CodeView debug information; bcc
            // writes no file checksums at all.
            | "ZH:MD5" | "ZH:SHA1" | "ZH:SHA_256"
            // How `__regcall` passes arguments, a calling convention bcc
            // does not have: a program that uses it does not compile.
            | "Gregcall4" => {},
            // A warning setting clang-cl does not read.
            | "w" if option != "w" && warning_setting(option).is_none() => {},
            // C++'s virtual-base mode, a number.
            | "vd" =>
                if value.parse::<u32>().is_err() {
                    self.command_line_error(
                        invocation,
                        None,
                        format_args!(
                            "invalid integral value '{value}' in '-vtordisp-mode={value}'"
                        ),
                    );
                },
            // The versions clang-cl takes under `/winsysroot`, `/vctoolsdir`
            // and `/winsdkdir`, which bcc refuses: without them clang-cl
            // looks only in the registry, which bcc does not read either.
            | "vctoolsversion" | "winsdkversion" => drop(shown(self)),
            // A function no program defines, as the name is empty.
            | "funcoverride:" if value.is_empty() => {},
            // Applied.
            | "FA" => self.listing = true,
            | "Brepro" | "Brepro-" => self.reproducible = name == "Brepro",
            | "showFilenames" | "showFilenames-" => {
                self.show_filenames = name == "showFilenames";
                self.filename_options.push(text.to_owned());
            },
            | "vlen" => self.vector_length = None,
            | "vlen=256" | "vlen=512" => self.vector_length = Some(format!("/{name}")),
            | "JMC" => self.just_my_code = true,
            | "JMC-" => self.just_my_code = false,
            | "C" => self.keep_comments = Some(text.to_owned()),
            | "guard:" => self.guard(invocation, text, option, value),
            // Refused.
            | "arm64EC" => self.own_refused(invocation, text, "bcc generates x86-64 code only"),
            | "diasdkdir" | "vctoolsdir" | "winsdkdir" | "winsysroot" =>
                if let Some(shown) = shown(self) {
                    self.own_refused(
                        invocation,
                        &shown,
                        "bcc does not search a Visual Studio installation; set INCLUDE and LIB, \
                         or give the directories with /imsvc and /link /LIBPATH:",
                    );
                },
            | "fsanitize=address" =>
                self.own_refused(invocation, text, "bcc does not implement AddressSanitizer"),
            // Code generation's, which a step that writes no code does not
            // use (`State::check_own_options`).
            | "GA" => self.code_options.push((
                text.to_owned(),
                "bcc reads thread-local variables through _tls_index in every image",
            )),
            | "Gregcall" => self.code_options.push((
                text.to_owned(),
                "bcc's default calling convention is the x64 one, not __regcall",
            )),
            | "hotpatch" => self.code_options.push((
                text.to_owned(),
                "bcc does not make functions patchable or ask the linker to pad them",
            )),
            | "QIntel-jcc-erratum" => self.code_options.push((
                text.to_owned(),
                "bcc does not align jumps for the Intel JCC erratum",
            )),
            | "funcoverride:" => self.code_options.push((
                text.to_owned(),
                "bcc does not make functions loader-replaceable",
            )),
            // A precompiled header only saves the time of compiling the
            // headers before it again: the source still includes them, or
            // `/FI` does, so bcc compiles them as written.
            | "Yc" => super::ignored(invocation, option, "bcc does not build precompiled headers"),
            | "Yu" => super::ignored(invocation, option, "bcc does not use precompiled headers"),
            | "kernel" =>
                self.own_refused(invocation, text, "bcc does not compile for kernel mode"),
            | "showIncludes:user" => self.own_refused(
                invocation,
                text,
                "bcc's /showIncludes lists system headers too",
            ),
            | "pathmap:" => self.own_refused(
                invocation,
                text,
                "bcc does not remap file names in __FILE__ or debug information",
            ),
            | _ => return false,
        }
        true
    }

    /// `text`, clang-cl's option `name` of `kind` with the `value` joined to
    /// it, as its driver shows it: an option whose value may be the next
    /// argument (`separate`) with its value apart, taking it from `rest`
    /// where none is joined (`/AI dir`), and refusing it where none follows.
    fn shown<'r>(
        &mut self,
        invocation: &Invocation,
        text: &str,
        kind: &str,
        name: &str,
        value: &str,
        rest: &mut impl Iterator<Item = &'r OsStr>,
    ) -> Option<String> {
        if kind != "separate" {
            return Some(text.to_owned());
        }
        let value = if value.is_empty() {
            let Some(next) = rest.next() else {
                self.missing_values(invocation, text, 1);
                return None;
            };
            next.to_string_lossy().into_owned()
        } else {
            value.to_owned()
        };
        Some(format!("{}{name} {value}", &text[..1]))
    }

    /// Refuses `shown`, one of clang-cl's own options bcc does not
    /// implement, for `why`.
    fn own_refused(&mut self, invocation: &Invocation, shown: &str, why: &str) {
        self.command_line_error(
            invocation,
            None,
            format_args!("'{shown}' is not supported: {why}"),
        );
    }

    /// Reads clang-cl's `/guard:` with a `value` [`State::option`] does not
    /// spell as it does: clang-cl reads the value in any case, and refuses
    /// any but `cf`, `cf,nochecks`, `cf-`, `ehcont` and `ehcont-`.
    fn guard(&mut self, invocation: &Invocation, text: &str, option: &str, value: &str) {
        match value.to_ascii_lowercase().as_str() {
            | "cf" | "cf,nochecks" | "ehcont" =>
                super::ignored(invocation, option, "bcc does not emit Control Flow Guard"),
            | "cf-" | "ehcont-" => {},
            | _ => self.command_line_error(
                invocation,
                None,
                format_args!("invalid value '{value}' in '{}guard:'", &text[..1]),
            ),
        }
    }

    /// Checks clang-cl's own options that depend on the whole command line,
    /// once it is read: `/C` keeps comments in preprocessed text, which bcc
    /// does not, and is an error without `/E`, `/EP` or `/P`; `/JMC` with
    /// debug information (`debug_information`) adds calls that only a
    /// debugger's stepping reads, which bcc ignores with a warning where a
    /// step compiles, and otherwise warns as clang-cl does, as `/vlen=256`
    /// and `/vlen=512` do without AVX-512 (which bcc refuses), once for each
    /// source; `/showFilenames` is unused where nothing is compiled. The
    /// options of code generation bcc does not implement are refused where
    /// a step compiles.
    pub(super) fn check_own_options(&mut self, invocation: &Invocation, debug_information: bool) {
        if let Some(text) = self.keep_comments.take() {
            if self.preprocess.is_some() {
                self.own_refused(
                    invocation,
                    &text,
                    "bcc does not keep comments in preprocessed output",
                );
            } else {
                self.command_line_error(
                    invocation,
                    None,
                    format_args!("invalid argument '{text}' only allowed with '/E, /P or /EP'"),
                );
            }
        }
        let compiles = self.preprocess.is_none() && !self.syntax_only;
        if std::mem::take(&mut self.just_my_code) {
            if !debug_information {
                self.option_ignored.push(
                    "/JMC requires debug info. Use '/Zi', '/Z7' or debug options that enable \
                     debugger's stepping function; option ignored"
                        .into(),
                );
            } else if compiles {
                super::ignored(
                    invocation,
                    "JMC",
                    "bcc does not instrument functions for Just My Code debugging",
                );
            }
        }
        for (text, why) in std::mem::take(&mut self.code_options) {
            if compiles {
                self.own_refused(invocation, &text, why);
            }
        }
        if let Some(text) = self.vector_length.take() {
            self.option_ignored.push(format!(
                "invalid argument '{text}' not allowed with '/arch:SSE2'"
            ));
        }
        if !compiles {
            self.unused.append(&mut self.filename_options);
            self.show_filenames = false;
        }
    }

    /// Reports the arguments no job uses as clang-cl's driver does, after
    /// everything else and only if nothing failed, at the severity
    /// [`State::unused_severity`] says. Returns whether a fatal error
    /// stopped the command line.
    pub(super) fn report_unused(&mut self, invocation: &Invocation) -> bool {
        let unused = std::mem::take(&mut self.unused);
        if self.errors != 0 {
            return false;
        }
        let severity = self.unused_severity;
        for shown in unused {
            let message = format!("argument unused during compilation: '{shown}'");
            if self.driver_warning(
                invocation,
                severity,
                "unused-command-line-argument",
                &message,
            ) {
                return true;
            }
        }
        false
    }
}

/// The own options of clang-cl's that the `cl` personality reads too, as
/// `cl.exe` takes each without a word: the precompiled header's, which bcc
/// ignores with a warning, and the rest, which change `cl.exe`'s output in
/// ways bcc does not implement.
const CL_READ: &[&str] = &[
    "GA",
    "QIntel-jcc-erratum",
    "Yc",
    "Yu",
    "fsanitize=address",
    "kernel",
];

/// Whether `value` is a standard of `/std:` that [`State::option`] reads.
fn is_standard(value: &str) -> bool {
    matches!(value, "c11" | "c17" | "clatest") || value.starts_with("c++")
}
