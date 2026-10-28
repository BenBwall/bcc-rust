//! Combines explicit include directories, environment paths, and resource
//! header settings into the ordered header search.

use std::{
    env::{
        split_paths,
        var_os,
    },
    path::Path,
};
#[expect(
    clippy::disallowed_types,
    reason = "clap parses path arguments into `PathBuf`s; the compiler borrows them as `&Path`. \
              Startup builds clang's options as `OsString`s."
)]
use std::{
    ffi::OsString,
    path::PathBuf,
};

use clap::Args;

use crate::target::Target;

impl CliHeaderSearch {
    /// Groups the directories in search order. GCC searches `CPATH` like
    /// trailing `-I` directories and `C_INCLUDE_PATH` like trailing
    /// `-isystem` ones; `-isystem`, `C_INCLUDE_PATH`, the resource
    /// directory, the library's directories and `-idirafter` are system
    /// directories. The C library's directories are those under
    /// `--sysroot`, or, compiling for the host on a Linux host, the host's
    /// own ([`library_directories`]). C99: implementation-defined places,
    /// §6.10.2p2-3, pp. 149-150; PDF pp. 161-162.
    #[expect(
        clippy::disallowed_types,
        reason = "Startup owns the directories from argv and the environment as `Vec<PathBuf>`."
    )]
    pub(super) fn directories(&self, target: Target) -> SearchDirectories {
        let mut angled = self.include.clone();
        angled.extend(include_path_from_env("CPATH"));
        let mut system = self.system_include.clone();
        system.extend(include_path_from_env("C_INCLUDE_PATH"));
        let mut after = Vec::new();
        if !self.nostdinc && !self.nostdlibinc {
            after.extend(library_directories(target, self.sysroot.as_deref()));
        }
        after.extend(self.after_include.iter().cloned());
        SearchDirectories {
            quote: self.quote_include.clone(),
            angled,
            system,
            resource: !self.nostdinc && !self.nobuiltininc,
            after,
        }
    }
}

/// The C library's header directories for `target`, as Clang's Linux
/// driver finds them: under `sysroot`, `usr/local/include`, the multiarch
/// directory `usr/include/x86_64-linux-gnu` (or `-musl`) when the target is
/// Linux and the directory exists, and `usr/include`. Without a sysroot the
/// root is the host's own, but only when bcc runs on the Linux host it
/// compiles for; any other target is a cross target, which stays hermetic
/// until `--sysroot`, `-isystem` or `-idirafter` names its library. The
/// WASI target's library is bcc's own, in the embedded
/// [`WASI_DIRECTORY`](crate::headers::WASI_DIRECTORY).
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Startup owns the directories beside clap's arguments as `PathBuf`s."
)]
fn library_directories(target: Target, sysroot: Option<&Path>) -> Vec<PathBuf> {
    // WASI's C library is bcc's own, embedded beside the resource headers.
    if target == Target::Wasm32Wasi && sysroot.is_none() {
        return vec![PathBuf::from(crate::headers::WASI_DIRECTORY)];
    }
    let root = match sysroot {
        | Some(sysroot) => sysroot,
        | None if is_host(target) => Path::new("/"),
        | None => return Vec::new(),
    };
    let include = root.join("usr").join("include");
    let multiarch = match target {
        | Target::LinuxGnu => Some("x86_64-linux-gnu"),
        | Target::LinuxMusl => Some("x86_64-linux-musl"),
        | _ => None,
    }
    .map(|triple| include.join(triple))
    .filter(|directory| directory.is_dir());
    let mut directories = vec![root.join("usr").join("local").join("include")];
    directories.extend(multiarch);
    directories.push(include);
    directories
}

impl CliHeaderSearch {
    /// The header search options as the bundled clang reads them, for the
    /// C preprocessor that runs before an assembly file is assembled.
    /// `--sysroot` goes with the toolchain ([`Self::sysroot`]).
    #[expect(
        clippy::disallowed_types,
        reason = "Startup builds the clang command line beside clap's arguments."
    )]
    pub(super) fn clang_arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::new();
        let groups = [
            ("-iquote", &self.quote_include),
            ("-I", &self.include),
            ("-isystem", &self.system_include),
            ("-idirafter", &self.after_include),
        ];
        for (flag, directories) in groups {
            for directory in directories {
                arguments.push(flag.into());
                arguments.push(directory.into());
            }
        }
        let switches = [
            ("-nostdinc", self.nostdinc),
            ("-nostdlibinc", self.nostdlibinc),
            ("-nobuiltininc", self.nobuiltininc),
        ];
        arguments.extend(
            switches
                .into_iter()
                .filter(|(_, enabled)| *enabled)
                .map(|(flag, _)| flag.into()),
        );
        arguments
    }

    /// The `--sysroot` directory, which the bundled clang also links and
    /// assembles against, so headers and libraries come from one root.
    pub(super) fn sysroot(&self) -> Option<&Path> {
        self.sysroot.as_deref()
    }
}

/// The directories of each header search group, owned beside clap's
/// arguments until `run` copies them into the translation-unit arena.
#[expect(
    clippy::disallowed_types,
    reason = "Startup owns the directories from argv and the environment as `Vec<PathBuf>`."
)]
pub(super) struct SearchDirectories {
    pub(super) quote:    Vec<PathBuf>,
    pub(super) angled:   Vec<PathBuf>,
    pub(super) system:   Vec<PathBuf>,
    pub(super) resource: bool,
    pub(super) after:    Vec<PathBuf>,
}

/// Header search options, in GCC and Clang spellings. Lookup order:
/// `"…"` names look beside the including file and in `-iquote`; both forms
/// then search `-I`, `CPATH`, `-isystem`, `C_INCLUDE_PATH`, the built-in
/// resource directory, the C library's directories, and `-idirafter`.
#[derive(Args)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the repeated directories as `Vec<PathBuf>`."
)]
pub(super) struct CliHeaderSearch {
    /// Add directory to the search path for `"…"` includes only.
    #[clap(short = 'q', long = "iquote", value_name = "DIR")]
    quote_include:  Vec<PathBuf>,
    /// Add directory to the include search path.
    #[clap(short = 'I', long = "include-directory", value_name = "DIR")]
    include:        Vec<PathBuf>,
    /// Add directory to the system include search path, before the built-in
    /// headers.
    #[clap(short = 's', long = "isystem", value_name = "DIR")]
    system_include: Vec<PathBuf>,
    /// Add directory to the end of the search path, after the built-in
    /// headers and the C library's directories.
    #[clap(long = "idirafter", value_name = "DIR")]
    after_include:  Vec<PathBuf>,
    /// Use `DIR/usr/local/include` and `DIR/usr/include` as the C library's
    /// include directories.
    #[clap(long, value_name = "DIR")]
    sysroot:        Option<PathBuf>,
    /// Do not search the built-in headers or the C library's directories.
    #[clap(long)]
    nostdinc:       bool,
    /// Do not search the C library's directories.
    #[clap(long)]
    nostdlibinc:    bool,
    /// Do not search the built-in headers.
    #[clap(long)]
    nobuiltininc:   bool,
}

/// Whether bcc runs on the system `target` describes: an x86-64 Linux host
/// with the same C library. Only such a host's own headers belong to the
/// target.
const fn is_host(target: Target) -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && match target {
            | Target::LinuxGnu => cfg!(target_env = "gnu"),
            | Target::LinuxMusl => cfg!(target_env = "musl"),
            | Target::WindowsGnu
            | Target::WindowsMsvc
            | Target::Wasm32Unknown
            | Target::Wasm32Wasi => false,
        }
}

/// Returns the directories of a GCC-style search-path variable.
///
/// Elements use the platform separator (`;` on Windows, `:` elsewhere). As
/// in GCC and Clang, an empty element names the working directory, while an
/// unset or empty variable contributes nothing.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Startup reads CPATH-style variables beside clap's arguments; `run` borrows the \
              paths as `&Path`."
)]
fn include_path_from_env(env_var: &str) -> Vec<PathBuf> {
    match var_os(env_var) {
        | Some(value) if !value.is_empty() => split_paths(&value)
            .map(|path| {
                if path.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    path
                }
            })
            .collect(),
        | _ => Vec::new(),
    }
}
