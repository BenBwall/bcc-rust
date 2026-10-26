//! The extended assembly template grammar: literal text, `%` operand
//! references with optional modifiers, `%l` goto labels, `%=`, the `%%`,
//! `%{`, `%|` and `%}` escapes, and `{…|…}` dialect alternatives. Basic
//! assembly has no substitution and is one text piece.
//! GNU extension: GCC manual, "Extended Asm" (the assembler template and
//! goto labels) and "x86 Operand Modifiers".
//! <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>

use super::super::AsmPiece;
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// Why a template is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TemplateFault {
    /// A `%` followed by nothing a template may contain.
    Escape,
    /// `%N` or `%lN` beyond the operands or labels.
    OperandNumber,
    /// `%[name]` naming no operand, or `%l[name]` no label.
    Name,
    /// A modifier letter x86 does not define.
    Modifier,
}

/// The names a template may refer to.
pub(super) struct Names<'n> {
    /// Each operand's `[name]`, in operand order.
    pub(super) operands: &'n [Option<&'n str>],
    /// Each goto label, in label order.
    pub(super) labels:   &'n [&'n str],
}

/// Splits an extended template into pieces in `arena`.
pub(super) fn parse<'a, 't: 'a>(
    text: &'t str,
    names: &Names<'_>,
    arena: &'a Bump,
) -> Result<&'a [AsmPiece<'a>], TemplateFault> {
    let mut pieces = ArenaVec::new_in(arena);
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    let flush = |pieces: &mut ArenaVec<'a, AsmPiece<'a>>, start: usize, end: usize| {
        if start < end {
            pieces.push(AsmPiece::Text(&text[start..end]));
        }
    };
    while let Some(&byte) = bytes.get(index) {
        let piece = match byte {
            | b'{' => AsmPiece::DialectOpen,
            | b'|' => AsmPiece::DialectNext,
            | b'}' => AsmPiece::DialectClose,
            | b'%' => {
                flush(&mut pieces, start, index);
                let (piece, end) = escape(text, index + 1, names)?;
                pieces.push(piece);
                index = end;
                start = end;
                continue;
            },
            | _ => {
                index += 1;
                continue;
            },
        };
        flush(&mut pieces, start, index);
        pieces.push(piece);
        index += 1;
        start = index;
    }
    flush(&mut pieces, start, index);
    Ok(pieces.leak())
}

/// The piece the `%` escape starting at `index` writes, and where the text
/// continues after it.
fn escape<'t>(
    text: &'t str,
    index: usize,
    names: &Names<'_>,
) -> Result<(AsmPiece<'t>, usize), TemplateFault> {
    let bytes = text.as_bytes();
    let byte = *bytes.get(index).ok_or(TemplateFault::Escape)?;
    match byte {
        | b'%' | b'{' | b'|' | b'}' =>
            return Ok((AsmPiece::Text(&text[index..=index]), index + 1)),
        | b'=' => return Ok((AsmPiece::UniqueId, index + 1)),
        | _ => {},
    }
    let (modifier, index) = if byte.is_ascii_alphabetic() {
        (Some(byte), index + 1)
    } else {
        (None, index)
    };
    let label = modifier == Some(b'l');
    if modifier.is_some_and(|modifier| !label && !MODIFIERS.contains(&modifier)) {
        return Err(TemplateFault::Modifier);
    }
    let operands = names.operands.len();
    let (reference, end) = match bytes.get(index) {
        | Some(b'[') => {
            let length = text[index + 1..].find(']').ok_or(TemplateFault::Escape)?;
            let name = &text[index + 1..index + 1 + length];
            let end = index + length + 2;
            let found = if label {
                names.labels.iter().position(|&known| known == name)
            } else {
                names.operands.iter().position(|&known| known == Some(name))
            };
            (found.ok_or(TemplateFault::Name)?, end)
        },
        | Some(digit) if digit.is_ascii_digit() => {
            let mut end = index;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            let number: usize = text[index..end]
                .parse()
                .map_err(|_| TemplateFault::OperandNumber)?;
            // Labels are numbered after the operands.
            let number = if label {
                number
                    .checked_sub(operands)
                    .ok_or(TemplateFault::OperandNumber)?
            } else {
                number
            };
            (number, end)
        },
        | _ => return Err(TemplateFault::Escape),
    };
    let limit = if label { names.labels.len() } else { operands };
    if reference >= limit {
        return Err(TemplateFault::OperandNumber);
    }
    let reference = u32::try_from(reference).map_err(|_| TemplateFault::OperandNumber)?;
    let piece = if label {
        AsmPiece::Label(reference)
    } else {
        AsmPiece::Operand {
            operand: reference,
            modifier,
        }
    };
    Ok((piece, end))
}

/// The operand modifiers GCC documents for x86, besides `l`.
/// GNU extension: GCC manual, "Extended Asm", "x86 Operand Modifiers" and
/// "Generic Operand Modifiers".
const MODIFIERS: &[u8] = b"aAbcdghHknpPqtVwxz";
