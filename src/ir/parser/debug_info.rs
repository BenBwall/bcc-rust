//! The `debug` items and the ` dbg locN` suffix of an instruction, as
//! [`Module::display_located`](crate::ir::Module::display_located) prints them.
//!
//! The tables come before the functions, and each table's entries are
//! numbered from 0 in order, so what an item names is already known, except
//! that a type may name a later type, as a structure's member points back at
//! the structure: the verifier checks those. The first pass skips the items;
//! the second reads them, when every function and global they name has been
//! declared.

use super::{
    Cursor,
    ParseError,
    Symbol,
    Token,
    TokenKind,
};
use crate::{
    ir::{
        DebugEncoding,
        DebugEnumerator,
        DebugFile,
        DebugLanguage,
        DebugLevel,
        DebugLoc,
        DebugMember,
        DebugQualifier,
        DebugRecord,
        DebugScope,
        DebugType,
        DebugTypeData,
        DebugUnit,
        DebugVariable,
        DeclaredVariable,
        Entity,
        FuncId,
        GlobalVariable,
        LexicalBlock,
        Module,
        SourceLocation,
        StackSlot,
        Subprogram,
        Value,
        Variable,
        VariablePlace,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
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
        | TokenKind::Ident("type") => {
            let number = cursor.next();
            defined_entity(number, "type", module.debug_info().type_count())?;
            let data = parse_type(cursor, module, scratch)?;
            _ = module.debug_info_mut().0.add_type(data);
        },
        | TokenKind::Ident("scope") => {
            let number = cursor.next();
            let count = module.debug_info().scopes().count();
            defined_entity(number, "scope", count)?;
            if cursor.eat(TokenKind::Ident("block")) {
                let parent = parse_scope_id(cursor, module)?;
                let function = parse_function(cursor, module, scratch)?;
                let file = parse_file(cursor, module)?;
                let line = parse_number(cursor, "line")?;
                let column = parse_number(cursor, "column")?;
                _ = module.debug_info_mut().0.add_block(LexicalBlock {
                    parent,
                    function,
                    file,
                    line,
                    column,
                });
                return Ok(());
            }
            let function = parse_function(cursor, module, scratch)?;
            cursor.expect(TokenKind::Ident("name"), "`name`")?;
            let name = cursor.symbol()?;
            let file = parse_file(cursor, module)?;
            let line = parse_number(cursor, "line")?;
            let scope_line = parse_number(cursor, "scope_line")?;
            let ty = if cursor.eat(TokenKind::Ident("type")) {
                parse_type_reference(cursor)?
            } else {
                None
            };
            let (debug, arena) = module.debug_info_mut();
            _ = debug.add_subprogram(
                arena,
                Subprogram {
                    function,
                    name: name.text_name(scratch)?,
                    file,
                    line,
                    scope_line,
                    ty,
                },
            );
        },
        | TokenKind::Ident("variable") => {
            let number = cursor.next();
            let count = module.debug_info().variables().count();
            defined_entity(number, "var", count)?;
            let name = cursor.symbol()?;
            cursor.expect(TokenKind::Ident("scope"), "`scope`")?;
            let scope = parse_scope_id(cursor, module)?;
            let file = parse_file(cursor, module)?;
            let line = parse_number(cursor, "line")?;
            cursor.expect(TokenKind::Ident("type"), "`type`")?;
            let ty = parse_type_reference(cursor)?;
            let argument = if cursor.peek().kind == TokenKind::Ident("argument") {
                Some(parse_number(cursor, "argument")?)
            } else {
                None
            };
            let (debug, arena) = module.debug_info_mut();
            let name = &*arena.alloc_str(name.text_name(scratch)?);
            _ = debug.add_variable(Variable {
                name,
                scope,
                file,
                line,
                ty,
                argument,
            });
        },
        | TokenKind::Ident("global") => {
            let symbol = cursor.symbol()?;
            let Some(Symbol::Global(global)) = module.symbol(symbol.text_name(scratch)?) else {
                return Err(symbol.expected("a global"));
            };
            cursor.expect(TokenKind::Ident("name"), "`name`")?;
            let name = cursor.symbol()?;
            let file = parse_file(cursor, module)?;
            let line = parse_number(cursor, "line")?;
            cursor.expect(TokenKind::Ident("type"), "`type`")?;
            let ty = parse_type_reference(cursor)?;
            let scope = if cursor.eat(TokenKind::Ident("scope")) {
                Some(parse_scope_id(cursor, module)?)
            } else {
                None
            };
            let (debug, arena) = module.debug_info_mut();
            let name = &*arena.alloc_str(name.text_name(scratch)?);
            debug.add_global(GlobalVariable {
                global,
                name,
                scope,
                file,
                line,
                ty,
            });
        },
        | TokenKind::Ident("loc") => {
            let number = cursor.next();
            defined_entity(number, "loc", module.debug_info().location_count())?;
            let location = parse_location(cursor, module)?;
            _ = module.debug_info_mut().0.location(location);
        },
        | _ => {
            return Err(
                token.expected("`file`, `unit`, `type`, `scope`, `variable`, `global` or `loc`")
            );
        },
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

/// `debug declare varN (slotN | vN) locN` in a body.
pub(super) fn parse_declare<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<DeclaredVariable, ParseError<'t>> {
    _ = cursor.next();
    cursor.expect(TokenKind::Ident("declare"), "`declare`")?;
    let variable = named_entity(
        cursor.next(),
        "var",
        module.debug_info().variables().count(),
    )?;
    let token = cursor.next();
    let place = match token.kind {
        | TokenKind::Slot(number) => VariablePlace::Slot(StackSlot::new(number as usize)),
        | TokenKind::Value(number) => VariablePlace::Address(Value::new(number as usize)),
        | _ => return Err(token.expected("a stack slot or a value")),
    };
    Ok(DeclaredVariable {
        variable: DebugVariable::new(variable),
        place,
        location: parse_location_id(cursor, module)?,
    })
}

/// `@name`, naming a declared function.
fn parse_function<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
    scratch: &Bump,
) -> Result<FuncId, ParseError<'t>> {
    let symbol = cursor.symbol()?;
    match module.symbol(symbol.text_name(scratch)?) {
        | Some(Symbol::Function(function)) => Ok(function),
        | _ => Err(symbol.expected("a function")),
    }
}

fn parse_scope_id<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
) -> Result<DebugScope, ParseError<'t>> {
    named_entity(cursor.next(), "scope", module.debug_info().scopes().count()).map(DebugScope::new)
}

/// `typeN` or `void`. The verifier checks that the type exists.
fn parse_type_reference<'t>(
    cursor: &mut Cursor<'_, 't>,
) -> Result<Option<DebugType>, ParseError<'t>> {
    let token = cursor.next();
    if token.kind == TokenKind::Ident("void") {
        return Ok(None);
    }
    entity_number(token, "type")
        .map(|number| Some(DebugType::new(number)))
        .ok_or_else(|| token.expected("a debug type or `void`"))
}

/// `@name` or `anonymous`, copied to the module's arena.
fn parse_optional_name<'t, 'ir>(
    cursor: &mut Cursor<'_, 't>,
    arena: &'ir Bump,
    scratch: &Bump,
) -> Result<Option<&'ir str>, ParseError<'t>> {
    if cursor.eat(TokenKind::Ident("anonymous")) {
        return Ok(None);
    }
    let name = cursor.symbol()?;
    Ok(Some(&*arena.alloc_str(name.text_name(scratch)?)))
}

fn parse_name<'t, 'ir>(
    cursor: &mut Cursor<'_, 't>,
    arena: &'ir Bump,
    scratch: &Bump,
) -> Result<&'ir str, ParseError<'t>> {
    let name = cursor.symbol()?;
    Ok(arena.alloc_str(name.text_name(scratch)?))
}

fn parse_u64<'t>(
    cursor: &mut Cursor<'_, 't>,
    keyword: &'static str,
) -> Result<u64, ParseError<'t>> {
    let token = cursor.next();
    if token.kind != TokenKind::Ident(keyword) {
        return Err(token.expected("a keyword and a number"));
    }
    let (token, number) = cursor.unsigned()?;
    u64::try_from(number).map_err(|_| token.expected("a number that fits in 64 bits"))
}

/// The text of a debug type after its number, as the printer writes it.
fn parse_type<'t, 'ir>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'ir>,
    scratch: &Bump,
) -> Result<DebugTypeData<'ir>, ParseError<'t>> {
    let arena = module.debug_info_mut().1;
    let token = cursor.next();
    let TokenKind::Ident(keyword) = token.kind else {
        return Err(token.expected("a kind of debug type"));
    };
    Ok(match keyword {
        | "base" => {
            let name = parse_name(cursor, arena, scratch)?;
            let size_bits = parse_u64(cursor, "size")?;
            cursor.expect(TokenKind::Ident("encoding"), "`encoding`")?;
            let token = cursor.next();
            let encoding = match token.kind {
                | TokenKind::Ident(keyword) => DebugEncoding::from_keyword(keyword),
                | _ => None,
            }
            .ok_or_else(|| token.expected("a debug encoding"))?;
            DebugTypeData::Base {
                name,
                size_bits,
                encoding,
            }
        },
        | "pointer" => DebugTypeData::Pointer {
            pointee: parse_type_reference(cursor)?,
        },
        | "array" => {
            let element = parse_type_reference(cursor)?;
            let count = if cursor.peek().kind == TokenKind::Ident("count") {
                Some(parse_u64(cursor, "count")?)
            } else {
                None
            };
            DebugTypeData::Array {
                element,
                count,
                size_bits: parse_u64(cursor, "size")?,
                align_bits: parse_number(cursor, "align")?,
            }
        },
        | "vector" => DebugTypeData::Vector {
            element:    parse_type_reference(cursor)?,
            count:      parse_u64(cursor, "count")?,
            size_bits:  parse_u64(cursor, "size")?,
            align_bits: parse_number(cursor, "align")?,
        },
        | "struct" | "union" => {
            let name = parse_optional_name(cursor, arena, scratch)?;
            let file = parse_file(cursor, module)?;
            let line = parse_number(cursor, "line")?;
            let size_bits = parse_u64(cursor, "size")?;
            let align_bits = parse_number(cursor, "align")?;
            let complete = !cursor.eat(TokenKind::Ident("incomplete"));
            let mut members = ArenaVec::new_in(scratch);
            if complete {
                cursor.expect(TokenKind::Ident("members"), "`members`")?;
                cursor.expect(TokenKind::LParen, "`(`")?;
                while !cursor.eat(TokenKind::RParen) {
                    if !members.is_empty() {
                        cursor.expect(TokenKind::Comma, "`,`")?;
                    }
                    let name = parse_optional_name(cursor, arena, scratch)?;
                    let ty = parse_type_reference(cursor)?;
                    let line = parse_number(cursor, "line")?;
                    let offset_bits = parse_u64(cursor, "offset")?;
                    let bit_field = if cursor.peek().kind == TokenKind::Ident("bits") {
                        Some(parse_number(cursor, "bits")?)
                    } else {
                        None
                    };
                    members.push(DebugMember {
                        name,
                        ty,
                        line,
                        offset_bits,
                        bit_field,
                    });
                }
            }
            DebugTypeData::Record(DebugRecord {
                union: keyword == "union",
                name,
                file,
                line,
                size_bits,
                align_bits,
                complete,
                members: arena.alloc_slice_copy(&members),
            })
        },
        | "enum" => {
            let name = parse_optional_name(cursor, arena, scratch)?;
            let file = parse_file(cursor, module)?;
            let line = parse_number(cursor, "line")?;
            let size_bits = parse_u64(cursor, "size")?;
            let align_bits = parse_number(cursor, "align")?;
            cursor.expect(TokenKind::Ident("base"), "`base`")?;
            let base = parse_type_reference(cursor)?;
            cursor.expect(TokenKind::Ident("values"), "`values`")?;
            cursor.expect(TokenKind::LParen, "`(`")?;
            let mut enumerators = ArenaVec::new_in(scratch);
            while !cursor.eat(TokenKind::RParen) {
                if !enumerators.is_empty() {
                    cursor.expect(TokenKind::Comma, "`,`")?;
                }
                let name = parse_name(cursor, arena, scratch)?;
                let token = cursor.next();
                let TokenKind::Int {
                    negative,
                    magnitude,
                } = token.kind
                else {
                    return Err(token.expected("an enumerator's value"));
                };
                let magnitude = i128::try_from(magnitude)
                    .map_err(|_| token.expected("an enumerator's value"))?;
                let value = if negative { -magnitude } else { magnitude };
                enumerators.push(DebugEnumerator {
                    name,
                    value: i64::try_from(value)
                        .map_err(|_| token.expected("an enumerator's value"))?,
                });
            }
            DebugTypeData::Enumeration {
                name,
                file,
                line,
                size_bits,
                align_bits,
                base,
                enumerators: arena.alloc_slice_copy(&enumerators),
            }
        },
        | "typedef" => {
            let name = parse_name(cursor, arena, scratch)?;
            cursor.expect(TokenKind::Ident("type"), "`type`")?;
            let ty = parse_type_reference(cursor)?;
            DebugTypeData::Typedef {
                name,
                ty,
                file: parse_file(cursor, module)?,
                line: parse_number(cursor, "line")?,
            }
        },
        | "function" => {
            let result = parse_type_reference(cursor)?;
            cursor.expect(TokenKind::LParen, "`(`")?;
            let mut parameters = ArenaVec::new_in(scratch);
            while !cursor.eat(TokenKind::RParen) {
                if !parameters.is_empty() {
                    cursor.expect(TokenKind::Comma, "`,`")?;
                }
                parameters.push(parse_type_reference(cursor)?);
            }
            DebugTypeData::Function {
                result,
                parameters: arena.alloc_slice_copy(&parameters),
                variadic: cursor.eat(TokenKind::Ident("variadic")),
                prototyped: cursor.eat(TokenKind::Ident("prototyped")),
            }
        },
        | keyword => match DebugQualifier::from_keyword(keyword) {
            | Some(qualifier) => DebugTypeData::Qualified {
                qualifier,
                ty: parse_type_reference(cursor)?,
            },
            | None => return Err(token.expected("a kind of debug type")),
        },
    })
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
