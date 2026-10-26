//! The GNU operand constraint grammar for x86-64: one constraint string to
//! the registers, memory and immediates it allows, its `=`/`+`/`&`
//! modifiers, and a matching operand or flag output. Alternatives separated
//! by `,` are merged: analysis picks one location for the whole statement.
//! GNU extension: GCC manual, "Constraints for asm Operands": "Simple
//! Constraints", "Multiple Alternatives", "Modifiers", "Machine
//! Constraints" (x86 family) and "Flag Output Operands".
//! <https://gcc.gnu.org/onlinedocs/gcc/Constraints.html>

use super::super::{
    AsmCondition,
    AsmRegisterClass,
    AsmRole,
};

/// What one constraint string allows.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Constraint<'c> {
    pub(super) role:          Option<AsmRole>,
    pub(super) early_clobber: bool,
    /// General registers by hardware encoding.
    pub(super) general:       u16,
    pub(super) sse:           bool,
    /// The first register class letter analysis accepts but lowering does
    /// not implement.
    pub(super) other:         Option<u8>,
    pub(super) memory:        bool,
    /// `i`, `n`, `g`, `X`, `s`, `E` or `F`: any constant of its kind.
    pub(super) immediate:     Option<bool>,
    /// The x86 letters that allow an integer in a range, as a mask of
    /// [`RANGES`] entries.
    pub(super) ranges:        u16,
    pub(super) tied:          Option<Tie<'c>>,
    pub(super) flag:          Option<AsmCondition>,
}

/// The output operand an input shares its location with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tie<'c> {
    Number(u32),
    Name(&'c str),
}

/// Why a constraint string is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConstraintFault {
    Invalid,
    /// An output constraint that does not start with `=` or `+`.
    MissingOutputModifier,
}

/// Parses `text` as an output (`output`) or input constraint.
pub(super) fn parse(text: &str, output: bool) -> Result<Constraint<'_>, ConstraintFault> {
    let bytes = text.as_bytes();
    let mut constraint = Constraint::default();
    let mut index = 0;
    if output {
        constraint.role = Some(match bytes.first() {
            | Some(b'=') => AsmRole::Output,
            | Some(b'+') => AsmRole::Update,
            | _ => return Err(ConstraintFault::MissingOutputModifier),
        });
        index = 1;
    } else {
        constraint.role = Some(AsmRole::Input);
    }
    let mut empty = true;
    while let Some(&byte) = bytes.get(index) {
        index += 1;
        match byte {
            | b'&' if output => constraint.early_clobber = true,
            // Commutative pairs, disparagement and blanks change no location.
            | b'%' | b',' | b'*' | b'?' | b'!' | b' ' | b'\t' => {},
            | b'#' =>
                while bytes.get(index).is_some_and(|&byte| byte != b',') {
                    index += 1;
                },
            | b'@' if output => {
                let rest = &text[index..];
                let condition = rest
                    .strip_prefix("cc")
                    .and_then(AsmCondition::named)
                    .ok_or(ConstraintFault::Invalid)?;
                constraint.flag = Some(condition);
                empty = false;
                index = bytes.len();
            },
            | b'0'..=b'9' if !output => {
                let start = index - 1;
                while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                    index += 1;
                }
                let number = text[start..index]
                    .parse()
                    .map_err(|_| ConstraintFault::Invalid)?;
                constraint.tied = Some(Tie::Number(number));
                empty = false;
            },
            | b'[' if !output => {
                let length = text[index..].find(']').ok_or(ConstraintFault::Invalid)?;
                constraint.tied = Some(Tie::Name(&text[index..index + length]));
                index += length + 1;
                empty = false;
            },
            | b'Y' => {
                // Two-letter SSE and mask classes (`Yz`, `Yk`, ...).
                if bytes
                    .get(index)
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric())
                {
                    return Err(ConstraintFault::Invalid);
                }
                index += 1;
                _ = constraint.other.get_or_insert(byte);
                empty = false;
            },
            | letter => {
                letter_location(letter, &mut constraint).ok_or(ConstraintFault::Invalid)?;
                empty = false;
            },
        }
    }
    if empty {
        return Err(ConstraintFault::Invalid);
    }
    Ok(constraint)
}

/// Adds the location one constraint letter allows; `None` for a letter GCC
/// does not define on x86-64.
fn letter_location(letter: u8, constraint: &mut Constraint<'_>) -> Option<()> {
    let general = |mask: u16| Some(mask);
    let registers = match letter {
        | b'r' | b'q' | b'l' | b'p' => general(AsmRegisterClass::ANY_GENERAL),
        | b'Q' => general(AsmRegisterClass::HIGH_BYTE),
        | b'R' => general(AsmRegisterClass::LEGACY),
        // The call-clobbered registers of both conventions' intersection.
        | b'U' => general(0x0FC7),
        | b'a' => general(1 << 0),
        | b'c' => general(1 << 1),
        | b'd' => general(1 << 2),
        | b'b' => general(1 << 3),
        | b'S' => general(1 << 6),
        | b'D' => general(1 << 7),
        | b'x' | b'v' => {
            constraint.sse = true;
            None
        },
        | b'A' | b'f' | b't' | b'u' | b'y' | b'k' => {
            _ = constraint.other.get_or_insert(letter);
            None
        },
        | b'm' | b'o' | b'V' | b'<' | b'>' => {
            constraint.memory = true;
            None
        },
        | b'g' | b'X' => {
            constraint.memory = true;
            constraint.immediate = Some(true);
            general(AsmRegisterClass::ANY_GENERAL)
        },
        | b'i' | b's' => {
            constraint.immediate = Some(true);
            None
        },
        | b'n' | b'E' | b'F' => {
            constraint.immediate = Some(constraint.immediate.unwrap_or(false));
            None
        },
        | _ => {
            let range = RANGES.iter().position(|&(known, ..)| known == letter)?;
            constraint.ranges |= 1 << range;
            None
        },
    };
    if let Some(mask) = registers {
        constraint.general |= mask;
    }
    Some(())
}

/// Whether an immediate letter allows an integer.
pub(super) type Allows = fn(i128) -> bool;

/// The x86 immediate letters and the integers each allows.
/// GNU extension: GCC manual, "Machine Constraints", x86 family.
pub(super) const RANGES: [(u8, Allows); 9] = [
    (b'I', |value| (0..=31).contains(&value)),
    (b'J', |value| (0..=63).contains(&value)),
    (b'K', |value| (-128..=127).contains(&value)),
    (b'L', |value| matches!(value, 0xFF | 0xFFFF | 0xFFFF_FFFF)),
    (b'M', |value| (0..=3).contains(&value)),
    (b'N', |value| (0..=255).contains(&value)),
    (b'O', |value| (0..=127).contains(&value)),
    (b'e', |value| {
        (i128::from(i32::MIN)..=i128::from(i32::MAX)).contains(&value)
    }),
    (b'Z', |value| (0..=i128::from(u32::MAX)).contains(&value)),
];
