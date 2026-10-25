//! Syntax and keyword tables of the MSVC extensions to translation phase 7,
//! parsed by `grammar/msvc.rs`.
//!
//! C99: vendor extensions to declarators §6.7.5, p. 114; PDF p. 126 and
//! statements §6.8, p. 131; PDF p. 143. Microsoft Learn specifies SEH and
//! inline assembly grammar. Target assembly, ABI, layout and control-flow
//! constraints belong to later analysis.

use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            syntax::{
                Expression,
                Statement,
            },
        },
        preprocessing::{
            KeywordTokenType,
            PreprocessorErrorType,
            Token,
        },
    },
    util::arena_list::ArenaList,
};

/// An opaque, balanced MSVC assembly token sequence, including its introducer.
/// C99: extension to §6.8, p. 131; PDF p. 143. Assembly interpretation is
/// deferred.
/// MSVC extension: Microsoft Learn, "__asm".
/// <https://learn.microsoft.com/en-us/cpp/assembler/inline/asm>
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct MsAsm<'tu> {
    pub(crate) tokens:         ArenaList<'tu, Token>,
    pub(crate) braced:         bool,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// Guarded SEH compound, optional exception filter, and handler compound.
/// C99: extension to §6.8, p. 131; PDF p. 143. An absent filter denotes
/// finally; missing required syntax is represented by recovered children.
/// MSVC extension: Microsoft Learn, "try-except statement".
/// <https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement>
/// MSVC extension: Microsoft Learn, "try-finally statement".
/// <https://learn.microsoft.com/en-us/cpp/cpp/try-finally-statement>
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Seh<'tu> {
    pub(crate) body:            &'tu Statement<'tu>,
    pub(crate) filter:          Option<&'tu Expression<'tu>>,
    pub(crate) handler:         &'tu Statement<'tu>,
    pub(crate) handler_keyword: Option<Token>,
}

/// Keys for the phase-7 constant-conversion errors that MSVC assembly
/// withdraws, one per error kind, since one conversion reports each kind at
/// most once.
pub(in crate::translation_phases::parsing) const CONSTANT_DIAGNOSTICS: [&str; 11] = [
    "invalid hexadecimal floating constant",
    "invalid decimal floating constant",
    "invalid hexadecimal integer constant",
    "invalid binary integer constant",
    "invalid octal integer constant",
    "invalid decimal integer constant",
    "integer constant overflow",
    "floating constant out of range",
    "signed constant forced to unsigned",
    "unsigned constant promoted",
    "signed constant promoted",
];

/// The key of a phase-7 constant-conversion error. An assembler operand such
/// as MASM's `0FFh` is a pp-number that is not a C constant, so the MSVC
/// assembly owning it withdraws these errors.
/// C99: pp-numbers §6.4.8, p. 65; PDF p. 77 become constants in phase 7
/// (§5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22), under §6.4.4.1-§6.4.4.2,
/// pp. 54-58; PDF pp. 66-70. MSVC assembly is an extension.
pub(in crate::translation_phases::parsing) fn constant_diagnostic(
    error: &PreprocessorErrorType<'_>,
) -> Option<&'static str> {
    let index = match error {
        | PreprocessorErrorType::InvalidHexadecimalFloatLiteral => 0,
        | PreprocessorErrorType::InvalidDecimalFloatLiteral => 1,
        | PreprocessorErrorType::InvalidHexadecimalIntegerLiteral => 2,
        | PreprocessorErrorType::InvalidBinaryIntegerLiteral => 3,
        | PreprocessorErrorType::InvalidOctalIntegerLiteral => 4,
        | PreprocessorErrorType::InvalidDecimalIntegerLiteral => 5,
        | PreprocessorErrorType::IntegerLiteralOverflow => 6,
        | PreprocessorErrorType::FloatConstantOutOfRange { .. } => 7,
        | PreprocessorErrorType::ForcedSignedToUnsignedConversion { .. } => 8,
        | PreprocessorErrorType::ForcedUnsignedPromotion { .. } => 9,
        | PreprocessorErrorType::ForcedSignedPromotion { .. } => 10,
        | _ => return None,
    };
    Some(CONSTANT_DIAGNOSTICS[index])
}

/// Recognizes Microsoft calling-convention declaration modifiers.
/// MSVC extension: Microsoft Learn, "Argument Passing and Naming Conventions".
/// <https://learn.microsoft.com/en-us/cpp/cpp/argument-passing-and-naming-conventions>
pub(in crate::translation_phases::parsing) fn calling_convention(
    keyword: KeywordTokenType,
) -> bool {
    matches!(
        keyword,
        KeywordTokenType::Cdecl
            | KeywordTokenType::Stdcall
            | KeywordTokenType::Fastcall
            | KeywordTokenType::Vectorcall
            | KeywordTokenType::Thiscall
    )
}

/// Recognizes Microsoft pointer and integer type modifiers.
/// MSVC extension: Microsoft Learn, "Microsoft-Specific Modifiers".
/// <https://learn.microsoft.com/en-us/cpp/cpp/microsoft-specific-modifiers>
pub(in crate::translation_phases::parsing) fn type_modifier(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::Ptr32
            | KeywordTokenType::Ptr64
            | KeywordTokenType::Unaligned
            | KeywordTokenType::W64
            | KeywordTokenType::Sptr
            | KeywordTokenType::Uptr
    )
}

/// Whether a source line ends between `previous` and `next`. Macro tokens
/// stand at their invocation: `previous` at its end, and `next` at the
/// invocation's start, which is the first character after `previous`
/// that is neither white space nor a comment, so an invocation whose
/// arguments span lines stays on the line where it starts. Phase-2
/// splices join lines, while a comment's new-line ends one, as in the
/// text of the line.
/// C99: phases 2-3 are §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
pub(in crate::translation_phases::parsing) fn new_line(
    parser: &Parser<'_, '_, '_>,
    previous: Token,
    next: Token,
) -> bool {
    let Some(a) = parser.context.user_source_end(previous.source_vectors) else {
        return false;
    };
    let Some(b) = parser.context.user_source_end(next.source_vectors) else {
        return false;
    };
    if a.source_file_index != b.source_file_index {
        return true;
    }
    if a.line == b.line || a.end() >= b.index as usize {
        return false;
    }
    let Some(text) = parser.context.source_text(a.source_file_index) else {
        return true;
    };
    let bytes = text.as_bytes();
    let trigraphs = parser
        .context
        .configuration
        .accepts(crate::configuration::Feature::Trigraphs);
    let spliced = |index: usize| {
        let before = if bytes[index] == b'\n' && index > 0 && bytes[index - 1] == b'\r' {
            index - 1
        } else {
            index
        };
        before > 0 && bytes[before - 1] == b'\\'
            || trigraphs && before >= 3 && &bytes[before - 3..before] == b"??/"
    };
    let mut index = a.end();
    let mut comment = false;
    while index < b.index as usize {
        match bytes[index] {
            | b'\r' if bytes.get(index + 1) == Some(&b'\n') => {},
            | b'\n' | b'\r' if !spliced(index) => return true,
            | b'\n' | b'\r' | b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\\' => {},
            | b'?' if trigraphs && bytes[index..].starts_with(b"??/") => index += 2,
            | b'*' if comment && bytes.get(index + 1) == Some(&b'/') => {
                comment = false;
                index += 1;
            },
            | _ if comment => {},
            | b'/' if bytes.get(index + 1) == Some(&b'*') => {
                comment = true;
                index += 1;
            },
            // A line comment runs to the new-line that ends the line.
            | b'/' if bytes.get(index + 1) == Some(&b'/') => return true,
            | _ => return false,
        }
        index += 1;
    }
    false
}
