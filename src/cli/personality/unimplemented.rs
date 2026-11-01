//! What the GCC, Clang and TCC personalities do with a `-f` or `-m` option
//! their compiler knows and bcc does not implement. An option is ignored
//! only when bcc's output already means what the option asks for: it
//! changes diagnostics, debug information, reports or build mechanics; it
//! permits an optimization bcc may leave undone; it tunes code generation
//! without changing what the program computes; it asks for what bcc always
//! does (`-fno-common`, `-fno-strict-aliasing`, `-mno-avx`); or it applies
//! only to another language. Hardening and instrumentation that a correct
//! program does not depend on are ignored with a warning, so the build goes
//! on and says what it lacks. Every other known option is refused, so that
//! no layout, ABI or language rule silently differs from what the command
//! line asked for. A `-m` option is judged for the target: `-mms-bitfields`
//! is what a Windows target does and a layout change on Linux. Asking for a
//! newer processor (`-march=native`, `-mavx2`) warns, since baseline x86-64
//! code is correct on it.
//!
//! The lists name patterns: `*` ends a prefix, a trailing `=` takes any
//! value, and `-f[no-]name` stands for both `-fname` and `-fno-name`. The
//! refusals are matched first, so a narrower refusal can sit inside a
//! broader ignored prefix.

use crate::target::Target;

/// How a personality treats a known option bcc does not implement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Treatment {
    /// Accepted with no effect.
    Ignore,
    /// Accepted with a warning that gives the reason.
    Warn(&'static str),
    /// Refused with an error that gives the reason.
    Reject(&'static str),
}

/// The treatment of `option`, a `-f` or `-m` option its compiler knows,
/// for `target`.
pub(super) fn treatment(option: &str, target: Target) -> Treatment {
    // bcc reads and writes UTF-8.
    if let Some(charset) = option
        .strip_prefix("-finput-charset=")
        .or_else(|| option.strip_prefix("-fexec-charset="))
        && matches!(charset.to_ascii_lowercase().as_str(), "utf-8" | "utf8")
    {
        return Treatment::Ignore;
    }
    if let Some(&(_, why)) = REFUSED.iter().find(|(pattern, _)| matches(pattern, option)) {
        return Treatment::Reject(why);
    }
    if option.starts_with("-m") {
        return machine(option, target);
    }
    if IGNORED.iter().any(|pattern| matches(pattern, option)) {
        return Treatment::Ignore;
    }
    if let Some(&(_, why)) = WARNED.iter().find(|(pattern, _)| matches(pattern, option)) {
        return Treatment::Warn(why);
    }
    Treatment::Reject(UNSUPPORTED)
}

/// Why an option bcc does not implement is refused by default.
const UNSUPPORTED: &str = "bcc does not implement it, and ignoring it could change the program";

/// Whether `option` matches `pattern` (see the module comment).
fn matches(pattern: &str, option: &str) -> bool {
    if let Some((head, tail)) = pattern.split_once("[no-]") {
        let negative = format!("{head}no-{tail}");
        let positive = format!("{head}{tail}");
        return matches(&positive, option) || matches(&negative, option);
    }
    match pattern.strip_suffix('*') {
        | Some(prefix) => option.starts_with(prefix),
        | None if pattern.ends_with('=') => option.starts_with(pattern),
        | None => option == pattern,
    }
}

/// Options that ask for what bcc does not do, with the reason, matched
/// before the broader lists below.
const REFUSED: &[(&str, &str)] = &[
    (
        "-fno-aggressive-loop-optimizations",
        "bcc's optimizer may rely on undefined behavior in loops",
    ),
    (
        "-fno-finite-loops",
        "bcc's optimizer may assume that loops terminate",
    ),
    (
        "-fshort-enums",
        "bcc gives every enumeration the size of int",
    ),
    ("-fshort-wchar", "wchar_t has the target's width in bcc"),
    (
        "-funsigned-bitfields",
        "plain int bit-fields are signed in bcc",
    ),
    (
        "-fsingle-precision-constant",
        "bcc gives floating constants their type in C",
    ),
    (
        "-frounding-math",
        "bcc assumes the default rounding mode when it folds constants",
    ),
    (
        "-ftrapv-*",
        "bcc's -ftrapv traps on overflow directly, without a handler",
    ),
    (
        "-fnon-call-exceptions",
        "bcc's code does not let a trapping instruction throw, and its optimizer may remove one \
         that would trap",
    ),
    (
        "-fsplit-stack",
        "bcc's functions run on one contiguous stack, which a program that relies on split stacks \
         would overflow",
    ),
    (
        "-fsemantic-interposition",
        "bcc does not promise interposable calls",
    ),
    (
        "-fno-toplevel-reorder",
        "bcc may reorder top-level definitions",
    ),
    (
        "-fno-unit-at-a-time",
        "bcc may reorder top-level definitions",
    ),
    ("-fsanitize=", "bcc has no sanitizers"),
    (
        "-fsanitize-coverage=",
        "bcc has no coverage instrumentation",
    ),
    (
        "-fkeep-system-includes",
        "bcc's -E output expands every include",
    ),
    (
        "-frewrite-includes",
        "bcc's -E output expands every include",
    ),
    ("-fno-digraphs", "bcc's mode decides digraphs"),
    (
        "-fraw-string-literals",
        "bcc has no raw string literals in C",
    ),
    (
        "-fwritable-strings",
        "bcc places string literals in read-only memory",
    ),
    ("-fexec-charset=", "bcc writes UTF-8 execution strings only"),
    ("-finput-charset=", "bcc reads UTF-8 source only"),
    (
        "-fwide-exec-charset=",
        "bcc writes wide strings in the target's encoding only",
    ),
    (
        "-fno-extended-identifiers",
        "bcc accepts extended identifiers",
    ),
    ("-fno-strict-bool*", "bcc assumes that a _Bool holds 0 or 1"),
    (
        "-fno-strict-float-cast-overflow",
        "bcc leaves out-of-range floating conversions undefined",
    ),
    ("-fno-merge-constants", "bcc may merge identical constants"),
    (
        "-fstrict-volatile-bitfields",
        "bcc may access volatile bit-fields by byte",
    ),
    (
        "-fstrict-flex-arrays*",
        "bcc treats every trailing array as flexible",
    ),
    (
        "-fpcc-struct-return",
        "bcc returns small structures in registers",
    ),
    (
        "-fstack-reuse=*",
        "bcc may reuse the stack slots of dead variables",
    ),
    (
        "-fno-define-target-os-macros",
        "bcc defines the target's macros",
    ),
    ("-fgnuc-version=", "the personality decides __GNUC__"),
    (
        "-fms-compatibility*",
        "bcc implements Microsoft extensions, not compatibility",
    ),
    ("-fmsc-version=", "the MSVC personality decides _MSC_VER"),
];

/// Options with no effect on what bcc's output means, or that apply only to
/// another language.
const IGNORED: &[&str] = &[
    // Diagnostics, reports and dumps.
    "-f[no-]diagnostics-*",
    "-fmessage-length=",
    "-fmax-errors=",
    "-ferror-limit=",
    "-f[no-]show-column",
    "-ftabstop=",
    "-f[no-]color-diagnostics",
    "-fansi-escape-codes",
    "-f[no-]caret-diagnostics*",
    "-f[no-]show-source-location",
    "-f[no-]spell-checking*",
    "-fmacro-backtrace-limit=",
    "-ftemplate-backtrace-limit=",
    "-fconstexpr-backtrace-limit=",
    "-fshow-overloads=",
    "-f[no-]elide-type",
    "-f[no-]crash-diagnostics*",
    "-fshow-skipped-includes",
    "-f[no-]safe-buffer-usage-suggestions",
    "-fmax-tokens=",
    "-fbracket-depth=",
    "-fmax-include-depth=",
    "-flarge-source-files",
    "-f[no-]track-macro-expansion*",
    "-fdebug-cpp",
    "-f[no-]analyzer*",
    "-f[no-]opt-info*",
    "-f[no-]save-optimization-record*",
    "-foptimization-record-*",
    "-ftime-report*",
    "-ftime-trace*",
    "-fproc-stat-report*",
    "-fmem-report*",
    "-fpre-ipa-mem-report",
    "-fpost-ipa-mem-report",
    "-f[no-]stack-usage",
    "-fcallgraph-info*",
    "-f[no-]dump-*",
    "-fada-spec-parent=*",
    "-f[no-]verbose-asm",
    "-f[no-]record-gcc-switches",
    "-f[no-]record-command-line",
    "-f[no-]ident",
    "-f[no-]working-directory",
    "-f[no-]checking*",
    "-f[no-]compare-debug*",
    "-frandom-seed*",
    "-fdbg-cnt*",
    "-freport-bug",
    "-fno-temp-file",
    "-f[no-]integrated-cc1",
    "-f[no-]integrated-as",
    "-f[no-]integrated-objemitter",
    "-f[no-]verify-intermediate-code",
    "-f[no-]discard-value-names",
    "-f[no-]canonical-system-headers",
    "-fcanon-prefix-map",
    "-f[no-]minimize-whitespace",
    "-f[no-]use-line-directives",
    "-f[no-]preserve-as-comments",
    "-fparse-all-comments",
    "-fretain-comments-from-system-headers",
    "-fcomment-block-commands=",
    "-fno-rewrite-includes",
    "-fno-keep-system-includes",
    "-fbuild-session-*",
    "-fenable-tree-*",
    "-fenable-rtl-*",
    "-fenable-ipa-*",
    "-fdisable-tree-*",
    "-fdisable-rtl-*",
    "-fdisable-ipa-*",
    "-f[no-]enable-new-pm-codegen",
    // Debug information, which bcc does not emit.
    "-f[no-]debug-*",
    "-f[no-]dwarf2-cfi-asm",
    "-f[no-]dwarf-directory-asm",
    "-f[no-]eliminate-*",
    "-femit-class-debug-always",
    "-femit-struct-debug-*",
    "-fgnat-encodings=",
    "-f[no-]merge-debug-strings",
    "-f[no-]var-tracking*",
    "-f[no-]standalone-debug",
    "-f[no-]split-dwarf-inlining",
    "-f[no-]force-dwarf-frame",
    "-ffile-compilation-dir=",
    "-fcoverage-compilation-dir=",
    "-fcoverage-prefix-map=",
    "-femit-dwarf-unwind=",
    "-f[no-]emit-compact-unwind-non-canonical",
    "-f[no-]extend-variable-liveness*",
    // Optimizations bcc may or may not make, and code generation choices
    // that do not change what the program computes.
    "-f[no-]align-*",
    "-f[no-]allocation-dce",
    "-f[no-]allow-store-data-races",
    "-f[no-]asynchronous-unwind-tables",
    "-f[no-]unwind-tables",
    "-f[no-]auto-inc-dec",
    "-f[no-]avoid-store-forwarding",
    "-f[no-]bit-tests",
    "-f[no-]branch-*",
    "-f[no-]btr-bb-exclusive",
    "-f[no-]caller-saves",
    "-f[no-]code-hoisting",
    "-f[no-]combine-stack-adjustments",
    "-f[no-]compare-elim",
    "-f[no-]conserve-stack",
    "-f[no-]cprop-registers",
    "-f[no-]crossjumping",
    "-f[no-]cse-*",
    "-f[no-]dce",
    "-f[no-]defer-pop",
    "-f[no-]delayed-branch",
    "-f[no-]delete-dead-exceptions",
    "-f[no-]dse",
    "-f[no-]early-inlining",
    "-f[no-]expensive-optimizations",
    "-f[no-]ext-dce",
    "-f[no-]fold-mem-offsets",
    "-f[no-]fold-simple-inlines",
    "-f[no-]force-addr",
    "-f[no-]forward-propagate",
    "-f[no-]function-cse",
    "-fno-function-sections",
    "-fno-data-sections",
    "-f[no-]gcse*",
    "-f[no-]graphite*",
    "-f[no-]guess-branch-probability",
    "-f[no-]hoist-adjacent-loads",
    "-f[no-]if-conversion*",
    "-f[no-]indirect-inlining",
    "-f[no-]inline*",
    "-f[no-]ipa-*",
    "-f[no-]ira-*",
    "-f[no-]isolate-erroneous-paths-*",
    "-f[no-]ivopts",
    "-f[no-]jump-tables",
    "-f[no-]keep-inline-functions",
    "-f[no-]keep-static-consts",
    "-f[no-]keep-static-functions",
    "-f[no-]keep-gc-roots-live",
    "-f[no-]keep-persistent-storage-variables",
    "-f[no-]late-combine-instructions",
    "-f[no-]limit-function-alignment",
    "-fmin-function-alignment=",
    "-fpreferred-function-alignment=",
    "-f[no-]live-patching*",
    "-f[no-]live-range-shrinkage",
    "-f[no-]loop-*",
    "-f[no-]lra-remat",
    "-f[no-]lto*",
    "-f[no-]fat-lto-objects",
    "-f[no-]use-linker-plugin",
    "-f[no-]whole-program",
    "-f[no-]malloc-dce*",
    "-fmerge-constants",
    "-f[no-]merge-all-constants",
    "-f[no-]modulo-sched*",
    "-f[no-]move-loop-*",
    "-f[no-]omit-frame-pointer",
    "-f[no-]optimize-*",
    "-f[no-]partial-inlining",
    "-f[no-]peel-loops",
    "-f[no-]peephole*",
    "-f[no-]plt",
    "-f[no-]predictive-commoning",
    "-f[no-]prefetch-loop-arrays",
    "-f[no-]printf-return-value",
    "-f[no-]free",
    "-f[no-]ree",
    "-f[no-]regmove",
    "-f[no-]rename-registers",
    "-f[no-]reorder-*",
    "-f[no-]rerun-*",
    "-f[no-]reschedule-modulo-scheduled-loops",
    "-f[no-]sched-*",
    "-f[no-]sched2-*",
    "-f[no-]schedule-*",
    "-f[no-]section-anchors",
    "-f[no-]see",
    "-f[no-]sel-sched-*",
    "-f[no-]selective-scheduling*",
    "-f[no-]shrink-wrap*",
    "-fsimd-cost-model=",
    "-f[no-]split-ivs-in-unroller",
    "-f[no-]split-loops",
    "-f[no-]split-paths",
    "-f[no-]split-wide-types*",
    "-f[no-]ssa-backprop",
    "-f[no-]ssa-phiopt",
    "-f[no-]stdarg-opt",
    "-f[no-]store-merging",
    "-f[no-]strength-reduce",
    "-f[no-]thread-jumps",
    "-f[no-]tracer",
    "-f[no-]tree-*",
    "-f[no-]unroll-*",
    "-f[no-]unsafe-loop-optimizations",
    "-f[no-]unswitch-loops",
    "-f[no-]variable-expansion-in-unroller",
    "-f[no-]vect-cost-model*",
    "-f[no-]version-loops-for-strides",
    "-f[no-]web",
    "-f[no-]zee",
    "-f[no-]unconstrained-commons",
    "-f[no-]unreachable-traps",
    "-ffinite-loops",
    "-f[no-]vpt",
    "-f[no-]profile-use*",
    "-f[no-]auto-profile*",
    "-f[no-]branch-probabilities",
    "-f[no-]profile-correction",
    "-f[no-]profile-partial-training",
    "-f[no-]profile-reorder-functions",
    "-fprofile-dir=",
    "-fprofile-prefix-*",
    "-f[no-]profile-abs-path",
    "-fprofile-exclude-files=",
    "-fprofile-filter-files=",
    "-fprofile-note=",
    "-f[no-]profile-report",
    "-fprofile-reproducible=",
    "-fprofile-update=",
    "-fprofile-info-section*",
    "-f[no-]profile-instr-use*",
    "-f[no-]profile-sample-use*",
    "-f[no-]profile-sample-accurate",
    "-fprofile-remapping-file=",
    "-fprofile-list=",
    "-f[no-]sample-profile-use-profi",
    "-f[no-]pseudo-probe-for-profiling",
    "-f[no-]addrsig",
    "-f[no-]unique-section-names",
    "-f[no-]unique-basic-block-section-names",
    "-f[no-]unique-internal-linkage-names",
    "-fbasic-block-sections=",
    "-f[no-]basic-block-address-map",
    "-f[no-]separate-named-sections",
    "-f[no-]partition-static-data-sections",
    "-f[no-]stack-size-section",
    "-f[no-]split-machine-functions",
    "-f[no-]split-lto-unit",
    "-f[no-]unified-lto",
    "-flto-jobs=",
    "-flto-partitions=",
    "-f[no-]global-isel",
    "-f[no-]experimental-isel",
    "-f[no-]experimental-loop-fusion",
    "-f[no-]vectorize",
    "-f[no-]slp-vectorize",
    "-f[no-]fine-grained-bitfield-accesses",
    "-finline-hint-functions",
    "-finline-max-stacksize=",
    "-f[no-]use-init-array",
    "-f[no-]direct-access-external-data",
    "-f[no-]zero-initialized-in-bss",
    "-fuse-ld=",
    "-f[no-]openmp-simd",
    // Permissions bcc need not use, and the floating-point behavior bcc
    // already has: no contraction, no excess precision on x86-64, errno set
    // by the math functions, IEEE results. bcc's own command line takes
    // `-ffast-math`, `-fno-signed-zeros` and their relatives, which the
    // personalities pass on before consulting these lists.
    "-fno-rounding-math",
    "-f[no-]protect-parens",
    "-fcomplex-arithmetic=",
    "-flimited-precision=",
    "-fveclib=",
    "-ffp-exception-behavior=ignore",
    "-ffp-eval-method=source",
    "-fdenormal-fp-math=ieee",
    "-fpermitted-flt-eval-methods=",
    "-fexcess-precision=",
    "-f[no-]float-store",
    "-ffp-contract=",
    "-f[no-]fp-int-builtin-inexact",
    "-fstrict-float-cast-overflow",
    // What bcc already does.
    "-f[no-]strict-aliasing",
    "-f[no-]struct-path-tbaa",
    "-f[no-]pointer-tbaa",
    "-faggressive-loop-optimizations",
    "-fno-semantic-interposition",
    "-fstrict-bool",
    "-f[no-]assumptions",
    "-fno-strict-volatile-bitfields",
    "-fsso-struct=native",
    "-fsso-struct=little-endian",
    "-fstack-reuse=all",
    "-fargument-alias",
    "-fargument-noalias*",
    "-ftoplevel-reorder",
    "-funit-at-a-time",
    "-fno-short-enums",
    "-fno-short-wchar",
    "-fsigned-bitfields",
    "-fno-unsigned-bitfields",
    "-freg-struct-return",
    "-fno-pcc-struct-return",
    "-fno-merge-all-constants",
    "-fextended-identifiers",
    "-fdigraphs",
    "-fno-raw-string-literals",
    "-ftrampolines",
    "-fno-trampolines",
    "-ftrampoline-impl=",
    "-f[no-]force-enable-int128",
    "-fno-emulated-tls",
    "-fno-strict-flex-arrays",
    "-fno-pack-derived",
    // Acceptance: the options change only which programs are accepted,
    // and a program bcc refuses is reported, not miscompiled.
    "-fallow-parameterless-variadic-functions",
    "-f[no-]cond-mismatch",
    "-f[no-]lax-vector-conversions*",
    "-fheinous-gnu-extensions",
    "-f[no-]asm",
    "-f[no-]gnu-keywords",
    "-f[no-]gnu-inline-asm",
    "-f[no-]double-square-bracket-attributes",
    "-f[no-]defer-ts",
    "-f[no-]named-loops",
    "-frequire-return-statement",
    "-f[no-]allow-editor-placeholders",
    // Turning off what bcc does not have.
    "-fno-ms-*",
    "-fno-borland-extensions",
    "-fno-blocks",
    "-fno-asm-blocks",
    "-fno-pascal-strings",
    "-fno-fixed-point",
    "-fno-zvector",
    "-fno-altivec",
    "-fno-apple-pragma-pack",
    "-fno-xl-pragma-pack",
    "-fno-objc*",
    "-fno-cuda*",
    "-fno-gpu*",
    "-fno-hip*",
    "-fno-sycl",
    "-fno-offload*",
    "-fno-openmp*",
    "-fnoopenmp*",
    "-fno-openacc",
    "-fno-ptrauth*",
    "-fno-xray*",
    "-fno-profile-*",
    "-fno-coverage*",
    "-fno-test-coverage",
    "-fno-instrument-functions",
    "-fno-split-stack",
    "-fno-stack-check",
    "-fno-stack-limit",
    "-fno-jmc",
    "-fno-memory-profile",
    "-fno-modules*",
    "-fno-module-*",
    "-fno-implicit-module*",
    "-fno-prebuilt-implicit-modules",
    "-fno-pch-*",
    "-fno-sanitize*",
    "-fsanitize-*",
    "-fno-exceptions",
    "-fno-non-call-exceptions",
    "-fno-async-exceptions",
    "-fno-stack-protector",
    "-fcf-protection=none",
    "-fno-stack-clash-protection",
    "-ftrivial-auto-var-init=uninitialized",
    "-fzero-call-used-regs=skip",
    "-fno-hardened",
    "-fno-harden-*",
    "-fno-hardcfr-*",
    "-fstrub=disable",
    // C++ only.
    "-f[no-]rtti*",
    "-f[no-]cxx-*",
    "-f[no-]c++-*",
    "-f[no-]elide-constructors",
    "-f[no-]access-control",
    "-f[no-]sized-deallocation",
    "-f[no-]aligned-allocation",
    "-f[no-]threadsafe-statics",
    "-f[no-]use-cxa-atexit",
    "-f[no-]register-global-dtors-with-atexit",
    "-f[no-]operator-names",
    "-f[no-]delayed-template-parsing",
    "-f[no-]char8_t",
    "-f[no-]coroutines",
    "-f[no-]coro-aligned-allocation",
    "-fmodules-ts",
    "-fmodule-header*",
    "-fabi-version=",
    "-fclang-abi-compat=",
    "-f[no-]check-new",
    "-f[no-]nothrow-opt",
    "-f[no-]strict-enums",
    "-f[no-]strict-return",
    "-ftemplate-depth*",
    "-fconstexpr-depth=",
    "-fconstexpr-steps=",
    "-foperator-arrow-depth=",
    "-fnew-alignment=",
    "-f[no-]force-emit-vtables",
    "-f[no-]whole-program-vtables",
    "-f[no-]virtual-function-elimination",
    "-f[no-]strict-vtable-pointers",
    "-f[no-]assume-*",
    "-f[no-]complete-member-pointers",
    "-f[no-]new-infallible",
    "-f[no-]visibility-inlines-hidden*",
    "-fvisibility-global-new-delete*",
    "-f[no-]devirtualize*",
    "-f[no-]experimental-relative-c++-abi-vtables",
    "-f[no-]lifetime-dse*",
    "-f[no-]gnu-unique",
    "-f[no-]experimental-library",
    "-f[no-]skip-odr-check-in-gmf",
    "-f[no-]reflection",
    "-f[no-]pch-instantiate-templates",
    "-fvtable-verify=",
    "-f[no-]vtv-*",
];

/// Hardening and instrumentation a correct program does not depend on:
/// ignored with a warning that bcc's output lacks them.
const WARNED: &[(&str, &str)] = &[
    ("-fstack-protector*", "bcc does not emit stack protectors"),
    (
        "-fstack-clash-protection",
        "bcc does not probe large stack allocations",
    ),
    (
        "-fstack-check*",
        "bcc does not probe large stack allocations",
    ),
    ("-fstack-limit*", "bcc does not check the stack limit"),
    (
        "-fcf-protection*",
        "bcc does not emit control-flow protection",
    ),
    ("-fhardened", "bcc does not apply GCC's hardening"),
    ("-fharden-*", "bcc does not apply GCC's hardening"),
    ("-fhardcfr-*", "bcc does not apply GCC's hardening"),
    ("-fstrub=*", "bcc does not scrub stack frames"),
    (
        "-ftrivial-auto-var-init*",
        "bcc does not initialize automatic variables",
    ),
    (
        "-fzero-call-used-regs=",
        "bcc does not zero registers on return",
    ),
    (
        "-fexceptions",
        "bcc emits no landing pads, so cleanups do not run while an exception unwinds",
    ),
    (
        "-fhandle-exceptions",
        "bcc emits no landing pads, so cleanups do not run while an exception unwinds",
    ),
    (
        "-ffunction-sections",
        "bcc places functions in one section, so the linker's --gc-sections removes less",
    ),
    (
        "-fdata-sections",
        "bcc places data in one section, so the linker's --gc-sections removes less",
    ),
];

/// The treatment of a `-m` option for `target`. bcc generates baseline
/// x86-64 code (with x87, MMX, SSE and SSE2), tuned for no processor, with
/// the target's ABI and the small code model. Asking for a newer processor
/// warns rather than fails: baseline code is correct on it.
fn machine(option: &str, target: Target) -> Treatment {
    if let Some(treatment) = abi(option, target) {
        return treatment;
    }
    let name = &option[2..];
    let (negative, feature) = match name.strip_prefix("no-") {
        | Some(feature) => (true, feature),
        | None => (false, name),
    };
    // bcc's code uses no MMX register, and its MMX intrinsics work in SSE
    // registers, as GCC's and Clang's do on x86-64; the option changes only
    // whether `__MMX__` is defined, which the GCC and Clang personalities
    // follow.
    if option == "-mno-mmx" {
        return Treatment::Ignore;
    }
    if BASELINE.contains(&feature) {
        return if negative {
            Treatment::Reject("bcc's x86-64 code uses the baseline instruction set")
        } else {
            Treatment::Ignore
        };
    }
    if feature.starts_with("avx") || feature.starts_with("amx") || EXTENSIONS.contains(&feature) {
        return if negative {
            Treatment::Ignore
        } else {
            Treatment::Warn(
                "bcc generates baseline x86-64 code and does not define the extension's macros",
            )
        };
    }
    if let Some(architecture) = option.strip_prefix("-march=") {
        // The baseline under its names; a newer processor runs its code.
        return if matches!(
            architecture,
            "x86-64" | "x86-64-v1" | "k8" | "opteron" | "athlon64"
        ) {
            Treatment::Ignore
        } else {
            Treatment::Warn(
                "bcc generates baseline x86-64 code (-march=x86-64) and does not define the \
                 architecture's macros",
            )
        };
    }
    if MACHINE_IGNORED
        .iter()
        .any(|pattern| matches(pattern, option))
    {
        return Treatment::Ignore;
    }
    if let Some(&(_, why)) = MACHINE_WARNED
        .iter()
        .find(|(pattern, _)| matches(pattern, option))
    {
        return Treatment::Warn(why);
    }
    Treatment::Reject(UNSUPPORTED)
}

/// The treatment of a `-m` option whose meaning depends on the target's
/// ABI: one that asks for what the target already does is ignored, one
/// that asks for another target's layout or calling convention is refused.
fn abi(option: &str, target: Target) -> Option<Treatment> {
    let windows = matches!(target, Target::WindowsGnu | Target::WindowsMsvc);
    let msvc = target == Target::WindowsMsvc;
    let follows = |matches: bool, why: &'static str| {
        Some(if matches {
            Treatment::Ignore
        } else {
            Treatment::Reject(why)
        })
    };
    match option {
        | "-mms-bitfields" => follows(
            windows,
            "bit-fields follow the System V layout on this target",
        ),
        | "-mno-ms-bitfields" => follows(
            !windows,
            "bit-fields follow the Microsoft layout on this target",
        ),
        | "-mlong-double-80" | "-m128bit-long-double" =>
            follows(!msvc, "long double is 64 bits wide on the MSVC target"),
        | "-mlong-double-64" => follows(msvc, "long double is the 80-bit x87 type on this target"),
        | "-mno-red-zone" => follows(
            windows,
            "System V code may use the red zone below the stack pointer",
        ),
        // Windows code uses no red zone, which is always safe.
        | "-mred-zone" => Some(Treatment::Ignore),
        | "-mabi=sysv" => follows(!windows, "the target's calling convention is Microsoft's"),
        | "-mabi=ms" => follows(windows, "the target's calling convention is System V's"),
        | "-mstack-arg-probe" => follows(windows, "bcc probes no stack frames on this target"),
        | "-mno-stack-arg-probe" => follows(!windows, "Windows code probes large stack frames"),
        | "-mglibc" => follows(
            target == Target::LinuxGnu,
            "the target's C library is not glibc",
        ),
        | "-mmusl" => follows(
            target == Target::LinuxMusl,
            "the target's C library is not musl",
        ),
        | "-mconsole" | "-mthreads" => follows(
            target == Target::WindowsGnu,
            "it is a MinGW-w64 option, and the target is not MinGW-w64",
        ),
        | _ => None,
    }
}

/// The instruction set every x86-64 processor has, which bcc uses.
const BASELINE: [&str; 6] = ["80387", "x87", "mmx", "sse", "sse2", "fxsr"];

/// Instruction set extensions beyond the baseline, other than the `avx*`
/// and `amx*` families.
const EXTENSIONS: &[&str] = &[
    "3dnow",
    "3dnowa",
    "abm",
    "adx",
    "aes",
    "apxf",
    "bmi",
    "bmi2",
    "cldemote",
    "clflushopt",
    "clwb",
    "clzero",
    "cmpccxadd",
    "crc32",
    "cx16",
    "enqcmd",
    "evex512",
    "f16c",
    "fma",
    "fma4",
    "fsgsbase",
    "gfni",
    "hle",
    "hreset",
    "invpcid",
    "kl",
    "lwp",
    "lzcnt",
    "movbe",
    "movdir64b",
    "movdiri",
    "movrs",
    "mpx",
    "mwait",
    "mwaitx",
    "pclmul",
    "pcommit",
    "pconfig",
    "pku",
    "popcnt",
    "prefetchi",
    "prfchw",
    "ptwrite",
    "raoint",
    "rdpid",
    "rdpru",
    "rdrnd",
    "rdseed",
    "rtm",
    "sahf",
    "serialize",
    "sgx",
    "sha",
    "sha512",
    "shstk",
    "sm3",
    "sm4",
    "sse3",
    "sse4",
    "sse4.1",
    "sse4.2",
    "sse4a",
    "sse5",
    "ssse3",
    "tbm",
    "tsxldtrk",
    "uintr",
    "usermsr",
    "vaes",
    "vpclmulqdq",
    "waitpkg",
    "wbnoinvd",
    "widekl",
    "xop",
    "xsave",
    "xsavec",
    "xsaveopt",
    "xsaves",
];

/// `-m` options with no effect on what bcc's output means.
const MACHINE_IGNORED: &[&str] = &[
    "-m64",
    "-mfancy-math-387",
    "-mno-fancy-math-387",
    "-mieee-fp",
    "-mhard-float",
    "-mno-soft-float",
    "-mfp-ret-in-387",
    "-mfpmath=sse",
    "-mpc80",
    "-mtune=",
    "-mtune-ctrl=",
    "-mcpu=",
    "-mcmodel=small",
    "-mtls-dialect=",
    "-m[no-]tls-direct-seg-refs",
    "-maccumulate-outgoing-args",
    "-mno-accumulate-outgoing-args",
    "-m[no-]push-args",
    "-m[no-]align-stringops",
    "-minline-all-stringops",
    "-minline-stringops-dynamically",
    "-mmemcpy-strategy=",
    "-mmemset-strategy=",
    "-mstringop-strategy=",
    "-mbranch-cost=",
    "-mprefer-vector-width=",
    "-m[no-]prefer-avx128",
    "-mmove-max=",
    "-mstore-max=",
    "-m[no-]avx256-split-unaligned-*",
    "-m[no-]8bit-idiv",
    "-m[no-]stv",
    "-m[no-]vzeroupper",
    "-m[no-]dispatch-scheduler",
    "-m[no-]unroll-only-small-loops",
    "-m[no-]relax-cmpxchg-loop",
    "-m[no-]partial-vector-fp-math",
    "-malign-functions=",
    "-malign-jumps=",
    "-malign-loops=",
    "-malign-data=",
    "-malign-branch=",
    "-malign-branch-boundary=",
    "-mbranches-within-32B-boundaries",
    "-mpad-max-prefix-size=",
    "-m[no-]omit-leaf-frame-pointer",
    "-m[no-]gather",
    "-m[no-]scatter",
    "-m[no-]recip",
    "-mrecip=",
    "-m[no-]fused-madd",
    "-mneeded",
    "-mdump-tune-features",
    "-m[no-]call-ms2sysv-xlogues",
    "-m[no-]noreturn-no-callee-saved-registers",
    "-m[no-]force-indirect-call",
    "-m[no-]direct-extern-access",
    "-masm=att",
    "-mincoming-stack-boundary=4",
    "-mpreferred-stack-boundary=4",
    "-mno-daz-ftz",
    "-mno-rtd",
    "-mno-iamcu",
    "-mno-stackrealign",
    "-mno-skip-rax-setup",
    "-mno-retpoline*",
    "-mindirect-branch=keep",
    "-mfunction-return=keep",
    "-mharden-sls=none",
    "-mno-seses",
    "-mstack-protector-guard*",
    "-mstack-probe-size=",
];

/// `-m` hardening and instrumentation, ignored with a warning.
const MACHINE_WARNED: &[(&str, &str)] = &[
    (
        "-mindirect-branch*",
        "bcc does not emit Spectre mitigations",
    ),
    (
        "-mfunction-return=",
        "bcc does not emit Spectre mitigations",
    ),
    ("-mharden-sls=", "bcc does not emit Spectre mitigations"),
    ("-mretpoline*", "bcc does not emit Spectre mitigations"),
    ("-mseses", "bcc does not emit Spectre mitigations"),
    ("-mmitigate-rop", "bcc does not emit ROP mitigations"),
    ("-mcet-switch", "bcc does not emit control-flow protection"),
    (
        "-mmanual-endbr",
        "bcc does not emit control-flow protection",
    ),
    ("-mfentry*", "bcc does not instrument functions for tracing"),
    (
        "-mrecord-mcount",
        "bcc does not instrument functions for tracing",
    ),
    (
        "-mnop-mcount",
        "bcc does not instrument functions for tracing",
    ),
    (
        "-mrecord-return",
        "bcc does not instrument functions for tracing",
    ),
    (
        "-minstrument-return=",
        "bcc does not instrument functions for tracing",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_options_are_refused_and_harmless_ones_ignored() {
        let linux = |option| treatment(option, Target::LinuxGnu);
        for refused in [
            "-fshort-enums",
            "-fexec-charset=latin1",
            "-fsplit-stack",
            "-fsanitize=address",
            "-fprofile-arcs",
            "-fopenmp",
            "-mno-sse",
            "-mno-sse2",
            "-mgeneral-regs-only",
            "-mno-red-zone",
            "-mcmodel=kernel",
            "-mlong-double-64",
            "-mregparm=3",
            "-mms-bitfields",
            "-mabi=ms",
        ] {
            assert!(matches!(linux(refused), Treatment::Reject(_)), "{refused}");
        }
        for ignored in [
            "-fdiagnostics-color=always",
            "-fno-diagnostics-show-caret",
            "-fmessage-length=0",
            "-fdebug-prefix-map=/a=/b",
            "-fno-omit-frame-pointer",
            "-fno-strict-aliasing",
            "-fstrict-aliasing",
            "-fvisibility-inlines-hidden",
            "-flto=auto",
            "-ffat-lto-objects",
            "-fasynchronous-unwind-tables",
            "-fno-stack-protector",
            "-fcf-protection=none",
            "-ftree-vectorize",
            "-fno-tree-vectorize",
            "-free",
            "-ffp-contract=off",
            "-fexcess-precision=standard",
            "-fno-sanitize=all",
            "-finput-charset=UTF-8",
            "-mtune=generic",
            "-march=x86-64",
            "-march=x86-64-v1",
            "-m64",
            "-msse2",
            "-mno-avx",
            "-mno-avx512f",
            "-mno-sse4.2",
            "-mno-mmx",
            "-mtls-dialect=gnu2",
            "-mno-omit-leaf-frame-pointer",
            "-mlong-double-80",
            "-mno-ms-bitfields",
            "-mabi=sysv",
        ] {
            assert_eq!(linux(ignored), Treatment::Ignore, "{ignored}");
        }
        for warned in [
            "-fstack-protector-strong",
            "-fstack-clash-protection",
            "-fcf-protection",
            "-fcf-protection=full",
            "-fexceptions",
            "-ftrivial-auto-var-init=zero",
            "-ffunction-sections",
            "-fdata-sections",
            "-march=native",
            "-march=x86-64-v3",
            "-mavx2",
            "-msse4.2",
            "-mindirect-branch=thunk",
            "-mfentry",
        ] {
            assert!(matches!(linux(warned), Treatment::Warn(_)), "{warned}");
        }
    }

    #[test]
    fn machine_options_follow_the_target() {
        for (option, linux, mingw, msvc) in [
            ("-mms-bitfields", false, true, true),
            ("-mno-ms-bitfields", true, false, false),
            ("-mlong-double-80", true, true, false),
            ("-mlong-double-64", false, false, true),
            ("-mno-red-zone", false, true, true),
            ("-mabi=ms", false, true, true),
            ("-mabi=sysv", true, false, false),
            ("-mno-stack-arg-probe", true, false, false),
            ("-mconsole", false, true, false),
        ] {
            for (target, ignored) in [
                (Target::LinuxGnu, linux),
                (Target::WindowsGnu, mingw),
                (Target::WindowsMsvc, msvc),
            ] {
                let treatment = treatment(option, target);
                assert_eq!(
                    treatment == Treatment::Ignore,
                    ignored,
                    "{option} {target:?}: {treatment:?}"
                );
                if !ignored {
                    assert!(
                        matches!(treatment, Treatment::Reject(_)),
                        "{option} {target:?}"
                    );
                }
            }
        }
    }
}
