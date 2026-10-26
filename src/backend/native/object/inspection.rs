//! Each function's encoded bytes, instruction by instruction, as the ELF
//! encoder lays out the text: the explorer's machine-code view. Fixups to
//! symbols the object does not define stay zero, as in an unlinked object.

use super::{
    Kind,
    Result,
    listing,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// A function's code.
pub(crate) struct FunctionCode<'a> {
    pub(crate) name:         &'a str,
    pub(crate) bytes:        &'a [u8],
    pub(crate) instructions: &'a [InstructionCode<'a>],
}

/// One instruction: its offset from the function's start, its encoding,
/// and the listing line it encodes.
#[derive(Clone, Copy)]
pub(crate) struct InstructionCode<'a> {
    pub(crate) offset: u64,
    pub(crate) bytes:  &'a [u8],
    pub(crate) text:   &'a str,
}

/// Encodes the private ELF listing and returns its functions in address
/// order.
pub(crate) fn functions<'a>(text: &'a str, arena: &'a Bump) -> Result<&'a [FunctionCode<'a>]> {
    let scratch = Bump::new();
    let mut object = listing::parse(text, &scratch)?;
    object.layout()?;
    object.materialize()?;
    // The listing parser makes one item of each instruction line, in order.
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| is_instruction(line));
    let mut instructions = ArenaVec::new_in(&scratch);
    for item in &object.items {
        let length = match item.kind {
            | Kind::Code(code) => u64::from(code.len),
            | Kind::Branch {
                condition, near, ..
            } => match (near, condition) {
                | (false, _) => 2,
                | (true, None) => 5,
                | (true, Some(_)) => 6,
            },
            | _ => continue,
        };
        let line = lines.next().unwrap_or_default();
        instructions.push((item.section, item.offset, length, line));
    }
    let mut functions = ArenaVec::new_in(&scratch);
    for symbol in &object.symbols {
        let Some(section) = symbol.section.filter(|_| symbol.kind == FUNCTION) else {
            continue;
        };
        let data = &object.sections[section].data;
        let start = symbol.value;
        let end = start.saturating_add(symbol.size);
        let slice = |from: u64, to: u64| {
            usize::try_from(from)
                .ok()
                .zip(usize::try_from(to).ok())
                .and_then(|(from, to)| data.get(from..to))
                .unwrap_or_default()
        };
        let code = arena.alloc_slice_fill_iter(
            instructions
                .iter()
                .filter(|&&(at, offset, _, _)| at == section && (start..end).contains(&offset))
                .map(|&(_, offset, length, line)| InstructionCode {
                    offset: offset - start,
                    bytes:  arena.alloc_slice_copy(slice(offset, offset + length)),
                    text:   arena.alloc_str(line),
                }),
        );
        functions.push((
            (section, start),
            FunctionCode {
                name:         arena.alloc_str(symbol.name.trim_matches('"')),
                bytes:        arena.alloc_slice_copy(slice(start, end)),
                instructions: code,
            },
        ));
    }
    functions.sort_by_key(|&(place, _)| place);
    Ok(arena.alloc_slice_fill_iter(functions.into_iter().map(|(_, code)| code)))
}

/// `.type name, @function`.
const FUNCTION: u8 = 2;

/// Whether a trimmed listing line is an instruction rather than a label, a
/// symbol assignment or a directive.
fn is_instruction(line: &str) -> bool {
    !line.is_empty()
        && !line.ends_with(':')
        && !line.starts_with('.')
        && listing::assignment(line).is_none()
}
