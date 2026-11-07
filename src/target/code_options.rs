//! The relocation and thread-local storage models: Clang's `-fpic`,
//! `-fPIC`, `-fpie`, `-fPIE`, `-fno-pic` family and `-ftls-model`.
//!
//! [`CodeOptions::thread_local_model`] decides which TLS model a
//! thread-local global requests, as Clang's IR together with LLVM's
//! `TargetMachine::getTLSModel` would: an executable (`Static` or `Pie`)
//! reaches its own definitions with local-exec, because Clang marks them
//! `dso_local`; a shared library (`Pic`) assumes nothing about where a
//! default-visibility symbol is defined and uses general-dynamic, which LLVM
//! refines to local-dynamic for internal or hidden symbols. `-ftls-model`
//! names the least optimized model allowed, so a more specific implied
//! model wins (Drepper, "ELF Handling For Thread-Local Storage", §4).
//!
//! Windows has one TLS model, through `_tls_index` and the TEB, and Clang
//! forces position-independent code on x86-64 Windows, so the options do
//! not change code for the Windows targets.
//!
//! [`CodeOptions::strict_volatile_bit_fields`] sets the number and width of
//! the memory accesses to a `volatile` bit-field, for memory-mapped registers.
//!
//! These options are not part of C, which only gives a thread-local object
//! one instance per thread. [`CodeAssumptions`] beside them defines what C
//! leaves undefined about pointers (`-fno-delete-null-pointer-checks`,
//! `-fwrapv-pointer`) and about repeated tentative definitions
//! (`-fcommon`).
//! C11: §6.2.4 paragraph 4, p. 38; PDF p. 56.

use super::Target;

/// The code generation options that depend on how the output is linked or
/// on the machine it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent command-line option."
)]
pub(crate) struct CodeOptions {
    /// `-fstack-protector`, `-fcf-protection` and their kin.
    pub(crate) hardening:                  Hardening,
    /// The relocation model the last `-fpic`-family flag selected, or
    /// `None` for the target's default.
    pub(crate) relocation:                 Option<Relocation>,
    /// The `-ftls-model`, general-dynamic by default as in Clang.
    pub(crate) tls_model:                  TlsModel,
    /// A `volatile` bit-field is accessed at the width of its declared
    /// type: GCC's `-fstrict-volatile-bitfields`, which is off by default
    /// on x86-64. Clang has no such option.
    /// `lowering/bit_fields/volatile_access.rs` holds the rule. Extension: C99
    /// §6.7.3 paragraph 6, p. 109; PDF p. 121 leaves what makes up an access
    /// to a volatile object implementation-defined.
    pub(crate) strict_volatile_bit_fields: bool,
    /// Each function has a section of its own, which the linker's
    /// `--gc-sections` can remove: `-ffunction-sections`.
    pub(crate) function_sections:          bool,
    /// Each data object has a section of its own: `-fdata-sections`.
    pub(crate) data_sections:              bool,
    /// Objects hold debug information. Only the link of an MSVC target reads
    /// this, because its linker drops the information unless asked to keep
    /// it; no code generation does.
    pub(crate) debug_info:                 bool,
}

/// Which functions check a canary before they return: `-fstack-protector`
/// and its `-strong` and `-all` forms. GCC manual, "Instrumentation Options".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StackProtector {
    #[default]
    Off,
    Basic,
    Strong,
    All,
}

/// Run-time checks and frame shapes a build asks for, on Linux. None changes
/// what a program means. GCC manual, "Instrumentation Options" and "Code Gen
/// Options".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each is a separate option's on or off, in any combination."
)]
pub(crate) struct Hardening {
    pub(crate) stack_protector:    StackProtector,
    /// `-fcf-protection=branch`: indirect branches land on `endbr64`.
    pub(crate) branch_protection:  bool,
    /// `-fcf-protection=return`: the object says it works with a shadow stack.
    pub(crate) return_protection:  bool,
    /// `-fstack-clash-protection`: a large frame probes each page.
    pub(crate) stack_clash:        bool,
    /// `-fno-omit-frame-pointer`: every function keeps a frame pointer.
    pub(crate) keep_frame_pointer: bool,
    /// `-fsanitize=address`: LLVM's `AddressSanitizer` checks each access.
    pub(crate) sanitize_address:   bool,
    /// `-fsanitize=thread`: LLVM's `ThreadSanitizer` records each access.
    pub(crate) sanitize_thread:    bool,
}

impl Hardening {
    pub(crate) const NONE: Self = Self {
        stack_protector:    StackProtector::Off,
        branch_protection:  false,
        return_protection:  false,
        stack_clash:        false,
        keep_frame_pointer: false,
        sanitize_address:   false,
        sanitize_thread:    false,
    };

    /// What makes the bundled clang link the sanitizer's run-time library.
    pub(crate) const fn sanitizer_link_flag(self) -> Option<&'static str> {
        if self.sanitize_address {
            Some("-fsanitize=address")
        } else if self.sanitize_thread {
            Some("-fsanitize=thread")
        } else {
            None
        }
    }
}

/// What code generation may assume where C leaves the behaviour
/// undefined, as GCC's and Clang's options define it: none of these by
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct CodeAssumptions {
    /// An object may live at address 0, so dereferencing a pointer does
    /// not prove it non-null and null checks after it stay: GCC's and
    /// Clang's `-fno-delete-null-pointer-checks`, LLVM's
    /// `null_pointer_is_valid`. Extension: C99 §6.5.3.2 paragraph 4, p. 79;
    /// PDF p. 91 leaves dereferencing a null pointer undefined.
    pub(crate) null_pointer_is_valid: bool,
    /// Pointer arithmetic wraps in two's complement rather than staying
    /// within the object it points into: `-fwrapv-pointer`, and
    /// `-fno-strict-overflow`, which also implies `-fwrapv`. Extension:
    /// C99 §6.5.6 paragraph 8, p. 83; PDF p. 95 leaves the overflow
    /// undefined.
    pub(crate) wrapv_pointer:         bool,
    /// A tentative definition with external linkage that the unit leaves
    /// tentative is a common symbol, which the linker merges with the
    /// other units' definitions of its name: `-fcommon`, which GCC 10 and
    /// Clang 11 stopped defaulting to. Extension: C99 §J.5.11, p. 514;
    /// PDF p. 526 (multiple external definitions); §6.9.2 paragraph 2,
    /// p. 143; PDF p. 155 (tentative definitions).
    pub(crate) common_symbols:        bool,
}

impl CodeOptions {
    /// The relocation model code for `target` uses: the selected one on
    /// Linux, position-independent executables by default as in Clang and
    /// GCC; always `Pic(Big)` on Windows, as Clang forces it there.
    pub(crate) fn relocation(self, target: Target) -> Relocation {
        match target {
            | Target::LinuxGnu | Target::LinuxMusl =>
                self.relocation.unwrap_or(Relocation::Pie(PicLevel::Big)),
            | Target::WindowsGnu | Target::WindowsMsvc => Relocation::Pic(PicLevel::Big),
            // WebAssembly code addresses linear memory and the function
            // table directly; Clang defines no `__PIC__` there by default.
            | Target::Wasm32Unknown | Target::Wasm32Wasi => Relocation::Static,
        }
    }

    /// The TLS model an ELF thread-local global requests: `defined` tells
    /// whether the module defines it (an alias counts, as it names a
    /// definition). LLVM may still refine a requested general-dynamic model
    /// to local-dynamic or initial-exec from the symbol's locality.
    pub(crate) fn thread_local_model(self, target: Target, defined: bool) -> TlsModel {
        let implied = match self.relocation(target) {
            | Relocation::Static | Relocation::Pie(_) if defined => TlsModel::LocalExec,
            | Relocation::Static | Relocation::Pie(_) | Relocation::Pic(_) =>
                TlsModel::GlobalDynamic,
        };
        implied.max(self.tls_model)
    }
}

/// How code addresses symbols, as GCC's `-fpic` family selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Relocation {
    /// `-fno-pic`: code for an executable at a fixed address.
    Static,
    /// `-fpie` or `-fPIE`: code for a position-independent executable,
    /// which still defines every symbol it refers to by its own definition.
    Pie(PicLevel),
    /// `-fpic` or `-fPIC`: code a shared library can contain, where another
    /// module may define (interpose) a default-visibility symbol.
    Pic(PicLevel),
}

impl Relocation {
    /// The `-fpic`-family flag `flag` selects, if it is one. As in Clang,
    /// every `-fno-` spelling turns position independence off.
    pub(crate) fn parse(flag: &str) -> Option<Self> {
        Some(match flag {
            | "-fpic" => Self::Pic(PicLevel::Small),
            | "-fPIC" => Self::Pic(PicLevel::Big),
            | "-fpie" => Self::Pie(PicLevel::Small),
            | "-fPIE" => Self::Pie(PicLevel::Big),
            | "-fno-pic" | "-fno-PIC" | "-fno-pie" | "-fno-PIE" => Self::Static,
            | _ => return None,
        })
    }

    /// The value of `__PIC__` and `__pic__`, and whether `__PIE__` and
    /// `__pie__` share it, or `None` when the code is not
    /// position-independent.
    pub(crate) const fn macro_level(self) -> Option<(u8, bool)> {
        match self {
            | Self::Static => None,
            | Self::Pie(level) => Some((level as u8, true)),
            | Self::Pic(level) => Some((level as u8, false)),
        }
    }
}

/// The PIC level: `-fpic` and `-fpie` (1) allow a smaller GOT on some
/// targets than `-fPIC` and `-fPIE` (2); x86-64 code is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PicLevel {
    Small = 1,
    Big = 2,
}

/// A TLS access model, ordered as LLVM orders them, from the most general
/// to the most specific (Drepper, "ELF Handling For Thread-Local Storage",
/// §4.1-§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum TlsModel {
    /// Any module, through `__tls_get_addr` with the symbol's module and
    /// offset.
    #[default]
    GlobalDynamic,
    /// A symbol of the same module, through one `__tls_get_addr` call for
    /// the module's block.
    LocalDynamic,
    /// A module loaded at startup, through the GOT's thread-pointer offset.
    InitialExec,
    /// The executable's own symbols, at a link-time thread-pointer offset.
    LocalExec,
}

impl TlsModel {
    /// The model `-ftls-model=name` selects, with GCC's and Clang's names.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            | "global-dynamic" => Self::GlobalDynamic,
            | "local-dynamic" => Self::LocalDynamic,
            | "initial-exec" => Self::InitialExec,
            | "local-exec" => Self::LocalExec,
            | _ => return None,
        })
    }
}
