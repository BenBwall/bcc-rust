#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Integration tests exercise the compiled CLI."
)]

#[path = "support/native.rs"]
mod native;

/// The relocation models (`-fpic`, `-fPIC`, `-fpie`, `-fPIE`, `-fno-pic`),
/// `-ftls-model` and the `-shared`, `-pie` and `-no-pie` link kinds, with
/// thread-local storage (issue #65). Thread-local objects of shared
/// libraries bcc builds are reached from several threads, from the
/// executable that links them and through `dlopen`, and executables keep
/// their models. C11: §6.2.4 paragraph 4, p. 38; PDF p. 56.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Test harnesses own process output and file paths outside compilation."
)]
mod tests {
    use std::{
        env,
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    use super::native::run_native;

    /// A library's thread-local objects: an exported definition, an
    /// internal and a hidden one (which LLVM reaches with local-dynamic in
    /// `-fPIC` code), and a weak definition. Each thread counts in its own
    /// copies from the initial values.
    const LIBRARY: &str = r#"
__thread int counter = 100;
static __thread long scale = 3;
__attribute__((visibility("hidden"))) __thread int calls;
__attribute__((weak)) __thread int weak_total = 1;
int *lib_counter(void) { return &counter; }
int lib_record(int index) {
    for (int i = 0; i <= index * 100; i++) {
        counter++;
        calls++;
    }
    int *mine = &counter;
    *mine += (int)scale;
    weak_total += index;
    return counter * 100 + (calls == index * 100 + 1) * 10 + weak_total;
}
"#;

    /// The library also reads a thread-local object the executable defines,
    /// which only the general-dynamic and initial-exec models reach.
    const LIBRARY_FROM_MAIN: &str = r"
extern __thread int from_main;
int lib_from_main(void) { return from_main; }
";

    const PTHREADS: &str = "
typedef unsigned long pthread_t;
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int pthread_join(pthread_t, void **);
int printf(const char *, ...);
";

    /// Links the library at startup: four threads reach the library's
    /// `counter` through `extern` in the executable and through the
    /// library's function, and the library reads the executable's
    /// `from_main`, each thread its own copies. The main thread's copies keep
    /// their own values.
    const LINKED_MAIN: &str = r#"
extern __thread int counter;
__thread int from_main = 9;
int *lib_counter(void);
int lib_record(int);
int lib_from_main(void);
int results[4];
int same[4];
static void *worker(void *arg) {
    int index = (int)(long)arg;
    from_main += index;
    results[index] = lib_record(index);
    same[index] = lib_counter() == &counter && lib_from_main() == 9 + index;
    return 0;
}
int main(void) {
    pthread_t threads[4];
    counter = 5;
    for (int i = 0; i < 4; i++)
        pthread_create(&threads[i], 0, worker, (void *)(long)i);
    for (int i = 0; i < 4; i++)
        pthread_join(threads[i], 0);
    for (int i = 0; i < 4; i++)
        printf("thread %d: %d %d\n", i, results[i], same[i]);
    printf("main: %d %d %d\n", counter, from_main, lib_counter() == &counter);
    return 0;
}
"#;

    const LINKED_OUTPUT: &str =
        "thread 0: 10411 1\nthread 1: 20412 1\nthread 2: 30413 1\nthread 3: 40414 1\nmain: 5 9 1\n";

    /// Loads the library with `dlopen`, which static TLS models cannot serve
    /// in general, and runs it on four threads; each thread's `counter` has
    /// its own address.
    const DLOPEN_MAIN: &str = r#"
void *dlopen(const char *, int);
void *dlsym(void *, const char *);
char *dlerror(void);
static int (*record)(int);
static int *(*address)(void);
int results[4];
int *addresses[4];
static void *worker(void *arg) {
    int index = (int)(long)arg;
    results[index] = record(index);
    addresses[index] = address();
    return 0;
}
int main(int argc, char **argv) {
    void *library = dlopen(argv[1], 2);
    if (!library) {
        printf("%s\n", dlerror());
        return 2;
    }
    record = (int (*)(int))dlsym(library, "lib_record");
    address = (int *(*)(void))dlsym(library, "lib_counter");
    pthread_t threads[4];
    for (int i = 0; i < 4; i++)
        pthread_create(&threads[i], 0, worker, (void *)(long)i);
    for (int i = 0; i < 4; i++)
        pthread_join(threads[i], 0);
    int distinct = 1;
    for (int i = 0; i < 4; i++)
        for (int j = 0; j < 4; j++)
            distinct &= i == j || addresses[i] != addresses[j];
    for (int i = 0; i < 4; i++)
        printf("thread %d: %d\n", i, results[i]);
    printf("main: %d %d\n", *address(), distinct && address() != addresses[0]);
    return 0;
}
"#;

    const DLOPEN_OUTPUT: &str =
        "thread 0: 10411\nthread 1: 20412\nthread 2: 30413\nthread 3: 40414\nmain: 100 1\n";

    /// The bcc pipelines a library is built with.
    const ARMS: [&[&str]; 2] = [&["--llvm-opt=O0"], &["--opt=bcc", "--llvm-opt=O2"]];

    /// The position-independent models a shared library can be built with,
    /// and whether it may share thread-local objects with the executable:
    /// `-ftls-model=local-dynamic` promises that every thread-local object a
    /// module reaches is its own, so only the `dlopen` library, whose
    /// objects no other module names, may use it.
    const LIBRARY_MODELS: [(&[&str], bool); 3] = [
        (&["-fPIC"], true),
        (&["-fpic"], true),
        (&["-fPIC", "-ftls-model=local-dynamic"], false),
    ];

    #[test]
    fn shared_libraries_keep_per_thread_copies_when_linked_and_loaded() {
        let Some(host) = linux_host() else { return };
        let directory = temporary("shared");
        std::fs::create_dir_all(&directory).unwrap();
        let linked_library = format!("{LIBRARY}{LIBRARY_FROM_MAIN}");
        let linked_main = format!("{PTHREADS}{LINKED_MAIN}");
        let dlopen_main = format!("{PTHREADS}{DLOPEN_MAIN}");
        // The executables, from bcc's LLVM and native back ends and Clang.
        let mut mains = Vec::new();
        for (name, flags) in [
            ("llvm", &["--llvm-opt=O0"][..]),
            ("native", &["--backend=native"][..]),
        ] {
            let object = directory.join(format!("main-{name}.o"));
            build(&linked_main, &[flags, &["-c"]].concat(), &object);
            mains.push((name, object));
        }
        let clang_main = directory.join("main-clang.o");
        clang(&linked_main, &["-c", "-O2"], &directory, &clang_main);
        mains.push(("clang", clang_main));
        let dlopen_host = directory.join("dlopen-host");
        build(&dlopen_main, &["--llvm-opt=O0"], &dlopen_host);
        let mut libraries = Vec::new();
        for (index, arm) in ARMS.iter().enumerate() {
            for (model_index, (model, shared)) in LIBRARY_MODELS.iter().enumerate() {
                let flags = [*arm, *model, &["-shared"]].concat();
                let linked = shared.then(|| {
                    let linked = directory.join(format!("liblinked-{index}-{model_index}.so"));
                    build(&linked_library, &flags, &linked);
                    linked
                });
                let loaded = directory.join(format!("libloaded-{index}-{model_index}.so"));
                build(LIBRARY, &flags, &loaded);
                libraries.push((format!("{arm:?} {model:?}"), linked, Some(loaded)));
            }
        }
        // Initial-exec code can go into a library loaded at startup.
        let initial_exec = directory.join("liblinked-initial-exec.so");
        build(
            &linked_library,
            &["-fPIC", "-ftls-model=initial-exec", "-shared"],
            &initial_exec,
        );
        libraries.push(("initial-exec".to_owned(), Some(initial_exec), None));
        // A library from Clang links with bcc's executables in turn.
        let clang_library = directory.join("liblinked-clang.so");
        clang(
            &linked_library,
            &["-fPIC", "-shared", "-O2"],
            &directory,
            &clang_library,
        );
        libraries.push(("clang".to_owned(), Some(clang_library), None));
        for (library_name, library, loaded) in &libraries {
            for (main_name, object) in library.iter().flat_map(|_| &mains) {
                let library = library.as_ref().unwrap();
                let executable = directory.join(format!(
                    "linked-{main_name}-{}",
                    library.file_stem().unwrap().to_string_lossy()
                ));
                link(host, object, library, &executable);
                let ran = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(
                    (ran.status.code(), text(&ran.stdout).as_str()),
                    (Some(0), LINKED_OUTPUT),
                    "{library_name} with {main_name}: {}",
                    text(&ran.stderr)
                );
            }
            if let Some(loaded) = loaded {
                let ran = run_native(Command::new(&dlopen_host).arg(loaded)).unwrap();
                assert_eq!(
                    (ran.status.code(), text(&ran.stdout).as_str()),
                    (Some(0), DLOPEN_OUTPUT),
                    "dlopen {library_name}: {}",
                    text(&ran.stderr)
                );
            }
        }
        drop(std::fs::remove_dir_all(&directory));
    }

    /// One thread-local definition and one declaration, run as an
    /// executable.
    const EXECUTABLE: &str = r#"
int printf(const char *, ...);
__thread int seed = 40;
static __thread long scale = 2;
extern __thread int other;
__thread int other = 3;
int main(void) {
    seed += (int)scale;
    printf("%d %d\n", seed, other);
    return seed + other - 45;
}
"#;

    /// Executables keep working with each relocation model and link kind,
    /// from both back ends: position-independent ones by default, with
    /// `-fPIE` or `-pie`, and fixed-address ones with `-fno-pic -no-pie`.
    #[test]
    fn executables_run_with_each_relocation_model_and_link_kind() {
        if linux_host().is_none() {
            return;
        }
        let directory = temporary("executables");
        std::fs::create_dir_all(&directory).unwrap();
        for backend in ["--backend=llvm", "--backend=native"] {
            for (index, flags) in [
                &[][..],
                &["-fPIE", "-pie"],
                &["-fpie"],
                &["-fno-pic", "-no-pie"],
                &["-ftls-model=local-exec"],
                &["-ftls-model=initial-exec"],
            ]
            .into_iter()
            .enumerate()
            {
                let executable = directory.join(format!("{}-{index}", &backend[10..]));
                build(EXECUTABLE, &[&[backend][..], flags].concat(), &executable);
                let ran = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(
                    (ran.status.code(), text(&ran.stdout).as_str()),
                    (Some(0), "42 3\n"),
                    "{backend} {flags:?}"
                );
            }
        }
        drop(std::fs::remove_dir_all(&directory));
    }

    /// The native back end generates executable code only: it refuses
    /// `-fPIC` and `-shared`, reaches declarations through the GOT's
    /// thread-pointer offset (initial-exec), and honours
    /// `-ftls-model=local-exec` for them as LLVM does.
    #[test]
    fn native_back_end_refuses_pic_and_honours_local_exec() {
        let source = "extern __thread int other; int main(void) { return other; }";
        for flags in [&["-fPIC"][..], &["-fpic"], &["-fPIE", "-fPIC"]] {
            let output = bcc(
                source,
                &[&["--backend=native", "--emit=asm"][..], flags].concat(),
            );
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert!(
                text(&output.stderr).contains(
                    "the native back end generates code for executables only; -fpic, -fPIC and \
                     -shared need --backend=llvm"
                ),
                "{flags:?}: {}",
                text(&output.stderr)
            );
        }
        // `-shared` is refused with the default relocation model too, before
        // anything is written.
        let library = temporary("native-shared.so");
        for flags in [&[][..], &["-fPIE"], &["-fno-pic"]] {
            let output = bcc(
                "__thread int counter; int get(void) { return counter; }",
                &[
                    &[
                        "--backend=native",
                        "-shared",
                        "-o",
                        library.to_str().unwrap(),
                    ][..],
                    flags,
                ]
                .concat(),
            );
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert!(
                text(&output.stderr).contains(
                    "the native back end generates code for executables only; -fpic, -fPIC and \
                     -shared need --backend=llvm"
                ),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            assert!(!library.exists(), "{flags:?}");
        }
        for (flags, access) in [
            (&[][..], "other@GOTTPOFF(%rip)"),
            (&["-fno-pic"], "other@GOTTPOFF(%rip)"),
            (&["-fPIC", "-fPIE"], "other@GOTTPOFF(%rip)"),
            (&["-ftls-model=initial-exec"], "other@GOTTPOFF(%rip)"),
            (&["-ftls-model=local-exec"], "other@TPOFF("),
        ] {
            let output = bcc(
                source,
                &[&["--backend=native", "--emit=asm"][..], flags].concat(),
            );
            assert!(output.status.success(), "{}", text(&output.stderr));
            assert!(
                text(&output.stdout).contains(access),
                "{flags:?}: {}",
                text(&output.stdout)
            );
        }
    }

    /// `__PIC__`, `__pic__`, `__PIE__` and `__pie__` follow the last of the
    /// `-fpic` family as Clang's do on every target, and the MSVC target
    /// rejects position independence as Clang does.
    #[test]
    fn relocation_macros_match_clang_for_each_flag_and_target() {
        let probe = "#ifndef __PIC__\n#define __PIC__ 0\n#endif\n#ifndef __pic__\n#define __pic__ \
                     0\n#endif\n#ifndef __PIE__\n#define __PIE__ 0\n#endif\n#ifndef \
                     __pie__\n#define __pie__ 0\n#endif\nint main(void) { return __PIC__ + 3 * \
                     __pic__ + 9 * __PIE__ + 27 * __pie__; }\n";
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            let target = format!("--target={target}");
            for flags in [
                &[][..],
                &["-fPIC"],
                &["-fpic"],
                &["-fPIE"],
                &["-fpie"],
                &["-fno-pic"],
                &["-fno-PIE"],
                &["-fPIC", "-fno-pie"],
                &["-fpie", "-fPIC"],
                &["-fno-PIC", "-fpie"],
            ] {
                let dump = run_native(
                    Command::new(clang_path())
                        .arg(&target)
                        .args(flags)
                        .args(["-dM", "-E", "-x", "c", "-"]),
                )
                .unwrap();
                let flags = [&[target.as_str()][..], flags].concat();
                let actual = bcc(probe, &[&flags[..], &["--interpret"]].concat());
                if !dump.status.success() {
                    // Clang: "clang: error: unsupported option '-fPIC' for
                    // target 'x86_64-pc-windows-msvc'".
                    let message = text(&dump.stderr);
                    let message = message.trim().trim_start_matches("clang: ");
                    assert_eq!(actual.status.code(), Some(1), "{flags:?}");
                    assert_eq!(text(&actual.stderr).trim(), message, "{flags:?}");
                    continue;
                }
                let value = |name: &str| {
                    text(&dump.stdout)
                        .lines()
                        .find_map(|line| line.strip_prefix(&format!("#define {name} ")))
                        .map_or(0, |value| value.trim().parse::<i32>().unwrap())
                };
                let expected = value("__PIC__")
                    + 3 * value("__pic__")
                    + 9 * value("__PIE__")
                    + 27 * value("__pie__");
                assert_eq!(
                    actual.status.code(),
                    Some(expected),
                    "{flags:?}: {}",
                    text(&actual.stderr)
                );
            }
        }
    }

    #[test]
    fn invalid_tls_models_and_link_kinds_are_rejected() {
        let output = bcc("int x;", &["-ftls-model=bogus", "--emit=ir"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            text(&output.stderr).contains("invalid value 'bogus' in '-ftls-model=bogus'"),
            "{}",
            text(&output.stderr)
        );
        for flags in [&["-shared"][..], &["-shared", "-c", "-o", "x.o"]] {
            let output = bcc("int x;", flags);
            assert_eq!(output.status.code(), Some(2), "{flags:?}");
        }
    }

    /// Runs the compiler on the inline `source` with `flags`.
    fn bcc(source: &str, flags: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(flags)
                .args(["--input", source])
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE"),
        )
        .unwrap()
    }

    /// Builds `output` from `source` with bcc and `flags`.
    fn build(source: &str, flags: &[&str], output: &Path) {
        let built = bcc(source, &[flags, &["-o", output.to_str().unwrap()]].concat());
        assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
    }

    /// Compiles `source` with the bundled Clang and `flags` into `output`.
    fn clang(source: &str, flags: &[&str], directory: &Path, output: &Path) {
        let input = directory.join(format!(
            "{}.c",
            output.file_name().unwrap().to_string_lossy()
        ));
        std::fs::write(&input, source).unwrap();
        let built = run_native(
            Command::new(clang_path())
                .args(flags)
                .arg(&input)
                .arg("-o")
                .arg(output),
        )
        .unwrap();
        assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
    }

    /// Links `object` with the shared `library` (found beside it at run
    /// time) and pthreads into `executable`, with the bundled Clang and LLD.
    fn link(host: &str, object: &Path, library: &Path, executable: &Path) {
        let directory = library.parent().unwrap();
        let linked = run_native(
            Command::new(clang_path())
                .args([&format!("--target={host}"), "-fuse-ld=lld", "-pthread"])
                .arg(object)
                .arg(library)
                .arg(format!("-Wl,-rpath,{}", directory.display()))
                .arg("-o")
                .arg(executable),
        )
        .unwrap();
        assert!(linked.status.success(), "{}", text(&linked.stderr));
    }

    fn clang_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        })
    }

    /// The host triple, when it is the glibc Linux target whose shared
    /// libraries, pthreads and `dlopen` these tests run.
    fn linux_host() -> Option<&'static str> {
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            eprintln!("skipping: shared-library execution needs a glibc Linux host");
            return None;
        }
        if !clang_path().is_file() {
            eprintln!("skipping: no bundled clang");
            return None;
        }
        Some("x86_64-unknown-linux-gnu")
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        env::temp_dir().join(format!(
            "bcc-tls-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
