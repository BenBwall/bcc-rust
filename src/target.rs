//! [`Target::layout`] selects the scalar representations and ABI rules of
//! the requested C target, independently of the Rust host. Linux targets use
//! the System V LP64 model. Windows targets adjust that model for their type
//! widths, record layout, and variadic conventions.
//!
//! Selecting `x86_64-pc-windows-msvc` makes `long` four bytes and uses binary64
//! for `long double`. Selecting a Linux target keeps the LP64 representation.
//!
//! Read [`Target::layout`], [`Target`], and [`TargetLayout`], then
//! [`TargetLayout::integer`] and [`Target::predefined_macros`].
//!
//! Files by role:
//! - Target names and queries: `selection.rs`.
//! - Scalar types and layout tables: `model.rs`.
//! - Frozen macro lookup: `predefined.rs`; its `.h` tables stay beside it.
//!
//! C99: implementation-defined representations, §6.2.5, pp. 33-37;
//! PDF pp. 45-49. Record layout: §6.7.2.1 paragraphs 10-11, p. 102;
//! PDF p. 114. Enumeration representation: §6.7.2.2 paragraph 4, p. 105;
//! PDF p. 117.

// Representations
mod model;

// Target selection
mod selection;

// Predefined macros
mod predefined;

pub(crate) use model::{
    Layout,
    Scalar,
    TargetLayout,
};

impl Target {
    /// Scalar layout and ABI choices, including the Microsoft record rules.
    /// C99: §6.2.5, pp. 33-37; PDF pp. 45-49; §6.7.2.1p10-11, p. 102;
    /// PDF p. 114; enumeration choice §6.7.2.2p4, p. 105; PDF p. 117.
    pub(crate) const fn layout(self) -> TargetLayout {
        match self {
            | Self::LinuxGnu | Self::LinuxMusl => TargetLayout::LP64,
            | Self::WindowsGnu | Self::WindowsMsvc => {
                let mut layout = TargetLayout::LP64;
                layout.ms_bitfields = true;
                layout.va_list = Layout { size: 8, align: 8 };
                layout.va_list_is_pointer = true;
                layout.size_t = Scalar::UnsignedLongLong;
                layout.ptrdiff_t = Scalar::LongLong;
                layout.intmax_t = Scalar::LongLong;
                layout.uintmax_t = Scalar::UnsignedLongLong;
                layout.wchar_t = Scalar::UnsignedShort;
                layout.wint_t = Scalar::UnsignedShort;
                layout.scalars[Scalar::Long as usize] = Some(Layout { size: 4, align: 4 });
                layout.scalars[Scalar::UnsignedLong as usize] = Some(Layout { size: 4, align: 4 });
                if matches!(self, Self::WindowsMsvc) {
                    layout.fixed_enum_type = Some(Scalar::Int);
                    layout.scalars[Scalar::LongDouble as usize] =
                        Some(Layout { size: 8, align: 8 });
                    layout.scalars[Scalar::ComplexLongDouble as usize] = Some(Layout {
                        size:  16,
                        align: 8,
                    });
                }
                layout
            },
        }
    }
}

/// Selected C target, independent of the Rust compiler host.
/// C99: implementation-defined representations §6.2.5, pp. 33-37; PDF pp.
/// 45-49.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Target {
    #[default]
    LinuxGnu,
    LinuxMusl,
    WindowsGnu,
    WindowsMsvc,
}
