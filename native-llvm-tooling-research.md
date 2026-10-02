# Rust and C native toolchain helpers

Research date: 2026-09-30. Scope: replace unnecessary custom tool discovery or
provisioning while retaining static native-code linking and Rust/C LTO on
`x86_64-pc-windows-gnu`. The local `rustc -vV` reports Rust 1.98.0 and LLVM
22.1.8. This investigation did not install dependencies or alter the build.

## Most useful existing crates

**`llvm-tools` is the closest match for finding tools belonging to the active
Rust toolchain.** Its `LlvmTools` helper finds the sysroot by invoking `rustc`,
then locates the directory containing `llvm-objdump`; `tool()` returns an
existing requested executable. It assumes the rustup `llvm-tools-preview`
component is already installed. It neither downloads Clang/libclang nor sets
Rust/C LTO flags. Its source invokes the literal `rustc` on `PATH`, so a build
script using a custom `RUSTC` must account for that difference.
[llvm-tools 0.1.1 source](https://docs.rs/llvm-tools/0.1.1/src/llvm_tools/lib.rs.html)

**The existing `cc` dependency already handles producing the native static
archive.** It accepts a supplied C compiler, archiver and flags through `CC`,
`AR`, `CFLAGS` and target-specific variants. It invokes an installed compiler;
it does not ship one or automatically match its LLVM version to rustc.
Supplying compatible Clang and LLVM bitcode flags remains the caller's job.
[cc documentation](https://docs.rs/cc/1.5.1/cc/)

| Candidate | Verified purpose | Gap for this repository |
| --- | --- | --- |
| `llvm-tools` | Locate executables in the Rust toolchain's installed LLVM tools directory | No compiler acquisition, libclang discovery, or LTO configuration; the requested executable must be present |
| `cargo-binutils` | Cargo commands and `rust-*` proxies invoking Rust-shipped LLVM inspection tools | Useful for disassembly and symbol checks; its documented workflow does not compile C or configure cross-language LTO |
| `cc` | Compile native files into a static archive using selected external tools | Requires the compatible compiler and flags to be supplied |
| `clang-sys` | Bind to or load an installed libclang; search paths are configurable | Its version features select a minimum API, rather than the LLVM version of rustc; it does not install Clang |
| `llvm-sys` | Bind to an installed LLVM C API | Its strict-versioning option is about the crate's LLVM API version, not selecting a C compiler matching rustc |
| `cargo-zigbuild` | Use Zig as a linker and C/C++ toolchain for cross compilation | The README documents Linux/macOS targets; Zig is independently installed and the wrapper does not promise rustc LLVM matching |
| `cargo-xwin` | Provision MSVC CRT/SDK files and configure Windows MSVC cross builds | It requires installed Clang and targets MSVC rather than this GNU target |

The table follows the respective primary documentation:
[llvm-tools](https://docs.rs/llvm-tools/0.1.1/llvm_tools/),
[cargo-binutils](https://github.com/rust-embedded/cargo-binutils#usage),
[cc](https://docs.rs/cc/1.5.1/cc/),
[clang-sys](https://github.com/KyleMayes/clang-sys#environment-variables),
[llvm-sys](https://docs.rs/crate/llvm-sys/231.0.0/source/README.md),
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild#caveats),
[cargo-xwin](https://github.com/rust-cross/cargo-xwin#prerequisite).

## An acquisition crate with substantial remaining setup

`lvm-fetch` 0.0.10-alpha exposes discovery, download, extraction and cached
installation for caller-specified LLVM version requirements. However, its
actual built-in manifest source only covers LLVM 18.1.8; all its checksum
values are placeholders, and its Windows entry is MSVC. Installation rejects
unverifiable entries. Using it for our LLVM 22.1.8 Windows GNU setup would
require supplying the correct asset manifest and checksum, adapting host
selection, and still configuring target headers, libclang and LTO separately.
It is not an automatic replacement for the current setup.
[lvm-fetch documentation](https://docs.rs/lvm-fetch/0.0.10-alpha/lvm_fetch/),
[built-in manifest source](https://docs.rs/lvm-fetch/0.0.10-alpha/src/lvm_fetch/manifest.rs.html)

## What no helper can replace by itself

Cargo's normal `lto` profile setting optimizes Rust crates. Cargo's own manual
directs cross-language users to rustc's linker-plugin LTO instructions and
states that it must be configured through `RUSTFLAGS`. A compatible C compiler
must emit LLVM bitcode, and the final linker must support that bitcode.
[Cargo LTO profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#lto),
[rustc linker-plugin LTO](https://doc.rust-lang.org/rustc/linker-plugin-lto.html)

Rust's manual recommends matching LLVM versions for best results, but explicitly
describes `rustc -vV`'s reported version as an approximation because Rust can
use unreleased revisions. Matching `22.1.8` therefore establishes the reported
release version; it does not establish identical LLVM source revisions.
[Toolchain compatibility](https://doc.rust-lang.org/rustc/linker-plugin-lto.html#toolchain-compatibility)

The practical reusable pieces are `cc` for the static archive and Rust-shipped
LLVM tools for linking or verification where those executables are available.
`llvm-tools` can simplify discovering those executables. A matching external
Clang/libclang distribution, target-specific configuration, and an actual LTO
verification still remain necessary. No reviewed crate or Cargo extension
provides the complete requested Windows GNU workflow automatically.

## Initial setup, replaced by the pinned source build

The build now uses `llvm-tools` 0.1.1 to select Rust's own `llvm-ar`, alongside
the existing `cc` crate. `rust-toolchain.toml` requests the `llvm-tools`
component automatically. Because the helper invokes `rustc` on `PATH`, the
build verifies that its selected directory belongs to Cargo's actual `RUSTC`.
Executable verification also uses the active Rust sysroot's LLVM inspection
tools, rather than copies downloaded with Clang.

The installed Rust 1.98.0 Windows GNU component was inspected locally. It
contains `llvm-ar`, `llvm-nm`, `llvm-objdump`, and `llvm-readobj`, but no Clang
executable or libclang. Its archiver reports
`22.1.8-rust-1.98.0-stable`. Matching external Clang/libclang remains necessary;
the setup script now extracts only those tools and Clang's resource headers.

## Can nightly expose Rust's matching Clang?

### The proposal exists and is approved, but approval is not distribution

The remembered issue is likely
[`rust-lang/rust#56371`, Shipping clang as a rustup component](https://github.com/rust-lang/rust/issues/56371).
It remains open. The related
[`rust-lang/rfcs#3847`, Include Clang in llvm-tools](https://github.com/rust-lang/rfcs/pull/3847)
also remains open and unmerged; its latest review on September 24, 2026 points
to a separate compiler proposal. That separate proposal,
[`rust-lang/compiler-team#907`, ship clang and llvm-ar in rustup](https://github.com/rust-lang/compiler-team/issues/907),
was accepted on September 14, 2025. The acceptance authorizes the work; it does
not establish that Clang is in a released nightly component.
[Acceptance comment](https://github.com/rust-lang/compiler-team/issues/907#issuecomment-3289917575)

### Current nightly distribution on Windows GNU

The official 2026-09-30 Windows GNU `llvm-tools` archive was downloaded and
its published SHA-256 verified locally:

```text
a20cce331d87f4f43f785108d51503c952017dfb568818064845112ad6228052
```

Its 15 executables are `llvm-profdata`, `llc`, `llvm-size`, `llubi`,
`llvm-readobj`, `llvm-link`, `llvm-nm`, `llvm-strip`, `llvm-objdump`, `llvm-ar`,
`llvm-as`, `llvm-dis`, `opt`, `llvm-cov` and `llvm-objcopy`. There is no Clang
executable or libclang in that archive. The current nightly manifest has no
Clang component. Its `offload-preview` component is unavailable for both
Windows GNU and Windows MSVC, while available for Linux GNU.
[Dated official nightly manifest](https://static.rust-lang.org/dist/2026-09-30/channel-rust-nightly.toml)

Direct inspection evidence is saved in
[the archive record](/C:/Users/benbw/Documents/GitRepo/bcc-rust/target/native-llvm-research/nightly-2026-09-30-windows-gnu.json)
and [its contents listing](/C:/Users/benbw/Documents/GitRepo/bcc-rust/target/native-llvm-research/nightly-2026-09-30-windows-gnu-contents.txt).

This agrees with Rust's current packaging source, whose `LLVM_TOOLS` list
omits Clang and whose LLVM-tools distribution copies only that list. This
source inspection is pinned to Rust commit
`21b707e3f97e0b522ebd2f277a862339625ad83f` from September 30, 2026.
[LLVM-tools packaging source](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/build_steps/dist.rs)

### Linux offload is a runtime component

The compiler development guide documents installing nightly `offload` on
`x86_64` Linux. Its distribution source packages OpenMP/offload runtime
libraries and Rust's offload library. It does not package the Clang driver.
The guide's `--enable-clang` instruction applies to building Rust from source,
not to an option that exposes a hidden installed compiler.
[Offload installation guide](https://rustc-dev-guide.rust-lang.org/offload/installation.html),
[Offload distribution source](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/build_steps/dist.rs)

### CI LLVM archives are separate, internal artifacts

Rust bootstrap can download `rust-dev-<version>-<host>.tar.xz` from its CI
artifact server through `llvm.download-ci-llvm`. This is **`rust-dev`**, not the
rustup **`rustc-dev`** component. Source explicitly labels these archives as
internal development artifacts without end-user stability. The downloader
also warns that old artifacts expire.
[CI LLVM downloader](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/download.rs),
[RustDev packaging](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/build_steps/dist.rs)

That packaging copies all executables from the actual LLVM build's `bin`
directory, along with LLVM headers, LLVM libraries and selected supporting
files. Consequently, an archive **can** contain Clang if its producing build
enabled `llvm.clang`; an LLVM archive name alone does not establish this.
The current Windows GNU distribution job does not enable `llvm.clang`, whose
default is false. The Linux distribution uses a separate external Clang for
bootstrapping/offload. These CI configurations do not establish a downloadable
matching Clang for our Windows GNU toolchain; no CI tarball was fetched or its
contents verified in this follow-up.
[RustDev packaging](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/build_steps/dist.rs),
[Windows CI configuration](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/ci/github-actions/jobs.yml),
[Linux CI configuration](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/ci/docker/host-x86_64/dist-x86_64-linux/Dockerfile),
[Configuration defaults](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/config/config.rs)

### Building the exact LLVM source is possible

The source-build option `llvm.clang = true` enables the Clang project in
Rust's LLVM submodule build. Building that submodule at the revision belonging
to a particular Rust release or nightly can therefore produce Clang from the
same LLVM source as that compiler. This is a compiler/source-build workflow,
not an unstable Cargo or rustc switch that installs Clang into an existing
rustup toolchain. Use `llvm.download-ci-llvm = false` to force that source build
rather than reusing a CI LLVM archive whose Clang contents are unspecified.
No Clang-acquisition/exposure switch appears in the reviewed
current rustc option definitions; `llvm-args` and `llvm-plugins` configure the
existing LLVM backend.
[Clang source-build configuration](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/src/bootstrap/src/core/build_steps/llvm.rs),
[rustc option definitions](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/compiler/rustc_session/src/options.rs)

## Current implementation: sparse source submodules

`vendor/rust` is now a shallow, filtered submodule at the active Rust 1.98.0
compiler commit, `88d9e12ae178fab0fb5cc050a94da85685d449ea`. Its LLVM gitlink
selects `52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04`. Cargo builds these pinned
LLVM/Clang/LLD sources through the `cmake` crate; the native build checks their
reported LLVM version against rustc.
[Rust's LLVM gitlink](https://github.com/rust-lang/rust/tree/88d9e12ae178fab0fb5cc050a94da85685d449ea/src),
[LLVM version metadata](https://github.com/rust-lang/llvm-project/blob/52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04/cmake/Modules/LLVMVersion.cmake)

LLVM's `install-distribution` target limits installation and its build
dependencies to Clang, resource headers, libclang, llvm-ar and LLD. Sparse
checkout excludes tests, docs and examples while retaining their required
CMake entrypoints. LLD's Windows aliases are installed as copies by setting
`LLVM_USE_SYMLINKS=OFF`, allowing Cargo to invoke them directly.
[Distribution target](https://github.com/rust-lang/llvm-project/blob/52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04/llvm/cmake/modules/LLVMDistributionSupport.cmake),
[LLD aliases](https://github.com/rust-lang/llvm-project/blob/52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04/lld/cmake/modules/AddLLD.cmake)

The custom release downloader, linker wrapper and executable verification
scripts have been removed. The compiler corpus runner remains independent
of this native-toolchain setup.

The sparse checkout also retains the single libunwind header
`include/mach-o/compact_unwind_encoding.h` required by LLD's Mach-O build.
A standard-library file lock serializes configuration, compilation and
installation in the shared LLVM cache across independent Cargo builds.
[LLD's libunwind include path](https://github.com/rust-lang/llvm-project/blob/52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04/lld/MachO/CMakeLists.txt),
[Rust file locking](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock)

### Source-build validation, 1 October 2026

On `x86_64-pc-windows-gnu`, the source build and release executable build
succeeded. Clang and LLD report the pinned LLVM commit in their version
banners. The C archive contains LLVM bitcode; all five C helper symbols were
optimized away, and disassembly of Rust's `string_to_long_double` contains
the inlined C long-double load and classification instructions. Only Windows
system DLLs are imported. A directory containing only the executable, with
`PATH` restricted to Windows directories, correctly parsed double, float,
and long-double hex constants. Evidence is saved under ignored
`target/rust-llvm-source-verification/`, including the executable SHA-256.

The canonical test run passed all 245 library tests, then failed a CLI
include-fixture test. The separate torture-regression run passed 11 tests
and failed a recovery-diagnostic assertion. Clippy and formatting checks
also fail in the concurrent parser/diagnostic changes. The new build modules
passed their formatting check and produced no Clippy findings. These
workspace-wide failures were preserved rather than changing unrelated work.
