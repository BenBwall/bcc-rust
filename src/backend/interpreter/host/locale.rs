//! The program's locale: `setlocale` and the character encoding the wide
//! character functions convert with.
//!
//! The host knows glibc's two built-in locales: "C" (also spelled
//! "POSIX"), whose multibyte characters are single ASCII bytes, and
//! "C.UTF-8" (or "C.utf8"), whose `LC_CTYPE` encodes them as UTF-8; any
//! other name fails as an uninstalled locale does. The empty name, which
//! selects the environment's locale, selects "C", as for an environment
//! without `LANG` or `LC_*` variables, so that a run does not depend on the
//! host's settings. Only `LC_CTYPE` changes behaviour; every category keeps
//! its name, and `setlocale(LC_ALL, NULL)` describes mixed categories as
//! glibc's composite `LC_CTYPE=...;LC_NUMERIC=...` name does.
//!
//! A wide stream converts as glibc's streams do: in the "C" locale a wide
//! character beyond ASCII is written as `?` and a byte beyond ASCII read as
//! an encoding error (`EILSEQ`); in "C.UTF-8" both are UTF-8. The `printf`
//! family's `%lc` and `%ls`, which convert with `wcrtomb`, fail with
//! `EILSEQ` on a character the locale cannot encode.
//!
//! C99: §7.11.1.1 paragraphs 2-8, pp. 205-206; PDF pp. 217-218
//! (`setlocale`); §7.20 paragraph 3, p. 306; PDF p. 318 (`MB_CUR_MAX`).

use super::{
    DefaultHost,
    HostCall,
    HostReturn,
    integer,
};
use crate::{
    backend::interpreter::{
        arithmetic::sext,
        memory::{
            Memory,
            ObjectKind,
        },
        trap::Fault,
        value::RuntimeValue,
    },
    ir::{
        Align,
        Type,
    },
    util::bump::ArenaVec,
};

/// The functions this module runs.
pub(super) const FUNCTIONS: &[&str] = &["setlocale", "__ctype_get_mb_cur_max"];

/// glibc's `LC_ALL`, and the number of categories it covers.
const LC_ALL: i128 = 6;
const CATEGORIES: usize = 13;

/// glibc's category names, by category number (`LC_ALL` is 6).
const CATEGORY_NAMES: [&str; CATEGORIES] = [
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
    "LC_ALL",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
];

/// One of the locales the host knows, by the name a category reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Name {
    C,
    /// "C.UTF-8", or "C.utf8" as the program spelled it.
    Utf8 {
        lowercase: bool,
    },
}

impl Name {
    fn text(self) -> &'static str {
        match self {
            | Self::C => "C",
            | Self::Utf8 { lowercase: false } => "C.UTF-8",
            | Self::Utf8 { lowercase: true } => "C.utf8",
        }
    }

    fn parse(name: &[u8]) -> Option<Self> {
        Some(match name {
            | b"C" | b"POSIX" | b"" => Self::C,
            | b"C.UTF-8" => Self::Utf8 { lowercase: false },
            | b"C.utf8" => Self::Utf8 { lowercase: true },
            | _ => return None,
        })
    }
}

/// The program's locale, by category.
#[derive(Clone, Copy, Debug)]
pub(super) struct Locale {
    categories: [Name; CATEGORIES],
}

impl Locale {
    /// The "C" locale, which a program starts in.
    pub(super) const fn new() -> Self {
        Self {
            categories: [Name::C; CATEGORIES],
        }
    }

    /// Whether `LC_CTYPE` encodes multibyte characters as UTF-8.
    pub(super) fn utf8(self) -> bool {
        matches!(self.categories[0], Name::Utf8 { .. })
    }
}

impl DefaultHost<'_> {
    /// Runs a locale function, or `None` for another name.
    pub(super) fn locale_functions(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        FUNCTIONS
            .contains(&call.name)
            .then(|| self.run_locale(call, memory))
    }

    fn run_locale(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let arg = |index: usize| {
            call.args
                .get(index)
                .copied()
                .unwrap_or(RuntimeValue::Poison)
        };
        if call.name == "__ctype_get_mb_cur_max" {
            let maximum = if self.locale.utf8() { 6 } else { 1 };
            return Ok(HostReturn::Value(Some(RuntimeValue::int(
                Type::I64,
                maximum,
            ))));
        }
        let category = sext(Type::I32, integer(arg(0))?);
        let Some(index) = usize::try_from(category)
            .ok()
            .filter(|&index| index < CATEGORIES)
        else {
            return Ok(HostReturn::Value(Some(RuntimeValue::NULL)));
        };
        if arg(1) != RuntimeValue::NULL {
            self.text.clear();
            memory.read_c_string(arg(1), &mut self.text)?;
            let Some(name) = Name::parse(&self.text) else {
                return Ok(HostReturn::Value(Some(RuntimeValue::NULL)));
            };
            if category == LC_ALL {
                self.locale.categories = [name; CATEGORIES];
            } else {
                self.locale.categories[index] = name;
            }
        }
        self.text.clear();
        describe(self.locale, category, &mut self.text);
        let object = memory.allocate(
            ObjectKind::ConstantGlobal,
            self.text.len() as u64 + 1,
            Align::BYTE,
        )?;
        memory.initialize(object, &self.text);
        Ok(HostReturn::Value(Some(RuntimeValue::Ptr(object))))
    }
}

/// The name `setlocale` returns for `category`: its locale's, or for
/// `LC_ALL` with mixed categories glibc's composite name.
fn describe(locale: Locale, category: i128, text: &mut ArenaVec<'_, u8>) {
    let categories = &locale.categories;
    if category != LC_ALL {
        let index = usize::try_from(category).expect("a checked category");
        text.extend_from_slice(categories[index].text().as_bytes());
        return;
    }
    if categories.iter().all(|&name| name == categories[0]) {
        text.extend_from_slice(categories[0].text().as_bytes());
        return;
    }
    let mut first = true;
    for (index, name) in categories.iter().enumerate() {
        if index == 6 {
            continue;
        }
        if !first {
            text.push(b';');
        }
        first = false;
        text.extend_from_slice(CATEGORY_NAMES[index].as_bytes());
        text.push(b'=');
        text.extend_from_slice(name.text().as_bytes());
    }
}

/// Appends the multibyte form of the wide character `character` as
/// `wcrtomb` converts it, saying whether the locale can encode it.
pub(super) fn encode(utf8: bool, character: u32, out: &mut ArenaVec<'_, u8>) -> bool {
    if character < 0x80 {
        out.push(u8::try_from(character).expect("ASCII"));
        return true;
    }
    if !utf8 {
        return false;
    }
    let Some(character) = char::from_u32(character) else {
        return false;
    };
    let mut buffer = [0; 4];
    out.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
    true
}

/// Appends what a wide stream writes for `character`: its multibyte form,
/// or `?` where the locale cannot encode it, as glibc's stream conversion
/// writes it in the "C" locale.
pub(super) fn encode_for_stream(utf8: bool, character: u32, out: &mut ArenaVec<'_, u8>) {
    if !encode(utf8, character, out) {
        out.push(b'?');
    }
}

/// The wide character at the start of `bytes` as `mbrtowc` reads it, with
/// its length: `None` for an encoding error, or for a sequence `bytes`
/// ends inside.
pub(super) fn decode(utf8: bool, bytes: &[u8]) -> Option<(u32, usize)> {
    let &first = bytes.first()?;
    if first < 0x80 {
        return Some((u32::from(first), 1));
    }
    if !utf8 {
        return None;
    }
    let length = match first {
        | 0xC2..=0xDF => 2,
        | 0xE0..=0xEF => 3,
        | 0xF0..=0xF4 => 4,
        | _ => return None,
    };
    let text = std::str::from_utf8(bytes.get(..length)?).ok()?;
    let character = text.chars().next()?;
    Some((u32::from(character), length))
}

/// How many bytes the multibyte sequence starting with `first` has in
/// UTF-8, or 1 for any byte that does not start one.
pub(super) const fn sequence_length(utf8: bool, first: u8) -> usize {
    match first {
        | 0xC2..=0xDF if utf8 => 2,
        | 0xE0..=0xEF if utf8 => 3,
        | 0xF0..=0xF4 if utf8 => 4,
        | _ => 1,
    }
}
