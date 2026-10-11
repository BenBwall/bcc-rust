use super::Target;

impl Target {
    pub(crate) fn parse(triple: &str) -> Option<Self> {
        match triple {
            | "x86_64-unknown-linux-gnu" => Some(Self::LinuxGnu),
            | "x86_64-unknown-linux-musl" => Some(Self::LinuxMusl),
            | "x86_64-w64-windows-gnu" | "x86_64-w64-mingw32" | "x86_64-pc-windows-gnu" =>
                Some(Self::WindowsGnu),
            | "x86_64-pc-windows-msvc" => Some(Self::WindowsMsvc),
            | _ => None,
        }
    }

    pub(crate) const fn triple(self) -> &'static str {
        match self {
            | Self::LinuxGnu => "x86_64-unknown-linux-gnu",
            | Self::LinuxMusl => "x86_64-unknown-linux-musl",
            | Self::WindowsGnu => "x86_64-w64-windows-gnu",
            | Self::WindowsMsvc => "x86_64-pc-windows-msvc",
        }
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
}
