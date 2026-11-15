/// Shared libraries from the native back end (`-fPIC -shared`): a library
/// is loaded with `dlopen` by a native executable, which calls into it
/// through a function pointer the library's data holds and writes a global
/// the library reads; and the definitions of an executable linked with a
/// library preempt the library's own, as the System V gABI's symbol lookup
/// orders them. LLD refuses text relocations unless asked, so each library
/// that links also has none.
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

    use crate::native::run_native;

    /// A library with every kind of reference shared-library code makes:
    /// default-visibility data and functions it defines (through the GOT
    /// and PLT), internal and hidden ones (PC-relative), a C library
    /// function, a function only the executable defines, address constants
    /// in writable and read-only data, and a thread-local counter.
    const LIBRARY: &str = r#"
unsigned long strlen(const char *);
int host_value(void);
int counter = 40;
static int total;
__attribute__((visibility("hidden"))) int twice(int x) { return x * 2; }
static int square(int x) { return x * x; }
const char *const names[] = {"zero", "one", "two"};
static __thread int calls;
int bump(int by) {
    calls++;
    total += twice(by) + square(by);
    counter += by;
    return counter + calls * 1000 + (int)strlen(names[by % 3]);
}
int (*bump_pointer)(int) = bump;
int (*const helpers[])(int) = {twice, square, bump};
int stats(void) { return total * 100 + helpers[0](3) + host_value(); }
"#;

    /// Loads the library named by its argument and calls through its data.
    const DLOPEN_MAIN: &str = r#"
void *dlopen(const char *, int);
void *dlsym(void *, const char *);
char *dlerror(void);
int dlclose(void *);
int printf(const char *, ...);
int host_value(void) { return 5000000; }
int main(int argc, char **argv) {
    void *library = dlopen(argv[1], 2);
    if (!library) {
        printf("%s\n", dlerror());
        return 2;
    }
    int (**bump)(int) = (int (**)(int))dlsym(library, "bump_pointer");
    int *counter = (int *)dlsym(library, "counter");
    int (*stats)(void) = (int (*)(void))dlsym(library, "stats");
    const char *const *names = (const char *const *)dlsym(library, "names");
    int first = (*bump)(1);
    int second = (*bump)(2);
    *counter += 100;
    int third = (*bump)(3);
    printf("%d %d %d %d %d %s\n", first, second, third, *counter, stats(), names[2]);
    return dlclose(library);
}
"#;

    const DLOPEN_OUTPUT: &str = "1044 2046 3150 146 5002606 two\n";

    /// A library whose own definitions an executable redefines. bcc's
    /// optimizer, like Clang's, may inline a preemptible definition into its
    /// callers in the library (GCC's `-fno-semantic-interposition`), so the
    /// function stays out of line here.
    const PREEMPTED: &str = "
int shared_value = 1;
__attribute__((noinline)) int shared_function(void) { return 10; }
static int internal_value = 2;
__attribute__((visibility(\"hidden\"))) int hidden_function(void) { return 20; }
int probe(void) {
    return shared_value * 1000 + shared_function() * 10 + internal_value + hidden_function();
}
";

    /// The executable's definitions, which the library's references reach.
    const PREEMPTING_MAIN: &str = "
int printf(const char *, ...);
int shared_value = 7;
int shared_function(void) { return 3; }
int internal_value = 50;
int hidden_function(void) { return 60; }
int probe(void);
int main(void) {
    printf(\"%d\\n\", probe());
    return 0;
}
";

    /// The executable's `shared_value` and `shared_function`, and the
    /// library's own internal and hidden symbols.
    const PREEMPTING_OUTPUT: &str = "7052\n";

    /// The native pipelines each library is built with: the listing
    /// assembled by the bundled Clang, or encoded by bcc, after either
    /// middle end.
    const ARMS: [&[&str]; 4] = [
        &["--backend=native"],
        &["--backend=native", "--native-object"],
        &["--backend=native", "--opt=bcc"],
        &["--backend=native", "--opt=bcc", "--native-object"],
    ];

    #[test]
    fn native_shared_libraries_load_link_and_are_preempted() {
        if !cfg!(all(
            target_os = "linux",
            target_env = "gnu",
            target_arch = "x86_64"
        )) {
            eprintln!("skipping: native shared libraries run on an x86-64 glibc Linux host");
            return;
        }
        if !clang_path().is_file() {
            eprintln!("skipping: no bundled clang");
            return;
        }
        let directory = temporary("libraries");
        std::fs::create_dir_all(&directory).unwrap();
        let host = directory.join("dlopen-host");
        build(
            DLOPEN_MAIN,
            &["--backend=native", "--opt=bcc", "-rdynamic"],
            &host,
        );
        let main = directory.join("preempting.c");
        std::fs::write(&main, PREEMPTING_MAIN).unwrap();
        for (index, arm) in ARMS.iter().enumerate() {
            for pic in ["-fPIC", "-fpic"] {
                let library = directory.join(format!("libloaded-{index}{pic}.so"));
                build(LIBRARY, &[*arm, &[pic, "-shared"]].concat(), &library);
                let ran = run_native(Command::new(&host).arg(&library)).unwrap();
                assert_eq!(
                    (ran.status.code(), text(&ran.stdout).as_str()),
                    (Some(0), DLOPEN_OUTPUT),
                    "{arm:?} {pic}: {}",
                    text(&ran.stderr)
                );
            }
            let library = directory.join(format!("libpreempted-{index}.so"));
            build(PREEMPTED, &[*arm, &["-fPIC", "-shared"]].concat(), &library);
            let executable = directory.join(format!("preempting-{index}"));
            let rpath = format!("-Wl,-rpath,{}", directory.display());
            let built = run_native(
                Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                    .args(*arm)
                    .arg(&main)
                    .arg(&library)
                    .arg(&rpath)
                    .arg("-o")
                    .arg(&executable)
                    .env("NO_COLOR", "1"),
            )
            .unwrap();
            assert!(built.status.success(), "{arm:?}: {}", text(&built.stderr));
            let ran = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(
                (ran.status.code(), text(&ran.stdout).as_str()),
                (Some(0), PREEMPTING_OUTPUT),
                "{arm:?}: {}",
                text(&ran.stderr)
            );
        }
        drop(std::fs::remove_dir_all(&directory));
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

    fn clang_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/llvm/bin/clang")
    }

    /// Output bytes as text.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        env::temp_dir().join(format!(
            "bcc-native-shared-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
