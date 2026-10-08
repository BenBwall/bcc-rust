//! Counts global allocations made while the compiler runs.
//!
//! This integration test has its own global allocator, leaving the library's
//! ordinary unit-test process untouched. Arena memory comes from the
//! operating system's virtual memory, so everything counted here is an
//! allocation outside `util::bump`. Counting is per thread, so tests running
//! in parallel do not see each other's allocations.
//!
//! Set `BCC_ALLOCATION_SITES=1` to also record where each counted allocation
//! comes from: [`tests::report_allocation_sites`] then prints the call sites
//! with their counts for the golden diagnostic fixtures and for every file
//! listed, one path per line, in the file named by `BCC_ALLOCATION_INPUTS`.
//!
//! With `--features benchmarking-internals`, the `measurements` tests also
//! assert that compiling a translation unit allocates nothing from the
//! global allocator: every phase takes its memory from the virtual-memory
//! arenas of `util::bump`, `util::region_vec`, and `util::region_bit_set`.
//! Each counted interval is one call of the benchmark API, which runs
//! translation phases 1 through 7 under the batch pipeline for one
//! translation unit and then drops all of its arenas. The inputs are the
//! generated benchmark inputs that are C, a source written to reach the
//! rarer paths of every phase, and a main file with headers read from disk.
//! Every input must produce no diagnostics; reporting them is covered by
//! `reporting_golden_diagnostics_makes_no_global_allocations` above. Not
//! covered: `__DATE__` and `__TIME__` (which the benchmark API pins to the
//! Unix epoch, and which otherwise read the clock), include directories and
//! `SOURCE_DATE_EPOCH` from the command line or environment, and the CLI's
//! argument parsing. One exception is allowed, only for files read from
//! disk: the standard library's `File::open` and `Path::is_file` convert
//! each path to the operating system's encoding in a buffer of their own,
//! which the compiler cannot supply.
#![expect(
    unused_crate_dependencies,
    reason = "This test binary uses only the measurement API."
)]

#[cfg(test)]
#[cfg_attr(
    windows,
    expect(
        clippy::missing_const_for_thread_local,
        reason = "Every thread-local initializer here is `const`; the lint misreads the macro in \
                  this test crate."
    )
)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod counting {
    use std::{
        alloc::{
            GlobalAlloc,
            Layout,
            System,
        },
        backtrace::Backtrace,
        cell::{
            Cell,
            RefCell,
        },
        collections::BTreeMap,
    };

    /// Full call stacks kept by [`count_keeping`]; later allocations are
    /// only counted.
    pub(crate) const KEPT_STACKS: usize = 16;

    /// Calls and requested bytes.
    pub(crate) type Totals = (usize, usize);

    // `RECORDING` is set while this thread records a site, whose own
    // allocations are not counted.
    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static CAPTURE: Cell<bool> = const { Cell::new(false) };
        static RECORDING: Cell<bool> = const { Cell::new(false) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
        static BYTES: Cell<usize> = const { Cell::new(0) };
        static SITES: RefCell<BTreeMap<String, Totals>> = const { RefCell::new(BTreeMap::new()) };
        static KEEP: Cell<bool> = const { Cell::new(false) };
        static STACKS: RefCell<Vec<(usize, Backtrace)>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) struct Counting;

    // SAFETY: every operation delegates to `System` with the caller's
    // arguments.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                record(layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract.
            unsafe {
                System.dealloc(ptr, layout);
            }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
            let moved = unsafe { System.realloc(ptr, layout, new_size) };
            if !moved.is_null() {
                record(new_size);
            }
            moved
        }
    }

    fn record(size: usize) {
        // Thread-local storage may already be gone while a thread exits.
        let counted = ACTIVE.try_with(Cell::get).unwrap_or(false)
            && !RECORDING.try_with(Cell::get).unwrap_or(true);
        if !counted {
            return;
        }
        CALLS.set(CALLS.get() + 1);
        BYTES.set(BYTES.get() + size);
        if CAPTURE.get() {
            RECORDING.set(true);
            let call_stack = allocation_site();
            SITES.with_borrow_mut(|sites| {
                let totals = sites.entry(call_stack).or_default();
                totals.0 += 1;
                totals.1 += size;
            });
            RECORDING.set(false);
        }
        if KEEP.get() {
            RECORDING.set(true);
            STACKS.with_borrow_mut(|stacks| {
                if stacks.len() < KEPT_STACKS {
                    stacks.push((size, Backtrace::force_capture()));
                }
            });
            RECORDING.set(false);
        }
    }

    /// The innermost compiler frames of the current call stack, each with its
    /// source location, preceded by the frame they called into.
    fn allocation_site() -> String {
        let trace = Backtrace::force_capture().to_string();
        let mut frames = Vec::new();
        let mut callee = None;
        let mut lines = trace.lines().peekable();
        while let Some(line) = lines.next() {
            let line = line.trim_start();
            if line.starts_with("at ") {
                continue;
            }
            let Some((_, name)) = line.split_once(": ") else {
                continue;
            };
            let location = lines
                .peek()
                .and_then(|next| next.trim_start().strip_prefix("at "))
                .map(|path| {
                    let path = path.replace('\\', "/");
                    path.rfind("/src/")
                        .map_or(path.clone(), |start| path[start + 1..].to_owned())
                });
            // Compiler functions, methods, and trait implementations, but not
            // library code instantiated with compiler types.
            let compiler = name.starts_with("bcc_rust::")
                || name.starts_with("<bcc_rust::")
                || (name.starts_with('<') && name.contains(" as bcc_rust::"));
            if compiler {
                if name.contains("compile_file_measured") {
                    break;
                }
                frames.push(match location {
                    | Some(location) => format!("{name} ({location})"),
                    | None => name.to_owned(),
                });
                if frames.len() == 4 {
                    break;
                }
            } else if frames.is_empty() && !name.starts_with("allocation_count::") {
                callee = Some(name.to_owned());
            }
        }
        let mut site = frames.join("\n      < ");
        if let Some(callee) = callee {
            site.push_str("\n      > ");
            site.push_str(&callee);
        }
        site
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    /// Whether `BCC_ALLOCATION_SITES` asks for allocation sites.
    pub(crate) fn sites_requested() -> bool {
        std::env::var_os("BCC_ALLOCATION_SITES").is_some_and(|value| value == "1")
    }

    /// Runs `body`, returning the global allocations this thread made in it.
    pub(crate) fn count(capture: bool, body: impl FnOnce()) -> Totals {
        CALLS.set(0);
        BYTES.set(0);
        CAPTURE.set(capture);
        ACTIVE.set(true);
        body();
        ACTIVE.set(false);
        CAPTURE.set(false);
        (CALLS.get(), BYTES.get())
    }

    /// Like [`count`], also returning the full call stacks of the first
    /// [`KEPT_STACKS`] allocations.
    #[cfg(feature = "benchmarking-internals")]
    pub(crate) fn count_keeping(body: impl FnOnce()) -> (Totals, Vec<(usize, Backtrace)>) {
        STACKS.with_borrow_mut(Vec::clear);
        KEEP.set(true);
        let totals = count(false, body);
        KEEP.set(false);
        (totals, STACKS.take())
    }

    /// Takes the sites recorded on this thread so far.
    pub(crate) fn take_sites() -> BTreeMap<String, Totals> {
        SITES.take()
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use std::{
        fs,
        io,
        path::{
            Path,
            PathBuf,
        },
    };

    use bcc_rust::CompileStep;

    use crate::counting::{
        Totals,
        count,
        sites_requested,
        take_sites,
    };

    fn golden_fixtures() -> Vec<PathBuf> {
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics");
        let mut fixtures: Vec<_> = fs::read_dir(directory)
            .expect("diagnostic fixtures must be readable")
            .map(|entry| entry.expect("fixture entry must be readable").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
            .collect();
        fixtures.sort();
        assert!(
            !fixtures.is_empty(),
            "the diagnostic corpus must not be empty"
        );
        fixtures
    }

    /// Compiles `path`, counting the global allocations of `measured` steps.
    fn compile(path: &Path, measured: &[CompileStep], capture: bool) -> Vec<(CompileStep, Totals)> {
        let mut totals = Vec::new();
        compile_fixture(path, &mut io::sink(), |step, run| {
            if measured.contains(&step) {
                totals.push((step, count(capture, run)));
            } else {
                run();
            }
        })
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        totals
    }

    fn compile_fixture(
        path: &Path,
        out: &mut dyn io::Write,
        measure: impl FnMut(CompileStep, &mut dyn FnMut()),
    ) -> io::Result<()> {
        match fs::read_to_string(path.with_extension("args")) {
            | Ok(arguments) => bcc_rust::compile_file_with_arguments_measured(
                path,
                &arguments.split_whitespace().collect::<Vec<_>>(),
                out,
                measure,
            ),
            | Err(error) if error.kind() == io::ErrorKind::NotFound =>
                bcc_rust::compile_file_measured(path, out, measure),
            | Err(error) => Err(error),
        }
    }

    /// The counter sees an allocation made through the global allocator, so a
    /// count of zero means none was made.
    #[test]
    fn counting_sees_global_allocations() {
        let (calls, bytes) = count(false, || drop(std::hint::black_box(Box::new(7_u64))));
        assert_eq!((calls, bytes), (1, 8));
    }

    /// Building, folding, ordering, and rendering every golden diagnostic,
    /// as the CLI reports them, allocates only in arenas.
    #[test]
    fn reporting_golden_diagnostics_makes_no_global_allocations() {
        let mut failures = Vec::new();
        for fixture in golden_fixtures() {
            // Room for the whole report, so writing it does not allocate.
            let mut stderr = Vec::with_capacity(1 << 20);
            let mut totals = (0, 0);
            compile_fixture(&fixture, &mut stderr, |step, run| {
                if step == CompileStep::Report {
                    totals = count(sites_requested(), run);
                } else {
                    run();
                }
            })
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.display()));
            assert!(!stderr.is_empty(), "{} reported nothing", fixture.display());
            let (calls, bytes) = totals;
            if calls != 0 {
                failures.push(format!(
                    "{}: {calls} global allocations, {bytes} bytes",
                    fixture.display()
                ));
            }
        }
        for (site, (calls, _)) in take_sites() {
            println!("{calls:>6}  {site}");
        }
        assert!(
            failures.is_empty(),
            "reporting diagnostics allocated outside the arenas; rerun with \
             BCC_ALLOCATION_SITES=1 and --nocapture to print their sites:\n{}",
            failures.join("\n")
        );
    }

    /// Prints where global allocations come from, per compile step, when
    /// `BCC_ALLOCATION_SITES=1`; otherwise does nothing.
    #[test]
    fn report_allocation_sites() {
        if !sites_requested() {
            return;
        }
        let mut inputs = golden_fixtures();
        if let Some(list) = std::env::var_os("BCC_ALLOCATION_INPUTS") {
            let list = fs::read_to_string(list).expect("the input list must be readable");
            inputs.extend(
                list.lines()
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from),
            );
        }
        for step in [CompileStep::Parse, CompileStep::Report] {
            let mut calls = 0;
            let mut bytes = 0;
            let mut files = 0;
            drop(take_sites());
            for input in &inputs {
                for (_, totals) in compile(input, &[step], true) {
                    calls += totals.0;
                    bytes += totals.1;
                    files += usize::from(totals.0 > 0);
                }
            }
            let mut sites: Vec<_> = take_sites().into_iter().collect();
            sites.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.0));
            println!(
                "== {step:?}: {calls} global allocations, {bytes} bytes, in {files} of {} inputs",
                inputs.len()
            );
            for (site, (calls, bytes)) in sites {
                println!("{calls:>8} calls {bytes:>10} bytes  {site}");
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "benchmarking-internals")]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod measurements {
    use std::{
        backtrace::Backtrace,
        fmt::Write as _,
    };

    use bcc_rust::{
        BenchmarkInput,
        ParseBenchmarkSummary,
    };

    use crate::counting::{
        KEPT_STACKS,
        count,
        count_keeping,
    };

    #[test]
    fn report_parser_mix_global_allocations() {
        let input = BenchmarkInput::ParserMix;
        // Force source generation outside the measured interval.
        _ = input.bytes();
        let mut result = None;
        let (calls, bytes) = count(false, || result = Some(bcc_rust::parse(input)));
        println!(
            "parser mix: result={result:?}, global allocation calls={calls}, requested              bytes={bytes}"
        );
    }

    /// The global allocations `compile` made on this thread.
    struct Allocations {
        calls:  usize,
        bytes:  usize,
        stacks: Vec<(usize, Backtrace)>,
    }

    fn count_compile(
        compile: impl FnOnce() -> ParseBenchmarkSummary,
    ) -> (ParseBenchmarkSummary, Allocations) {
        let mut summary = None;
        let ((calls, bytes), stacks) = count_keeping(|| summary = Some(compile()));
        let summary = summary.expect("the measured compilation ran");
        (
            summary,
            Allocations {
                calls,
                bytes,
                stacks,
            },
        )
    }

    fn assert_no_allocations(
        name: &str,
        summary: ParseBenchmarkSummary,
        allocations: &Allocations,
    ) {
        assert_eq!(
            summary.diagnostics, 0,
            "{name}: the input must compile without diagnostics"
        );
        if allocations.calls == 0 {
            return;
        }
        let mut message = format!(
            "{name}: {} global allocations ({} bytes) outside the arenas; the first stacks:\n",
            allocations.calls, allocations.bytes
        );
        for (size, stack) in &allocations.stacks {
            _ = writeln!(message, "--- {size} bytes\n{stack}");
        }
        panic!("{message}");
    }

    #[test]
    fn compiling_benchmark_inputs_allocates_only_from_arenas() {
        // The preprocessor stress inputs are not C that parses: each of their
        // lines draws a parser diagnostic.
        for input in BenchmarkInput::ALL
            .into_iter()
            .chain(BenchmarkInput::PARSER_STRESS)
        {
            // Generate the source outside the measured interval.
            _ = input.bytes();
            let (summary, allocations) = count_compile(|| bcc_rust::parse(input));
            assert!(summary.external_declarations > 0, "{}", input.name());
            assert_no_allocations(input.name(), summary, &allocations);
        }
    }

    /// Reaches the paths the generated inputs leave out.
    const FEATURE_SOURCE: &str = concat!(
        "#define STR(x) #x\r\n",
        "#define XSTR(x) STR(x)\r\n",
        "#define CAT(a, b) a ## b\r\n",
        "#define LONG_MACRO(a, b, \\\n         c) ((a) + (b) + (c))\n",
        "#if defined(STR) && (1 + 2 * 3 == 7) && 'a' == 97\n",
        "static const char *file = __FILE__;\n",
        "static int line = __LINE__;\n",
        "#else\n",
        "#error not reached\n",
        "#endif\n",
        "_Pragma(\"STDC FP_CONTRACT ON\")\n",
        "#pragma STDC FENV_ACCESS OFF\n",
        "#line 100 \"renamed.c\"\n",
        "static const char *renamed = __FILE__ \"-\" XSTR(__LINE__);\n",
        "static const char *joined = \"a\" \"b\" L\"c\" \"\\x41\\101\\n\";\n",
        "static const char *quoted = STR(\"q\\\"\" 'c' a  +  b);\n",
        "static int trigraph??(2??) = ??< 1, 2 ??>;\n",
        "static int \\u00e9t\\u00E9 = 3;\n",
        "static int CAT(pas, ted) = LONG_MACRO(1, 2, 3);\n",
        "static int wide = L'x' + '\\n' + '\\377' + 'ab';\n",
        "typedef struct { int x; enum { RED, GREEN } color; } pair;\n",
        "int old_style(a, b) int a; char *b; { return a + *b; }\n",
        "int body(pair p, enum { ONE = 1 } e) {\n",
        "    int total = 0;\n",
        "    switch (p.color) { case RED: total = 1; break; default: total = e; }\n",
        "    for (int i = 0; i < 3; i++) { if (i == 2) goto done; total += i; }\n",
        "done:\n",
        "    return total + (int)sizeof(pair) + ONE + GREEN;\n",
        "}\n",
    );

    #[test]
    fn compiling_rare_features_allocates_only_from_arenas() {
        let (summary, allocations) = count_compile(|| bcc_rust::parse_source(FEATURE_SOURCE));
        assert!(summary.external_declarations > 0);
        assert_no_allocations("feature source", summary, &allocations);
    }

    #[test]
    fn compiling_iso_syntax_allocates_only_from_arenas() {
        // Reserved spellings and unambiguous grammar are extensions under the
        // library's C99/Allow default, so this also traverses policy seams.
        let source = "[[vendor::tag((1),[2],{3})]] _Alignas(16) _Atomic(int) object; \
                      _Thread_local int thread; _Static_assert(1,\"message\"); unsigned \
                      _BitInt(16) bits; enum E : unsigned { A [[deprecated]] }; int f(int) { int \
                      a[3]; int n=_Generic(a,int*:1,default:0); n+=_Alignof(int)+_Countof a; \
                      if(int x=1;x) n=x; switch(n){case 1 ... 3:break;} outer: for(;;){break \
                      outer;} label: int x=(static int){}; return n; }\n";
        let (summary, allocations) = count_compile(|| bcc_rust::parse_source(source));
        assert_eq!(summary.external_declarations, 6);
        assert_no_allocations("ISO syntax source", summary, &allocations);
    }

    #[test]
    fn compiling_gnu_syntax_allocates_only_from_arenas() {
        let source = "__attribute__((used)) unsigned __int128 wide[0]; __typeof__(wide) copy; \
                      __auto_type value=1; struct Empty {}; __asm__(\"nop\"); int \
                      f(void){__label__ L; int nested(int x){return x;} int a[4]={[1 ... 3]=2}; \
                      struct S{int x;}; struct S s={x:1}; __asm__ \
                      volatile(\"\":[out]\"=r\"(value):\"r\"(value):\"memory\"); __asm__ \
                      goto(\"\"::::L); void *p=&&L; goto *p; L: return __extension__ ({ \
                      __builtin_va_arg(ap,int)+__builtin_offsetof(struct \
                      S,x)+__builtin_types_compatible_p(int,long)+__builtin_choose_expr(1,\
                      __real__ value,__imag__ value); }) ?: 2;}\n";
        let (summary, allocations) = count_compile(|| bcc_rust::parse_source(source));
        assert_eq!(summary.external_declarations, 6);
        assert_no_allocations("GNU syntax source", summary, &allocations);
    }

    #[test]
    fn compiling_pedantic_suppression_allocates_only_from_arenas() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/diagnostics/language-extension-suppression.c");
        let mut parse_calls = None;
        let mut report_calls = None;
        bcc_rust::compile_file_with_arguments_measured(
            &path,
            &["-std=c17", "-pedantic"],
            &mut std::io::sink(),
            |step, run| {
                let totals = count(false, run);
                match step {
                    | bcc_rust::CompileStep::Parse => parse_calls = Some(totals),
                    | bcc_rust::CompileStep::Report => report_calls = Some(totals),
                }
            },
        )
        .expect("suppression fixture compiles and renders");
        assert_eq!(parse_calls, Some((0, 0)));
        assert_eq!(report_calls, Some((0, 0)));
    }

    #[test]
    fn compiling_msvc_syntax_allocates_only_from_arenas() {
        let source = include_str!("fixtures/diagnostics/language/msvc-parser.c");
        let (summary, allocations) = count_compile(|| bcc_rust::parse_msvc_source(source));
        assert_eq!(summary.external_declarations, 8);
        assert_no_allocations("MSVC syntax source", summary, &allocations);
        let (summary, allocations) = count_compile(|| {
            bcc_rust::parse_msvc_source(
                "int f(void) { __try {} __except() {} __asm mov eax, [ebx\nreturn 0; } int \
                 following;\n",
            )
        });
        assert_eq!(summary.external_declarations, 2);
        assert!(summary.diagnostics > 0);
        assert_eq!(allocations.calls, 0);
    }

    /// Recovery paths also belong to the zero-global-allocation contract.
    #[test]
    fn compiling_malformed_sources_allocates_only_from_arenas() {
        for (name, source, expected_declarations) in [
            ("parser recovery", "int x = ;\nint y;\n", 2),
            ("preprocessor recovery", "#if (1 + )\n#endif\nint y;\n", 1),
        ] {
            let (summary, allocations) = count_compile(|| bcc_rust::parse_source(source));
            assert!(summary.diagnostics > 0, "{name}: expected diagnostics");
            assert_eq!(
                summary.external_declarations, expected_declarations,
                "{name}: lost the following declaration or added a spurious one"
            );
            assert_eq!(
                allocations.calls, 0,
                "{name}: {} global allocations ({} bytes) outside the arenas",
                allocations.calls, allocations.bytes
            );
        }
    }

    /// A translation unit split over files on disk: a header found beside
    /// the main file, one in a subdirectory, and a `#pragma once` header
    /// included twice.
    const DISK_FILES: [(&str, &str); 4] = [
        (
            "main.c",
            "#include \"local.h\"\n#include \"nested/deep.h\"\n#include \"once.h\"\n#include \
             \"once.h\"\nint main(void) { return local + deep + once; }\n",
        ),
        ("local.h", "static int local = 1;\n"),
        (
            "nested/deep.h",
            "#include \"sibling.h\"\nstatic int deep = sibling;\n",
        ),
        ("nested/sibling.h", "enum { sibling = 2 };\n"),
    ];
    const ONCE_HEADER: (&str, &str) = ("once.h", "#pragma once\nstatic int once = 3;\n");

    /// Whether an allocation happened inside the standard library's
    /// file-system calls, judged by the frames of its stack.
    fn in_std_file_system(stack: &Backtrace) -> bool {
        stack.to_string().lines().any(|line| {
            let frame = line.trim_start();
            frame.split_once(": ").is_some_and(|(number, function)| {
                number.parse::<usize>().is_ok()
                    && (function.starts_with("<std::fs::File>::open")
                        || function.starts_with("<std::path::Path>::is_file"))
            })
        })
    }

    #[test]
    fn compiling_files_with_includes_allocates_only_from_arenas_and_std_file_system() {
        let directory =
            std::env::temp_dir().join(format!("bcc-allocation-count-{}", std::process::id()));
        for (name, text) in DISK_FILES.into_iter().chain([ONCE_HEADER]) {
            let path = directory.join(name);
            std::fs::create_dir_all(path.parent().expect("files have a directory"))
                .expect("the temporary directory is writable");
            std::fs::write(&path, text).expect("the temporary file is writable");
        }
        let main = directory.join(DISK_FILES[0].0);
        let (summary, mut allocations) =
            count_compile(|| bcc_rust::parse_file(&main).expect("the main file is readable"));
        drop(std::fs::remove_dir_all(&directory));
        // Only kept stacks can be classified, so every allocation must have
        // been kept to excuse any of them.
        if allocations.calls <= KEPT_STACKS {
            allocations
                .stacks
                .retain(|(_, stack)| !in_std_file_system(stack));
            allocations.calls = allocations.stacks.len();
            allocations.bytes = allocations.stacks.iter().map(|(size, _)| size).sum();
        }
        assert_eq!(summary.external_declarations, 5);
        assert_no_allocations(&main.display().to_string(), summary, &allocations);
    }
}
