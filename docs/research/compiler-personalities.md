# Which other compilers should bcc mimic?

Research date: 2026-11-19. bcc mimics GCC, Clang, MSVC `cl`, clang-cl and
TCC (`--gcc-mode`, `--clang-mode`, `--msvc-mode`, `--clang-cl-mode`,
`--tcc-mode`); [driver-modes.md](../design/driver-modes.md) records how a
personality works. This note surveys the other C compilers whose personality
could be worth adding, ranks them by value over cost, and gives a concrete
shape for the top two. It implements nothing.

Seven compilers that install without root or a licence were installed under
`build/scratch.personalities/` and run on this x86-64 Linux machine (Ubuntu
24.04, glibc 2.39 headers): zig 0.17.1, cproc, chibicc, slimcc, pcc, Open
Watcom V2 and SDCC 4.6.0. Each was asked for `--version`, `-v`, `-E -dM` and
the diagnostics for the inputs that driver-modes.md's TCC section compares
(an undeclared identifier, a missing `;`, a missing member, too few
arguments, a redeclaration, a non-lvalue, a missing header, `break` and
`goto` out of place, a pointer from an integer, a missing source and an
unknown option). The outputs, the inputs and the script are in
[`compiler-personalities-data/`](compiler-personalities-data/), with local
paths replaced by `<scratch>`. `scripts/build_system_probes.py` was also run
against zig cc, cproc, slimcc and pcc, with CMake 3.31.3 and Autoconf 2.71
([`build-system-probes.txt`](compiler-personalities-data/build-system-probes.txt)).
The compilers that need a licence accepted were not downloaded. Their facts
come from vendor documentation and source, cited where they are used.

## Recommendation

What a personality buys is set by three things: whether people build real
code with that compiler on a target bcc has (x86-64 Linux or Windows, or
wasm32), whether build systems and headers take a different path for it, and
whether its command line differs enough that GCC's or Clang's personality
cannot stand in. What it costs is set by how far its driver is from one bcc
already reads, and whether a reference compiler can be run to check parity,
which is how every existing personality was built.

| Rank | Compiler | Value | Cost | Testable here | Verdict |
| --- | --- | --- | --- | --- | --- |
| 1 | Intel oneAPI `icx` / `icx-cl` | Medium-high: Intel's current compiler for HPC and numerical code; CMake `IntelLLVM`, Meson `intel-llvm`; code tests `__INTEL_LLVM_COMPILER` | Small: Clang's and clang-cl's personalities plus an identity, a banner and an Intel option table (~600-900 lines) | No: needs Intel's EULA accepted (the user's decision) | **Add**, after the user decides on a reference compiler |
| 2 | `zig cc` | Medium: a popular hermetic cross compiler (cargo-zigbuild, Uber's hermetic_cc_toolchain, cgo) | Small: Clang's personality plus zig's triples, `-mcpu`, its build-mode defaults and its own errors (~500-800 lines) | Yes: a free tarball, surveyed here | **Add** |
| 3 | NVIDIA HPC `nvc` | Medium in HPC, low elsewhere | Medium-large: its own PGI-derived driver and option set | No: NVIDIA's EULA | Later, only with demand |
| 4 | cproc, slimcc, chibicc | Low: few users; no CMake identity; their value is portability checking | Small-medium each, but each has its own driver and messages | Yes | Not as personalities; see [A strict mode instead](#a-strict-mode-instead) |
| — | IBM Open XL, `armclang`, SDCC, Open Watcom, AppleClang, TI, Fujitsu, Cray | Real users, but on targets bcc does not have | Large: a new target first | Some | No, until bcc has the target |
| — | Emscripten `emcc` | High for wasm, but its value is the JavaScript runtime and linker | Very large: emulating emcc means its JS glue | Yes (emsdk) | No |
| — | Classic Intel `icc`, Oracle `cc`, pcc, lcc, Plan 9 `kencc`, CompCert, kefir | Discontinued, frozen, or niche; or licence-encumbered | Medium | Partly | No |

The two recommended are both Clang-derived, so most of each personality is
already written: Clang's command line, diagnostics and rules. What is new is
a short identity and the options that only that driver has.

### 1. `--icx-mode` (and `--icx-cl-mode`)

**What icx is.** Intel's oneAPI DPC++/C++ Compiler, the LLVM-based
successor to the classic `icc`, which Intel discontinued in oneAPI 2024.0
(its last version, 2021.10, shipped in oneAPI 2023.2;
[release notes](https://www.intel.com/content/www/us/en/developer/articles/release-notes/oneapi-c-compiler-release-notes.html)).
The latest is 2026.2.0 of 2026-10-22
([release notes](https://www.intel.com/content/www/us/en/developer/articles/release-notes/oneapi-dpcpp/2026.html)).
On Linux `icx` takes Clang's options; on Windows `icx` and `icx-cl` take
`cl`'s, and `icx-cc` Clang's
([invoking the compiler](https://www.intel.com/content/www/us/en/docs/dpcpp-cpp-compiler/developer-guide-reference/2026-2/invoke-the-compiler.html)).
It "accepts all Clang options", translates the classic `icc` options it can,
and warns about the rest with `command line warning #10430`
([porting guide](https://www.intel.com/content/www/us/en/developer/articles/guide/porting-guide-for-icc-users-to-dpcpp-or-icx.html)).
Its diagnostics are Clang's, without numbers.

**Why it is first.** Of the compilers left, it is the one with many users on
targets bcc already has (x86-64 Linux and Windows), its own identity in both
CMake (`IntelLLVM`, tested before Clang) and Meson (`intel-llvm`), and code
that branches on it: `__INTEL_LLVM_COMPILER` is the documented test
([predefined macros](https://www.intel.com/content/www/us/en/docs/dpcpp-cpp-compiler/developer-guide-reference/2026-2/additional-predefined-macros.html)).
And because it is Clang, a personality is mostly Clang's and clang-cl's.

**Identity.** Clang's macros plus `__INTEL_LLVM_COMPILER` as `VVVVMMUU`
(2026.2.0 is `20260200`); `__GNUC__` 4.2.1 on Linux and `_MSC_VER` on
Windows, as Clang gives them (Intel's macro table). CMake's test also reads
`__INTEL_CLANG_COMPILER` with `__clang__`; whether icx defines either was
not confirmed from Intel's documentation, so a personality should take both
from a real `icx -dM -E`. `--version` prints `Intel(R) oneAPI DPC++/C++
Compiler 2026.2.0 (2026.2.0.YYYYMMDD)` in the form Intel documents for
`__VERSION__`
([Intel article](https://www.intel.com/content/www/us/en/developer/articles/technical/use-predefined-macros-for-specific-code-for-intel-dpcpp-compiler-intel-cpp-compiler-intel-cpp-compiler-classic.html)).
Meson tests for `clang` before `Intel(R) oneAPI`, so the banner must not
contain the word `clang` (or `Clang`); bcc's Clang banner does. On Windows
Meson looks for `Intel(R) oneAPI DPC++/C++ Compiler for applications` on
stderr.

**Defaults that differ from Clang.** `-O2` with no `-O` option, but `-O0`
when `-g` is given alone
([`-O`](https://www.intel.com/content/www/us/en/docs/dpcpp-cpp-compiler/developer-guide-reference/2026-2/o-001.html)),
and `-fp-model=fast` (`/fp:fast`): "Unlike Clang", icx enables value-unsafe
floating-point optimizations by default
([`-fp-model`](https://www.intel.com/content/www/us/en/docs/dpcpp-cpp-compiler/developer-guide-reference/2026-2/fp-model-fp.html)).
bcc already implements `-ffast-math` and its parts through the LLVM back
end (driver-modes.md, "Options a compiler knows"), so both defaults map onto
existing options; the native back end, which forms no fast-math, would keep
IEEE semantics, which the fast model permits.

**Shape.**

- Flags `--icx-mode` and `--icx-cl-mode`; executable names `icx` (and
  `icx-cc`) and `icx-cl`. Windows' `icx` takes `cl`'s options, so an
  executable named `icx` selects the cl-style personality when bcc runs on
  Windows, as Intel's own drivers differ by host.
- **Reused:** the Clang personality (`gnu.rs`) for `icx`, the clang-cl
  personality (`msvc.rs`) for `icx-cl`, Clang's rules under the
  [identity policy](../design/driver-modes.md#identity-policy) and Clang's
  diagnostics. The identity could be Clang's with an Intel flavour that adds
  the macros and changes the banner, rather than a sixth
  `CompilerIdentity`, so the 40-odd files that match on `CompilerIdentity`
  stay as they are.
- **New:** the identity macros and banner; the `-O2` and `-fp-model=fast`
  defaults; `-fp-model=fast|precise|strict` and `/fp:` mapped
  onto bcc's floating options (`strict` refused, as bcc refuses
  `-frounding-math`); a table of Intel's own options with the three kinds
  bcc's personalities use (ignored, mapped, refused): `-x`/`-ax` CPU codes
  and `-march=` names onto bcc's `-march=` levels, `-ipo` and `-qopt-*`
  ignored as optimizations bcc may skip, `-qopenmp` and `-fiopenmp` refused
  as `-fopenmp` is, `-fsycl` refused, and the classic `icc` spellings with
  Intel's `#10430` warning. Rough size: 600-900 lines with tests, mostly the
  option table, which can be frozen from `icx -help` as the GCC and Clang
  lists are.
- **What blocks it.** Downloading or using icx means accepting Intel's End
  User License Agreement
  ([EULA](https://www.intel.com/content/www/us/en/developer/articles/license/end-user-license-agreement.html)),
  even through the public apt repository or pip
  ([download page](https://www.intel.com/content/www/us/en/developer/tools/oneapi/dpc-compiler-download.html)).
  It was not downloaded for this note. Every bcc personality was checked
  against the real compiler, so the user should decide whether to accept the
  EULA for a reference `icx` (a CI job could install it from Intel's apt
  repository); without one, the identity would rest on documentation alone,
  and this note would rank zig cc first instead.
- **Since added.** The user accepted the EULA on 2026-11-19, and
  `--icx-mode` and `--icx-cl-mode` were built against a real icx 2026.2.0
  ([driver modes, "icx"](../design/driver-modes.md#icx)). That comparison
  settled the open points above: icx defines `__INTEL_CLANG_COMPILER` beside
  `__INTEL_LLVM_COMPILER`, both `20260200`, and not `__INTEL_COMPILER`.

### 2. `--zig-cc-mode`

**What zig cc is.** `zig cc` is Clang's driver linked into the `zig`
executable, with zig's own front end for the command line
(`src/main.zig`), its own target syntax, and a bundled libc for every
target it supports: musl, glibc stubs for a chosen version (`.2.17` was
accepted here), MinGW-w64 and WASI. zig 0.17.1 bundles Clang 22.1.8.
It is the C toolchain behind cargo-zigbuild and Uber's
hermetic_cc_toolchain for Bazel
([README](https://github.com/uber/hermetic_cc_toolchain): "a C/C++ toolchain
that can (cross-)compile C/C++ programs on top of `zig cc`", offered for
"C/C++/CGo projects").

**Identity: Clang's, unchanged.** `zig cc --version` prints `clang version
22.1.8`, and with `-mcpu=baseline`, `-dM -E` lists the pinned Clang
23.1.5's macros but for the version, four that Clang 23 added
(`__SIG_ATOMIC_MIN__`, `__SIG_ATOMIC_TYPE__`, `__WCHAR_MIN__`,
`__WINT_MIN__`), and zig's defaults: `__GLIBC_MINOR__`, `__SSP_STRONG__`
and no `__PIE__`. There is no zig macro. CMake therefore finds `Clang 22.1.8` with a GNU front end,
all compile features to `c_std_23`, `-fPIC`, `-MD` dependency files, and the
same check results as Clang; Autoconf finds a GNU compiler; the probe
project builds and passes its test
([probes](compiler-personalities-data/build-system-probes.txt)). Meson has no
zig case and reads it as Clang
([`detect.py`](https://github.com/mesonbuild/meson/blob/1.12.1/mesonbuild/compilers/detect.py)).
So `--clang-mode` already satisfies every identity check; a zig personality is
about the command line and defaults, not about identity.

**What differs from Clang**, all measured with zig 0.17.1:

| | Clang | zig cc |
| --- | --- | --- |
| Target | `--target=x86_64-unknown-linux-gnu` | `-target x86_64-linux-gnu`, `x86_64-linux-gnu.2.17` (a glibc version), `x86_64-linux-musl`, `x86_64-windows-gnu`, `wasm32-wasi`; a four-part triple is `error: unable to parse target query 'x86_64-unknown-linux-gnu': UnknownOperatingSystem` |
| Default target | host, baseline CPU | host, **native CPU** (`-target-cpu znver5` here: 58 more macros than the baseline CPU, `__AVX512F__` among them), `-D__GLIBC_MINOR__=39` from the host's glibc; with any `-target`, the baseline CPU |
| CPU | `-march=x86-64-v3` | `-mcpu=x86_64_v3` (zig's names, with underscores); `-march=x86-64-v3` is `error: unknown CPU: 'x86'` and a list of CPUs |
| `-O0`, `-Og`, none | nothing added | zig's Debug mode: 17 `-fsanitize=undefined` checks (all but `return` and `unreachable` recoverable), `-stack-protector 2` (`__SSP_STRONG__`), and zig's `libubsan_rt.a` linked |
| `-O1` to `-O3`, `-Ofast` | `__OPTIMIZE__` | ReleaseFast: also `-DNDEBUG` ([`Compilation.zig`](https://codeberg.org/ziglang/zig/src/tag/0.17.1/src/Compilation.zig#L6379-L6387), [`main.zig`](https://codeberg.org/ziglang/zig/src/tag/0.17.1/src/main.zig#L2335)) |
| `-O1` to `-O3`, `-Ofast` with `-fsanitize=undefined` or `-fsanitize-trap=undefined` | `__OPTIMIZE__` | ReleaseSafe ([`main.zig`](https://codeberg.org/ziglang/zig/src/tag/0.17.1/src/main.zig#L3078-L3082)): `-O2 -D_FORTIFY_SOURCE=2`, `-stack-protector 2`, no `NDEBUG` |
| `-Os`, `-Oz` | `__OPTIMIZE_SIZE__` | ReleaseSmall: also `-DNDEBUG`, and no debug information or symbols unless a `-g` option is given ([`Compilation/Config.zig`](https://codeberg.org/ziglang/zig/src/tag/0.17.1/src/Compilation/Config.zig#L495-L500), `root_strip`) |
| `-fPIE` | PIE | PIC only (no `__PIE__`), linked as a PIE; `-fPIE -fno-PIC` is `error: unable to create module "e": making a Position Independent Executable requires enabling Position Independent Code` ([`Module.zig`](https://codeberg.org/ziglang/zig/src/tag/0.17.1/src/Module.zig#L182-L186)) |
| Missing source | `clang: error: no such file or directory: 'missing.c'` | `missing.c:1:1: error: checking cache failed: file_open FileNotFound /abs/path/missing.c` |
| Unknown option | `clang: error: unknown argument: '-fbogus-option'` | `error: unknown clang option: "-fbogus-option"` |
| `-dumpmachine` | the target triple | `x86_64-unknown-linux-musl` on this glibc host |
| Compiler diagnostics | Clang's | the same, word for word |
| Linker | the system's, through the driver | zig's own (LLD-based), `ld.lld -r` in `-v` |

**Shape.**

- Flag `--zig-cc-mode`; selected by name for an executable named `zig-cc`
  or `zigcc`. A program named `zig` whose first argument is `cc` (what
  build systems are given as `CC="zig cc"`) selects it too, and the `cc` is
  dropped; any other first argument (`zig build`, `zig c++`) is refused, as
  bcc builds no Zig.
- **Reused:** the whole Clang personality (`gnu.rs` with Clang's rules,
  option lists and unimplemented-option kinds), Clang's identity and
  diagnostics, with the version macros, banner and `-dumpversion` at zig's
  bundled Clang version.
- **New:** a translator from zig's target queries (`ARCH-OS-ABI[.VERSION]`)
  to bcc's targets, refusing the ones bcc lacks; `-mcpu=` names onto bcc's
  `-march=` levels (`baseline`, `x86_64`, `x86_64_v2`, `x86_64_v3`,
  `native`), with zig's error for a GNU spelling; zig's defaults for each
  `-O` level (UBSan and `-fstack-protector-strong` at Debug,
  `-DNDEBUG` at the release levels), which bcc's existing UBSan and the
  hardening options cover; `-D__GLIBC_MINOR__` from the selected glibc;
  native CPU only when no `-target` is given; and zig's two driver errors.
  Rough size: 500-800 lines with tests, about half of `tcc.rs`.
- **Open questions:** the glibc version suffix is a promise about which
  symbol versions the program links against; bcc links against the host's
  or a `--sysroot`'s C library, so `.2.17` would have to be refused or
  mean only the macro. And zig's native default turns on every feature of
  the build machine (AVX-512 here); the personality can follow it only as
  far as bcc's own `-march=native` does.
- **Parity test:** `BCC_ZIG=/path/to/zig`, as `BCC_TCC` drives the TCC
  comparison.

## Surveyed compilers

Each entry gives status, command line, identity, diagnostics, whether it can
be tested here, and cost and value. "Tested" means installed and run for
this note; the outputs are in
[`compiler-personalities-data/`](compiler-personalities-data/).

### Intel oneAPI `icx` (not installed)

See [the shape above](#1---icx-mode-and---icx-cl-mode).

### Intel classic `icc` (not installed)

Discontinued in oneAPI 2024.0; the last version, 2021.10, prints a
deprecation remark on every run from 2021.7 on
([release notes](https://www.intel.com/content/www/us/en/developer/articles/release-notes/oneapi-c-compiler-release-notes.html)).
Identity `__INTEL_COMPILER` (`VVVV` since 2021) and `__INTEL_COMPILER_UPDATE`;
CMake `Intel`, Meson `(ICC)`. Its diagnostics are numbered
(`unknown-pragma.c(1): warning #161: unrecognized #pragma`, porting guide).
Code still tests `__INTEL_COMPILER`, but claiming a compiler Intel has
withdrawn helps nobody. **No**; an icx personality covers its users.

### NVIDIA HPC SDK `nvc` (not installed)

- **Status.** The successor to PGI's compilers
  ([NVIDIA](https://developer.nvidia.com/legacy-pgi-support)); 26.11 is
  current, on x86-64 and Arm Linux. NVIDIA names VASP, Gaussian, ANSYS
  Fluent, GROMACS and NAMD among the users of its toolchain
  ([HPC SDK](https://developer.nvidia.com/hpc-sdk)). Downloading means
  agreeing to the HPC SDK Software License Agreement, though the tarballs
  and package repositories are public
  ([downloads](https://developer.nvidia.com/hpc-sdk-downloads)).
- **Command line.** Its own PGI-heritage driver with an LLVM back end
  (`__NVCOMPILER_LLVM__`), not Clang's: `-O1` by default, `-fast`,
  `-M<flag>` code-generation options, `-Minfo` reports, `-tp` for the CPU,
  `-mp` and `-acc` for OpenMP and OpenACC, `-dryrun`
  ([reference guide](https://docs.nvidia.com/hpc-sdk/compilers/hpc-compilers-ref-guide/index.html),
  [user guide](https://docs.nvidia.com/hpc-sdk/compilers/hpc-compilers-user-guide/index.html)).
- **Identity.** `__NVCOMPILER`, `__NVCOMPILER_MAJOR__` 26,
  `__NVCOMPILER_MINOR__` 11, `__NVCOMPILER_PATCHLEVEL__`; whether it also
  defines `__PGI` or `__GNUC__`, and the C diagnostic format, were not
  confirmed. CMake `NVHPC`, Meson by `NVIDIA Compilers and Tools`.
- **Cost and value.** A new driver, not a Clang variant, for users who mostly
  want OpenACC, OpenMP offload and CUDA interoperation, none of which bcc
  has. **Later, only with demand.**

### IBM Open XL `ibm-clang` and classic XL C `xlc` (not installed)

- **Status.** Open XL C/C++ for AIX 17.1.4 (Fix Pack 5, 2026-09-25) is
  maintained ([fix list](https://www.ibm.com/support/pages/fix-list-xl-cc-aix));
  it is paid, with a trial or a registration-gated community edition for
  Linux on Power. Classic XL C/C++ 16.1 was withdrawn in January 2025
  ([IBM](https://www.ibm.com/support/pages/node/1272838)).
- **Targets.** AIX and Linux on Power; z/OS has its own edition. No x86-64.
- **Identity.** Open XL: `__clang__`, `__open_xl__` and its version macros,
  and none of the classic ones
  ([Open XL macros](https://www.ibm.com/docs/en/SSRZSMX_17.1.4/user_guide/openxlmacros.html));
  CMake `IBMClang`, Meson reads it as Clang. Classic `xlc`: `__IBMC__`,
  `__xlc__`, `__xlC__`; `xlclang`: `__ibmxl__`
  ([XL macros](https://www.ibm.com/docs/en/SSGH3R_16.1.0/compiler_ref_cpp/xlmacros.html)).
- **Command line.** `ibm-clang` is Clang plus a few `-q` options
  ([options](https://www.ibm.com/docs/en/SSRZSMX_17.1.4/user_guide/options.html)).
- **Verdict.** Cheap as an identity, but useless without a Power target.
  **No.**


### Arm: `armclang`, `armcc`, Arm Toolchain for Embedded, Arm GNU Toolchain (not installed)

- **Status.** Arm Compiler for Embedded 6.24 (2025-03-31) is "the final
  planned feature release" of `armclang`; later updates fix defects only
  ([release note](https://documentation-service.arm.com/static/67e440d14191d739c8e9f11a)).
  `armcc` 5.06u7 was the last Arm Compiler 5 update
  ([Arm blog](https://developer.arm.com/community/arm-community-blogs/b/tools-software-ides-blog/posts/arm-development-studio-2020-1-now-available)).
  Their open replacements are the Arm Toolchain for Embedded (plain Clang,
  LLD and picolibc; 23.1.0 of 2026-09-10,
  [arm/arm-toolchain](https://github.com/arm/arm-toolchain)) and the Arm
  GNU Toolchain (GCC; 15.3.Rel2,
  [gitlab.arm.com](https://gitlab.arm.com/tooling/gnu-toolchains-for-arm)).
- **Licence.** `armclang` and `armcc` are commercial (downloaded "depending on
  your license and entitlements"); the other two are free.
- **Targets.** Arm and AArch64 only; x86-64 is a host, never a target.
- **Identity.** `armclang`: `__ARMCOMPILER_VERSION` (`Mmmuuxx`) and its
  synonym `__ARMCC_VERSION`, `__GNUC__` "always set", Clang's version in
  `__VERSION__`; its `--target` is mandatory
  ([armclang guide](https://documentation-service.arm.com/static/5eecb775b7429076afea07a7)).
  `armcc`: EDG-based, `__CC_ARM`, `__ARMCC_VERSION` (`PVVbbbb`), `__GNUC__`
  only under `--gnu`, diagnostics as `"main.c", line 5: Error:  #65:
  expected a ";"`
  ([armcc guide](https://documentation-service.arm.com/static/5f721c6c1b758617cd950f6f)).
  The open toolchains claim plain Clang or GCC.
- **Verdict.** All need an Arm target first, and the open ones are Clang or
  GCC already. **No.**

### Oracle Developer Studio `cc` (not installed)

- **Status.** 12.6 (C 5.14) of July 2017 is the latest
  ([docs](https://docs.oracle.com/cd/E77782_01/html/E77788/bjapp.html)).
  Solaris on SPARC and x86, and Linux on x86-64. Download needs an Oracle
  account and licence acceptance
  ([installation guide](https://docs.oracle.com/cd/E77782_01/html/E77785/gmidr.html)).
- **Command line.** Its own: `-std=c11` by default (`c89` to `gnu11`), the
  older `-xc99=` and `-Xc`/`-Xa`/`-Xt`/`-Xs` modes, `-errfmt=`,
  `-features=extensions`, `-V`
  ([cc(1)](https://docs.oracle.com/cd/E77782_01/html/E77803/cc-1.html)).
- **Identity.** `__SUNPRO_C 0x5140`, no `__GNUC__`; CMake `SunPro`.
- **Diagnostics.** `"hello.c", line 5: warning: function has no return
  statement: main`
  ([guide](https://docs.oracle.com/cd/E77782_01/html/E77788/bjapr.html)).
- **Verdict.** Frozen for nine years and licence-gated. **No.**

### CompCert `ccomp` (not installed)

- **Status.** 3.18 of 2026-08-30
  ([releases](https://github.com/AbsInt/CompCert/releases)); the formally
  verified compiler, qualified for DO-178C avionics work through AbsInt
  ([compcert.org](https://compcert.org/)). The INRIA Non-Commercial licence
  allows research and evaluation only
  ([LICENSE](https://github.com/AbsInt/CompCert/blob/master/LICENSE)), and
  building needs OCaml and Coq/Rocq, so it was not installed.
- **Command line.** GCC-like with its own opt-ins (`-flongdouble`,
  `-fstruct-passing`, `-fpacked-structs`, `-finline-asm`,
  `-funstructured-switch`, `-fall`), `-target`,
  `-fdiagnostics-format=ccomp|msvc|vi`
  ([manual](https://compcert.org/man/manual003.html)).
- **Identity.** `__COMPCERT__`, `__COMPCERT_MAJOR__`, `__COMPCERT_MINOR__`,
  `__COMPCERT_VERSION__`, and `__STDC_NO_ATOMICS__`, `__STDC_NO_COMPLEX__`,
  `__STDC_NO_THREADS__`, `__STDC_NO_VLA__`; GCC preprocesses with
  `-U__GNUC__`
  ([`Frontend.ml`](https://github.com/AbsInt/CompCert/blob/master/driver/Frontend.ml)).
  Banner `The CompCert C verified compiler, version 3.18`, which Meson
  matches on `CompCert`. No CMake ID.
- **Dialect.** No VLAs or `_Complex`; `long double`, struct passing and
  unstructured `switch` only on request
  ([manual](https://compcert.org/man/manual005.html)).
- **Diagnostics.** `file.c:12: error: ...` with no column.
- **Verdict.** x86-64 is a target, but its users want CompCert's proof, which
  bcc cannot give, and its dialect is a subset bcc's strict mode would cover.
  **No.**

### lcc (not installed)

- **Status.** 4.2, C89 only. The last source changes are bug fixes from
  November 2014 (`etc/lcc.c`, `cpp/unix.c` and four files under `src/`);
  only documentation has changed since
  ([drh/lcc](https://github.com/drh/lcc/commits/master)); its licence forbids selling it
  ([CPYRIGHT](https://github.com/drh/lcc/blob/master/CPYRIGHT)). No x86-64
  back end ([`bind.c`](https://github.com/drh/lcc/blob/master/src/bind.c)).
- **Identity.** `__LCC__` 1 and gcc's `cpp` run with `-U__GNUC__`
  ([`etc/linux.c`](https://github.com/drh/lcc/blob/master/etc/linux.c)).
  CMake's `LCC` is MCST's Elbrus compiler, not this one.
- **Verdict.** **No.**

### Plan 9 C, kencc (not installed)

- **Status.** Bell Labs' tree ends in 2015 (now MIT-licensed,
  [plan9foundation/plan9](https://github.com/plan9foundation/plan9)); 9front
  maintains it (2026-08-02 release). One compiler per architecture (`6c` for
  amd64, `8c` for 386), writing Plan 9 objects for its own linkers
  ([2c(1)](https://man.9front.org/1/2c)).
- **Dialect.** C89 plus unnamed substructures with pointer promotion,
  `(struct S){...}` displays, `#pragma lib` and `varargck`; programs include
  `<u.h>` and `<libc.h>`; its preprocessor has no `#if`
  ([How to Use the Plan 9 C Compiler](https://9p.io/sys/doc/comp.html)).
  No identity macro. plan9port's `9c` is a script that runs GCC or Clang
  ([`bin/9c`](https://github.com/9fans/plan9port/blob/master/bin/9c)).
- **Verdict.** Its value is Plan 9's object format and libraries, which bcc
  does not have. GCC's `-fplan9-extensions` covers the language part, if
  bcc ever wants it. **No.**

### cproc (tested)

- **Status.** Maintained by Michael Forney (last commit 2026-06-01, `d1c53dd`;
  QBE 2026-11-09). "Implements most of C11 as well as many C23 features"
  and "a few GNU C extensions"; it aims to "closely follow the C standard"
  and asks that non-conforming code be patched rather than accepted
  ([README](https://git.sr.ht/~mcf/cproc)). Used by hobbyist distributions
  and as a portability check; no vendor ships it.
- **Command line.** A small POSIX `c99`-like driver
  ([`driver.c`](https://git.sr.ht/~mcf/cproc/tree/master/item/driver.c)):
  `-c -S -E -D -U -I -o -l -L -s -g -O -M -MD -MF -MM -MMD -MT -P -std=
  -pthread -static -nostdlib -nostdinc -include -isystem -idirafter -iquote
  -dumpmachine`; anything else is `cproc: unknown option '--version'` with
  its usage line and status 2, so `-dM`, `--version` and `-fbogus` all fail.
  It runs the system `cpp`, then `cproc-qbe`, `qbe` and `as` as separate
  processes (`-v` prints `cproc: spawning ...` for each).
- **Identity.** No identity macro at all. The system `cpp` runs with `-undef`
  and cproc's own list (`__STDC_NO_ATOMICS__`, `__STDC_NO_COMPLEX__`,
  `__extension__=`, the endianness macros, `__x86_64__`, `__LP64__`,
  `__linux__`, `__unix__`, four `__SIZEOF_*__`, `__BITINT_MAXWIDTH__=64`):
  no `__GNUC__`, so headers take their ISO paths. CMake finds no compiler ID
  but a working C17 compiler without extensions; Autoconf finds a compiler
  that is not GNU and no C11 option.
- **Diagnostics.** `undeclared.c:1:22: error: undeclared identifier: y`, then
  the driver's `cproc: compile: process N exited with status 1` and
  `process signaled: Terminated` for the other stages. A missing header is
  GCC's own message, from GCC's `cpp`. One error per run.
- **Cost and value.** Small to read its options, but every message differs and
  the multi-process driver has to be mimicked in its errors. Low value: nobody
  needs bcc to *be* cproc; what cproc offers is a non-GNU identity (see
  [A strict mode instead](#a-strict-mode-instead)).

### chibicc and slimcc (tested)

- **Status.** chibicc (Rui Ueyama) is a teaching compiler, frozen since
  2020-12-07 (`90d1f7f`); its README calls it a "toy compiler" that can
  build Git, SQLite and libpng
  ([README](https://github.com/rui314/chibicc)). slimcc is its active fork
  (2026-10-24, `2388c39`), with C23 and many C2y features and GNU
  extensions; its README claims it is "a drop-in C compiler for most
  portably-written programs in glibc environment" and its CI builds a long
  list of third-party projects
  ([README](https://github.com/fuhsnn/slimcc)).
- **Command line.** GCC-shaped subsets. chibicc refuses `--version`, `-v` and
  `-dM` (`unknown argument: -v`); slimcc has `--version` (`slimcc version
  0.0`), `-v` and `-dM`. Both refuse an unknown option.
- **Identity.** chibicc defines `__chibicc__` and GCC-style macros
  (`__SIZEOF_*__`, `__x86_64__`, `__STDC_VERSION__ 201112L`) but no
  `__GNUC__`; slimcc defines `__slimcc__`, `__STDC_VERSION__ 201710L` and
  57 macros, no `__GNUC__`. Neither has a CMake or Meson identity.
- **Diagnostics.** Both print the source line with a caret and no column:
  `undeclared.c:1: int f(void) { return y; }` then `^ undefined variable`
  under it (slimcc puts the line on its own line). One error per run;
  chibicc accepts the redeclaration and the pointer assignment silently.
- **Cost and value.** As cproc: cheap but of little use.

### pcc (tested)

- **Status.** The last release is 1.1.0 of 2014-12-10; later work is CVS
  snapshots on `pcc.ludd.ltu.se`, which did not answer during this survey
  (OpenBSD packages a 2023-10-21 snapshot,
  [openports](https://openports.pl/path/lang/pcc/pcc)). The GitHub mirror
  used here ends at snapshot 20180916, whose last commit (2018-09-21) adds
  only `COPYING` and `README.md`
  ([IanHarvey/pcc](https://github.com/IanHarvey/pcc)). Building it needed
  `-fcommon` (GCC 10's default broke it) and a yacc and flex built from
  source.
- **Command line.** GCC-like; it ignores an unknown option silently.
- **Identity.** `__PCC__ 1`, `__PCC_MINOR__ 2`, and **`__GNUC__ 4`,
  `__GNUC_MINOR__ 3`**: pcc claims GCC 4.3, so headers and Autoconf take it
  for GCC. CMake has no pcc ID.
- **Diagnostics.** `undeclared.c, line 1: y undefined`, then `cannot recover
  from earlier errors: goodbye!` and `error: .../ccom terminated with status
  1`.
- **Does it work?** Not on a current system: glibc 2.39's `<bits/floatn.h>`
  is a `syntax error`, so `<stdlib.h>` cannot be included, its own `libpcc`
  fails to build, and CMake's and Autoconf's first compile fails. **Not
  worth a personality.**

### Open Watcom V2 `owcc`, `wcl386` (tested)

- **Status.** Maintained by the community (GitHub builds every few weeks; the
  2026-11-01 build was used). Used for DOS, OS/2, 16-bit Windows and retro
  work, and by projects that still support those hosts.
- **Targets.** 16-bit and 32-bit x86 only: `owcc -march=` takes `i86` to
  `i386` (plus historical `axp`, `mips`, `ppc`), and there is no x86-64 code
  generator. It compiled and ran the `printf` test as a 32-bit Linux
  program.
- **Command line.** Two drivers: `wcl386` with Watcom's `-x=value` and `/x`
  options, and `owcc`, a POSIX-style front end (`-c -o -D -I -E -g -O -W...
  -b <target> -mcmodel= -mabi=watcall`). `owcc` rejects `-dM` and `--version`
  (`Invalid option '-'`) and prints a banner on every compile. There is also
  a `cl`-compatible `cl.exe` wrapper, which Meson refuses to probe because
  its help pages through a pager
  ([`detect.py`](https://github.com/mesonbuild/meson/blob/1.12.1/mesonbuild/compilers/detect.py)).
- **Identity.** `__WATCOMC__ 1300` (version × 100 + 1100, as CMake decodes
  it), `__386__`, `__FLAT__`, `__X86__`, `_M_IX86 600` (`300` under `-b nt`),
  `M_I386`, `__LINUX__` and `__UNIX__` (or `__NT__` and `_WIN32`),
  `_INTEGRAL_MAX_BITS 64`, `_STDCALL_SUPPORTED`, `__SW_3R`, and
  `__STDC_VERSION__ 199409L` (`199901L` under `-std=c99`). CMake ID
  `OpenWatcom`.
- **Diagnostics.** `undeclared.c(1): Error! E1011: Symbol 'y' has not been
  declared`, notes as `Note! N2002: 'g' defined in: args.c(1)`, then
  `Error: Compiler returned a bad status compiling 'undeclared.c'`.
- **Cost and value.** Needs i386 and i86 targets with Watcom's calling
  conventions and memory models before any driver work. **No.**

### SDCC (tested)

- **Status.** Active; 4.6.0 (June 2026) was used, a GPL binary tarball from
  [SourceForge](https://sourceforge.net/projects/sdcc/files/sdcc-linux-amd64/4.6.0/).
  The standard compiler for 8-bit microcontrollers (8051, Z80 family,
  STM8, HC08, PDK, 6502, F8).
- **Targets.** Only those; `--version` lists 29 ports and no x86.
- **Command line.** Its own (`-mz80`, `--std-c11`, `--model-large`); it warns
  about and ignores an unknown option (`warning 117: unknown compiler option
  '-fbogus-option' ignored`).
- **Identity.** `SDCC 460`, `__SDCC 4_6_0`, `__SDCC_VERSION_MAJOR` 4 and its
  relatives, `__SDCC_mcs51`, `__SDCC_CHAR_UNSIGNED`, `__STDC_NO_VLA__`,
  `__SIZEOF_DOUBLE__ 4`. CMake ID `SDCC`, Meson `sdcc`.
- **Diagnostics.** `undeclared.c:1: error 20: Undefined identifier 'y'`
  (numbered, no column); its preprocessor is a GCC fork and reports a
  missing header in GCC's form.
- **Cost and value.** Needs 8-bit targets. **No.**

### Emscripten `emcc` (not installed)

- **Status.** Active (7.0.0 is the latest tag; `main` is `7.0.1-git`); the
  usual way to build C and C++ for the web. MIT/UIUC
  licensed; `emsdk` installs it without root, but it downloads LLVM,
  Binaryen and Node.js (well over a gigabyte), so it was left out to keep
  the disk free.
- **Command line.** `emcc` is a Python driver
  ([`emcc.py`](https://github.com/emscripten-core/emscripten/blob/main/emcc.py))
  that runs Clang with `-target wasm32-unknown-emscripten` and its sysroot
  ([`tools/compile.py`](https://github.com/emscripten-core/emscripten/blob/main/tools/compile.py)),
  then its own linker step that writes JavaScript glue beside the module,
  driven by `-s SETTING=VALUE` options.
- **Identity.** Clang's, plus `__EMSCRIPTEN__` from Clang's Emscripten
  target (the pinned Clang defines it for that triple, with `__wasm__`,
  `__ILP32__` and `unix`). The version macros (`__EMSCRIPTEN_MAJOR__` ...)
  are not predefined but come from a generated `<emscripten/version.h>`
  ([`tools/system_libs.py`](https://github.com/emscripten-core/emscripten/blob/main/tools/system_libs.py));
  `-pthread` adds `__EMSCRIPTEN_SHARED_MEMORY__`. `--version` prints `emcc
  (Emscripten gcc/clang-like replacement + linker emulating GNU ld) 7.0.1`
  ([`tools/cmdline.py`](https://github.com/emscripten-core/emscripten/blob/main/tools/cmdline.py)),
  which Meson matches on `Emscripten`.
- **Cost and value.** bcc targets `wasm32-unknown-unknown` and
  `wasm32-wasip1`, not Emscripten's ABI, and a program built "with emcc" is
  one that runs on emcc's JavaScript runtime. Compiling C to an object for
  emcc's linker is a small target addition; standing in for `emcc` itself is
  reimplementing Emscripten. **No** as a personality; an
  `wasm32-unknown-emscripten` target is the useful piece if wanted.

### kefir (not built)

An independent C17/C23 compiler for x86-64 System V. Its sourcehut
repository now says "Development of Kefir C compiler has been halted"
(last commit 2026-11-01, [repository](https://git.sr.ht/~jprotopopov/kefir)).
Not a candidate.

### Others

- **AppleClang** (CMake ID `AppleClang`, tested by `__apple_build_version__`;
  Meson by `Apple` in Clang's banner): Clang with Apple's version numbers.
  Cheap as an identity, but bcc has no Darwin target.
- **TI's compilers** (`TIClang` and `TI`), **Fujitsu** (`FujitsuClang`,
  `__CLANG_FUJITSU`), **Cray** (`CrayClang`, `__cray__`), **Green Hills**
  (`GHS`), **IAR**, **TASKING**, **Renesas**, **Microchip XC**: embedded or
  supercomputer targets bcc lacks.
- **OrangeC** (`__ORANGEC__`) and **Embarcadero C++Builder**
  (`__CODEGEARC_VERSION__`, its `bcc32c`/`bcc64` are Clang-based): Windows
  compilers with small C user bases. Embarcadero's could reuse the clang-cl
  machinery, but needs a licence to test.
- **cc65** and **vbcc**: 6502 and other retro targets.

### zig cc (tested)

See [the shape above](#2---zig-cc-mode).

## A strict mode instead

cproc, chibicc and slimcc are useful to their users because they are *not*
GCC: they define no `__GNUC__`, so glibc's `<sys/cdefs.h>`, Autoconf and
project headers take their portable paths, and code that leans on an
unguarded extension fails to build. bcc can give that without copying any
of their command lines or messages: an identity that claims no other
compiler (say `--compiler-identity=iso`, with `-std=c11` or `c17` and
`-pedantic-errors`), which defines `__bcc__` and the standard macros but
none of GCC's or Clang's, and lets bcc's own diagnostics stand. Under the
[identity policy](../design/driver-modes.md#identity-policy) it would follow
GCC's rules, as TCC does. That is a smaller change than any personality
and serves the same users better; this note does not rank it, since it is
not a personality.

## Build-system identity checks

CMake identifies a C compiler by compiling a source whose preprocessor tests
run in a fixed order (`Modules/CMakeCompilerIdDetection.cmake`): Intel,
IntelLLVM, PathScale, Embarcadero, Borland, Watcom, OpenWatcom, SunPro, HP,
Compaq, zOS, IBMClang, XLClang, XL, VisualAge, NVHPC, PGI, CrayClang, Cray,
TI, FujitsuClang, Fujitsu, GHS, Tasking, OrangeC, TinyCC, Bruce, SCO, ARMCC,
AppleClang, ARMClang, TIClang, Clang, LCC, GNU, MSVC, ADSP, IAR, SDCC. So
a vendor identity must be tested before Clang's, and claiming one is a
matter of one macro (CMake 3.31.3,
`Modules/Compiler/*-DetermineCompiler.cmake`):

| CMake ID | Test | Version from |
| --- | --- | --- |
| `IntelLLVM` | `(__clang__ && __INTEL_CLANG_COMPILER) \|\| __INTEL_LLVM_COMPILER` | `__INTEL_LLVM_COMPILER`, `VVVVRRPP` from 2021.2 (`VVVVRP` before) |
| `Intel` | `__INTEL_COMPILER \|\| __ICC` | `__INTEL_COMPILER` (`VRP` before 2021, `VVVV` from 2021), `__INTEL_COMPILER_UPDATE` |
| `NVHPC` | `__NVCOMPILER` | `__NVCOMPILER_MAJOR__`, `__NVCOMPILER_MINOR__`, `__NVCOMPILER_PATCHLEVEL__` |
| `PGI` | `__PGI` | `__PGIC__`, `__PGIC_MINOR__`, `__PGIC_PATCHLEVEL__` |
| `IBMClang` | `__open_xl__ && __clang__` | `__open_xl_version__`, `__open_xl_release__`, ... |
| `XLClang` | `__ibmxl__ && __clang__` | `__ibmxl_version__`, ... |
| `XL` | `__IBMC__ && !__COMPILER_VER__ && __IBMC__ >= 800` | `__IBMC__` |
| `SunPro` | `__SUNPRO_C` | `__SUNPRO_C`, `0xVRRP` |
| `ARMClang` | `__clang__ && __ARMCOMPILER_VERSION` | `__ARMCOMPILER_VERSION`, `VRRPPBB` |
| `ARMCC` | `__ARMCC_VERSION && !__clang__` | `__ARMCC_VERSION` |
| `OpenWatcom` | `__WATCOMC__` | `(__WATCOMC__ - 1100) / 100` |
| `TinyCC` | `__TINYC__` | none |
| `SDCC` | `__SDCC_VERSION_MAJOR \|\| SDCC` | `__SDCC_VERSION_*` |
| `LCC` | `__LCC__ && (__GNUC__ \|\| __GNUG__ \|\| __MCST__)` | MCST's Elbrus `lcc`, not Fraser and Hanson's |
| `Clang` | `__clang__` | `__clang_major__`, ... |

zig cc, Emscripten (which CMake reads as Clang and configures through its
`Emscripten.cmake` toolchain file), cproc, chibicc, slimcc, pcc, CompCert,
kefir and the Plan 9 compilers have no CMake ID.

Meson instead reads `--version` and stops at the first string it knows, in
order ([`detect.py`, 1.12.1](https://github.com/mesonbuild/meson/blob/1.12.1/mesonbuild/compilers/detect.py)):
GCC by `Free Software Foundation`, `Emscripten`, `Arm C/C++/Fortran Compiler`,
`armclang`, the TI compilers, then **any `clang` or `Clang`**, then
`Microsoft`, `PGI Compilers`, `NVIDIA Compilers and Tools`, `(ICC)`,
`Intel(R) oneAPI`, `CompCert`, `SDCC` and others. A personality claiming
Intel or NVIDIA must therefore keep the word `clang` out of its banner, which
bcc's Clang banner (`clang version 23.1.5 (bcc 0.1.0, a Clang-compatible C
compiler; not Clang)`) does not.

## Sources

Run on this machine (2026-11-19, x86-64 Linux, glibc 2.39 headers, GCC
13.3, the pinned Clang 23.1.5, CMake 3.31.3, Autoconf 2.71):

- zig 0.17.1 (`zig-x86_64-linux-0.17.1.tar.xz`, SHA-256
  `3581863e...a7e9b17a`, from [ziglang.org](https://ziglang.org/download/));
  source read at [tag 0.17.1](https://codeberg.org/ziglang/zig/src/tag/0.17.1).
- cproc `d1c53dd` (2026-06-01) with QBE `180d2eb` (2026-11-09);
  chibicc `90d1f7f` (2020-12-07); slimcc `2388c39` (2026-10-24), built with
  `platform/linux-glibc-generic.c`.
- pcc `1.2.0.DEVEL 20180916` from the
  [IanHarvey/pcc](https://github.com/IanHarvey/pcc) mirror (`05a6d54`),
  built with `CFLAGS="-O1 -fcommon -std=gnu89"`, byacc and flex 2.6.4.
- Open Watcom V2, the
  [2026-11-01 build](https://github.com/open-watcom/open-watcom-v2/releases/tag/2026-11-01-Build)
  (`ow-snapshot.tar.xz`, no installer).
- SDCC 4.6.0 (`sdcc-4.6.0-amd64-unknown-linux2.5.tar.bz2`, `#16555`).

Read, not run:

- CMake 3.31.3's `Modules/CMakeCompilerIdDetection.cmake` and
  `Modules/Compiler/*-DetermineCompiler.cmake`, as installed.
- Meson 1.12.1's
  [`mesonbuild/compilers/detect.py`](https://github.com/mesonbuild/meson/blob/1.12.1/mesonbuild/compilers/detect.py).
- Emscripten `main` (`emscripten-version.txt` 7.0.1-git):
  [`tools/compile.py`](https://github.com/emscripten-core/emscripten/blob/main/tools/compile.py),
  [`tools/cmdline.py`](https://github.com/emscripten-core/emscripten/blob/main/tools/cmdline.py),
  [`tools/system_libs.py`](https://github.com/emscripten-core/emscripten/blob/main/tools/system_libs.py).
- The vendor pages cited in each section above.
- bcc's own [driver-modes.md](../design/driver-modes.md) and
  `src/cli/personality/` for the existing personalities' size.
