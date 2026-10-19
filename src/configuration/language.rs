//! Language revisions, dialect selection, extension policy, Microsoft
//! feature switches, and borrowed preprocessing options.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct LanguageMode {
    pub(crate) standard: CStandard,
    pub(crate) gnu:      bool,
}

impl LanguageMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (standard, gnu) = match value {
            | "c89" | "c90" | "iso9899:1990" => (CStandard::C89, false),
            | "iso9899:199409" => (CStandard::C95, false),
            | "gnu89" | "gnu90" => (CStandard::C89, true),
            | "c99" | "c9x" | "iso9899:1999" | "iso9899:199x" => (CStandard::C99, false),
            | "gnu99" | "gnu9x" => (CStandard::C99, true),
            | "c11" | "c1x" | "iso9899:2011" => (CStandard::C11, false),
            | "gnu11" | "gnu1x" => (CStandard::C11, true),
            | "c17" | "c18" | "iso9899:2017" | "iso9899:2018" => (CStandard::C17, false),
            | "gnu17" | "gnu18" => (CStandard::C17, true),
            | "c23" | "c2x" | "iso9899:2024" => (CStandard::C23, false),
            | "gnu23" | "gnu2x" => (CStandard::C23, true),
            | "c2y" => (CStandard::C2y, false),
            | "gnu2y" => (CStandard::C2y, true),
            | _ => return None,
        };
        Some(Self { standard, gnu })
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum CStandard {
    C89,
    C95,
    C99,
    C11,
    C17,
    C23,
    C2y,
}

impl CStandard {
    /// C99: §6.10.8p1, p. 160; PDF p. 172. Later editions and the C95
    /// amendment prescribe their own versions. C2y follows Clang (202400L).
    pub(crate) const fn version_macro(self) -> Option<&'static str> {
        match self {
            | Self::C89 => None,
            | Self::C95 => Some("199409L\0"),
            | Self::C99 => Some("199901L\0"),
            | Self::C11 => Some("201112L\0"),
            | Self::C17 => Some("201710L\0"),
            | Self::C23 => Some("202311L\0"),
            | Self::C2y => Some("202400L\0"),
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::C89 => "C89",
            | Self::C95 => "C95",
            | Self::C99 => "C99",
            | Self::C11 => "C11",
            | Self::C17 => "C17",
            | Self::C23 => "C23",
            | Self::C2y => "C2y",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub(crate) enum MsvcFeature {
    Declspec,
    IntTypes,
    CallingConventions,
    TypeQualifiers,
    Inline,
    Seh,
    Asm,
    Pragma,
    AnonymousStructs,
    VaArgs,
}

impl MsvcFeature {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            | "declspec" => Some(Self::Declspec),
            | "int-types" => Some(Self::IntTypes),
            | "calling-conventions" => Some(Self::CallingConventions),
            | "type-qualifiers" => Some(Self::TypeQualifiers),
            | "inline" => Some(Self::Inline),
            | "seh" => Some(Self::Seh),
            | "asm" => Some(Self::Asm),
            | "pragma" => Some(Self::Pragma),
            | "anonymous-structs" => Some(Self::AnonymousStructs),
            | "va-args" => Some(Self::VaArgs),
            | _ => None,
        }
    }
}

impl MsvcFeature {
    pub(crate) const ALL: [Self; 10] = [
        Self::Declspec,
        Self::IntTypes,
        Self::CallingConventions,
        Self::TypeQualifiers,
        Self::Inline,
        Self::Seh,
        Self::Asm,
        Self::Pragma,
        Self::AnonymousStructs,
        Self::VaArgs,
    ];
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ExtensionPolicy {
    Allow,
    Warn,
    Deny,
}

/// Startup preprocessing operations, borrowed from the caller or an arena.
/// This is separate from the scalar language configuration so that the latter
/// remains a lifetime-free, constant value. GCC command-line extension to C99
/// §6.10.3p9, p. 152; PDF p. 164 and §6.10.3.5, p. 155; PDF p. 167.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreprocessingOption<'a> {
    Define(&'a str),
    Undefine(&'a str),
    Include(&'a str),
}
