//! Which assembler each driver personality runs for assembly inputs, and
//! the assembler options and probes build systems rely on. GCC's driver
//! runs the system's `as` (GCC manual, "Overall Options", "Assembler
//! Options"), so `--gcc-mode` does when one is found; Clang's integrated
//! assembler rejects `-Wa,-v` and answers `-Wa,--version` itself; TCC
//! ignores `-Wa,` options. OpenSSL's perlasm (`x86_64-xlate.pl`) runs
//! `$CC -Wa,-v -c -o /dev/null -x assembler /dev/null` and then `$CC
//! --version` to choose between GNU and Solaris section syntax.
//!
//! The GNU `as` cases compare with the host's `as` itself and are skipped
//! where it is missing; the expected messages of the other compilers are
//! those of GCC 13.3, Clang 23.1.1 and TCC 0.9.28rc on x86-64 Linux.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    /// The probe OpenSSL's perlasm runs for a GNU assembler, which names
    /// `/dev/null` for its input and output; an empty file in the test's
    /// directory stands for it, so the probe also runs on Windows.
    const PROBE: [&str; 7] = [
        "-Wa,-v",
        "-c",
        "-o",
        "probe.o",
        "-x",
        "assembler",
        "empty.s",
    ];

    /// The CET note perlasm writes when it takes the assembler for GNU's
    /// (`gnu.s`) and otherwise (`solaris.s`).
    const GNU_SECTION: &str = ".section \".note.gnu.property\", \"a\"\n.long 1\n";
    const SOLARIS_SECTION: &str = ".section .note.gnu.property, #alloc\n.long 1\n";

    /// The note `--gcc-mode` prints when it finds no `as`.
    const NO_ASSEMBLER: &str = "gcc: note: no system assembler `as` for x86_64-unknown-linux-gnu \
                                found; assembling with the bundled clang's integrated assembler, \
                                which rejects GNU as options such as -Wa,-v\n";

    /// A fresh working directory, removed afterwards.
    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-assembler-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("empty.s"), "").unwrap();
            Self(directory)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Runs bcc in this directory with `mode` first, unless it is empty,
        /// and the environment variables `variables` set.
        fn bcc(&self, mode: &str, args: &[&str], variables: &[(&str, &str)]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args((!mode.is_empty()).then_some(mode))
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("COMPILER_PATH")
                .envs(variables.iter().copied())
                .output()
                .unwrap()
        }

        /// A directory, made here, that holds no `as`, as `PATH`.
        fn empty_path(&self) -> String {
            let empty = self.path("empty");
            std::fs::create_dir_all(&empty).unwrap();
            empty.to_str().unwrap().to_owned()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
    }

    /// Whether bcc looks for the system assembler on this host: an x86-64
    /// Linux one, compiling for itself.
    fn host_looks_for_as() -> bool {
        cfg!(all(target_os = "linux", target_arch = "x86_64"))
    }

    /// The host's GNU `as`, run with `args`, when `PATH` has one.
    fn gnu_as(args: &[&str]) -> Option<Output> {
        let version = Command::new("as").arg("--version").output().ok()?;
        String::from_utf8_lossy(&version.stdout)
            .starts_with("GNU assembler")
            .then(|| Command::new("as").args(args).output().unwrap())
    }

    /// `--gcc-mode` runs the system's GNU `as` as GCC does, so the probe
    /// and `-Wa,--version` print what that `as` prints, and the Solaris
    /// syntax perlasm writes for other assemblers assembles too.
    #[test]
    fn gcc_mode_runs_the_gnu_assembler_gcc_runs() {
        if !host_looks_for_as() {
            return;
        }
        let Some(version) = gnu_as(&["-v", "--64", "-o", "/dev/null", "/dev/null"]) else {
            eprintln!("skipped: no GNU as on PATH");
            return;
        };
        let directory = Directory::new();
        let probe = directory.bcc("--gcc-mode", &PROBE, &[]);
        assert!(probe.status.success(), "{}", stderr(&probe));
        assert_eq!(stderr(&probe), stderr(&version));
        assert!(stderr(&probe).starts_with("GNU assembler version "));
        let banner = gnu_as(&["--version"]).unwrap();
        let answered = directory.bcc(
            "--gcc-mode",
            &[
                "-Wa,--version",
                "-c",
                "-o",
                "probe.o",
                "-x",
                "assembler",
                "empty.s",
            ],
            &[],
        );
        assert!(answered.status.success(), "{}", stderr(&answered));
        assert_eq!(stdout(&answered), stdout(&banner));
        for (name, text) in [("gnu.s", GNU_SECTION), ("solaris.s", SOLARIS_SECTION)] {
            directory.write(name, text);
            let assembled = directory.bcc("--gcc-mode", &["-c", name, "-o", "x.o"], &[]);
            assert!(assembled.status.success(), "{name}: {}", stderr(&assembled));
            assert_eq!(stderr(&assembled), "", "{name}");
        }
    }

    /// The GNU options GCC passes to `as` reach it, from `-Wa,` and
    /// `-Xassembler`, also for a `.S` input the preprocessor reads first.
    #[test]
    fn gcc_mode_passes_assembler_options_to_the_gnu_assembler() {
        if !host_looks_for_as() || gnu_as(&["--version"]).is_none() {
            return;
        }
        let directory = Directory::new();
        directory.write("f.S", "#ifdef X\n.globl f\nf:\n ret\n#endif\n");
        let listed = directory.bcc(
            "--gcc-mode",
            &["-DX", "-Wa,-adhln=f.lst,--noexecstack", "-c", "f.S"],
            &[],
        );
        assert!(listed.status.success(), "{}", stderr(&listed));
        let listing = std::fs::read_to_string(directory.path("f.lst")).unwrap();
        assert!(listing.contains(".globl f"), "{listing}");
        assert!(directory.path("f.o").is_file());
        let listed = directory.bcc(
            "--gcc-mode",
            &["-Xassembler", "-al=g.lst", "-g", "-c", "f.S", "-o", "g.o"],
            &[],
        );
        assert!(listed.status.success(), "{}", stderr(&listed));
        assert!(directory.path("g.lst").is_file());
        // An option only GNU as knows is GNU as's error, as with GCC.
        let refused = directory.bcc("--gcc-mode", &["-Wa,--bogus", "-c", "f.S"], &[]);
        assert_eq!(refused.status.code(), Some(1));
        assert!(
            stderr(&refused).contains("unrecognized option '--bogus'"),
            "{}",
            stderr(&refused)
        );
    }

    /// Without an `as`, `--gcc-mode` assembles with the integrated
    /// assembler and says so, once; the probe then fails, as Clang's does.
    #[test]
    fn gcc_mode_without_an_assembler_notes_the_integrated_one() {
        if !host_looks_for_as() {
            return;
        }
        let directory = Directory::new();
        let path = directory.empty_path();
        directory.write("gnu.s", GNU_SECTION);
        directory.write("other.s", GNU_SECTION);
        let assembled = directory.bcc(
            "--gcc-mode",
            &["-c", "gnu.s", "other.s"],
            &[("PATH", &path)],
        );
        assert!(assembled.status.success(), "{}", stderr(&assembled));
        assert_eq!(stderr(&assembled), NO_ASSEMBLER);
        let probe = directory.bcc("--gcc-mode", &PROBE, &[("PATH", &path)]);
        assert_eq!(probe.status.code(), Some(1));
        assert_eq!(
            stderr(&probe),
            format!(
                "{NO_ASSEMBLER}gcc: error: cannot assemble `empty.s`: clang failed (exit status: \
                 1):\nclang: error: unsupported argument '-v' to option '-Wa,'\n\n"
            )
        );
        directory.write("solaris.s", SOLARIS_SECTION);
        let solaris = directory.bcc("--gcc-mode", &["-c", "solaris.s"], &[("PATH", &path)]);
        assert_eq!(solaris.status.code(), Some(1));
        assert!(stderr(&solaris).contains("error: expected string"));
    }

    /// GCC manual, "Directory Options": the driver looks for `as` as each
    /// `-B` prefix followed by the name, then in `COMPILER_PATH`, then in
    /// `PATH`; a prefix that names a directory gets its separator.
    #[test]
    #[cfg(unix)]
    fn gcc_mode_finds_the_assembler_through_b_prefixes_and_compiler_path() {
        use std::os::unix::fs::PermissionsExt;
        if !host_looks_for_as() {
            return;
        }
        let directory = Directory::new();
        let path = directory.empty_path();
        directory.write("f.s", GNU_SECTION);
        // Each `as` records which one ran and its arguments.
        for name in ["prefix/x-as", "dir/as", "compiler/as"] {
            let program = directory.path(name);
            std::fs::create_dir_all(program.parent().unwrap()).unwrap();
            let log = directory.path("ran.txt");
            std::fs::write(
                &program,
                format!("#!/bin/sh\necho {name} \"$@\" > '{}'\n", log.display()),
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let compiler = directory.path("compiler");
        let compiler = compiler.to_str().unwrap();
        let ran = |args: &[&str], variables: &[(&str, &str)]| {
            drop(std::fs::remove_file(directory.path("ran.txt")));
            let output = directory.bcc("--gcc-mode", args, variables);
            assert!(output.status.success(), "{args:?}: {}", stderr(&output));
            std::fs::read_to_string(directory.path("ran.txt")).unwrap_or_default()
        };
        let prefix = "-Bprefix/x-";
        assert_eq!(
            ran(
                &[prefix, "-c", "f.s"],
                &[("PATH", &path), ("COMPILER_PATH", compiler)]
            ),
            "prefix/x-as --64 -o f.o f.s\n"
        );
        assert_eq!(
            ran(&["-B", "dir", "-c", "f.s"], &[("PATH", &path)]),
            "dir/as --64 -o f.o f.s\n"
        );
        assert_eq!(
            ran(
                &["-c", "f.s"],
                &[("PATH", &path), ("COMPILER_PATH", compiler)]
            ),
            "compiler/as --64 -o f.o f.s\n"
        );
        // COMPILER_PATH comes before PATH.
        let dir = directory.path("dir");
        assert_eq!(
            ran(
                &["-c", "f.s"],
                &[("PATH", dir.to_str().unwrap()), ("COMPILER_PATH", compiler)]
            ),
            "compiler/as --64 -o f.o f.s\n"
        );
        let warned = directory.bcc("--gcc-mode", &[prefix, "-c", "f.s"], &[("PATH", &path)]);
        assert_eq!(
            stderr(&warned),
            "gcc: warning: '-B prefix/x-' finds only the system assembler: bcc links with its \
             bundled linker\n"
        );
    }

    /// Clang's integrated assembler rejects `-Wa,-v` and answers
    /// `-Wa,--version` with Clang's banner, and perlasm then finds `clang`
    /// in `--version`; `-fno-integrated-as` runs the system's `as`.
    #[test]
    fn clang_mode_keeps_the_integrated_assembler_unless_told_otherwise() {
        let directory = Directory::new();
        let probe = directory.bcc("--clang-mode", &PROBE, &[]);
        assert_eq!(probe.status.code(), Some(1));
        assert!(
            stderr(&probe).contains("clang: error: unsupported argument '-v' to option '-Wa,'\n"),
            "{}",
            stderr(&probe)
        );
        let answered = directory.bcc(
            "--clang-mode",
            &[
                "-Wa,--version",
                "-c",
                "-o",
                "probe.o",
                "-x",
                "assembler",
                "empty.s",
            ],
            &[],
        );
        assert!(answered.status.success(), "{}", stderr(&answered));
        assert!(
            stdout(&answered).starts_with("clang version 23.1.1 "),
            "{}",
            stdout(&answered)
        );
        let version = directory.bcc("--clang-mode", &["--version"], &[]);
        assert!(stdout(&version).starts_with("clang version "));
        directory.write("solaris.s", SOLARIS_SECTION);
        let solaris = directory.bcc("--clang-mode", &["-c", "solaris.s"], &[]);
        assert_eq!(solaris.status.code(), Some(1));
        assert!(stderr(&solaris).contains("solaris.s:1:30: error: expected string"));
        if !host_looks_for_as() {
            return;
        }
        if let Some(version) = gnu_as(&["-v", "--64", "-o", "/dev/null", "/dev/null"]) {
            let mut external = vec!["-fno-integrated-as"];
            external.extend(PROBE);
            let probe = directory.bcc("--clang-mode", &external, &[]);
            assert!(probe.status.success(), "{}", stderr(&probe));
            assert_eq!(stderr(&probe), stderr(&version));
            // The last of the two wins.
            external.insert(1, "-fintegrated-as");
            assert_eq!(
                directory.bcc("--clang-mode", &external, &[]).status.code(),
                Some(1)
            );
        }
        let path = directory.empty_path();
        directory.write("gnu.s", GNU_SECTION);
        let missing = directory.bcc(
            "--clang-mode",
            &["-fno-integrated-as", "-c", "gnu.s"],
            &[("PATH", &path)],
        );
        assert_eq!(missing.status.code(), Some(1));
        assert_eq!(
            stderr(&missing),
            "clang: error: cannot assemble `gnu.s`: no system assembler `as` for \
             x86_64-unknown-linux-gnu in the -B prefixes, COMPILER_PATH or PATH\n"
        );
    }

    /// TCC ignores `-Wa,` options, warning only under `-Wunsupported`, so
    /// the probe prints nothing, and `--version` names no `clang`.
    #[test]
    fn tcc_mode_ignores_assembler_options_as_tcc_does() {
        let directory = Directory::new();
        let probe = directory.bcc("--tcc-mode", &PROBE, &[]);
        assert!(probe.status.success(), "{}", stderr(&probe));
        assert_eq!(
            (stdout(&probe), stderr(&probe)),
            (String::new(), String::new())
        );
        let mut warned = vec!["-Wunsupported"];
        warned.extend(PROBE);
        let warned = directory.bcc("--tcc-mode", &warned, &[]);
        assert!(warned.status.success());
        assert_eq!(
            stderr(&warned),
            "tcc: warning: unsupported option '-Wa,-v'\n"
        );
        let version = directory.bcc("--tcc-mode", &["--version"], &[]);
        assert!(!stdout(&version).contains("clang"), "{}", stdout(&version));
    }

    /// bcc's own command line keeps the integrated assembler, and takes
    /// Clang's `-fno-integrated-as` for the system one.
    #[test]
    fn bcc_keeps_the_integrated_assembler_by_default() {
        let directory = Directory::new();
        let probe = directory.bcc("", &PROBE, &[]);
        assert_eq!(probe.status.code(), Some(1));
        if !host_looks_for_as() {
            return;
        }
        if let Some(version) = gnu_as(&["-v", "--64", "-o", "/dev/null", "/dev/null"]) {
            for option in [
                "-fno-integrated-as",
                "--assembler=system",
                "--assembler=auto",
            ] {
                let mut external = vec![option];
                external.extend(PROBE);
                let probe = directory.bcc("", &external, &[]);
                assert!(probe.status.success(), "{option}: {}", stderr(&probe));
                assert_eq!(stderr(&probe), stderr(&version), "{option}");
            }
        }
    }
}
