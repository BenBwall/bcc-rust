//! Version banners and driver queries (`--version`, `-v`, `-dumpversion`,
//! `-dumpmachine`, `-print-prog-name=`) in each compiler's form. Build
//! systems read them: Meson recognizes GCC by a mention of the Free Software
//! Foundation and Clang by `clang`; `CMake` and Autoconf read the predefined
//! macros, and scripts parse the version at the end of GCC's first line.
//! Every banner names bcc and says it is not the compiler it mimics.

use std::fmt::Write as _;

use super::Personality;
use crate::target::Target;

/// The version of GCC the GCC personality claims, as `__GNUC__`,
/// `__GNUC_MINOR__` and `__GNUC_PATCHLEVEL__` do.
pub(super) const GCC_VERSION: &str = "14.2.0";
/// The pinned Clang's version, which the target tables' `__clang_major__`
/// and its relatives give.
pub(super) const CLANG_VERSION: &str = "23.1.1";
/// The `cl.exe` version `_MSC_VER` 1933 and `_MSC_FULL_VER` 193300000
/// give.
pub(super) const MSVC_VERSION: &str = "19.33.0";
/// The TCC release `__TINYC__` 927 gives.
pub(super) const TCC_VERSION: &str = "0.9.27";
/// bcc's own version.
const BCC_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `--version` for GCC and Clang, `-v` for TCC, and the banner `cl.exe`
/// prints before it compiles.
pub(super) fn banner(personality: Personality, name: &str, target: Target) -> String {
    let mut text = String::new();
    match personality {
        | Personality::Gcc => {
            _ = writeln!(text, "{name} (bcc {BCC_VERSION}) {GCC_VERSION}");
            text.push_str(
                "bcc is a C compiler with a GCC-compatible command line. It is not GCC, and\nit \
                 is not affiliated with the GNU Project or the Free Software Foundation.\n",
            );
        },
        | Personality::Clang | Personality::ClangCl => {
            _ = writeln!(
                text,
                "clang version {CLANG_VERSION} (bcc {BCC_VERSION}, a Clang-compatible C compiler; \
                 not Clang)"
            );
            _ = writeln!(text, "Target: {}", target.triple());
            text.push_str("Thread model: posix\n");
            if let Some(directory) = std::env::current_exe()
                .ok()
                .as_deref()
                .and_then(std::path::Path::parent)
            {
                _ = writeln!(text, "InstalledDir: {}", directory.display());
            }
        },
        | Personality::Msvc => {
            _ = writeln!(
                text,
                "bcc {BCC_VERSION}, compatible with Microsoft (R) C/C++ Optimizing Compiler \
                 Version {MSVC_VERSION} for x64; not a Microsoft product"
            );
        },
        | Personality::Tcc => {
            _ = writeln!(
                text,
                "tcc version {TCC_VERSION} (x86_64 {}) (bcc {BCC_VERSION}, TCC-compatible; not \
                 TCC)",
                if matches!(target, Target::WindowsGnu | Target::WindowsMsvc) {
                    "Windows"
                } else {
                    "Linux"
                }
            );
        },
    }
    text
}

/// What `-v` prints to stderr for GCC and Clang: the target and the
/// version, in the lines `gcc -v` ends with.
pub(super) fn verbose(personality: Personality, target: Target) -> String {
    match personality {
        | Personality::Gcc => {
            let mut text = String::new();
            _ = writeln!(text, "Target: {}", gcc_machine(target));
            text.push_str("Thread model: posix\n");
            _ = writeln!(text, "gcc version {GCC_VERSION} (bcc {BCC_VERSION})");
            text
        },
        | _ => banner(personality, personality.name(), target),
    }
}

/// `-dumpversion`: GCC's major version, as GCC 7 and later print it,
/// Clang's full version, TCC's release.
pub(super) const fn dump_version(personality: Personality) -> &'static str {
    match personality {
        | Personality::Gcc => "14",
        | Personality::Tcc => TCC_VERSION,
        | _ => CLANG_VERSION,
    }
}

/// The target triple GCC names: Debian's spelling for Linux, MinGW-w64's for
/// Windows. Clang names bcc's own triple.
pub(super) const fn gcc_machine(target: Target) -> &'static str {
    match target {
        | Target::LinuxGnu => "x86_64-linux-gnu",
        | Target::LinuxMusl => "x86_64-linux-musl",
        | Target::WindowsGnu => "x86_64-w64-mingw32",
        | Target::WindowsMsvc => "x86_64-pc-windows-msvc",
    }
}

/// `-print-prog-name=NAME`: the bundled tool bcc would run for `NAME`
/// (LLD for `ld`, `llvm-ar` for `ar`), or `NAME` itself, as GCC prints a
/// program it does not find.
pub(super) fn program_name(name: &str) -> String {
    let tool = match name {
        | "ld" => "ld.lld",
        | "ar" => "llvm-ar",
        | "nm" => "llvm-nm",
        | "ranlib" => "llvm-ranlib",
        | "objcopy" => "llvm-objcopy",
        | "strip" => "llvm-strip",
        | other => other,
    };
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("llvm")
        .join("bin");
    let path = directory.join(if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.into()
    });
    if path.is_file() {
        path.display().to_string()
    } else {
        name.into()
    }
}
