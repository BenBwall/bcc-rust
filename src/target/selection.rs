//! Parses target triples and exposes names and ABI-specific choices.

use super::Target;

impl Target {
    pub(crate) fn parse(triple: &str) -> Option<Self> {
        match triple {
            | "x86_64-unknown-linux-gnu" => Some(Self::LinuxGnu),
            | "x86_64-unknown-linux-musl" => Some(Self::LinuxMusl),
            | "x86_64-w64-windows-gnu" | "x86_64-w64-mingw32" | "x86_64-pc-windows-gnu" =>
                Some(Self::WindowsGnu),
            | "x86_64-pc-windows-msvc" => Some(Self::WindowsMsvc),
            | "wasm32-unknown-unknown" | "wasm32" => Some(Self::Wasm32Unknown),
            | "wasm32-wasip1" | "wasm32-wasi" | "wasm32-unknown-wasi" => Some(Self::Wasm32Wasi),
            | _ => None,
        }
    }

    pub(crate) const fn triple(self) -> &'static str {
        match self {
            | Self::LinuxGnu => "x86_64-unknown-linux-gnu",
            | Self::LinuxMusl => "x86_64-unknown-linux-musl",
            | Self::WindowsGnu => "x86_64-w64-windows-gnu",
            | Self::WindowsMsvc => "x86_64-pc-windows-msvc",
            | Self::Wasm32Unknown => "wasm32-unknown-unknown",
            | Self::Wasm32Wasi => "wasm32-wasip1",
        }
    }

    /// Whether this is a WebAssembly target, which only the WebAssembly
    /// back end compiles.
    pub(crate) const fn is_wasm(self) -> bool {
        matches!(self, Self::Wasm32Unknown | Self::Wasm32Wasi)
    }

    /// Preserve the original default-target inspection spelling.
    pub(crate) const fn inspection_name(self) -> &'static str {
        match self {
            | Self::LinuxGnu => "x86_64-sysv-lp64",
            | _ => self.triple(),
        }
    }

    /// Whether an external function whose file-scope declarations all say
    /// `inline`, without `extern`, is a C99 inline definition. The Microsoft
    /// ABI instead emits every inline function as a discardable external
    /// definition, as MSVC and Clang do for C, so the restrictions on inline
    /// definitions do not apply there.
    ///
    /// C99: inline definitions §6.7.4 paragraphs 3 and 6, pp. 112-113;
    /// PDF pp. 124-125. The Microsoft ABI rule is an extension under §4
    /// paragraph 6, p. 7; PDF p. 19.
    pub(crate) const fn c99_inline_definitions(self) -> bool {
        !matches!(self, Self::WindowsMsvc)
    }

    /// Whether source may name the IEEE binary128 type, as `__float128` or
    /// through the GNU `TF`, `KF`, `TC` and `KC` modes. Like the pinned
    /// Clang, the MSVC target keeps binary128 only for literals.
    /// GCC manual, "Additional Floating Types".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Floating-Types.html>
    pub(crate) const fn float128_type(self) -> bool {
        !matches!(self, Self::WindowsMsvc)
    }

    /// Whether source may name `_Float16`. The pinned Clang rejects it on
    /// the WebAssembly targets ("_Float16 is not supported on this
    /// target"), and so does bcc.
    /// C23: Annex H, §H.2.2 paragraph 1, p. 553; PDF p. 566.
    pub(crate) const fn float16_type(self) -> bool {
        !self.is_wasm()
    }
}
