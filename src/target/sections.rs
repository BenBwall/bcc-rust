//! LLVM-compatible ELF named-section classification for GNU section attributes.
//! This is C extension policy, independent of assembler section-name defaults.

/// The kind LLVM's `getELFKindForNamedSection` assigns to an explicit name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElfSectionKind {
    Bss,
    ThreadBss,
    ThreadData,
    InitArray,
    FiniArray,
    PreinitArray,
    Note,
    Other,
}

impl ElfSectionKind {
    pub(crate) fn named(name: &str) -> Self {
        if family(name, ".tbss") || linkonce(name, "tb.") {
            Self::ThreadBss
        } else if family(name, ".tdata") || linkonce(name, "td.") {
            Self::ThreadData
        } else if family(name, ".bss")
            || family(name, ".sbss")
            || linkonce(name, "b.")
            || linkonce(name, "sb.")
        {
            Self::Bss
        } else if family(name, ".init_array") {
            Self::InitArray
        } else if family(name, ".fini_array") {
            Self::FiniArray
        } else if family(name, ".preinit_array") {
            Self::PreinitArray
        } else if name.starts_with(".note") {
            Self::Note
        } else {
            Self::Other
        }
    }

    pub(crate) fn is_bss(self) -> bool {
        matches!(self, Self::Bss | Self::ThreadBss)
    }

    pub(crate) fn is_thread_local(self) -> bool {
        matches!(self, Self::ThreadBss | Self::ThreadData)
    }

    pub(crate) fn asm_type(self) -> &'static str {
        match self {
            | Self::Bss | Self::ThreadBss => "nobits",
            | Self::InitArray => "init_array",
            | Self::FiniArray => "fini_array",
            | Self::PreinitArray => "preinit_array",
            | Self::Note => "note",
            | Self::ThreadData | Self::Other => "progbits",
        }
    }
}

pub(crate) fn family(name: &str, stem: &str) -> bool {
    name == stem || name.strip_prefix(stem).is_some_and(|s| s.starts_with('.'))
}

fn linkonce(name: &str, kind: &str) -> bool {
    [".gnu.linkonce.", ".llvm.linkonce."].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with(kind))
    })
}
