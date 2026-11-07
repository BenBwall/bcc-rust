//! GCC's reading of a format, as GCC 14's `c-format.cc` makes it for its
//! `gnu_printf` and `gnu_scanf` archetypes on a GNU target: each conversion
//! specification's flags, operand number, width, precision and length
//! modifier, its conversion character's table entry, and the type each data
//! argument must have after the default argument promotions. GCC words the
//! format string's diagnostics at the character they are about when the
//! string is written where the call is, and otherwise at the string with a
//! note at the character; what concerns the call is at the call.
//! C99: §7.19.6.1p4-p17, pp. 275-281; PDF pp. 287-293;
//! §7.19.6.2p3-p13, pp. 282-287; PDF pp. 294-299.

use super::{
    super::{
        Analyzer,
        ArrayBound,
        BindingKind,
        Scalar,
        TypeId,
        TypeKind,
        TypeQualifiers,
        facts::FormatKind,
    },
    FormatCall,
    FormatNote,
    literal::{
        Leaf,
        Spelled,
    },
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator,
            Expression,
            ExpressionType as E,
            UnaryOperator,
        },
        preprocessing::KeywordTokenType,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

/// What GCC counts over a format's string literals before it decides
/// about the call as a whole (`format_check_results`).
#[derive(Default)]
struct Results {
    non_literal:       usize,
    /// Strings that checked out, or failed in a way that says nothing of
    /// the arguments' number.
    other:             usize,
    extra_args:        usize,
    dollar_extra_args: usize,
    empty:             usize,
    /// Strings a constant array cuts off before their null character.
    unterminated:      usize,
    /// Where the first string with too many arguments is.
    extra_site:        Option<SourceVectors>,
}

/// A conversion's table entry (`format_char_info`).
struct Conversion {
    chars:         &'static [u8],
    pointer_count: u8,
    /// The standard that has the conversion, for `-Wpedantic`.
    standard:      Standard,
    /// The wanted type for each length modifier, in [`LENGTHS`] order.
    types:         [Wanted; 12],
    /// The flags, width (`w`) and precision (`p`) it takes.
    flags:         &'static [u8],
    /// `c`: character types are lenient; `W`: it writes through its
    /// pointer; `R`: it reads through it; `[`: a scan set follows; `i`:
    /// the `0` flag is ignored with a precision.
    flags2:        &'static [u8],
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Standard {
    C89,
    C99,
    C23,
    Extension,
}

/// A wanted argument type: the type, the name GCC gives it if it has one,
/// and the standard that has the combination.
#[derive(Clone, Copy)]
struct Wanted {
    ty:       Option<Want>,
    name:     Option<&'static str>,
    standard: Standard,
}

/// The types GCC's tables name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Scalar(Scalar),
    Size,
    SignedSize,
    Ptrdiff,
    UnsignedPtrdiff,
    Intmax,
    Uintmax,
    WideInt,
    WideChar,
    /// A decimal floating type, which no argument of bcc's has.
    Decimal32,
    Decimal64,
    Decimal128,
}

const BAD: Wanted = Wanted {
    ty:       None,
    name:     None,
    standard: C89,
};

const fn t(ty: Want, name: Option<&'static str>, standard: Standard) -> Wanted {
    Wanted {
        ty: Some(ty),
        name,
        standard,
    }
}

const fn s(scalar: Scalar, standard: Standard) -> Wanted {
    t(Want::Scalar(scalar), None, standard)
}

use Scalar as S;
use Standard::{
    C23,
    C89,
    C99,
    Extension as EXT,
};

/// The length modifiers, in table order: none, `hh`, `h`, `l`, `ll`, `L`,
/// `z`, `t`, `j`, `H`, `D`, `DD`.
const LENGTHS: usize = 12;

const SIGNED: [Wanted; LENGTHS] = [
    s(S::Int, C89),
    s(S::SignedChar, C99),
    s(S::Short, C89),
    s(S::Long, C89),
    s(S::LongLong, C99),
    s(S::LongLong, EXT),
    t(Want::SignedSize, Some("signed size_t"), C99),
    t(Want::Ptrdiff, Some("ptrdiff_t"), C99),
    t(Want::Intmax, Some("intmax_t"), C99),
    BAD,
    BAD,
    BAD,
];

const fn unsigned(standard: Standard) -> [Wanted; LENGTHS] {
    let c99 = if matches!(standard, C23) { C23 } else { C99 };
    [
        s(S::UnsignedInt, standard),
        s(S::UnsignedChar, c99),
        s(S::UnsignedShort, standard),
        s(S::UnsignedLong, standard),
        s(S::UnsignedLongLong, c99),
        s(S::UnsignedLongLong, EXT),
        t(Want::Size, Some("size_t"), c99),
        t(Want::UnsignedPtrdiff, Some("unsigned ptrdiff_t"), c99),
        t(Want::Uintmax, Some("uintmax_t"), c99),
        BAD,
        BAD,
        BAD,
    ]
}

const fn floating(first: Scalar, standard: Standard) -> [Wanted; LENGTHS] {
    [
        s(first, standard),
        BAD,
        BAD,
        s(
            S::Double,
            if matches!(standard, C89) {
                C99
            } else {
                standard
            },
        ),
        BAD,
        s(S::LongDouble, standard),
        BAD,
        BAD,
        BAD,
        t(Want::Decimal32, None, EXT),
        t(Want::Decimal64, None, EXT),
        t(Want::Decimal128, None, EXT),
    ]
}

const fn only(first: Wanted, long: Wanted) -> [Wanted; LENGTHS] {
    [
        first, BAD, BAD, long, BAD, BAD, BAD, BAD, BAD, BAD, BAD, BAD,
    ]
}

const WRITTEN: [Wanted; LENGTHS] = [
    s(S::Int, C89),
    s(S::SignedChar, C99),
    s(S::Short, C89),
    s(S::Long, C89),
    s(S::LongLong, C99),
    BAD,
    t(Want::SignedSize, Some("signed size_t"), C99),
    t(Want::Ptrdiff, Some("ptrdiff_t"), C99),
    t(Want::Intmax, Some("intmax_t"), C99),
    BAD,
    BAD,
    BAD,
];

const fn conversion(
    chars: &'static [u8],
    pointer_count: u8,
    standard: Standard,
    types: &[Wanted; LENGTHS],
    flags: &'static [u8],
    flags2: &'static [u8],
) -> Conversion {
    Conversion {
        chars,
        pointer_count,
        standard,
        types: *types,
        flags,
        flags2,
    }
}

/// `print_char_table`.
const PRINTF: &[Conversion] = &[
    conversion(b"di", 0, C89, &SIGNED, b"-wp0 +'I", b"i"),
    conversion(b"oxX", 0, C89, &unsigned(C89), b"-wp0#", b"i"),
    conversion(b"u", 0, C89, &unsigned(C89), b"-wp0'I", b"i"),
    conversion(b"fgG", 0, C89, &floating(S::Double, C89), b"-wp0 +#'I", b""),
    conversion(b"eE", 0, C89, &floating(S::Double, C89), b"-wp0 +#I", b""),
    conversion(
        b"c",
        0,
        C89,
        &only(s(S::Int, C89), t(Want::WideInt, Some("wint_t"), C99)),
        b"-w",
        b"",
    ),
    conversion(
        b"s",
        1,
        C89,
        &only(s(S::Char, C89), t(Want::WideChar, Some("wchar_t"), C99)),
        b"-wp",
        b"cR",
    ),
    conversion(b"p", 1, C89, &only(s(S::Void, C89), BAD), b"-w", b"c"),
    conversion(b"n", 1, C89, &WRITTEN, b"", b"W"),
    conversion(b"F", 0, C99, &floating(S::Double, C99), b"-wp0 +#'I", b""),
    conversion(b"aA", 0, C99, &floating(S::Double, C99), b"-wp0 +#", b""),
    conversion(b"bB", 0, C23, &unsigned(C23), b"-wp0#", b"i"),
    conversion(
        b"C",
        0,
        EXT,
        &only(t(Want::WideInt, Some("wint_t"), EXT), BAD),
        b"-w",
        b"",
    ),
    conversion(
        b"S",
        1,
        EXT,
        &only(t(Want::WideChar, Some("wchar_t"), EXT), BAD),
        b"-wp",
        b"R",
    ),
    conversion(b"m", 0, EXT, &only(s(S::Void, EXT), BAD), b"-wp", b""),
];

/// `scan_char_table`.
const SCANF: &[Conversion] = &[
    conversion(b"di", 1, C89, &SIGNED, b"*w'I", b"W"),
    conversion(b"u", 1, C89, &unsigned(C89), b"*w'I", b"W"),
    conversion(b"oxX", 1, C89, &unsigned(C89), b"*w", b"W"),
    conversion(b"efgEG", 1, C89, &floating(S::Float, C89), b"*w'", b"W"),
    conversion(
        b"c",
        1,
        C89,
        &only(s(S::Char, C89), t(Want::WideChar, Some("wchar_t"), C99)),
        b"*mw",
        b"cW",
    ),
    conversion(
        b"s",
        1,
        C89,
        &only(s(S::Char, C89), t(Want::WideChar, Some("wchar_t"), C99)),
        b"*amw",
        b"cW",
    ),
    conversion(
        b"[",
        1,
        C89,
        &only(s(S::Char, C89), t(Want::WideChar, Some("wchar_t"), C99)),
        b"*amw",
        b"cW[",
    ),
    conversion(b"p", 2, C89, &only(s(S::Void, C89), BAD), b"*w", b"W"),
    conversion(b"n", 1, C89, &WRITTEN, b"", b"W"),
    conversion(b"F", 1, C99, &floating(S::Float, C99), b"*w'", b"W"),
    conversion(b"aA", 1, C99, &floating(S::Float, C99), b"*w'", b"W"),
    conversion(b"b", 1, C23, &unsigned(C23), b"*w", b"W"),
    conversion(
        b"C",
        1,
        EXT,
        &only(t(Want::WideChar, Some("wchar_t"), EXT), BAD),
        b"*mw",
        b"W",
    ),
    conversion(
        b"S",
        1,
        EXT,
        &only(t(Want::WideChar, Some("wchar_t"), EXT), BAD),
        b"*amw",
        b"W",
    ),
];

/// A length modifier as read: its spelling, its table index and standard.
#[derive(Clone, Copy)]
struct Length {
    chars:    &'static str,
    index:    usize,
    standard: Standard,
    /// Whether an argument of the wanted type's kind and width will do
    /// (`scalar_identity_flag`), as for Microsoft's `I64`.
    identity: bool,
}

/// `printf_length_specs` and `scanf_length_specs`: a spelling, its
/// doubled spelling, and their indexes and standards.
/// A length modifier's spelling, table index and standard, and those of
/// its doubled spelling.
type LengthSpec = (
    &'static str,
    usize,
    Standard,
    Option<(&'static str, usize, Standard)>,
    bool,
);

const PRINTF_LENGTHS: &[LengthSpec] = &[
    ("h", 2, C89, Some(("hh", 1, C99)), false),
    ("l", 3, C89, Some(("ll", 4, C99)), false),
    ("q", 4, EXT, None, false),
    ("L", 5, C89, None, false),
    ("z", 6, C99, None, false),
    ("Z", 6, EXT, None, false),
    ("t", 7, C99, None, false),
    ("j", 8, C99, None, false),
    ("H", 9, EXT, None, false),
    ("D", 10, EXT, Some(("DD", 11, EXT)), false),
];

const SCANF_LENGTHS: &[LengthSpec] = &[
    ("h", 2, C89, Some(("hh", 1, C99)), false),
    ("l", 3, C89, Some(("ll", 4, C99)), false),
    ("q", 4, EXT, None, false),
    ("L", 5, C89, None, false),
    ("z", 6, C99, None, false),
    ("t", 7, C99, None, false),
    ("j", 8, C99, None, false),
    ("H", 9, EXT, None, false),
    ("D", 10, EXT, Some(("DD", 11, EXT)), false),
];

/// `ms_printf_length_specs`, which `ms_scanf` shares: `I32`, `I64` and
/// `I` are `l`, `ll` and `z` for any argument of their width, and `L`
/// is `double`'s width on a `long double` it does not have.
const MS_LENGTHS: &[LengthSpec] = &[
    ("h", 2, C89, None, false),
    ("l", 3, C89, Some(("ll", 4, C89)), false),
    ("L", 5, C89, None, true),
    ("I32", 3, EXT, None, true),
    ("I64", 4, EXT, None, true),
    ("I", 6, EXT, None, true),
];

/// A Microsoft table row: its first seven columns, to `z`.
const fn ms(columns: [Wanted; 7]) -> [Wanted; LENGTHS] {
    let mut row = [BAD; LENGTHS];
    let mut index = 0;
    while index < 7 {
        row[index] = columns[index];
        index += 1;
    }
    row
}

const MS_SIGNED: [Wanted; LENGTHS] = ms([
    s(S::Int, C89),
    BAD,
    s(S::Short, C89),
    s(S::Long, C89),
    s(S::LongLong, C99),
    BAD,
    t(Want::SignedSize, Some("signed size_t"), C99),
]);

const MS_UNSIGNED: [Wanted; LENGTHS] = ms([
    s(S::UnsignedInt, C89),
    BAD,
    s(S::UnsignedShort, C89),
    s(S::UnsignedLong, C89),
    s(S::UnsignedLongLong, C99),
    BAD,
    t(Want::Size, Some("size_t"), C99),
]);

const MS_WRITTEN: [Wanted; LENGTHS] = {
    let mut row = ms([
        s(S::Int, C89),
        BAD,
        s(S::Short, C89),
        s(S::Long, C89),
        s(S::LongLong, C99),
        BAD,
        BAD,
    ]);
    row[8] = t(Want::Intmax, Some("intmax_t"), C99);
    row
};

const fn ms_floating(standard: Standard) -> [Wanted; LENGTHS] {
    ms([
        s(S::Double, standard),
        BAD,
        BAD,
        s(S::Double, C99),
        BAD,
        s(S::Double, standard),
        BAD,
    ])
}

const fn ms_wide(wide: Wanted, standard: Standard) -> [Wanted; LENGTHS] {
    ms([wide, BAD, s(S::Short, standard), BAD, BAD, BAD, BAD])
}

const fn ms_characters(h: Wanted) -> [Wanted; LENGTHS] {
    ms([
        s(S::Char, C89),
        BAD,
        h,
        t(Want::WideChar, Some("wchar_t"), C99),
        BAD,
        BAD,
        BAD,
    ])
}

/// `ms_print_char_table` (MinGW's `msformat-c.cc`).
const MS_PRINTF: &[Conversion] = &[
    conversion(b"di", 0, C89, &MS_SIGNED, b"-wp0 +'", b"i"),
    conversion(b"oxX", 0, C89, &MS_UNSIGNED, b"-wp0#", b"i"),
    conversion(b"u", 0, C89, &MS_UNSIGNED, b"-wp0'", b"i"),
    conversion(b"fgG", 0, C89, &ms_floating(C89), b"-wp0 +#'", b""),
    conversion(b"eE", 0, C89, &ms_floating(C89), b"-wp0 +#", b""),
    conversion(
        b"c",
        0,
        C89,
        &ms([
            s(S::Int, C89),
            BAD,
            s(S::Short, C89),
            t(Want::WideInt, Some("wint_t"), C99),
            BAD,
            BAD,
            BAD,
        ]),
        b"-w",
        b"",
    ),
    conversion(
        b"s",
        1,
        C89,
        &ms_characters(s(S::Short, C89)),
        b"-wp",
        b"cR",
    ),
    conversion(b"p", 1, C89, &only(s(S::Void, C89), BAD), b"-w", b"c"),
    conversion(b"n", 1, C89, &MS_WRITTEN, b"", b"W"),
    conversion(b"aA", 0, C99, &ms_floating(C99), b"-wp0 +#", b""),
    conversion(
        b"C",
        0,
        EXT,
        &ms_wide(t(Want::WideInt, Some("wint_t"), EXT), C89),
        b"-w",
        b"",
    ),
    conversion(
        b"S",
        1,
        EXT,
        &ms_wide(t(Want::WideChar, Some("wchar_t"), EXT), C89),
        b"-wp",
        b"R",
    ),
];

/// `ms_scan_char_table`.
const MS_SCANF: &[Conversion] = &[
    conversion(b"di", 1, C89, &MS_SIGNED, b"*w'", b"W"),
    conversion(b"u", 1, C89, &MS_UNSIGNED, b"*w'", b"W"),
    conversion(b"oxX", 1, C89, &MS_UNSIGNED, b"*w", b"W"),
    conversion(
        b"efgEG",
        1,
        C89,
        &only(s(S::Float, C89), s(S::Double, C89)),
        b"*w'",
        b"W",
    ),
    conversion(b"c", 1, C89, &ms_characters(s(S::Short, C89)), b"*w", b"cW"),
    conversion(
        b"s",
        1,
        C89,
        &ms_characters(s(S::Short, C89)),
        b"*aw",
        b"cW",
    ),
    conversion(b"[", 1, C89, &ms_characters(BAD), b"*aw", b"cW["),
    conversion(b"p", 2, C89, &only(s(S::Void, C89), BAD), b"*w", b"W"),
    conversion(
        b"n",
        1,
        C89,
        &ms([
            s(S::Int, C89),
            BAD,
            s(S::Short, C89),
            s(S::Long, C89),
            s(S::LongLong, C99),
            BAD,
            BAD,
        ]),
        b"",
        b"W",
    ),
    conversion(
        b"C",
        1,
        EXT,
        &ms_wide(t(Want::WideChar, Some("wchar_t"), EXT), C89),
        b"*w",
        b"W",
    ),
    conversion(
        b"S",
        1,
        EXT,
        &ms_wide(t(Want::WideChar, Some("wchar_t"), EXT), C89),
        b"*aw",
        b"W",
    ),
];

/// A flag's name in GCC's words (`printf_flag_specs`, `scanf_flag_specs`).
const fn flag_name(flag: u8, scanf: bool) -> &'static str {
    match flag {
        | b' ' => "' ' flag",
        | b'+' => "'+' flag",
        | b'#' => "'#' flag",
        | b'0' => "'0' flag",
        | b'-' => "'-' flag",
        | b'\'' => "''' flag",
        | b'I' => "'I' flag",
        | b'w' => "field width",
        | b'p' => "precision",
        | b'L' => "length modifier",
        | b'*' => "assignment suppression",
        | b'a' => "'a' flag",
        | b'm' => "'m' flag",
        | _ if scanf => "flag",
        | _ => "flag",
    }
}

/// One wanted type to check against an argument, in the order GCC checks
/// them: the field width, the precision, then the conversion's own.
#[derive(Clone, Copy)]
struct WantedArgument<'tu> {
    kind:          Kind,
    wanted:        Wanted,
    pointer_count: u8,
    /// The conversion's `flags2`: `c` for lenient character types, `R`
    /// when it reads through its pointer; none for a width or precision.
    flags2:        &'static [u8],
    writing:       bool,
    /// The argument, if there is one.
    argument:      Option<&'tu Expression<'tu>>,
    /// Its position among the call's arguments, from 1.
    number:        usize,
    /// The specification's text GCC quotes: from its start for the
    /// conversion, the `*` or `.*` for a width or precision.
    quoted:        (usize, usize),
    /// The byte the caret is at.
    caret:         usize,
    /// Whether any argument of the wanted type's kind and width will do.
    identity:      bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Format,
    Width,
    Precision,
}

/// The state of one string's reading (`argument_parser` and the loop of
/// `check_format_info_main`).
struct Reading<'a, 'tu> {
    call:               &'a FormatCall<'a, 'tu>,
    leaf:               Leaf<'tu>,
    bytes:              &'tu [u8],
    scanf:              bool,
    /// The last data argument read, from 1 as GCC numbers them; the next
    /// is the call's argument at this index from 0.
    arg_num:            usize,
    /// -1 until a specification says, then whether they number operands.
    has_operand_number: i8,
    dollar_used:        ArenaVec<'a, bool>,
    dollar_max:         usize,
    dollar_warned:      bool,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks a call's format as GCC does (`check_format_info`).
    pub(super) fn gcc_check_format(&mut self, call: &FormatCall<'_, 'tu>) {
        let mut results = Results::default();
        let mut leaves = ArenaVec::new_in(self.scratch);
        self.gcc_leaves(call.format, true, &mut leaves, 0);
        // GCC reports what it says of the whole format at the last address
        // it read one from, and at the call when there was none.
        let mut location = None;
        for &leaf in &leaves {
            match leaf {
                | GccLeaf::Literal(leaf) => {
                    location = Some(leaf.site);
                    // A `strftime` format is not read here.
                    if call.kind != FormatKind::Unchecked {
                        self.gcc_check_leaf(call, leaf, &mut results);
                    }
                },
                | GccLeaf::NonLiteral(site) => {
                    location = site.or(location);
                    results.non_literal += 1;
                },
                | GccLeaf::Null => results.other += 1,
            }
        }
        let location = location.unwrap_or(call.input_location);
        if call.kind == FormatKind::Unchecked {
            if results.non_literal > 0 {
                self.format_warning(
                    call.call.source_vectors,
                    G::FormatNonliteral,
                    "format not a string literal, format string not checked",
                    location,
                    None,
                );
            }
            return;
        }
        if results.non_literal > 0 && !call.takes_va_list() {
            // GCC picks the words by its options, which the command line or
            // a pragma that enables them sets, and a pragma that ignores
            // one does not clear; the pragmas then say what is reported.
            let options = self.context.configuration;
            let here = self.context.configuration_at(call.call.source_vectors);
            let on = |group| {
                options.diagnostic_severity(group).is_some()
                    || here.diagnostic_severity(group).is_some()
            };
            let (security, nonliteral) = (on(G::FormatSecurity), on(G::FormatNonliteral));
            if call.data().is_empty() && security {
                self.format_warning(
                    call.call.source_vectors,
                    G::FormatSecurity,
                    "format not a string literal and no format arguments",
                    location,
                    None,
                );
            } else if call.data().is_empty() && nonliteral {
                self.format_warning(
                    call.call.source_vectors,
                    G::FormatNonliteral,
                    "format not a string literal and no format arguments",
                    location,
                    None,
                );
            } else {
                self.format_warning(
                    call.call.source_vectors,
                    G::FormatNonliteral,
                    "format not a string literal, argument types not checked",
                    location,
                    None,
                );
            }
        }
        let silent = results.non_literal > 0 || results.other > 0;
        if results.extra_args > 0 && !silent {
            let site = results.extra_site.unwrap_or(location);
            self.format_warning(
                call.call.source_vectors,
                G::FormatExtraArgs,
                "too many arguments for format",
                site,
                None,
            );
        }
        if results.dollar_extra_args > 0 && !silent {
            self.format_warning(
                call.call.source_vectors,
                G::FormatExtraArgs,
                "unused arguments in '$'-style format",
                location,
                None,
            );
        }
        if results.empty > 0 && !silent {
            let name = archetype(call.kind, call.ms);
            let words = format_in!(self.scratch, "zero-length {name} format string");
            self.format_warning(
                call.call.source_vectors,
                G::FormatZeroLength,
                words,
                location,
                None,
            );
        }
        if results.unterminated > 0 {
            self.format_warning(
                call.call.source_vectors,
                G::Format,
                "unterminated format string",
                location,
                None,
            );
        }
    }

    /// The string literals GCC reaches from the format argument `e`
    /// (`check_function_arguments_recurse` and `check_format_arg`):
    /// through conversions between pointers, a conditional's arms, a
    /// `format_arg` call's argument, a constant array's initializer and a
    /// constant offset. A null pointer constant counts as nothing.
    fn gcc_leaves(
        &mut self,
        e: &'tu Expression<'tu>,
        bare: bool,
        leaves: &mut ArenaVec<'_, GccLeaf<'tu>>,
        depth: u32,
    ) {
        if depth > 64 {
            leaves.push(GccLeaf::NonLiteral(None));
            return;
        }
        let site = e;
        let mut e = e;
        let mut bare = bare;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => {
                    e = expression;
                    bare = false;
                },
                | E::Cast {
                    operand_expression, ..
                } if self.pointer_target(self.expression_info(e).ty).is_some() && {
                    let operand = self.decayed_type(operand_expression);
                    self.pointer_target(operand).is_some()
                } =>
                    e = operand_expression,
                // GCC reads the operand a selection chooses, which keeps its
                // place for `_Generic` and takes the builtin's for
                // `__builtin_choose_expr`.
                | E::Generic(_) => match self.expression_info(e).selected_expression {
                    | Some(selected) => e = selected,
                    | None => break,
                },
                | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                    match self.expression_info(e).selected_expression {
                        | Some(selected) => {
                            e = selected;
                            bare = false;
                        },
                        | None => break,
                    },
                | _ => break,
            }
        }
        let info = self.expression_info(e);
        if (info.ice && info.integer.is_some_and(super::super::Integer::is_zero))
            || self.null_pointer_constant(info)
        {
            leaves.push(GccLeaf::Null);
            return;
        }
        // GCC places a format that is no string at its address, `&...` or
        // an array's, and anything else at the call.
        let non_literal = |analyzer: &Self, e: &'tu Expression<'tu>| {
            let address = matches!(
                analyzer.types.nodes[analyzer.expression_info(e).ty.index],
                TypeKind::Array(..)
            ) || matches!(
                e.kind,
                E::Unary {
                    operator: UnaryOperator::AddressOf,
                    ..
                }
            );
            GccLeaf::NonLiteral(address.then_some(site.source_vectors))
        };
        match e.kind {
            | E::StringLiteral(_) => match self.leaf(e, site, 0, true, bare) {
                | Some(leaf) => leaves.push(GccLeaf::Literal(leaf)),
                | None => leaves.push(non_literal(self, e)),
            },
            | E::Conditional(conditional) => {
                let condition = self.expression_info(conditional.condition_expression);
                let truth = condition
                    .ice
                    .then(|| Self::constant_truth(condition))
                    .flatten();
                if truth != Some(false) {
                    self.gcc_leaves(conditional.then_expression, true, leaves, depth + 1);
                }
                if truth != Some(true) {
                    self.gcc_leaves(conditional.else_expression, true, leaves, depth + 1);
                }
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                let formats = self.format_arguments_of(function_expression, arguments);
                if formats.is_empty() {
                    leaves.push(non_literal(self, e));
                }
                // GCC reads the attributes from the last written.
                for &argument in formats.iter().rev() {
                    self.gcc_leaves(argument, true, leaves, depth + 1);
                }
            },
            | E::Identifier(_) => match self
                .constant_array_string(e)
                .and_then(|literal| self.leaf(literal, site, 0, false, false))
            {
                | Some(mut leaf) => {
                    let ty = self.expression_info(e).ty;
                    if let TypeKind::Array(_, ArrayBound::Constant(count)) =
                        self.types.nodes[ty.index]
                    {
                        leaf.extent = usize::try_from(count).ok();
                    }
                    leaves.push(GccLeaf::Literal(leaf));
                },
                | None => leaves.push(non_literal(self, e)),
            },
            // GCC places a conversion of `"..." + N` within the string, and
            // of anything else at the outermost operator; what concerns the
            // whole string at the expression's first operand.
            | _ => match self
                .literal_offset(e)
                .and_then(|(literal, offset, direct)| {
                    let mut leaf =
                        self.leaf(literal, Self::gcc_leftmost(e), offset, true, bare && direct)?;
                    leaf.char_site = e.operator_source_vectors.unwrap_or(e.source_vectors);
                    Some(leaf)
                }) {
                | Some(leaf) => leaves.push(GccLeaf::Literal(leaf)),
                | None => leaves.push(non_literal(self, e)),
            },
        }
    }

    /// The first operand of `e`, through parentheses and the left operands
    /// of binary operators: where GCC's location of `e` begins.
    fn gcc_leftmost(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Binary {
                    left_expression, ..
                } => e = left_expression,
                | _ => return e,
            }
        }
    }

    /// Checks one string literal of a format (`check_format_arg` from the
    /// string on, and `check_format_info_main`).
    fn gcc_check_leaf(
        &mut self,
        call: &FormatCall<'_, 'tu>,
        leaf: Leaf<'tu>,
        results: &mut Results,
    ) {
        let mut bytes = self.format_bytes(&leaf);
        // An array shorter than its string keeps only its extent, which must
        // end in a null character.
        if let Some(extent) = leaf.extent
            && extent <= bytes.len()
        {
            if extent == 0 || bytes[extent - 1] != 0 {
                results.unterminated += 1;
                return;
            }
            bytes = &bytes[..extent - 1];
        }
        if bytes.is_empty() {
            results.empty += 1;
            return;
        }
        results.other += 1;
        let scratch = self.scratch;
        let mut reading = Reading {
            call,
            leaf,
            bytes,
            scanf: call.kind == FormatKind::Scanf,
            arg_num: call.first.unwrap_or(call.format_index + 1),
            has_operand_number: -1,
            dollar_used: ArenaVec::new_in(scratch),
            dollar_max: 0,
            dollar_warned: false,
        };
        if !self.gcc_read_specifications(&mut reading) {
            return;
        }
        let end = bytes.iter().position(|&byte| byte == 0);
        if let Some(nul) = end {
            self.gcc_char_warning(
                &reading,
                G::FormatContainsNul,
                "embedded '\\0' in format",
                nul,
            );
        }
        let data_left = call.first.is_some() && reading.arg_num < call.arguments.len();
        if data_left && reading.has_operand_number <= 0 {
            results.other -= 1;
            results.extra_args += 1;
            _ = results.extra_site.get_or_insert(leaf.site);
        }
        if reading.has_operand_number > 0 {
            self.gcc_finish_dollars(&reading, results);
        }
    }

    /// Reads each conversion specification of the string; `false` when
    /// GCC stops reading the string without counting its arguments.
    fn gcc_read_specifications(&mut self, r: &mut Reading<'_, 'tu>) -> bool {
        let at = |i: usize| r.bytes.get(i).copied().unwrap_or(0);
        let mut i = 0;
        while at(i) != 0 {
            let here = at(i);
            i += 1;
            if here != b'%' {
                continue;
            }
            if at(i) == 0 {
                self.gcc_char_warning(r, G::Format, "spurious trailing '%' in format", i - 1);
                continue;
            }
            if at(i) == b'%' {
                i += 1;
                continue;
            }
            let start = i - 1;
            match self.gcc_specification(r, start, &mut i) {
                | Step::Next => {},
                | Step::Stop => return false,
            }
        }
        true
    }

    /// Reads and checks the specification that begins with the `%` at
    /// `start`, leaving `i` after it.
    fn gcc_specification(&mut self, r: &mut Reading<'_, 'tu>, start: usize, i: &mut usize) -> Step {
        let bytes = r.bytes;
        let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
        let scanf = r.scanf;
        let first_arg = r.call.first;
        let pedantic = self.gcc_pedantic();
        let mut main_arg_num = 0;
        // An operand number, `n$`.
        if at(*i).is_ascii_digit() && r.has_operand_number != 0 {
            match self.gcc_dollar(r, i, false) {
                | Dollar::Error => return Step::Stop,
                | Dollar::Number(number) => {
                    r.has_operand_number = 1;
                    main_arg_num = number + r.call.first.unwrap_or(0);
                },
                | Dollar::None => {},
            }
        } else if self.gcc_avoid_dollar(r, *i) {
            return Step::Stop;
        }
        // Flags.
        let mut flags = ArenaVec::new_in(self.scratch);
        let flag_chars: &[u8] = match (scanf, r.call.ms) {
            | (true, false) => b"*'I",
            | (false, false) => b" +#0-'I",
            | (true, true) => b"*'",
            | (false, true) => b" +#0-'",
        };
        while at(*i) != 0 && flag_chars.contains(&at(*i)) {
            let flag = at(*i);
            if flags.contains(&flag) {
                let words = format_in!(
                    self.scratch,
                    "repeated {} in format",
                    flag_name(flag, scanf)
                );
                self.gcc_char_warning(r, G::Format, words, *i);
            } else {
                flags.push(flag);
            }
            *i += 1;
        }
        let mut wanted = ArenaVec::new_in(self.scratch);
        // The width, possibly `*` or `*m$`.
        if !scanf && at(*i) == b'*' {
            *i += 1;
            let mut taken = None;
            if r.has_operand_number != 0 {
                match self.gcc_dollar(r, i, r.has_operand_number == 1) {
                    | Dollar::Error => return Step::Stop,
                    | Dollar::Number(number) => {
                        r.has_operand_number = 1;
                        taken = Some(number + r.call.first.unwrap_or(0));
                    },
                    | Dollar::None => r.has_operand_number = 0,
                }
            } else if self.gcc_avoid_dollar(r, *i) {
                return Step::Stop;
            }
            if first_arg.is_some() {
                let number = taken.unwrap_or_else(|| {
                    r.arg_num += 1;
                    r.arg_num
                });
                wanted.push(WantedArgument {
                    kind: Kind::Width,
                    wanted: s(S::Int, C89),
                    pointer_count: 0,
                    flags2: b"",
                    writing: false,
                    argument: r.call.arguments.get(number - 1).copied(),
                    number,
                    quoted: (*i - 1, 1),
                    caret: *i - 1,
                    identity: false,
                });
            }
            flags.push(b'w');
        } else {
            let mut found = false;
            let mut non_zero = false;
            while at(*i).is_ascii_digit() {
                found = true;
                non_zero |= at(*i) != b'0';
                *i += 1;
            }
            if found && !non_zero && scanf {
                let words = format_in!(
                    self.scratch,
                    "zero width in {} format",
                    archetype(r.call.kind, r.call.ms)
                );
                self.format_warning(
                    r.call.call.source_vectors,
                    G::Format,
                    words,
                    r.leaf.site,
                    None,
                );
            }
            if found {
                flags.push(b'w');
            }
        }
        // The precision, possibly `.*` or `.*m$`.
        if !scanf && at(*i) == b'.' {
            *i += 1;
            if at(*i) == b'*' {
                *i += 1;
                let mut taken = None;
                if r.has_operand_number != 0 {
                    match self.gcc_dollar(r, i, r.has_operand_number == 1) {
                        | Dollar::Error => return Step::Stop,
                        | Dollar::Number(number) => {
                            r.has_operand_number = 1;
                            taken = Some(number + r.call.first.unwrap_or(0));
                        },
                        | Dollar::None => r.has_operand_number = 0,
                    }
                } else if self.gcc_avoid_dollar(r, *i) {
                    return Step::Stop;
                }
                if first_arg.is_some() {
                    let number = taken.unwrap_or_else(|| {
                        r.arg_num += 1;
                        r.arg_num
                    });
                    wanted.push(WantedArgument {
                        kind: Kind::Precision,
                        wanted: s(S::Int, C89),
                        pointer_count: 0,
                        flags2: b"",
                        writing: false,
                        argument: r.call.arguments.get(number - 1).copied(),
                        number,
                        quoted: (*i - 2, 2),
                        caret: *i - 1,
                        identity: false,
                    });
                }
            } else {
                while at(*i).is_ascii_digit() {
                    *i += 1;
                }
            }
            flags.push(b'p');
        }
        let format_start = *i;
        // The scanf allocation flag, `m`.
        let mut alloc = false;
        if scanf && !r.call.ms && at(*i) == b'm' {
            flags.push(b'm');
            alloc = true;
            *i += 1;
        }
        // The length modifier.
        let length = Self::gcc_length(r, i);
        if pedantic
            && let Some(length) = length
            && length.standard == EXT
        {
            let words = format_in!(
                self.scratch,
                "ISO C does not support the '{}' {} length modifier",
                length.chars,
                archetype(r.call.kind, r.call.ms)
            );
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                words,
                r.leaf.site,
                None,
            );
        }
        if scanf && length.is_some() {
            flags.push(b'L');
        }
        let conversion_char = at(*i);
        if conversion_char == 0 || conversion_char == b'%' {
            self.gcc_char_warning(
                r,
                G::Format,
                "conversion lacks type at end of format",
                (*i).max(1) - 1,
            );
            return Step::Next;
        }
        *i += 1;
        let table = match (scanf, r.call.ms) {
            | (true, false) => SCANF,
            | (false, false) => PRINTF,
            | (true, true) => MS_SCANF,
            | (false, true) => MS_PRINTF,
        };
        let Some(conversion) = table
            .iter()
            .find(|conversion| conversion.chars.contains(&conversion_char))
        else {
            let words = format_in!(
                self.scratch,
                "unknown conversion type character {} in format",
                quoted_char(&mut ArenaString::new_in(self.scratch), conversion_char)
            );
            self.gcc_char_warning(r, G::Format, words, *i - 1);
            return Step::Next;
        };
        let name = archetype(r.call.kind, r.call.ms);
        if pedantic && conversion.standard == EXT {
            let words = format_in!(
                self.scratch,
                "ISO C does not support the '%{}' {name} format",
                conversion_char as char
            );
            self.gcc_char_warning(r, G::Format, words, *i - 1);
        }
        // Flags the conversion does not take, which GCC then drops.
        let mut valid = ArenaVec::new_in(self.scratch);
        for &flag in &flags {
            // The length modifier stands in the flags only for the pairs.
            if flag == b'L' {
                valid.push(flag);
                continue;
            }
            if !conversion.flags.contains(&flag) {
                let words = format_in!(
                    self.scratch,
                    "{} used with '%{}' {name} format",
                    flag_name(flag, scanf),
                    conversion_char as char
                );
                self.gcc_char_warning(r, G::Format, words, *i - 1);
                continue;
            }
            if pedantic && matches!(flag, b'\'' | b'I' | b'm') {
                let words = format_in!(
                    self.scratch,
                    "ISO C does not support the {} {} flag",
                    quoted_char(&mut ArenaString::new_in(self.scratch), flag),
                    if scanf { "scanf" } else { "printf" }
                );
                self.format_warning(
                    r.call.call.source_vectors,
                    G::Format,
                    words,
                    r.leaf.site,
                    None,
                );
            }
            valid.push(flag);
        }
        let flags = valid;
        let alloc = alloc && flags.contains(&b'm');
        let suppressed = scanf && flags.contains(&b'*');
        // Pairs of flags.
        let pairs: &[(u8, u8, bool, u8)] = if scanf && r.call.ms {
            &[(b'*', b'L', false, 0)]
        } else if scanf {
            &[(b'*', b'L', false, 0), (b'a', b'm', false, 0)]
        } else {
            &[
                (b' ', b'+', true, 0),
                (b'0', b'-', true, 0),
                (b'0', b'p', true, b'i'),
            ]
        };
        for &(first, second, ignored, predicate) in pairs {
            if !flags.contains(&first)
                || !flags.contains(&second)
                || (predicate != 0 && !conversion.flags2.contains(&predicate))
            {
                continue;
            }
            let (first, second) = (flag_name(first, scanf), flag_name(second, scanf));
            let words = if !ignored {
                format_in!(
                    self.scratch,
                    "use of {first} and {second} together in {name} format"
                )
            } else if predicate != 0 {
                format_in!(
                    self.scratch,
                    "{first} ignored with {second} and '%{}' {name} format",
                    conversion_char as char
                )
            } else {
                format_in!(
                    self.scratch,
                    "{first} ignored with {second} in {name} format"
                )
            };
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                words,
                r.leaf.site,
                None,
            );
        }
        // A scan set.
        if conversion.flags2.contains(&b'[') {
            if at(*i) == b'^' {
                *i += 1;
            }
            if at(*i) == b']' {
                *i += 1;
            }
            while at(*i) != 0 && at(*i) != b']' {
                *i += 1;
            }
            if at(*i) != b']' {
                self.gcc_char_warning(
                    r,
                    G::Format,
                    "no closing ']' for '%[' format",
                    (*i).max(1) - 1,
                );
            }
        }
        // The wanted type for the length.
        let length_index = length.map_or(0, |length| length.index);
        let main = conversion.types[length_index];
        if main.ty.is_none() {
            let words = format_in!(
                self.scratch,
                "use of '{}' length modifier with '{}' type character has either no effect or \
                 undefined behavior",
                length.map_or("", |length| length.chars),
                conversion_char as char
            );
            self.gcc_char_warning(r, G::Format, words, *i - 1);
            r.arg_num += 1;
            return Step::Next;
        }
        if self.gcc_pedantic()
            && let Some(length) = length
            && main.standard == EXT
            && length.standard != EXT
            && conversion.standard != EXT
        {
            let words = format_in!(
                self.scratch,
                "ISO C does not support the '%{}{}' {name} format",
                length.chars,
                conversion_char as char
            );
            self.gcc_char_warning(r, G::Format, words, *i - 1);
        }
        // The argument.
        if first_arg.is_none() {
            return Step::Next;
        }
        let takes_none = conversion.pointer_count == 0 && main.ty == Some(Want::Scalar(S::Void));
        if takes_none || suppressed {
            if main_arg_num != 0 {
                let words = if suppressed {
                    "operand number specified with suppressed assignment"
                } else {
                    "operand number specified for format taking no argument"
                };
                self.format_warning(
                    r.call.call.source_vectors,
                    G::Format,
                    words,
                    r.leaf.site,
                    None,
                );
            }
        } else {
            let number = if main_arg_num != 0 {
                main_arg_num
            } else {
                if r.has_operand_number > 0 {
                    self.format_warning(
                        r.call.call.source_vectors,
                        G::Format,
                        "missing $ operand number in format",
                        r.leaf.site,
                        None,
                    );
                    return Step::Stop;
                }
                r.has_operand_number = 0;
                r.arg_num += 1;
                r.arg_num
            };
            wanted.push(WantedArgument {
                kind: Kind::Format,
                wanted: main,
                pointer_count: conversion.pointer_count + u8::from(alloc),
                flags2: conversion.flags2,
                writing: alloc || conversion.flags2.contains(&b'W'),
                argument: r.call.arguments.get(number - 1).copied(),
                number,
                quoted: (format_start, *i - format_start),
                caret: *i - 1,
                identity: length.is_some_and(|length| length.identity),
            });
        }
        for &wanted in &wanted {
            self.gcc_check_type(r, wanted, start);
        }
        Step::Next
    }

    /// Reads a length modifier at `i`.
    fn gcc_length(r: &Reading<'_, 'tu>, i: &mut usize) -> Option<Length> {
        let rest = r.bytes.get(*i..).unwrap_or_default();
        let specs = if r.call.ms {
            MS_LENGTHS
        } else if r.scanf {
            SCANF_LENGTHS
        } else {
            PRINTF_LENGTHS
        };
        let &(name, index, standard, double, identity) = specs
            .iter()
            .find(|spec| rest.starts_with(spec.0.as_bytes()))?;
        *i += name.len();
        if let Some((double, double_index, double_standard)) = double
            && r.bytes.get(*i) == Some(&name.as_bytes()[0])
        {
            *i += 1;
            return Some(Length {
                chars:    double,
                index:    double_index,
                standard: double_standard,
                identity: false,
            });
        }
        Some(Length {
            chars: name,
            index,
            standard,
            identity,
        })
    }

    /// Reads an operand number `n$` at `i` (`maybe_read_dollar_number`).
    fn gcc_dollar(&mut self, r: &mut Reading<'_, 'tu>, i: &mut usize, needed: bool) -> Dollar {
        let at = |index: usize| r.bytes.get(index).copied().unwrap_or(0);
        let mut j = *i;
        let mut number: u64 = 0;
        let mut overflow = false;
        if !at(j).is_ascii_digit() {
            return self.gcc_missing_dollar(r, needed);
        }
        // GCC flags a number past `INT_MAX` and reads all its digits.
        while at(j).is_ascii_digit() {
            number = number * 10 + u64::from(at(j) - b'0');
            if number > i32::MAX as u64 {
                overflow = true;
                number = i32::MAX as u64;
            }
            j += 1;
        }
        if at(j) != b'$' {
            return self.gcc_missing_dollar(r, needed);
        }
        *i = j + 1;
        if self.gcc_pedantic() && !r.dollar_warned {
            r.dollar_warned = true;
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                "ISO C does not support %n$ operand number formats",
                r.call.input_location,
                None,
            );
        }
        let count = r
            .call
            .first
            .map(|first| r.call.arguments.len().saturating_sub(first));
        let number = usize::try_from(number).unwrap_or(usize::MAX);
        if overflow || number == 0 || count.is_some_and(|count| number > count) {
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                "operand number out of range in format",
                r.call.input_location,
                None,
            );
            return Dollar::Error;
        }
        r.dollar_max = r.dollar_max.max(number);
        while r.dollar_used.len() < number {
            r.dollar_used.push(false);
        }
        r.dollar_used[number - 1] = true;
        Dollar::Number(number)
    }

    fn gcc_missing_dollar(&mut self, r: &Reading<'_, 'tu>, needed: bool) -> Dollar {
        if needed {
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                "missing $ operand number in format",
                r.call.input_location,
                None,
            );
            Dollar::Error
        } else {
            Dollar::None
        }
    }

    /// Whether an operand number follows a format without them
    /// (`avoid_dollar_number`), which GCC reports and stops at.
    fn gcc_avoid_dollar(&mut self, r: &Reading<'_, 'tu>, mut i: usize) -> bool {
        let at = |index: usize| r.bytes.get(index).copied().unwrap_or(0);
        if !at(i).is_ascii_digit() {
            return false;
        }
        while at(i).is_ascii_digit() {
            i += 1;
        }
        if at(i) == b'$' {
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                "'$'operand number used after format without operand number",
                r.call.input_location,
                None,
            );
            return true;
        }
        false
    }

    /// GCC's account of the operand numbers a string used
    /// (`finish_dollar_format_checking`).
    fn gcc_finish_dollars(&mut self, r: &Reading<'_, 'tu>, results: &mut Results) {
        let mut pointer_gap = false;
        for index in 0..r.dollar_max {
            if r.dollar_used.get(index).copied().unwrap_or(false) {
                continue;
            }
            let data = r.call.first.map_or(0, |first| first + index);
            let pointer = r.call.arguments.get(data).is_some_and(|&argument| {
                self.pointer_target(self.expression_info(argument).ty)
                    .is_some()
            });
            if r.scanf && (r.call.first.is_none() || pointer) {
                pointer_gap = true;
            } else {
                let words = format_in!(
                    self.scratch,
                    "format argument {} unused before used argument {} in '$'-style format",
                    index + 1,
                    r.dollar_max
                );
                self.format_warning(
                    r.call.call.source_vectors,
                    G::Format,
                    words,
                    r.leaf.site,
                    None,
                );
            }
        }
        let count = r
            .call
            .first
            .map_or(0, |first| r.call.arguments.len().saturating_sub(first));
        if pointer_gap || (r.call.first.is_some() && r.dollar_max < count) {
            results.other -= 1;
            results.dollar_extra_args += 1;
        }
    }

    /// Checks one wanted type against its argument (`check_format_types`).
    fn gcc_check_type(&mut self, r: &Reading<'_, 'tu>, wanted: WantedArgument<'tu>, start: usize) {
        let Some(want) = wanted.wanted.ty else {
            return;
        };
        let wanted_type = self.gcc_wanted_type(want, wanted.pointer_count);
        let Some(argument) = wanted.argument else {
            self.gcc_type_warning(r, wanted, wanted_type, None, start);
            return;
        };
        let argument_type = self.gcc_argument_type(argument);
        let mut current = argument_type;
        let mut parameter = Some(argument);
        let mut level = 0;
        while level < wanted.pointer_count {
            let Some(target) = self.pointer_target(current) else {
                self.gcc_type_warning(r, wanted, wanted_type, Some(argument_type), start);
                return;
            };
            if wanted.writing && level == 0 {
                if parameter.is_some_and(|parameter| self.gcc_null(parameter)) {
                    let words = format_in!(
                        self.scratch,
                        "writing through null pointer (argument {})",
                        wanted.number
                    );
                    self.format_warning(
                        r.call.call.source_vectors,
                        G::Format,
                        words,
                        r.call.input_location,
                        None,
                    );
                }
                let constant =
                    parameter
                        .and_then(Self::gcc_address_operand)
                        .is_some_and(|operand| {
                            matches!(Self::without_parentheses(operand).kind, E::StringLiteral(_))
                        });
                if target.qualifiers.contains(TypeQualifiers::CONST) || constant {
                    let words = format_in!(
                        self.scratch,
                        "writing into constant object (argument {})",
                        wanted.number
                    );
                    self.format_warning(
                        r.call.call.source_vectors,
                        G::Format,
                        words,
                        r.call.input_location,
                        None,
                    );
                }
            }
            // GCC leaves a null `%s` of its own `printf` to the middle end,
            // but not one of another table's, such as `ms_printf`'s.
            if wanted.flags2.contains(&b'R')
                && r.call.ms
                && level == 0
                && parameter.is_some_and(|parameter| self.gcc_null(parameter))
            {
                let words = format_in!(
                    self.scratch,
                    "reading through null pointer (argument {})",
                    wanted.number
                );
                self.format_warning(
                    r.call.call.source_vectors,
                    G::Format,
                    words,
                    r.call.input_location,
                    None,
                );
            }
            if level > 0
                && self.gcc_pedantic()
                && target.qualifiers.intersects(
                    TypeQualifiers::CONST
                        | TypeQualifiers::VOLATILE
                        | TypeQualifiers::RESTRICT
                        | TypeQualifiers::ATOMIC,
                )
            {
                let words = format_in!(
                    self.scratch,
                    "extra type qualifiers in format argument (argument {})",
                    wanted.number
                );
                self.format_warning(
                    r.call.call.source_vectors,
                    G::Format,
                    words,
                    r.call.input_location,
                    None,
                );
            }
            parameter = parameter.and_then(Self::gcc_address_operand);
            current = target;
            level += 1;
        }
        let current = self.types.non_atomic(current).unqualified();
        let pedantic = self.gcc_pedantic();
        let char_flag = wanted.flags2.contains(&b'c') && self.gcc_character(current);
        let wanted_main = wanted_type.map(|(ty, _)| ty);
        if let Some(wanted_main) = wanted_main {
            if self.gcc_compatible(wanted_main, current) {
                return;
            }
            let wanted_scalar = self.scalar_of(wanted_main);
            if wanted_scalar == Some(S::Void) && (!pedantic || (level == 1 && char_flag)) {
                return;
            }
            if let (Some(want_int), Some(have_int)) =
                (self.gcc_integer(wanted_main), self.gcc_integer(current))
                && (!pedantic || level == 0 || (level == 1 && char_flag))
                && self.gcc_same_but_sign(want_int, have_int)
            {
                return;
            }
            if wanted_scalar == Some(S::Char) && (!pedantic || level < 2) && char_flag {
                return;
            }
            if wanted.identity && self.gcc_same_width(wanted_main, current) {
                return;
            }
        }
        self.gcc_type_warning(r, wanted, wanted_type, Some(argument_type), start);
    }

    /// GCC's words for a wanted type its argument does not have.
    fn gcc_type_warning(
        &mut self,
        r: &Reading<'_, 'tu>,
        wanted: WantedArgument<'tu>,
        wanted_type: Option<(TypeId, &'tu str)>,
        argument_type: Option<TypeId>,
        start: usize,
    ) {
        let what = match wanted.kind {
            | Kind::Format => "format",
            | Kind::Width => "field width specifier",
            | Kind::Precision => "field precision specifier",
        };
        let quoted = r
            .bytes
            .get(wanted.quoted.0..wanted.quoted.0 + wanted.quoted.1)
            .map_or("", |text| std::str::from_utf8(text).unwrap_or(""));
        let percent = if wanted.kind == Kind::Format { "%" } else { "" };
        let mut pointers = ArenaString::new_in(self.scratch);
        if wanted.pointer_count > 0 {
            pointers.push(' ');
            for _ in 0..wanted.pointer_count {
                pointers.push('*');
            }
        }
        let name = match (wanted.wanted.name, wanted_type) {
            | (Some(name), _) => name,
            | (None, Some((_, spelling))) => spelling,
            | (None, None) => match wanted.wanted.ty {
                | Some(Want::Decimal32) => "_Decimal32",
                | Some(Want::Decimal64) => "_Decimal64",
                | Some(Want::Decimal128) => "_Decimal128",
                | _ => return,
            },
        };
        let words = match argument_type {
            | Some(ty) => {
                let Some(spelling) = self.gcc_argument_spelling(wanted.argument, ty) else {
                    return;
                };
                format_in!(
                    self.scratch,
                    "{what} '{percent}{quoted}' expects argument of type '{name}{pointers}', but \
                     argument {} has type '{spelling}'",
                    wanted.number
                )
            },
            | None => format_in!(
                self.scratch,
                "{what} '{percent}{quoted}' expects a matching '{name}{pointers}' argument"
            ),
        };
        let range_start = if wanted.kind == Kind::Format {
            start
        } else {
            wanted.quoted.0
        };
        self.gcc_range_warning(r, G::Format, words, range_start, wanted.caret);
    }

    /// GCC's spelling of an argument's type: a bit-field wider than `int`
    /// keeps its declared type and width, `long int:40`.
    fn gcc_argument_spelling(
        &self,
        argument: Option<&'tu Expression<'tu>>,
        ty: TypeId,
    ) -> Option<&'tu str> {
        if let Some(argument) = argument
            && let TypeKind::BitInt { width, .. } = self.types.nodes[ty.index]
            && let info = self.expression_info(argument)
            && info.promotion_width.is_some()
        {
            let declared =
                self.type_spelling(self.types.non_atomic(info.ty).unqualified(), true)?;
            return Some(format_in!(self.types.tu, "{declared}:{width}"));
        }
        self.type_spelling(ty, true)
    }

    /// Reports `words` about the byte `offset` of the string.
    fn gcc_char_warning(&mut self, r: &Reading<'_, 'tu>, group: G, words: &str, offset: usize) {
        self.gcc_range_warning(r, group, words, offset, offset);
    }

    /// Reports `words` about bytes `start` to `caret` of the string, with
    /// the caret at `caret`: there when the string's first token has them,
    /// at the string with a note there when a later token has them, and at
    /// the string alone when the places cannot be told.
    fn gcc_range_warning(
        &mut self,
        r: &Reading<'_, 'tu>,
        group: G,
        words: &str,
        start: usize,
        caret: usize,
    ) {
        let leaf = r.leaf;
        let positions = if leaf.bare && leaf.spelled != Spelled::MacroBody {
            self.byte_positions(leaf.literal)
        } else {
            None
        };
        // GCC places the bytes of `"..." + N` as if they were the string's
        // own from its start.
        let (Some(&first), Some(&last)) = (
            positions.and_then(|positions| positions.get(start)),
            positions.and_then(|positions| positions.get(caret)),
        ) else {
            self.format_warning(
                r.call.call.source_vectors,
                group,
                words,
                leaf.char_site,
                None,
            );
            return;
        };
        let place = self.byte_location(last);
        if leaf.spelled == Spelled::File && first.token == 0 && last.token == 0 {
            self.format_warning(r.call.call.source_vectors, group, words, place, None);
        } else {
            self.format_warning(
                r.call.call.source_vectors,
                group,
                words,
                leaf.site,
                Some((FormatNote::DefinedHere, place)),
            );
        }
    }

    /// Whether GCC is pedantic: `-Wpedantic` or `-pedantic-errors`.
    fn gcc_pedantic(&self) -> bool {
        self.context.configuration.extension_policy()
            != crate::configuration::ExtensionPolicy::Allow
    }

    /// The type GCC wants, after the default argument promotions for a
    /// value, with its spelling, or `None` for a decimal type.
    fn gcc_wanted_type(&mut self, want: Want, pointer_count: u8) -> Option<(TypeId, &'tu str)> {
        let target = self.types.target;
        let scalar = match want {
            | Want::Scalar(scalar) => scalar,
            | Want::Size => target.size_t,
            | Want::SignedSize => signed_of(target.size_t),
            | Want::Ptrdiff => target.ptrdiff_t,
            | Want::UnsignedPtrdiff => unsigned_of(target.ptrdiff_t),
            | Want::Intmax => target.intmax_t,
            | Want::Uintmax => target.uintmax_t,
            | Want::WideInt => target.wint_t,
            | Want::WideChar => target.wchar_t,
            | Want::Decimal32 | Want::Decimal64 | Want::Decimal128 => return None,
        };
        let scalar = if pointer_count == 0 {
            match scalar {
                | S::Char
                | S::SignedChar
                | S::UnsignedChar
                | S::Short
                | S::UnsignedShort
                | S::Bool => S::Int,
                | S::Float => S::Double,
                | other => other,
            }
        } else {
            scalar
        };
        let ty = self.types.scalar(scalar);
        Some((ty, self.type_spelling(ty, true)?))
    }

    /// The argument's type as GCC passes it: after the conversions of an
    /// lvalue, an array and a function, and the default argument
    /// promotions.
    fn gcc_argument_type(&mut self, argument: &'tu Expression<'tu>) -> TypeId {
        let info = self.expression_info(argument);
        let ty = self.decayed_type(argument);
        if self.integer_type(ty).is_some() || matches!(self.types.nodes[ty.index], TypeKind::Tag(_))
        {
            let promoted = self.promote(info, ty);
            return promoted.unqualified();
        }
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(S::Float) => self.types.scalar(S::Double),
            | _ => ty.unqualified(),
        }
    }

    /// Whether `e` is `0` once GCC strips its conversions.
    fn gcc_null(&self, e: &'tu Expression<'tu>) -> bool {
        let mut e = Self::without_parentheses(e);
        while let E::Cast {
            operand_expression, ..
        } = e.kind
        {
            e = Self::without_parentheses(operand_expression);
        }
        let info = self.expression_info(e);
        info.ice && info.integer.is_some_and(super::super::Integer::is_zero)
    }

    /// The operand of `&operand`, through parentheses.
    fn gcc_address_operand(e: &'tu Expression<'tu>) -> Option<&'tu Expression<'tu>> {
        match Self::without_parentheses(e).kind {
            | E::Unary {
                operator: UnaryOperator::AddressOf,
                operand_expression,
            } => Some(operand_expression),
            | _ => None,
        }
    }

    /// GCC's `types_compatible_p` of unqualified types.
    fn gcc_compatible(&mut self, a: TypeId, b: TypeId) -> bool {
        self.types
            .composite(a.unqualified(), b.unqualified())
            .is_some()
    }

    /// The scalar `ty` is, if it is one.
    fn scalar_of(&self, ty: TypeId) -> Option<Scalar> {
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(scalar) => Some(scalar),
            | _ => None,
        }
    }

    /// The integer scalar `ty` is, an `INTEGER_TYPE` to GCC: neither an
    /// enumeration nor `_Bool`.
    fn gcc_integer(&self, ty: TypeId) -> Option<Scalar> {
        self.scalar_of(ty).filter(|scalar| {
            matches!(
                scalar,
                S::Char
                    | S::SignedChar
                    | S::UnsignedChar
                    | S::Short
                    | S::UnsignedShort
                    | S::Int
                    | S::UnsignedInt
                    | S::Long
                    | S::UnsignedLong
                    | S::LongLong
                    | S::UnsignedLongLong
                    | S::Int128
                    | S::UnsignedInt128
            )
        })
    }

    /// Whether `have` is `want` but for its sign, as GCC's
    /// `c_common_signed_type` and `c_common_unsigned_type` tell.
    fn gcc_same_but_sign(&self, want: Scalar, have: Scalar) -> bool {
        if is_unsigned(want, self.types.target.char_signed) {
            unsigned_of(have) == want
        } else {
            signed_of(have) == want
        }
    }

    /// Whether `a` and `b` are both integer or both floating types of one
    /// width, which a Microsoft length modifier asks no more of.
    fn gcc_same_width(&self, a: TypeId, b: TypeId) -> bool {
        let floating = |ty| {
            matches!(
                self.scalar_of(ty),
                Some(S::Float | S::Double | S::LongDouble)
            )
        };
        let kinds = match (self.integer_type(a), self.integer_type(b)) {
            | (Some((a, _)), Some((b, _))) => return a == b,
            | (None, None) => floating(a) && floating(b),
            | _ => false,
        };
        kinds
            && self
                .types
                .layout(a)
                .zip(self.types.layout(b))
                .is_some_and(|(a, b)| a.size == b.size)
    }

    /// Whether `ty` is a character type.
    fn gcc_character(&self, ty: TypeId) -> bool {
        matches!(
            self.scalar_of(ty),
            Some(S::Char | S::SignedChar | S::UnsignedChar)
        )
    }

    /// The type of `e` after the conversions of an lvalue, an array and a
    /// function, without recording them.
    pub(super) fn decayed_type(&mut self, e: &'tu Expression<'tu>) -> TypeId {
        let info = self.expression_info(e);
        match self.types.nodes[info.ty.index] {
            | TypeKind::Array(element, _) => self.types.intern(TypeKind::Pointer(element)),
            | TypeKind::Function { .. } => self.types.intern(TypeKind::Pointer(info.ty)),
            | _ => self.types.non_atomic(info.ty).unqualified(),
        }
    }

    /// The arguments a call to a `format_arg` function passes as the
    /// format strings its result is, in parameter order.
    pub(super) fn format_arguments_of(
        &mut self,
        function: &'tu Expression<'tu>,
        arguments: crate::util::arena_list::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ArenaVec<'tu, &'tu Expression<'tu>> {
        let mut formats = ArenaVec::new_in(self.types.tu);
        let Some(binding) = self.expression_info(function).binding else {
            return formats;
        };
        if self.bindings[binding].kind != BindingKind::Function {
            return formats;
        }
        let mask = self.format_attributes(binding).1;
        formats.extend(
            arguments
                .iter()
                .enumerate()
                .filter(|&(index, _)| index < 32 && mask & (1 << index) != 0)
                .map(|(_, &argument)| argument),
        );
        formats
    }

    /// The string literal a `const` array's initializer is, `{ "..." }`
    /// included.
    pub(super) fn constant_array_string(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<&'tu Expression<'tu>> {
        let binding = self.expression_info(e).binding?;
        let initializer = self.constant_aggregate(binding)?;
        let expression = super::initializer_expression(initializer)?;
        Self::string_literal(expression).map(|(literal, _)| literal)
    }

    /// The literal and offset of `"..." + N`, `N + "..."`, `"..." - N` and
    /// `&"..."[N]`, nested as `fold` combines them into one offset, and
    /// whether the literal is the outermost sum's own operand. A negative
    /// offset is no string.
    pub(super) fn literal_offset(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<(&'tu Expression<'tu>, usize, bool)> {
        let (literal, offset, direct) = self.gcc_folded_offset(e, 0)?;
        Some((literal, usize::try_from(offset).ok()?, direct))
    }

    fn gcc_folded_offset(
        &self,
        e: &'tu Expression<'tu>,
        depth: u32,
    ) -> Option<(&'tu Expression<'tu>, i128, bool)> {
        let constant = |analyzer: &Self, e: &'tu Expression<'tu>| {
            let info = analyzer.expression_info(e);
            info.integer
                .filter(|_| info.ice)
                .and_then(super::super::Integer::to_i128)
        };
        if depth > 64 {
            return None;
        }
        let direct = |e: &'tu Expression<'tu>| matches!(e.kind, E::StringLiteral(_)).then_some(e);
        match Self::without_parentheses(e).kind {
            | E::StringLiteral(_) => Some((Self::without_parentheses(e), 0, false)),
            | E::Binary {
                operator: BinaryOperator::Addition,
                left_expression,
                right_expression,
            } => {
                let (pointer, number) = match constant(self, right_expression) {
                    | Some(number) => (left_expression, number),
                    | None => (right_expression, constant(self, left_expression)?),
                };
                let (literal, offset, _) = self.gcc_folded_offset(pointer, depth + 1)?;
                Some((
                    literal,
                    offset.checked_add(number)?,
                    direct(pointer).is_some(),
                ))
            },
            | E::Binary {
                operator: BinaryOperator::Subtraction,
                left_expression,
                right_expression,
            } => {
                let number = constant(self, right_expression)?;
                let (literal, offset, _) = self.gcc_folded_offset(left_expression, depth + 1)?;
                Some((
                    literal,
                    offset.checked_sub(number)?,
                    direct(left_expression).is_some(),
                ))
            },
            | E::Unary {
                operator: UnaryOperator::AddressOf,
                operand_expression,
            } => match Self::without_parentheses(operand_expression).kind {
                | E::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    let (base, index) = match constant(self, right_expression) {
                        | Some(index) => (left_expression, index),
                        | None => (right_expression, constant(self, left_expression)?),
                    };
                    let (literal, offset, _) = self.gcc_folded_offset(base, depth + 1)?;
                    Some((literal, offset.checked_add(index)?, false))
                },
                | _ => None,
            },
            | _ => None,
        }
    }
}

/// What GCC reaches from a format argument: a string literal, something
/// else with the place GCC gives it, or a null pointer constant.
#[derive(Clone, Copy)]
enum GccLeaf<'tu> {
    Literal(Leaf<'tu>),
    NonLiteral(Option<SourceVectors>),
    Null,
}

/// How a specification's reading continues.
enum Step {
    Next,
    /// GCC stops reading the string.
    Stop,
}

enum Dollar {
    None,
    Number(usize),
    Error,
}

/// GCC's name for the archetype it checks with: the GNU C library's, or
/// on Windows the Microsoft C runtime's for a builtin or `format(printf)`.
const fn archetype(kind: FormatKind, ms: bool) -> &'static str {
    match (kind, ms) {
        | (FormatKind::Scanf, false) => "gnu_scanf",
        | (FormatKind::Scanf, true) => "ms_scanf",
        | (_, false) => "gnu_printf",
        | (_, true) => "ms_printf",
    }
}

/// `c` as GCC quotes a character: itself when printable, a `\x` escape
/// otherwise.
fn quoted_char<'a>(out: &'a mut ArenaString<'_>, c: u8) -> &'a str {
    use std::fmt::Write;
    out.push('\'');
    if c.is_ascii_graphic() || c == b' ' {
        out.push(c as char);
    } else {
        let _ = write!(out, "\\x{c:02x}");
    }
    out.push('\'');
    out.as_str()
}

const fn is_unsigned(scalar: Scalar, char_signed: bool) -> bool {
    match scalar {
        | S::Char => !char_signed,
        | S::Bool
        | S::UnsignedChar
        | S::UnsignedShort
        | S::UnsignedInt
        | S::UnsignedLong
        | S::UnsignedLongLong
        | S::UnsignedInt128 => true,
        | _ => false,
    }
}

/// The signed integer type of `scalar`'s rank, as `c_common_signed_type`
/// gives it.
pub(super) const fn signed_of(scalar: Scalar) -> Scalar {
    match scalar {
        | S::Char | S::UnsignedChar => S::SignedChar,
        | S::UnsignedShort => S::Short,
        | S::UnsignedInt => S::Int,
        | S::UnsignedLong => S::Long,
        | S::UnsignedLongLong => S::LongLong,
        | S::UnsignedInt128 => S::Int128,
        | other => other,
    }
}

/// The unsigned integer type of `scalar`'s rank.
pub(super) const fn unsigned_of(scalar: Scalar) -> Scalar {
    match scalar {
        | S::Char | S::SignedChar => S::UnsignedChar,
        | S::Short => S::UnsignedShort,
        | S::Int => S::UnsignedInt,
        | S::Long => S::UnsignedLong,
        | S::LongLong => S::UnsignedLongLong,
        | S::Int128 => S::UnsignedInt128,
        | other => other,
    }
}
