//! Thread storage duration end to end (issue #61). `_Thread_local`, C23
//! `thread_local` and GNU `__thread` objects lower to thread-local globals.
//! The interpreter runs one thread, so there each behaves as an ordinary
//! global; the native arms and Clang also run programs whose threads keep
//! their own copies: on Windows through `CreateThread`, and on Linux through
//! pthreads under WSL, linked by the distribution's C compiler.
//! C11: §6.2.4 paragraph 4, p. 38; PDF p. 56.

use super::*;
use crate::configuration::{
    CStandard,
    ExtensionPolicy,
};

/// Every kind of thread-local declaration, used from one thread.
const SINGLE: &str = r#"
int printf(const char *, ...);
_Thread_local int counter = 40;
__thread long scale = 2;
static _Thread_local struct { int a; char name[4]; } record = {7, "tls"};
extern _Thread_local int shared;
_Thread_local int shared = 5;
const _Thread_local int fixed = 9;
_Thread_local int zeroed[3];
static int bump(void) { static __thread int calls; return ++calls; }
int *where(void) { return &counter; }
int main(void) {
    extern __thread long scale;
    bump();
    bump();
    *where() += 2;
    record.a += bump();
    int *p = &shared;
    *p *= 3;
    zeroed[1] = (int)scale;
    printf("%d %ld %d %s %d %d %d\n", counter, scale, record.a, record.name, shared, fixed,
           zeroed[0] + zeroed[1] + zeroed[2]);
    return counter + record.a + shared + fixed - 70;
}
"#;

const SINGLE_OUTPUT: (i32, &str) = (6, "42 2 10 tls 15 9 2\n");

/// C23 spells the specifier `thread_local`.
const C23: &str = "
static thread_local int depth = 1;
thread_local int total;
int main(void) { static thread_local int calls = 2; total = depth + calls; return total; }
";

#[test]
fn thread_storage_interprets_on_every_target_and_stage() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_module(SINGLE, target, optimize, profile, interpret);
            assert_eq!(
                (actual.0, actual.1.as_str()),
                SINGLE_OUTPUT,
                "{target:?} {optimize} {profile:?}"
            );
            let configuration = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Warn)
                .with_target(target);
            let actual =
                with_configured_module(C23, target, optimize, profile, configuration, interpret);
            assert_eq!(actual.0, 3, "C23 {target:?} {optimize} {profile:?}");
        }
    }
}

/// Clang marks a definition `dso_local`, which gives it the local-exec model
/// in an ELF executable; bcc requests that model directly. Every access goes
/// through `llvm.threadlocal.address`, and Windows keeps the default model.
#[test]
fn thread_storage_emits_llvm_thread_locals() {
    let source = "_Thread_local int counter = 1; extern __thread int other; static __thread int \
                  own; int main(void) { own = other; return counter + own; }";
    for (target, definition, declaration) in [
        (
            Target::LinuxGnu,
            "@counter = thread_local(localexec) global",
            "@other = external thread_local global",
        ),
        (
            Target::WindowsGnu,
            "@counter = thread_local global",
            "@other = external thread_local global",
        ),
        (
            Target::WindowsMsvc,
            "@counter = thread_local global",
            "@other = external thread_local global",
        ),
    ] {
        with_module(source, target, true, Profile::PostAbi, |module, target| {
            let mut text = String::new();
            crate::backend::llvm::emit_module(module, target, &mut text).unwrap();
            assert!(text.contains(definition), "{target:?}: {text}");
            assert!(text.contains(declaration), "{target:?}: {text}");
            assert!(
                text.contains("@own = internal thread_local"),
                "{target:?}: {text}"
            );
            assert_eq!(
                text.matches("call ptr @llvm.threadlocal.address.p0(ptr @")
                    .count(),
                3,
                "{target:?}: {text}"
            );
        });
    }
}

/// MSVC `__declspec(thread)` lowers like the standard specifier.
/// MSVC extension: Microsoft Learn, "thread".
#[test]
fn declspec_thread_lowers_to_a_thread_local_global() {
    let source = "__declspec(thread) int counter = 4; int main(void) { return counter; }";
    for target in [Target::WindowsMsvc, Target::WindowsGnu] {
        let configuration = CompilerConfiguration::default()
            .with_target(target)
            .with_msvc_extensions(true);
        with_configured_module(
            source,
            target,
            true,
            Profile::PostAbi,
            configuration,
            |module, target| {
                let mut text = String::new();
                crate::backend::llvm::emit_module(module, target, &mut text).unwrap();
                assert!(
                    text.contains("@counter = thread_local global"),
                    "{target:?}: {text}"
                );
                assert_eq!(interpret(module, target).0, 4);
            },
        );
    }
}

#[test]
fn thread_storage_native_arms_match_the_interpreter_and_clang() {
    let Some(target) = host() else { return };
    let Ok(toolchain) = Toolchain::bundled(target) else {
        eprintln!("skipping: no bundled toolchain");
        return;
    };
    let directory = TempDir::new("thread-local-single");
    for arm in &ARMS {
        let executable = directory.join(format!("single-{}.exe", arm.name));
        let expected = with_module(SINGLE, target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &executable,
                options(arm, target, OutputKind::Executable),
            )
            .unwrap();
            interpret(module, target)
        });
        assert_eq!(run_native(&executable), expected, "{}", arm.name);
    }
    for (name, output) in clang_runs(&toolchain, &directory, "single", SINGLE, target) {
        assert_eq!((output.0, output.1.as_str()), SINGLE_OUTPUT, "{name}");
    }
}

/// Four threads each count in their own `counter` and static-local `calls`
/// from the initial values, and add the thread-local `scale` through a
/// pointer to their own counter; main's copy keeps its own value.
const THREADS_BODY: &str = r#"
int printf(const char *, ...);
_Thread_local int counter = 100;
static __thread long scale = 3;
int results[4];

static void record(int index) {
    static _Thread_local int calls;
    for (int i = 0; i <= index * 1000; i++) {
        counter++;
        calls++;
    }
    int *mine = &counter;
    *mine += (int)scale;
    results[index] = counter * 10 + (calls == index * 1000 + 1);
}

static void prepare(void) { counter = 5; }

static int report(void) {
    for (int i = 0; i < 4; i++)
        printf("thread %d: %d\n", i, results[i]);
    printf("main: %d\n", counter);
    return counter == 5 ? 0 : 1;
}
"#;

const WINDOWS_THREADS: &str = "
void *CreateThread(void *, unsigned long long, unsigned long (*)(void *), void *, unsigned long,
                   unsigned long *);
unsigned long WaitForSingleObject(void *, unsigned long);
int CloseHandle(void *);
static unsigned long worker(void *arg) { record((int)(long long)arg); return 0; }
int main(void) {
    void *threads[4];
    prepare();
    for (int i = 0; i < 4; i++)
        threads[i] = CreateThread(0, 0, worker, (void *)(long long)i, 0, 0);
    for (int i = 0; i < 4; i++) {
        WaitForSingleObject(threads[i], 0xFFFFFFFF);
        CloseHandle(threads[i]);
    }
    return report();
}
";

const POSIX_THREADS: &str = "
typedef unsigned long pthread_t;
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int pthread_join(pthread_t, void **);
static void *worker(void *arg) { record((int)(long)arg); return 0; }
int main(void) {
    pthread_t threads[4];
    prepare();
    for (int i = 0; i < 4; i++)
        pthread_create(&threads[i], 0, worker, (void *)(long)i);
    for (int i = 0; i < 4; i++)
        pthread_join(threads[i], 0);
    return report();
}
";

const THREADS_OUTPUT: (i32, &str) = (
    0,
    "thread 0: 1041\nthread 1: 11041\nthread 2: 21041\nthread 3: 31041\nmain: 5\n",
);

#[test]
fn windows_threads_keep_their_own_copies() {
    if !cfg!(all(windows, target_env = "gnu")) {
        eprintln!("skipping: Windows threads need a Windows GNU host");
        return;
    }
    let target = Target::WindowsGnu;
    let Ok(toolchain) = Toolchain::bundled(target) else {
        eprintln!("skipping: no bundled toolchain");
        return;
    };
    let source = format!("{THREADS_BODY}{WINDOWS_THREADS}");
    let directory = TempDir::new("thread-local-windows");
    for arm in &ARMS {
        let executable = directory.join(format!("threads-{}.exe", arm.name));
        with_module(&source, target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &executable,
                options(arm, target, OutputKind::Executable),
            )
            .unwrap();
        });
        let actual = run_native(&executable);
        assert_eq!(
            (actual.0, actual.1.as_str()),
            THREADS_OUTPUT,
            "{}",
            arm.name
        );
    }
    for (name, output) in clang_runs(&toolchain, &directory, "threads", &source, target) {
        assert_eq!((output.0, output.1.as_str()), THREADS_OUTPUT, "{name}");
    }
}

/// Four threads add to `shared`, an `extern _Thread_local` object defined in
/// the other unit, through its name, through a pointer to it, and through a
/// function in that unit; each sees its own copy, and the pointer
/// `shared_address` returns is that thread's own. The defining unit's
/// thread-local initializers hold addresses of a string literal, a static
/// object and a function.
const MULTI_DEFINING: &str = r#"
static int plain = 7;
static int five(void) { return 5; }
_Thread_local int shared = 50;
_Thread_local const char *name = "tls";
_Thread_local int *to_plain = &plain;
_Thread_local int (*function)(void) = five;
int bump(int by) { shared += by; return shared; }
int *shared_address(void) { return &shared; }
"#;

const MULTI_USING: &str = r#"
int printf(const char *, ...);
extern _Thread_local int shared;
extern _Thread_local const char *name;
extern _Thread_local int *to_plain;
extern _Thread_local int (*function)(void);
int bump(int);
int *shared_address(void);
int results[4];

static void record(int index) {
    int *mine = &shared;
    for (int i = 0; i <= index; i++) {
        *mine += 10;
        bump(1);
    }
    results[index] = shared_address() == mine
        ? shared * 100 + *to_plain + function() + (name[0] == 't') : -1;
}

static void prepare(void) { shared = 5; }

static int report(void) {
    for (int i = 0; i < 4; i++)
        printf("thread %d: %d\n", i, results[i]);
    printf("main: %d\n", shared);
    return shared == 5 ? 0 : 1;
}
"#;

const MULTI_OUTPUT: &str =
    "thread 0: 6113\nthread 1: 7213\nthread 2: 8313\nthread 3: 9413\nmain: 5\n";

/// `extern _Thread_local` in one translation unit names the other's
/// definition, and both units compute the same thread's address.
const DEFINING_UNIT: &str = "
_Thread_local int counter = 7;
int get(void) { return counter; }
int *address(void) { return &counter; }
";

const USING_UNIT: &str = "
extern _Thread_local int counter;
int get(void);
int *address(void);
int main(void) { counter += 5; return get() == 12 && address() == &counter ? 0 : 1; }
";

#[test]
fn extern_thread_locals_link_across_translation_units() {
    let Some(target) = host() else { return };
    let Ok(toolchain) = Toolchain::bundled(target) else {
        eprintln!("skipping: no bundled toolchain");
        return;
    };
    let directory = TempDir::new("thread-local-units");
    for arm in &ARMS {
        let objects = objects(
            &directory,
            "units",
            arm,
            target,
            &[DEFINING_UNIT, USING_UNIT],
        );
        let executable = directory.join(format!("units-{}.exe", arm.name));
        let mut link = toolchain.command(
            &objects[0],
            &executable,
            options(arm, target, OutputKind::Executable),
        );
        let linked = crate::test_support::native::run_native(link.arg(&objects[1])).unwrap();
        assert!(linked.status.success(), "{linked:?}");
        assert_eq!(run_native(&executable).0, 0, "{}", arm.name);
    }
}

/// The two-unit program on Windows threads, from every arm.
#[test]
fn windows_threads_keep_their_own_extern_copies() {
    if !cfg!(all(windows, target_env = "gnu")) {
        eprintln!("skipping: Windows threads need a Windows GNU host");
        return;
    }
    let target = Target::WindowsGnu;
    let Ok(toolchain) = Toolchain::bundled(target) else {
        eprintln!("skipping: no bundled toolchain");
        return;
    };
    let using = format!("{MULTI_USING}{WINDOWS_THREADS}");
    let directory = TempDir::new("thread-local-windows-units");
    for arm in &ARMS {
        let objects = objects(&directory, "multi", arm, target, &[MULTI_DEFINING, &using]);
        let executable = directory.join(format!("multi-{}.exe", arm.name));
        let mut link = toolchain.command(
            &objects[0],
            &executable,
            options(arm, target, OutputKind::Executable),
        );
        let linked = crate::test_support::native::run_native(link.arg(&objects[1])).unwrap();
        assert!(linked.status.success(), "{linked:?}");
        let actual = run_native(&executable);
        assert_eq!(
            (actual.0, actual.1.as_str()),
            (0, MULTI_OUTPUT),
            "{}",
            arm.name
        );
    }
}

/// The interpreter runs one thread and has no operation that creates
/// another, so a program that tries to stops with a trap instead of two
/// threads silently sharing one copy.
#[test]
fn the_interpreter_stops_programs_that_create_threads() {
    for (target, tail) in [
        (Target::WindowsGnu, WINDOWS_THREADS),
        (Target::LinuxGnu, POSIX_THREADS),
    ] {
        let source = format!("{THREADS_BODY}{tail}");
        for (optimize, profile) in STAGES {
            with_module(&source, target, optimize, profile, |module, target| {
                let arena = Bump::new();
                let mut host = DefaultHost::for_target(&arena, target);
                let trap = interpreter::run(module, "main", &[], &mut host)
                    .expect_err("the program creates a thread");
                let message = trap.display_in(module).to_string();
                assert!(
                    message.contains("has no definition"),
                    "{target:?} {optimize} {profile:?}: {message}"
                );
            });
        }
    }
}

/// The Linux arms link under WSL with pthreads, beside Clang's objects, and
/// the two-unit program links there too.
#[test]
fn linux_threads_keep_their_own_copies_under_wsl() {
    if !cfg!(windows)
        || !crate::test_support::native::run_native(Command::new("wsl.exe").args(["-e", "true"]))
            .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping Linux thread-local execution: WSL runner unavailable");
        return;
    }
    let target = Target::LinuxGnu;
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("thread-local-linux");
    let source = format!("{THREADS_BODY}{POSIX_THREADS}");
    let using = format!("{MULTI_USING}{POSIX_THREADS}");
    let mut programs = Vec::new();
    for arm in &ARMS {
        let threads = objects(&directory, "threads", arm, target, &[&source]);
        programs.push((format!("threads-{}", arm.name), threads, THREADS_OUTPUT.1));
        let multi = objects(&directory, "multi", arm, target, &[MULTI_DEFINING, &using]);
        programs.push((format!("multi-{}", arm.name), multi, MULTI_OUTPUT));
        let units = objects(
            &directory,
            "units",
            arm,
            target,
            &[DEFINING_UNIT, USING_UNIT],
        );
        programs.push((format!("units-{}", arm.name), units, ""));
    }
    let input = directory.join("threads.c");
    std::fs::write(&input, &source).unwrap();
    for opt in ["-O0", "-O2"] {
        let object = directory.join(format!("threads-clang{opt}.o"));
        let compiled = crate::test_support::native::run_native(
            Command::new(toolchain.clang())
                .args(["--target=x86_64-unknown-linux-gnu", opt, "-c", "-o"])
                .arg(&object)
                .arg(&input),
        )
        .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        programs.push((
            format!("threads-clang{opt}"),
            vec![object],
            THREADS_OUTPUT.1,
        ));
    }
    for (name, objects, expected) in programs {
        let mut link = Command::new("wsl.exe");
        _ = link
            .args(["-e", "cc", "-pthread", "-o", &name])
            .args(objects.iter().map(|object| object.file_name().unwrap()))
            .current_dir(directory.path());
        let linked = crate::test_support::native::run_native(&mut link).unwrap();
        assert!(linked.status.success(), "{name}: {linked:?}");
        let ran = crate::test_support::native::run_native(
            Command::new("wsl.exe")
                .args(["-e", &format!("./{name}")])
                .current_dir(directory.path()),
        )
        .unwrap();
        assert_eq!(ran.status.code(), Some(0), "{name}: {ran:?}");
        assert_eq!(String::from_utf8_lossy(&ran.stdout), expected, "{name}");
    }
}

fn options(arm: &Arm, target: Target, kind: OutputKind) -> LlvmOptions {
    LlvmOptions {
        opt: Some(arm.opt),
        disable_llvm_passes: arm.disable_llvm_passes,
        target,
        kind,
    }
}

/// Compiles each source to an object in `arm`, named `name` and its
/// position.
fn objects(
    directory: &TempDir,
    name: &str,
    arm: &Arm,
    target: Target,
    sources: &[&str],
) -> Vec<std::path::PathBuf> {
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let object = directory.join(format!("{name}{index}-{}.o", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(module, &object, options(arm, target, OutputKind::Object)).unwrap();
            });
            object
        })
        .collect()
}

/// Builds `source` with the bundled Clang at -O0 and -O2 and runs it.
fn clang_runs(
    toolchain: &Toolchain,
    directory: &TempDir,
    name: &str,
    source: &str,
    target: Target,
) -> Vec<(String, (i32, String))> {
    let input = directory.join(format!("{name}.c"));
    std::fs::write(&input, source).unwrap();
    [OptLevel::O0, OptLevel::O2]
        .into_iter()
        .map(|opt| {
            let executable = directory.join(format!("{name}-clang{opt:?}.exe"));
            let output = toolchain
                .command(
                    &input,
                    &executable,
                    LlvmOptions {
                        opt: Some(opt),
                        disable_llvm_passes: false,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            (format!("clang {opt:?}"), run_native(&executable))
        })
        .collect()
}
