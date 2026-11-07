//! The `debug` items and the ` dbg locN` suffix of an instruction, as
//! [`Module::display_located`](crate::ir::Module::display_located) prints them.
//!
//! The tables come before the functions, and each table's entries are
//! numbered from 0 in order, so everything an item names is already known.
//! The first pass skips the items; the second reads them, when every function
//! they name has been declared.

use super::{
    Cursor,
    ParseError,
    Symbol,
    Token,
    TokenKind,
};
use crate::{
    ir::{
        DebugFile,
        DebugLanguage,
        DebugLevel,
        DebugLoc,
        DebugScope,
        DebugUnit,
        Entity,
        Module,
        SourceLocation,
        Subprogram,
    },
    util::bump::Bump,
};

/// Skips one `debug` item, which ends with its line.
pub(super) fn skip_item(cursor: &mut Cursor<'_, '_>) {
    _ = cursor.next();
    loop {
        let token = cursor.peek();
        if token.line_start || token.kind == TokenKind::End {
            return;
        }
        _ = cursor.next();
    }
}

/// Reads one `debug` item into the module's debug information.
pub(super) fn parse_item<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
    scratch: &Bump,
) -> Result<(), ParseError<'t>> {
    _ = cursor.next();
    let token = cursor.next();
    match token.kind {
        | TokenKind::Ident("file") => {
            let number = cursor.next();
            let count = module.debug_info().files().count();
            defined_entity(number, "file", count)?;
            let name = cursor.symbol()?;
            let (debug, arena) = module.debug_info_mut();
            _ = debug.file(arena, name.text_name(scratch)?);
        },
        | TokenKind::Ident("unit") => {
            let unit = parse_unit(cursor, module)?;
            module.debug_info_mut().0.set_unit(unit);
        },
        | TokenKind::Ident("scope") => {
            let number = cursor.next();
            let count = module.debug_info().scopes().count();
            defined_entity(number, "scope", count)?;
            let symbol = cursor.symbol()?;
            let Some(Symbol::Function(function)) = module.symbol(symbol.text_name(scratch)?) else {
                return Err(symbol.expected("a function"));
            };
            cursor.expect(TokenKind::Ident("name"), "`name`")?;
            let name = cursor.symbol()?;
            let file = parse_file(cursor, module)?;
            let line = parse_number(cursor, "line")?;
            let scope_line = parse_number(cursor, "scope_line")?;
            let (debug, arena) = module.debug_info_mut();
            _ = debug.add_subprogram(
                arena,
                Subprogram {
                    function,
                    name: name.text_name(scratch)?,
                    file,
                    line,
                    scope_line,
                },
            );
        },
        | TokenKind::Ident("loc") => {
            let number = cursor.next();
            defined_entity(number, "loc", module.debug_info().location_count())?;
            let location = parse_location(cursor, module)?;
            _ = module.debug_info_mut().0.location(location);
        },
        | _ => return Err(token.expected("`file`, `unit`, `scope` or `loc`")),
    }
    Ok(())
}

/// The optional ` dbg locN` that ends an instruction.
pub(super) fn instruction_location<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<Option<DebugLoc>, ParseError<'t>> {
    let token = cursor.peek();
    if token.line_start || token.kind != TokenKind::Ident("dbg") {
        return Ok(None);
    }
    _ = cursor.next();
    parse_location_id(cursor, module).map(Some)
}

/// `fileN language L level N [dwarf N] [optimized]`.
fn parse_unit<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<DebugUnit, ParseError<'t>> {
    let file = named_entity(cursor.next(), "file", module.debug_info().files().count())?;
    cursor.expect(TokenKind::Ident("language"), "`language`")?;
    let token = cursor.next();
    let language = match token.kind {
        | TokenKind::Ident(keyword) => DebugLanguage::from_keyword(keyword),
        | _ => None,
    }
    .ok_or_else(|| token.expected("`c89`, `c99` or `c11`"))?;
    cursor.expect(TokenKind::Ident("level"), "`level`")?;
    let (token, level) = cursor.unsigned()?;
    let level = u8::try_from(level)
        .ok()
        .and_then(DebugLevel::from_number)
        .ok_or_else(|| token.expected("a debug level from 1 through 3"))?;
    let dwarf_version = if cursor.eat(TokenKind::Ident("dwarf")) {
        let (token, version) = cursor.unsigned()?;
        Some(u8::try_from(version).map_err(|_| token.expected("a DWARF version"))?)
    } else {
        None
    };
    Ok(DebugUnit {
        file: DebugFile::new(file),
        language,
        level,
        dwarf_version,
        optimized: cursor.eat(TokenKind::Ident("optimized")),
    })
}

/// `fileN line N column N scope scopeN [at locN]`.
fn parse_location<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<SourceLocation, ParseError<'t>> {
    let file = DebugFile::new(named_entity(
        cursor.next(),
        "file",
        module.debug_info().files().count(),
    )?);
    let line = parse_number(cursor, "line")?;
    let column = parse_number(cursor, "column")?;
    cursor.expect(TokenKind::Ident("scope"), "`scope`")?;
    let scope = DebugScope::new(named_entity(
        cursor.next(),
        "scope",
        module.debug_info().scopes().count(),
    )?);
    let inlined_at = if cursor.eat(TokenKind::Ident("at")) {
        Some(parse_location_id(cursor, module)?)
    } else {
        None
    };
    Ok(SourceLocation {
        file,
        line,
        column,
        scope,
        inlined_at,
    })
}

fn parse_file<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<DebugFile, ParseError<'t>> {
    cursor.expect(TokenKind::Ident("file"), "`file`")?;
    named_entity(cursor.next(), "file", module.debug_info().files().count()).map(DebugFile::new)
}

fn parse_location_id<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<DebugLoc, ParseError<'t>> {
    named_entity(cursor.next(), "loc", module.debug_info().location_count()).map(DebugLoc::new)
}

/// `keyword N`.
fn parse_number<'t>(
    cursor: &mut Cursor<'_, 't>,
    keyword: &'static str,
) -> Result<u32, ParseError<'t>> {
    let token = cursor.next();
    if token.kind != TokenKind::Ident(keyword) {
        return Err(token.expected("a keyword and a number"));
    }
    let (token, number) = cursor.unsigned()?;
    u32::try_from(number).map_err(|_| token.expected("a number that fits in 32 bits"))
}

/// The number of `token`, which spells an entity with `prefix` that exists.
fn named_entity<'t>(token: Token<'t>, prefix: &str, count: usize) -> Result<usize, ParseError<'t>> {
    entity_number(token, prefix)
        .filter(|&number| number < count)
        .ok_or_else(|| token.expected("a debug entity the tables above define"))
}

/// Checks that `token` spells the next entity of a table of `count` entries.
fn defined_entity<'t>(token: Token<'t>, prefix: &str, count: usize) -> Result<(), ParseError<'t>> {
    if entity_number(token, prefix) == Some(count) {
        Ok(())
    } else {
        Err(token.expected("the next number of the debug table"))
    }
}

fn entity_number(token: Token<'_>, prefix: &str) -> Option<usize> {
    let TokenKind::Ident(word) = token.kind else {
        return None;
    };
    word.strip_prefix(prefix)?.parse().ok()
}
