//! Reads the textual form back into a [`Module`].
//!
//! The parser makes two passes over the tokens. The first declares every
//! function and global, so a call or relocation may name a symbol defined
//! further down. The second parses initializers and bodies. Before a body is
//! parsed, a scan of its tokens finds every block, value and slot it defines;
//! their numbers must run from 0 without gaps, and each becomes the entity of
//! that number. A use may therefore precede its definition in the text, and
//! printing a parsed module reproduces its numbering. Nothing here recurses:
//! the grammar is flat, and lists are read in loops.
//!
//! `lexer.rs` splits tokens; `names.rs` decodes quoted symbol names before
//! declarations and uses resolve them in the module's symbol table.

mod lexer;
mod names;

use std::fmt;

use lexer::{
    Token,
    TokenKind,
    lex,
};

use super::{
    FunctionBuilder,
    Module,
    entities::{
        Block,
        BlockLabel,
        ConstId,
        Entity,
        FuncId,
        StackSlot,
        Value,
    },
    function::StackSlotData,
    inline_asm::{
        AsmClobbers,
        AsmConstraint,
        AsmOperand,
        InlineAsm,
    },
    instructions::{
        Align,
        BlockCall,
        FloatCC,
        InstData,
        InstFlags,
        IntCC,
        ObjectSizeQuery,
        Opcode,
    },
    module::{
        AggregateFlags,
        AggregateLayout,
        AggregateParam,
        AggregateScalar,
        DllStorage,
        ExtendedParam,
        Extension,
        FirstMember,
        GlobalDecl,
        GlobalInit,
        InitFunction,
        InitKind,
        InlineControl,
        Linkage,
        ParamAbi,
        Relocation,
        RelocationTarget,
        ResultRegisters,
        Signature,
        Symbol,
        SymbolAttributes,
        Visibility,
    },
    types::Type,
    verifier::Profile,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// Parses a module from its textual form into `arena`.
pub(crate) fn parse_module<'ir, 't>(
    arena: &'ir Bump,
    text: &'t str,
) -> Result<Module<'ir>, ParseError<'t>> {
    let scratch = Bump::new();
    let tokens = lex(text, &scratch)?;
    let mut module = Module::new(arena);
    let mut cursor = Cursor {
        tokens: &tokens,
        pos:    0,
    };
    declare_items(&mut cursor, &mut module, &scratch)?;
    cursor.pos = 0;
    define_items(&mut cursor, &mut module, &scratch)?;
    Ok(module)
}

/// The first pass: declares every function and global, skipping bodies and
/// initializers.
fn declare_items<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
    scratch: &Bump,
) -> Result<(), ParseError<'t>> {
    loop {
        let token = cursor.peek();
        match token.kind {
            | TokenKind::End => return Ok(()),
            | TokenKind::Ident("target") => parse_target(cursor, module)?,
            | TokenKind::Ident("module") => {
                let text = parse_module_asm(cursor)?;
                module.add_module_asm(text.asm_text(scratch)?);
            },
            | TokenKind::Ident("global") => {
                let (name, declaration, _) = parse_global_header(cursor)?;
                if module.symbol(name.text_name(scratch)?).is_some() {
                    return Err(name.error(ParseErrorKind::DuplicateSymbol(name.text)));
                }
                _ = module.declare_global(name.text_name(scratch)?, declaration);
                skip_global_init(cursor)?;
            },
            | TokenKind::Ident("abi") => parse_abi(cursor, module)?,
            | TokenKind::Ident("constructor" | "destructor") => {
                _ = cursor.next();
                _ = cursor.symbol()?;
                cursor.expect(TokenKind::Ident("priority"), "`priority`")?;
                _ = cursor.unsigned()?;
            },
            | TokenKind::Ident("function") => {
                let header = parse_function_header(cursor, scratch)?;
                let name = header.name;
                if module.symbol(name.text_name(scratch)?).is_some() {
                    return Err(name.error(ParseErrorKind::DuplicateSymbol(name.text)));
                }
                let signature = module.intern(header.signature);
                let func =
                    module.declare_function(name.text_name(scratch)?, signature, header.linkage);
                module.function_mut(func).align = header.align;
                // Text written without the keyword still gets the external
                // contract of a cataloged name, so hand-written `@setjmp`
                // declarations stay protected.
                module.function_mut(func).returns_twice = header.returns_twice
                    || (header.linkage != Linkage::Internal
                        && crate::builtins::returns_twice(name.text_name(scratch)?));
                module.function_mut(func).inlining = header.inlining;
                if let Some(external) = header.inline_copy_of {
                    module.set_inline_copy_of(func, external.text_name(scratch)?);
                }
                if cursor.eat(TokenKind::LBrace) {
                    while !matches!(cursor.peek().kind, TokenKind::RBrace | TokenKind::End) {
                        _ = cursor.next();
                    }
                    cursor.expect(TokenKind::RBrace, "`}`")?;
                }
            },
            | _ => return Err(token.expected(EXPECTED_ITEM)),
        }
    }
}

const EXPECTED_ITEM: &str =
    "`target`, `module`, `abi`, `global`, `constructor`, `destructor` or `function`";

/// The second pass: parses initializers and bodies.
fn define_items<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
    scratch: &Bump,
) -> Result<(), ParseError<'t>> {
    loop {
        let token = cursor.peek();
        match token.kind {
            | TokenKind::End => return Ok(()),
            | TokenKind::Ident("target") => parse_target(cursor, module)?,
            | TokenKind::Ident("module") => _ = parse_module_asm(cursor)?,
            | TokenKind::Ident("abi") => parse_abi(cursor, module)?,
            | TokenKind::Ident("global") => {
                let (name, _, attributes) = parse_global_header(cursor)?;
                let Some(Symbol::Global(global)) = module.symbol(name.text_name(scratch)?) else {
                    unreachable!("the first pass declared every global");
                };
                let attributes = attributes.resolve(module, scratch)?;
                module.set_symbol_attributes(Symbol::Global(global), attributes);
                if cursor.eat(TokenKind::Equals) {
                    parse_global_init(cursor, module, global, scratch)?;
                }
            },
            | TokenKind::Ident(keyword @ ("constructor" | "destructor")) => {
                _ = cursor.next();
                let token = cursor.symbol()?;
                let Some(Symbol::Function(func)) = module.symbol(token.text_name(scratch)?) else {
                    return Err(token.error(ParseErrorKind::NotAFunction(token.text)));
                };
                cursor.expect(TokenKind::Ident("priority"), "`priority`")?;
                let (number, priority) = cursor.unsigned()?;
                let priority = u16::try_from(priority)
                    .map_err(|_| number.error(ParseErrorKind::NumberTooLarge))?;
                module.add_init_function(InitFunction {
                    kind: if keyword == "constructor" {
                        InitKind::Constructor
                    } else {
                        InitKind::Destructor
                    },
                    priority,
                    func,
                });
            },
            | TokenKind::Ident("function") => {
                let header = parse_function_header(cursor, scratch)?;
                let name = header.name;
                let Some(Symbol::Function(func)) = module.symbol(name.text_name(scratch)?) else {
                    unreachable!("the first pass declared every function");
                };
                let attributes = header.attributes.resolve(module, scratch)?;
                module.set_symbol_attributes(Symbol::Function(func), attributes);
                if cursor.eat(TokenKind::LBrace) {
                    parse_body(cursor, module, func, scratch)?;
                }
            },
            | _ => return Err(token.expected(EXPECTED_ITEM)),
        }
    }
}

/// `module asm "..."`: one file-scope assembly block, returned as its
/// string token. The first pass decodes it into the module, so blocks keep
/// their order; the second skips it.
fn parse_module_asm<'t>(cursor: &mut Cursor<'_, 't>) -> Result<Token<'t>, ParseError<'t>> {
    _ = cursor.next();
    cursor.expect(TokenKind::Ident("asm"), "`asm`")?;
    let text = cursor.next();
    if !matches!(text.kind, TokenKind::String(_)) {
        return Err(text.expected("a string"));
    }
    Ok(text)
}

/// `abi lowered`: the module is post-ABI.
fn parse_abi<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
) -> Result<(), ParseError<'t>> {
    _ = cursor.next();
    cursor.expect(TokenKind::Ident("lowered"), "`lowered`")?;
    module.set_profile(Profile::PostAbi);
    Ok(())
}

/// `target triple = "..."` or `target datalayout = "..."`.
fn parse_target<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
) -> Result<(), ParseError<'t>> {
    _ = cursor.next();
    let key = cursor.next();
    let is_triple = match key.kind {
        | TokenKind::Ident("triple") => true,
        | TokenKind::Ident("datalayout") => false,
        | _ => return Err(key.expected("`triple` or `datalayout`")),
    };
    cursor.expect(TokenKind::Equals, "`=`")?;
    let value = cursor.next();
    let TokenKind::String(text) = value.kind else {
        return Err(value.expected("a string"));
    };
    if is_triple {
        let layout = module.data_layout();
        module.set_target(text, layout);
    } else {
        let triple = module.triple();
        module.set_target(triple, text);
    }
    Ok(())
}

/// `global @name linkage [thread_local] [constant] size N, align N` and its
/// symbol attributes, up to the initializer.
fn parse_global_header<'t>(
    cursor: &mut Cursor<'_, 't>,
) -> Result<(Token<'t>, GlobalDecl, WrittenAttributes<'t>), ParseError<'t>> {
    _ = cursor.next();
    let name = cursor.symbol()?;
    let linkage = cursor.linkage()?;
    let thread_local = cursor.eat(TokenKind::Ident("thread_local"));
    let constant = cursor.eat(TokenKind::Ident("constant"));
    cursor.expect(TokenKind::Ident("size"), "`size`")?;
    let size = cursor.unsigned()?;
    cursor.expect(TokenKind::Comma, "`,`")?;
    let align = cursor.align()?;
    let attributes = WrittenAttributes::parse(cursor)?;
    Ok((
        name,
        GlobalDecl {
            linkage,
            size: u64::try_from(size.1)
                .map_err(|_| size.0.error(ParseErrorKind::NumberTooLarge))?,
            align,
            constant,
            thread_local,
        },
        attributes,
    ))
}

/// `[hidden|protected] [dllimport|dllexport] [section "name"] [used]
/// [alias @target]`, with the target resolved once every symbol is declared.
#[derive(Clone, Copy)]
struct WrittenAttributes<'t> {
    visibility: Visibility,
    dll:        DllStorage,
    section:    Option<&'t str>,
    used:       bool,
    alias:      Option<Token<'t>>,
}

impl<'t> WrittenAttributes<'t> {
    fn parse(cursor: &mut Cursor<'_, 't>) -> Result<Self, ParseError<'t>> {
        let visibility = if cursor.eat(TokenKind::Ident("hidden")) {
            Visibility::Hidden
        } else if cursor.eat(TokenKind::Ident("protected")) {
            Visibility::Protected
        } else {
            Visibility::Default
        };
        let dll = if cursor.eat(TokenKind::Ident("dllimport")) {
            DllStorage::Import
        } else if cursor.eat(TokenKind::Ident("dllexport")) {
            DllStorage::Export
        } else {
            DllStorage::Default
        };
        let section = if cursor.eat(TokenKind::Ident("section")) {
            Some(cursor.string()?.1)
        } else {
            None
        };
        let used = cursor.eat(TokenKind::Ident("used"));
        let alias = if cursor.eat(TokenKind::Ident("alias")) {
            Some(cursor.symbol()?)
        } else {
            None
        };
        Ok(Self {
            visibility,
            dll,
            section,
            used,
            alias,
        })
    }

    fn resolve(
        self,
        module: &Module<'_>,
        scratch: &Bump,
    ) -> Result<SymbolAttributes<'t>, ParseError<'t>> {
        let alias = self
            .alias
            .map(|token| {
                module
                    .symbol(token.text_name(scratch)?)
                    .ok_or_else(|| token.error(ParseErrorKind::UnknownSymbol(token.text)))
            })
            .transpose()?;
        Ok(SymbolAttributes {
            alias,
            section: self.section,
            used: self.used,
            visibility: self.visibility,
            dll: self.dll,
        })
    }
}

/// Skips `= zero` or `= bytes "..." [relocs [...]]` in the first pass.
fn skip_global_init<'t>(cursor: &mut Cursor<'_, 't>) -> Result<(), ParseError<'t>> {
    if !cursor.eat(TokenKind::Equals) {
        return Ok(());
    }
    if cursor.eat(TokenKind::Ident("zero")) {
        return Ok(());
    }
    cursor.expect(TokenKind::Ident("bytes"), "`zero` or `bytes`")?;
    _ = cursor.string()?;
    if cursor.eat(TokenKind::Ident("relocs")) {
        while !matches!(cursor.peek().kind, TokenKind::RBracket | TokenKind::End) {
            _ = cursor.next();
        }
        cursor.expect(TokenKind::RBracket, "`]`")?;
    }
    Ok(())
}

/// `zero`, or `bytes "hex" [relocs [offset: @symbol [+|- addend], ...]]`.
fn parse_global_init<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
    global: super::GlobalId,
    scratch: &Bump,
) -> Result<(), ParseError<'t>> {
    if cursor.eat(TokenKind::Ident("zero")) {
        module.define_global(global, GlobalInit::Zero);
        return Ok(());
    }
    cursor.expect(TokenKind::Ident("bytes"), "`zero` or `bytes`")?;
    let (token, hex) = cursor.string()?;
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(token.error(ParseErrorKind::BadHex));
    }
    let mut bytes = ArenaVec::with_capacity_in(hex.len() / 2, scratch);
    for pair in hex.as_bytes().as_chunks::<2>().0 {
        let digits = std::str::from_utf8(pair).map_err(|_| token.error(ParseErrorKind::BadHex))?;
        bytes
            .push(u8::from_str_radix(digits, 16).map_err(|_| token.error(ParseErrorKind::BadHex))?);
    }
    let mut relocations = ArenaVec::new_in(scratch);
    if cursor.eat(TokenKind::Ident("relocs")) {
        cursor.expect(TokenKind::LBracket, "`[`")?;
        while !cursor.eat(TokenKind::RBracket) {
            if !relocations.is_empty() {
                cursor.expect(TokenKind::Comma, "`,` or `]`")?;
            }
            let (offset_token, offset) = cursor.unsigned()?;
            cursor.expect(TokenKind::Colon, "`:`")?;
            let target = parse_relocation_target(cursor, module, scratch)?;
            let addend = if cursor.eat(TokenKind::Plus) {
                let (token, magnitude) = cursor.unsigned()?;
                i64::try_from(magnitude).map_err(|_| token.error(ParseErrorKind::NumberTooLarge))?
            } else if cursor.eat(TokenKind::Minus) {
                let (token, magnitude) = cursor.unsigned()?;
                0_i64
                    .checked_sub_unsigned(
                        u64::try_from(magnitude)
                            .map_err(|_| token.error(ParseErrorKind::NumberTooLarge))?,
                    )
                    .ok_or_else(|| token.error(ParseErrorKind::NumberTooLarge))?
            } else {
                0
            };
            relocations.push(Relocation {
                offset: u64::try_from(offset)
                    .map_err(|_| offset_token.error(ParseErrorKind::NumberTooLarge))?,
                target,
                addend,
            });
        }
    }
    module.define_global(
        global,
        GlobalInit::Bytes {
            bytes:       &bytes,
            relocations: &relocations,
        },
    );
    Ok(())
}

/// `@symbol`, `blockaddress(@f, labelN)` or `labeldiff.ty(@f, labelA,
/// labelB)`. Label numbers are checked by the verifier, since the body that
/// defines them may come later in the text.
fn parse_relocation_target<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &Module<'_>,
    scratch: &Bump,
) -> Result<RelocationTarget, ParseError<'t>> {
    let function = |cursor: &mut Cursor<'_, 't>| -> Result<FuncId, ParseError<'t>> {
        let name = cursor.symbol()?;
        match module.symbol(name.text_name(scratch)?) {
            | Some(Symbol::Function(func)) => Ok(func),
            | _ => Err(name.error(ParseErrorKind::NotAFunction(name.text))),
        }
    };
    let token = cursor.peek();
    match token.kind {
        | TokenKind::Ident("blockaddress") => {
            _ = cursor.next();
            cursor.expect(TokenKind::LParen, "`(`")?;
            let func = function(cursor)?;
            cursor.expect(TokenKind::Comma, "`,`")?;
            let label = cursor.label()?;
            cursor.expect(TokenKind::RParen, "`)`")?;
            Ok(RelocationTarget::Label { func, label })
        },
        | TokenKind::Ident("labeldiff") => {
            _ = cursor.next();
            cursor.expect(TokenKind::Dot, "`.` and a type")?;
            let width = cursor.ty()?;
            cursor.expect(TokenKind::LParen, "`(`")?;
            let func = function(cursor)?;
            cursor.expect(TokenKind::Comma, "`,`")?;
            let label = cursor.label()?;
            cursor.expect(TokenKind::Comma, "`,`")?;
            let base = cursor.label()?;
            cursor.expect(TokenKind::RParen, "`)`")?;
            Ok(RelocationTarget::LabelDifference {
                func,
                label,
                base,
                width,
            })
        },
        | _ => {
            let name = cursor.symbol()?;
            module
                .symbol(name.text_name(scratch)?)
                .map(RelocationTarget::Symbol)
                .ok_or_else(|| name.error(ParseErrorKind::UnknownSymbol(name.text)))
        },
    }
}

/// `function @name(params) [-> ty] linkage [align N] [returns_twice]
/// [noinline | alwaysinline] [inline_copy_of @name]` and its symbol
/// attributes, up to the body.
fn parse_function_header<'t, 's>(
    cursor: &mut Cursor<'_, 't>,
    scratch: &'s Bump,
) -> Result<FunctionHeader<'t, 's>, ParseError<'t>> {
    _ = cursor.next();
    let name = cursor.symbol()?;
    let signature = cursor.signature(scratch)?;
    let linkage = cursor.linkage()?;
    let align = if cursor.peek().kind == TokenKind::Ident("align") {
        Some(cursor.align()?)
    } else {
        None
    };
    let returns_twice = cursor.eat(TokenKind::Ident("returns_twice"));
    let inlining = [InlineControl::Never, InlineControl::Always]
        .into_iter()
        .find(|control| {
            control
                .keyword()
                .is_some_and(|keyword| cursor.eat(TokenKind::Ident(keyword)))
        })
        .unwrap_or_default();
    let inline_copy_of = if cursor.eat(TokenKind::Ident("inline_copy_of")) {
        Some(cursor.symbol()?)
    } else {
        None
    };
    let attributes = WrittenAttributes::parse(cursor)?;
    Ok(FunctionHeader {
        name,
        signature,
        linkage,
        align,
        returns_twice,
        inlining,
        inline_copy_of,
        attributes,
    })
}

struct FunctionHeader<'t, 's> {
    name:           Token<'t>,
    signature:      Signature<'s>,
    linkage:        Linkage,
    align:          Option<Align>,
    returns_twice:  bool,
    inlining:       InlineControl,
    inline_copy_of: Option<Token<'t>>,
    attributes:     WrittenAttributes<'t>,
}

/// Parses a body after its `{`, through its `}`.
fn parse_body<'t>(
    cursor: &mut Cursor<'_, 't>,
    module: &mut Module<'_>,
    func: FuncId,
    scratch: &Bump,
) -> Result<(), ParseError<'t>> {
    let counts = Definitions::scan(cursor, scratch)?;
    let mut builder = FunctionBuilder::new(module, func);
    for _ in 0..counts.blocks {
        _ = builder.create_block();
    }
    for _ in 0..counts.values {
        _ = builder.reserve_value();
    }
    for _ in 0..counts.slots {
        _ = builder.create_stack_slot(0, Align::BYTE);
    }
    for _ in 0..counts.labels {
        _ = builder.create_label(Block::new(0));
    }
    let mut body = BodyParser {
        cursor,
        builder,
        counts,
        scratch,
        switches: ArenaVec::new_in(scratch),
    };
    loop {
        let token = body.cursor.peek();
        match token.kind {
            | TokenKind::RBrace => {
                _ = body.cursor.next();
                break;
            },
            | TokenKind::Slot(_) | TokenKind::Label(_) | TokenKind::Value(_) if !token.line_start =>
                return Err(token.expected("a definition at the start of a line")),
            | TokenKind::Slot(_) => body.stack_slot()?,
            | TokenKind::Label(_) => body.label_definition()?,
            | TokenKind::Block(_) => body.block_header()?,
            | TokenKind::Value(_) => {
                let result = body.defined_value()?;
                body.cursor.expect(TokenKind::Equals, "`=`")?;
                body.instruction(Some(result))?;
            },
            | TokenKind::Ident(_) => body.instruction(None)?,
            | _ => return Err(token.expected("a block, an instruction or `}`")),
        }
    }
    body.normalize_switches();
    body.builder.finish();
    Ok(())
}

/// How many blocks, values, stack slots and labels a body defines.
#[derive(Clone, Copy, Debug)]
struct Definitions {
    blocks: usize,
    values: usize,
    slots:  usize,
    labels: usize,
}

impl Definitions {
    /// Scans a body's tokens for definitions, checking that each kind is
    /// numbered from 0 without gaps or repeats. A block is defined by its
    /// header, a value by `vN =` at the start of a line or `vN:` in a block
    /// header, a slot by `slotN =` and a label by `labelN =` at the start of
    /// a line. Body parsing enforces the same line boundary for named
    /// instructions, slots and labels.
    fn scan<'t>(cursor: &Cursor<'_, 't>, scratch: &Bump) -> Result<Self, ParseError<'t>> {
        let end = cursor.tokens[cursor.pos..]
            .iter()
            .position(|token| matches!(token.kind, TokenKind::RBrace | TokenKind::End))
            .map_or(cursor.tokens.len(), |offset| cursor.pos + offset);
        let tokens = &cursor.tokens[cursor.pos..end];
        let definition = |index: usize, header: bool| -> Option<(usize, u32)> {
            let token = tokens[index];
            let next = tokens.get(index + 1).map(|next| next.kind);
            match token.kind {
                | TokenKind::Block(number) if token.line_start => Some((0, number)),
                | TokenKind::Value(number)
                    if (token.line_start && next == Some(TokenKind::Equals))
                        || (header && next == Some(TokenKind::Colon)) =>
                    Some((1, number)),
                | TokenKind::Slot(number) if token.line_start && next == Some(TokenKind::Equals) =>
                    Some((2, number)),
                | TokenKind::Label(number) if token.line_start && next == Some(TokenKind::Equals) =>
                    Some((3, number)),
                | _ => None,
            }
        };
        let mut counts = [0_usize; 4];
        let mut header = false;
        for (index, token) in tokens.iter().enumerate() {
            // A parameter list may span lines; only its closing delimiter
            // (or the colon of a parameterless header) ends the header.
            if token.line_start && matches!(token.kind, TokenKind::Block(_)) {
                header = true;
            } else if token.kind == TokenKind::RParen
                || (token.kind == TokenKind::Colon
                    && index > 0
                    && matches!(tokens[index - 1].kind, TokenKind::Block(_)))
            {
                header = false;
            }
            if let Some((kind, _)) = definition(index, header) {
                counts[kind] += 1;
            }
        }
        let mut seen = counts.map(|count| {
            let mut seen = ArenaVec::with_capacity_in(count, scratch);
            seen.resize(count, false);
            seen
        });
        let mut header = false;
        for (index, &token) in tokens.iter().enumerate() {
            // A parameter list may span lines; only its closing delimiter
            // (or the colon of a parameterless header) ends the header.
            if token.line_start && matches!(token.kind, TokenKind::Block(_)) {
                header = true;
            } else if token.kind == TokenKind::RParen
                || (token.kind == TokenKind::Colon
                    && index > 0
                    && matches!(tokens[index - 1].kind, TokenKind::Block(_)))
            {
                header = false;
            }
            let Some((kind, number)) = definition(index, header) else {
                continue;
            };
            let slot = seen[kind]
                .get_mut(number as usize)
                .ok_or_else(|| token.error(ParseErrorKind::NotDense(token.text)))?;
            if *slot {
                return Err(token.error(ParseErrorKind::DuplicateDefinition(token.text)));
            }
            *slot = true;
        }
        Ok(Self {
            blocks: counts[0],
            values: counts[1],
            slots:  counts[2],
            labels: counts[3],
        })
    }
}

/// The state of one body's second pass.
struct BodyParser<'c, 'k, 't, 'm, 'ir, 's> {
    cursor:   &'c mut Cursor<'k, 't>,
    builder:  FunctionBuilder<'m, 'ir>,
    counts:   Definitions,
    scratch:  &'s Bump,
    /// Switches whose case constants wait for their value's type.
    switches: ArenaVec<'s, super::Inst>,
}

impl<'t, 's> BodyParser<'_, '_, 't, '_, '_, 's> {
    /// One instruction after its optional `vN =`.
    fn instruction(&mut self, result: Option<Value>) -> Result<(), ParseError<'t>> {
        let token = self.cursor.next();
        let TokenKind::Ident(name) = token.kind else {
            return Err(token.expected("an opcode"));
        };
        let opcode = Opcode::from_name(name)
            .ok_or_else(|| token.error(ParseErrorKind::UnknownOpcode(name)))?;
        if self.builder.current_block().is_none() {
            return Err(token.error(ParseErrorKind::OutsideBlock));
        }
        let ty = if opcode.has_type_suffix() {
            self.cursor.expect(TokenKind::Dot, "`.` and a type")?;
            self.cursor.ty()?
        } else {
            Type::Ptr
        };
        let data = self.operands(opcode, ty)?;
        match (result, self.builder.module().result_type(&data)) {
            | (Some(_), None) => return Err(token.error(ParseErrorKind::UnexpectedResult(opcode))),
            | (None, Some(_)) => return Err(token.error(ParseErrorKind::MissingResult(opcode))),
            | _ => {},
        }
        let inst = self.builder.insert_with_result(data, result);
        if opcode == Opcode::Switch {
            self.switches.push(inst);
        }
        Ok(())
    }

    /// The flags and operands of an instruction, as its record.
    fn operands(&mut self, opcode: Opcode, ty: Type) -> Result<InstData, ParseError<'t>> {
        Ok(match opcode {
            | Opcode::VaCopy => {
                let args = self.values::<2>()?;
                InstData::Binary {
                    opcode,
                    ty: Type::Ptr,
                    flags: InstFlags::empty(),
                    args,
                }
            },
            | _ if opcode.is_int_binary()
                || opcode.is_overflow()
                || opcode.is_float_binary()
                || matches!(opcode, Opcode::PtrAdd | Opcode::Subobject) =>
            {
                let flags = self.cursor.flags();
                InstData::Binary {
                    opcode,
                    ty,
                    flags,
                    args: self.values::<2>()?,
                }
            },
            | _ if opcode.is_conversion()
                || opcode.is_bit_unary()
                || matches!(
                    opcode,
                    Opcode::Fneg
                        | Opcode::Freeze
                        | Opcode::VaArg
                        | Opcode::VaStart
                        | Opcode::VaEnd
                        | Opcode::StackRestore
                        | Opcode::SjljLongjmp
                ) =>
                InstData::Unary {
                    opcode,
                    ty,
                    arg: self.value()?,
                },
            | Opcode::SjljSetjmp => InstData::Unary {
                opcode,
                ty: Type::I32,
                arg: self.value()?,
            },
            | Opcode::AdjustTrampoline => InstData::Unary {
                opcode,
                ty: Type::Ptr,
                arg: self.value()?,
            },
            | Opcode::DynAlloca => {
                let size = self.value()?;
                let align = if self.cursor.eat(TokenKind::Comma) {
                    self.cursor.align()?
                } else {
                    // Preserve legacy text's default stack alignment.
                    Align::from_bytes(16).unwrap()
                };
                InstData::DynAlloca { ty, size, align }
            },
            | Opcode::ObjectSize => {
                let dynamic = self.cursor.eat(TokenKind::Ident("dynamic"));
                let ptr = self.value()?;
                self.cursor.expect(TokenKind::Comma, "`,`")?;
                let (token, mode) = self.cursor.unsigned()?;
                let query = u8::try_from(mode)
                    .ok()
                    .and_then(|mode| ObjectSizeQuery::new(mode, dynamic))
                    .ok_or_else(|| token.expected("an object-size mode from 0 to 3"))?;
                InstData::ObjectSize { ty, query, ptr }
            },
            | Opcode::VaArgAggregate => {
                let list = self.value()?;
                self.cursor.expect(TokenKind::Colon, "`:`")?;
                let signature = self.cursor.signature(self.scratch)?;
                let sig = self.builder.intern(signature);
                InstData::VaArgAggregate { sig, list }
            },
            | Opcode::Icmp => {
                let token = self.cursor.next();
                let cond = match token.kind {
                    | TokenKind::Ident(name) => IntCC::from_name(name),
                    | _ => None,
                }
                .ok_or_else(|| token.expected("an integer condition"))?;
                InstData::IntCompare {
                    cond,
                    ty,
                    args: self.values::<2>()?,
                }
            },
            | Opcode::Fcmp => {
                let token = self.cursor.next();
                let cond = match token.kind {
                    | TokenKind::Ident(name) => FloatCC::from_name(name),
                    | _ => None,
                }
                .ok_or_else(|| token.expected("a floating condition"))?;
                InstData::FloatCompare {
                    cond,
                    ty,
                    args: self.values::<2>()?,
                }
            },
            | Opcode::Select => InstData::Select {
                ty,
                args: self.values::<3>()?,
            },
            | Opcode::Iconst => {
                let bits = self.cursor.int_bits(ty)?;
                self.builder.constant(opcode, ty, bits)
            },
            | Opcode::Fconst => {
                let token = self.cursor.next();
                let TokenKind::Hex(bits) = token.kind else {
                    return Err(token.expected("hexadecimal bits"));
                };
                if bits & !ty.mask() != 0 {
                    return Err(token.error(ParseErrorKind::IntegerOutOfRange(ty)));
                }
                self.builder.constant(opcode, ty, bits)
            },
            | Opcode::Poison => InstData::Nullary { opcode, ty },
            | Opcode::ReadTimestamp => InstData::Nullary {
                opcode,
                ty: Type::I64,
            },
            | Opcode::Null
            | Opcode::Trap
            | Opcode::DebugTrap
            | Opcode::StackSave
            | Opcode::FrameAddress
            | Opcode::ReturnAddress => InstData::Nullary {
                opcode,
                ty: Type::Ptr,
            },
            | Opcode::StackAddr => {
                let token = self.cursor.next();
                let TokenKind::Slot(number) = token.kind else {
                    return Err(token.expected("a stack slot"));
                };
                if number as usize >= self.counts.slots {
                    return Err(token.error(ParseErrorKind::Undefined(token.text)));
                }
                InstData::StackAddr {
                    slot: StackSlot::new(number as usize),
                }
            },
            | Opcode::GlobalAddr => {
                let token = self.cursor.symbol()?;
                match self.builder.module().symbol(token.text_name(self.scratch)?) {
                    | Some(Symbol::Global(global)) => InstData::GlobalAddr { global },
                    | _ => return Err(token.error(ParseErrorKind::NotAGlobal(token.text))),
                }
            },
            | Opcode::FuncAddr => InstData::FuncAddr {
                func: self.function()?,
            },
            | Opcode::BlockAddr => InstData::BlockAddr {
                label: self.label()?,
            },
            | Opcode::Atomic => {
                let flags = self.cursor.flags();
                let token = self.cursor.next();
                let op = if let TokenKind::Ident(name) = token.kind {
                    super::AtomicOp::from_name(name)
                } else {
                    None
                }
                .ok_or_else(|| token.expected("an atomic operation"))?;
                let token = self.cursor.next();
                let order = if let TokenKind::Ident(name) = token.kind {
                    super::MemoryOrder::from_name(name)
                } else {
                    None
                }
                .ok_or_else(|| token.expected("a memory order"))?;
                let token = self.cursor.next();
                let failure = if let TokenKind::Ident(name) = token.kind {
                    super::MemoryOrder::from_name(name)
                } else {
                    None
                }
                .ok_or_else(|| token.expected("a failure order"))?;
                let align = self.cursor.align()?;
                let args = self.argument_list(None)?;
                InstData::Atomic {
                    op,
                    ty,
                    flags,
                    align,
                    order,
                    failure,
                    args: self.builder.value_list(&args),
                }
            },
            | Opcode::Load => {
                let flags = self.cursor.flags();
                let addr = self.value()?;
                let (align, tag) = self.cursor.memory_facts()?;
                InstData::Load {
                    ty,
                    flags,
                    align,
                    tag,
                    addr,
                }
            },
            | Opcode::Store => {
                let flags = self.cursor.flags();
                let args = self.values::<2>()?;
                let (align, tag) = self.cursor.memory_facts()?;
                InstData::Store {
                    ty,
                    flags,
                    align,
                    tag,
                    args,
                }
            },
            | Opcode::Copy | Opcode::Fill | Opcode::Trampoline => {
                let flags = self.cursor.flags();
                let args = self.values::<3>()?;
                self.cursor.expect(TokenKind::Comma, "`,`")?;
                InstData::MemoryRange {
                    opcode,
                    flags,
                    align: self.cursor.align()?,
                    args,
                }
            },
            | Opcode::Call => {
                let func = self.function()?;
                let args = self.argument_list(None)?;
                InstData::Call {
                    func,
                    args: self.builder.value_list(&args),
                }
            },
            | Opcode::CallIndirect => {
                let callee = self.value()?;
                let args = self.argument_list(Some(callee))?;
                self.cursor
                    .expect(TokenKind::Colon, "`:` and a signature")?;
                let signature = self.cursor.signature(self.scratch)?;
                InstData::CallIndirect {
                    sig:  self.builder.intern(signature),
                    args: self.builder.value_list(&args),
                }
            },
            | Opcode::InlineAsm => self.inline_asm()?,
            | Opcode::Jump => InstData::Jump {
                dest: self.block_call()?,
            },
            | Opcode::Brif => {
                let cond = self.value()?;
                self.cursor.expect(TokenKind::Comma, "`,`")?;
                let then = self.block_call()?;
                self.cursor.expect(TokenKind::Comma, "`,`")?;
                InstData::Brif {
                    cond,
                    dests: [then, self.block_call()?],
                }
            },
            | Opcode::Switch => self.switch()?,
            | Opcode::Return => InstData::Return {
                value: if matches!(self.cursor.peek().kind, TokenKind::Value(_))
                    && !self.cursor.peek().line_start
                {
                    Some(self.value()?)
                } else {
                    None
                },
            },
            | Opcode::IndirectBr => {
                let addr = self.value()?;
                self.cursor.expect(TokenKind::Comma, "`,`")?;
                self.cursor.expect(TokenKind::LBracket, "`[`")?;
                let mut dests: ArenaVec<'_, BlockCall> = ArenaVec::new_in(self.scratch);
                while !self.cursor.eat(TokenKind::RBracket) {
                    if !dests.is_empty() {
                        self.cursor.expect(TokenKind::Comma, "`,` or `]`")?;
                    }
                    dests.push(self.block_call()?);
                }
                InstData::IndirectBr {
                    addr,
                    dests: self.builder.block_list(&dests),
                }
            },
            | Opcode::Unreachable => InstData::Unreachable,
            | _ => unreachable!("every opcode has an arm above"),
        })
    }

    /// `[sideeffect] "template" (out|in "constraint" type [bytes N] value,
    /// ...) [clobber(name, ...)]`: adds the descriptor to the module.
    fn inline_asm(&mut self) -> Result<InstData, ParseError<'t>> {
        let side_effects = self.cursor.eat(TokenKind::Ident("sideeffect"));
        let token = self.cursor.next();
        let template = token.asm_text(self.scratch)?;
        self.cursor.expect(TokenKind::LParen, "`(`")?;
        let mut operands = ArenaVec::new_in(self.scratch);
        let mut values = ArenaVec::new_in(self.scratch);
        while !self.cursor.eat(TokenKind::RParen) {
            if !operands.is_empty() {
                self.cursor.expect(TokenKind::Comma, "`,` or `)`")?;
            }
            let role = self.cursor.next();
            let output = match role.kind {
                | TokenKind::Ident("out") => true,
                | TokenKind::Ident("in") => false,
                | _ => return Err(role.expected("`out` or `in`")),
            };
            let (token, text) = self.cursor.string()?;
            let (early_clobber, text) = match text.strip_prefix('&') {
                | Some(rest) => (true, rest),
                | None => (false, text),
            };
            let constraint =
                AsmConstraint::parse(text).ok_or_else(|| token.expected("an asm constraint"))?;
            let ty = self.cursor.ty()?;
            let bytes = if self.cursor.eat(TokenKind::Ident("bytes")) {
                let (token, bytes) = self.cursor.unsigned()?;
                u32::try_from(bytes).map_err(|_| token.expected("a byte count"))?
            } else {
                ty.bytes()
            };
            operands.push(AsmOperand {
                output,
                early_clobber,
                constraint,
                ty,
                bytes,
            });
            values.push(self.value()?);
        }
        let mut clobbers = AsmClobbers::default();
        if self.cursor.eat(TokenKind::Ident("clobber")) {
            self.cursor.expect(TokenKind::LParen, "`(`")?;
            let mut first = true;
            while !self.cursor.eat(TokenKind::RParen) {
                if !first {
                    self.cursor.expect(TokenKind::Comma, "`,` or `)`")?;
                }
                first = false;
                let token = self.cursor.next();
                if !matches!(token.kind, TokenKind::Ident(name) if clobbers.add(name)) {
                    return Err(token.expected("a clobbered register"));
                }
            }
        }
        let asm = InlineAsm {
            template,
            operands: &operands,
            clobbers,
            side_effects,
        };
        Ok(InstData::InlineAsm {
            asm:  self.builder.add_inline_asm(&asm),
            args: self.builder.value_list(&values),
        })
    }

    /// `value, default, [case: dest, ...]`. Case constants are kept as
    /// written until [`BodyParser::normalize_switches`].
    fn switch(&mut self) -> Result<InstData, ParseError<'t>> {
        let value = self.value()?;
        self.cursor.expect(TokenKind::Comma, "`,`")?;
        let default = self.block_call()?;
        self.cursor.expect(TokenKind::Comma, "`,`")?;
        self.cursor.expect(TokenKind::LBracket, "`[`")?;
        let mut cases: ArenaVec<'_, (ConstId, BlockCall)> = ArenaVec::new_in(self.scratch);
        while !self.cursor.eat(TokenKind::RBracket) {
            if !cases.is_empty() {
                self.cursor.expect(TokenKind::Comma, "`,` or `]`")?;
            }
            let bits = self.cursor.int_bits(Type::I128)?;
            self.cursor.expect(TokenKind::Colon, "`:`")?;
            let constant = self.builder.wide_constant(bits);
            let dest = self.block_call()?;
            cases.push((constant, dest));
        }
        Ok(InstData::Switch {
            value,
            default,
            cases: self.builder.case_list(&cases),
        })
    }

    /// Truncates each switch's case constants to its value's type, now that
    /// every value has one.
    fn normalize_switches(&mut self) {
        let body = &mut self.builder.body;
        for &inst in &self.switches {
            let InstData::Switch { value, cases, .. } = body.insts[inst] else {
                continue;
            };
            let mask = body.values[value].ty.mask();
            let start = cases.0 as usize;
            let len = body.pool[start].index();
            for pair in 0..len {
                let constant = ConstId::new(body.pool[start + 1 + 2 * pair].index());
                body.constants[constant] &= mask;
            }
        }
    }

    /// `slotN = stack_slot size, align N`.
    fn stack_slot(&mut self) -> Result<(), ParseError<'t>> {
        let token = self.cursor.next();
        let TokenKind::Slot(number) = token.kind else {
            unreachable!("the caller saw a slot");
        };
        if number as usize >= self.counts.slots {
            return Err(token.error(ParseErrorKind::NotDense(token.text)));
        }
        self.cursor.expect(TokenKind::Equals, "`=`")?;
        self.cursor
            .expect(TokenKind::Ident("stack_slot"), "`stack_slot`")?;
        let (size_token, size) = self.cursor.unsigned()?;
        self.cursor.expect(TokenKind::Comma, "`,`")?;
        let align = self.cursor.align()?;
        self.builder.body.stack_slots[StackSlot::new(number as usize)] = StackSlotData {
            size: u32::try_from(size)
                .map_err(|_| size_token.error(ParseErrorKind::NumberTooLarge))?,
            align,
        };
        Ok(())
    }

    /// `labelN = block_label blockM`.
    fn label_definition(&mut self) -> Result<(), ParseError<'t>> {
        let token = self.cursor.next();
        let TokenKind::Label(number) = token.kind else {
            unreachable!("the caller saw a label");
        };
        if number as usize >= self.counts.labels {
            return Err(token.error(ParseErrorKind::NotDense(token.text)));
        }
        self.cursor.expect(TokenKind::Equals, "`=`")?;
        self.cursor
            .expect(TokenKind::Ident("block_label"), "`block_label`")?;
        let block = self.cursor.next();
        let TokenKind::Block(target) = block.kind else {
            return Err(block.expected("a block"));
        };
        if target as usize >= self.counts.blocks {
            return Err(block.error(ParseErrorKind::Undefined(block.text)));
        }
        self.builder.define_label(
            BlockLabel::new(number as usize),
            Block::new(target as usize),
        );
        Ok(())
    }

    /// A label operand of `block_addr`, defined by this body.
    fn label(&mut self) -> Result<BlockLabel, ParseError<'t>> {
        let token = self.cursor.peek();
        let label = self.cursor.label()?;
        if label.index() >= self.counts.labels {
            return Err(token.error(ParseErrorKind::Undefined(token.text)));
        }
        Ok(label)
    }

    /// `blockN:` or `blockN(vA: ty, ...):`, which makes the block current.
    fn block_header(&mut self) -> Result<(), ParseError<'t>> {
        let token = self.cursor.next();
        let TokenKind::Block(number) = token.kind else {
            unreachable!("the caller saw a block");
        };
        if !token.line_start {
            return Err(token.expected("a block header at the start of a line"));
        }
        if number as usize >= self.counts.blocks {
            return Err(token.error(ParseErrorKind::NotDense(token.text)));
        }
        let block = Block::new(number as usize);
        if self.cursor.eat(TokenKind::LParen) {
            let mut first = true;
            while !self.cursor.eat(TokenKind::RParen) {
                if !first {
                    self.cursor.expect(TokenKind::Comma, "`,` or `)`")?;
                }
                first = false;
                let value = self.defined_value()?;
                self.cursor.expect(TokenKind::Colon, "`:`")?;
                let ty = self.cursor.ty()?;
                self.builder.define_block_param(block, value, ty);
            }
        }
        self.cursor.expect(TokenKind::Colon, "`:`")?;
        self.builder.switch_to_block(block);
        Ok(())
    }

    /// Checks definition indices again at the untrusted-text boundary.
    fn defined_value(&mut self) -> Result<Value, ParseError<'t>> {
        let token = self.cursor.next();
        match token.kind {
            | TokenKind::Value(number) if (number as usize) < self.counts.values =>
                Ok(Value::new(number as usize)),
            | TokenKind::Value(_) => Err(token.error(ParseErrorKind::NotDense(token.text))),
            | _ => Err(token.expected("a value")),
        }
    }

    /// A value operand.
    fn value(&mut self) -> Result<Value, ParseError<'t>> {
        let token = self.cursor.next();
        match token.kind {
            | TokenKind::Value(number) if (number as usize) < self.counts.values =>
                Ok(Value::new(number as usize)),
            | TokenKind::Value(_) => Err(token.error(ParseErrorKind::Undefined(token.text))),
            | _ => Err(token.expected("a value")),
        }
    }

    /// `N` comma-separated value operands.
    fn values<const N: usize>(&mut self) -> Result<[Value; N], ParseError<'t>> {
        let mut values = [Value::new(0); N];
        for (index, slot) in values.iter_mut().enumerate() {
            if index > 0 {
                self.cursor.expect(TokenKind::Comma, "`,`")?;
            }
            *slot = self.value()?;
        }
        Ok(values)
    }

    /// `(v1, v2, ...)`, after `first` if there is one.
    fn argument_list(
        &mut self,
        first: Option<Value>,
    ) -> Result<ArenaVec<'s, Value>, ParseError<'t>> {
        let mut args = ArenaVec::new_in(self.scratch);
        args.extend(first);
        self.cursor.expect(TokenKind::LParen, "`(`")?;
        let mut first = true;
        while !self.cursor.eat(TokenKind::RParen) {
            if !first {
                self.cursor.expect(TokenKind::Comma, "`,` or `)`")?;
            }
            first = false;
            args.push(self.value()?);
        }
        Ok(args)
    }

    /// `blockN` or `blockN(args)`.
    fn block_call(&mut self) -> Result<BlockCall, ParseError<'t>> {
        let token = self.cursor.next();
        let block = match token.kind {
            | TokenKind::Block(number) if (number as usize) < self.counts.blocks =>
                Block::new(number as usize),
            | TokenKind::Block(_) => return Err(token.error(ParseErrorKind::Undefined(token.text))),
            | _ => return Err(token.expected("a block")),
        };
        let args = if self.cursor.peek().kind == TokenKind::LParen {
            self.argument_list(None)?
        } else {
            ArenaVec::new_in(self.scratch)
        };
        Ok(self.builder.block_call(block, &args))
    }

    /// `@name` of a function.
    fn function(&mut self) -> Result<FuncId, ParseError<'t>> {
        let token = self.cursor.symbol()?;
        match self.builder.module().symbol(token.text_name(self.scratch)?) {
            | Some(Symbol::Function(func)) => Ok(func),
            | _ => Err(token.error(ParseErrorKind::NotAFunction(token.text))),
        }
    }
}

/// A position in the token array.
struct Cursor<'k, 't> {
    tokens: &'k [Token<'t>],
    pos:    usize,
}

impl<'t> Cursor<'_, 't> {
    fn peek(&self) -> Token<'t> {
        self.tokens[self.pos]
    }

    /// The next token; the end token repeats once reached.
    fn next(&mut self) -> Token<'t> {
        let token = self.tokens[self.pos];
        if token.kind != TokenKind::End {
            self.pos += 1;
        }
        token
    }

    /// Consumes the next token if it is `kind`.
    fn eat(&mut self, kind: TokenKind<'_>) -> bool {
        let matches = self.peek().kind == kind;
        if matches {
            _ = self.next();
        }
        matches
    }

    fn expect(
        &mut self,
        kind: TokenKind<'_>,
        expected: &'static str,
    ) -> Result<(), ParseError<'t>> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.peek().expected(expected))
        }
    }

    fn symbol(&mut self) -> Result<Token<'t>, ParseError<'t>> {
        let token = self.next();
        match token.kind {
            | TokenKind::Symbol(_) | TokenKind::QuotedSymbol(_) => Ok(token),
            | _ => Err(token.expected("a `@` symbol")),
        }
    }

    /// `labelN`, unchecked against any body.
    fn label(&mut self) -> Result<BlockLabel, ParseError<'t>> {
        let token = self.next();
        match token.kind {
            | TokenKind::Label(number) => Ok(BlockLabel::new(number as usize)),
            | _ => Err(token.expected("a label")),
        }
    }

    fn string(&mut self) -> Result<(Token<'t>, &'t str), ParseError<'t>> {
        let token = self.next();
        match token.kind {
            | TokenKind::String(text) => Ok((token, text)),
            | _ => Err(token.expected("a string")),
        }
    }

    fn ty(&mut self) -> Result<Type, ParseError<'t>> {
        let token = self.next();
        match token.kind {
            | TokenKind::Ident(name) =>
                Type::from_name(name).ok_or_else(|| token.error(ParseErrorKind::UnknownType(name))),
            | _ => Err(token.expected("a type")),
        }
    }

    fn linkage(&mut self) -> Result<Linkage, ParseError<'t>> {
        let token = self.next();
        match token.kind {
            | TokenKind::Ident(name) => Linkage::from_name(name),
            | _ => None,
        }
        .ok_or_else(|| token.expected("`external`, `internal`, `weak`, `selectany` or `linkonce`"))
    }

    /// Any flag names that follow.
    fn flags(&mut self) -> InstFlags {
        let mut flags = InstFlags::empty();
        while let TokenKind::Ident(name) = self.peek().kind
            && let Some(flag) = InstFlags::from_keyword(name)
        {
            flags |= flag;
            _ = self.next();
        }
        flags
    }

    /// A non-negative decimal integer.
    fn unsigned(&mut self) -> Result<(Token<'t>, u128), ParseError<'t>> {
        let token = self.next();
        match token.kind {
            | TokenKind::Int {
                negative: false,
                magnitude,
            } => Ok((token, magnitude)),
            | _ => Err(token.expected("a non-negative integer")),
        }
    }

    /// `align N`.
    fn align(&mut self) -> Result<Align, ParseError<'t>> {
        self.expect(TokenKind::Ident("align"), "`align`")?;
        let (token, bytes) = self.unsigned()?;
        u64::try_from(bytes)
            .ok()
            .and_then(Align::from_bytes)
            .ok_or_else(|| token.error(ParseErrorKind::BadAlignment))
    }

    /// `, align N` and an optional `, tag N` after a memory access.
    fn memory_facts(&mut self) -> Result<(Align, Option<super::AccessTag>), ParseError<'t>> {
        self.expect(TokenKind::Comma, "`,`")?;
        let align = self.align()?;
        if !self.eat(TokenKind::Comma) {
            return Ok((align, None));
        }
        self.expect(TokenKind::Ident("tag"), "`tag`")?;
        let (token, index) = self.unsigned()?;
        let tag = u32::try_from(index)
            .ok()
            .and_then(super::AccessTag::new)
            .ok_or_else(|| token.error(ParseErrorKind::NumberTooLarge))?;
        Ok((align, Some(tag)))
    }

    /// A decimal integer as the bits of `ty`. It may be written signed or
    /// unsigned, but must fit the type's width either way.
    fn int_bits(&mut self, ty: Type) -> Result<u128, ParseError<'t>> {
        let token = self.next();
        let TokenKind::Int {
            negative,
            magnitude,
        } = token.kind
        else {
            return Err(token.expected("an integer"));
        };
        let fits = if negative {
            magnitude <= 1 << (ty.bits() - 1)
        } else {
            magnitude <= ty.mask()
        };
        if !fits {
            return Err(token.error(ParseErrorKind::IntegerOutOfRange(ty)));
        }
        Ok(if negative {
            magnitude.wrapping_neg() & ty.mask()
        } else {
            magnitude
        })
    }

    /// `(ty [abi | extension] [noundef] [chain], ..., [...]) [-> ty
    /// [extension]]`, with its lists in `scratch`; `chain` marks the last
    /// fixed parameter as a static chain, which the verifier checks.
    fn signature<'s>(&mut self, scratch: &'s Bump) -> Result<Signature<'s>, ParseError<'t>> {
        self.expect(TokenKind::LParen, "`(`")?;
        let mut params = ArenaVec::new_in(scratch);
        let mut aggregates = ArenaVec::new_in(scratch);
        let mut extensions = ArenaVec::new_in(scratch);
        let mut variadic = false;
        let mut variadic_fixed = None;
        let mut noundef = ArenaVec::new_in(scratch);
        let mut variadic_noundef = false;
        let mut chain = None;
        while !self.eat(TokenKind::RParen) {
            if !params.is_empty() || variadic {
                self.expect(TokenKind::Comma, "`,` or `)`")?;
            }
            if self.eat(TokenKind::Ellipsis) {
                if variadic {
                    return Err(self.peek().expected("one ellipsis"));
                }
                variadic = true;
                variadic_fixed = Some(
                    u32::try_from(params.len())
                        .map_err(|_| self.peek().error(ParseErrorKind::NumberTooLarge))?,
                );
                variadic_noundef = self.eat(TokenKind::Ident("noundef"));
            } else {
                params.push(self.ty()?);
                let index = u32::try_from(params.len() - 1)
                    .map_err(|_| self.peek().error(ParseErrorKind::NumberTooLarge))?;
                if let Some(abi) = self.param_abi(scratch)? {
                    aggregates.push(AggregateParam { index, abi });
                } else if let Some(extension) = self.extension() {
                    extensions.push(ExtendedParam { index, extension });
                }
                if self.eat(TokenKind::Ident("noundef")) {
                    noundef.push(index);
                }
                if self.eat(TokenKind::Ident("chain")) {
                    chain = Some(params.len());
                }
            }
        }
        let fixed = variadic_fixed.map_or(params.len(), |n| n as usize);
        if chain.is_some_and(|after| after != fixed) {
            return Err(self.peek().expected("`chain` on the last fixed parameter"));
        }
        let (result, result_extension, result_noundef) = if self.eat(TokenKind::Arrow) {
            if self.eat(TokenKind::Ident("noundef")) {
                (None, None, true)
            } else {
                (
                    Some(self.ty()?),
                    self.extension(),
                    self.eat(TokenKind::Ident("noundef")),
                )
            }
        } else {
            (None, None, false)
        };
        Ok(Signature {
            params: scratch.alloc_slice_copy(&params),
            result,
            variadic,
            variadic_fixed: variadic_fixed.filter(|&n| n as usize != params.len()),
            aggregates: scratch.alloc_slice_copy(&aggregates),
            extensions: scratch.alloc_slice_copy(&extensions),
            result_extension,
            noundef: scratch.alloc_slice_copy(&noundef),
            result_noundef,
            variadic_noundef,
            chain: chain.is_some(),
        })
    }

    /// The extension keyword after a parameter or result type, if one
    /// follows: `signed`, `unsigned`, `bool`, `signext` or `zeroext`.
    fn extension(&mut self) -> Option<Extension> {
        let TokenKind::Ident(keyword) = self.peek().kind else {
            return None;
        };
        let extension = Extension::from_keyword(keyword)?;
        _ = self.next();
        Some(extension)
    }

    /// The description of an aggregate parameter after its type, if one
    /// follows: `aggregate`, `result`, `byval`, `indirect` or `sret` with a
    /// layout, as `(size N, align N[, flag]...[, [offset: ty, ...]])`, or
    /// `registers(ty[, ty][, offset N][, complex], align N)`.
    fn param_abi<'s>(&mut self, scratch: &'s Bump) -> Result<Option<ParamAbi<'s>>, ParseError<'t>> {
        let TokenKind::Ident(keyword) = self.peek().kind else {
            return Ok(None);
        };
        let make: fn(AggregateLayout<'s>) -> ParamAbi<'s> = match keyword {
            | "aggregate" => ParamAbi::Aggregate,
            | "result" => ParamAbi::Result,
            | "byval" => ParamAbi::ByVal,
            | "indirect" => ParamAbi::Indirect,
            | "sret" => ParamAbi::Sret,
            | "registers" => return self.result_registers().map(Some),
            | _ => return Ok(None),
        };
        _ = self.next();
        self.expect(TokenKind::LParen, "`(`")?;
        self.expect(TokenKind::Ident("size"), "`size`")?;
        let (token, size) = self.unsigned()?;
        let size = u64::try_from(size).map_err(|_| token.error(ParseErrorKind::NumberTooLarge))?;
        self.expect(TokenKind::Comma, "`,`")?;
        let align = self.align()?;
        let mut flags = AggregateFlags::empty();
        let mut scalars = ArenaVec::new_in(scratch);
        let mut first = None;
        while self.eat(TokenKind::Comma) {
            if self.peek().kind == TokenKind::Ident("first") {
                let token = self.next();
                // One first member: a second would contradict the first.
                if first.is_some() {
                    return Err(token.error(ParseErrorKind::DuplicateDefinition(token.text)));
                }
                first = Some(self.first_member()?);
                continue;
            }
            if self.eat(TokenKind::LBracket) {
                while !self.eat(TokenKind::RBracket) {
                    if !scalars.is_empty() {
                        self.expect(TokenKind::Comma, "`,` or `]`")?;
                    }
                    let (token, offset) = self.unsigned()?;
                    let offset = u32::try_from(offset)
                        .map_err(|_| token.error(ParseErrorKind::NumberTooLarge))?;
                    self.expect(TokenKind::Colon, "`:`")?;
                    let ty = self.ty()?;
                    scalars.push(AggregateScalar { offset, ty });
                }
                break;
            }
            let token = self.next();
            flags |= match token.kind {
                | TokenKind::Ident(name) => AggregateFlags::NAMES
                    .iter()
                    .find(|&&(_, text)| text == name)
                    .map(|&(flag, _)| flag),
                | _ => None,
            }
            .ok_or_else(|| token.expected("`unaligned`, `flexible`, `first` or `[`"))?;
        }
        self.expect(TokenKind::RParen, "`)`")?;
        Ok(Some(make(AggregateLayout {
            size,
            align,
            flags,
            scalars: scratch.alloc_slice_copy(&scalars),
            first,
        })))
    }

    /// `(size N[, extension])` after `first`: a transparent union's first
    /// member.
    fn first_member(&mut self) -> Result<FirstMember, ParseError<'t>> {
        self.expect(TokenKind::LParen, "`(`")?;
        self.expect(TokenKind::Ident("size"), "`size`")?;
        let (token, size) = self.unsigned()?;
        let size = u64::try_from(size).map_err(|_| token.error(ParseErrorKind::NumberTooLarge))?;
        let mut extension = None;
        if self.eat(TokenKind::Comma) {
            let token = self.next();
            extension = Some(
                match token.kind {
                    | TokenKind::Ident(name) => Extension::from_keyword(name),
                    | _ => None,
                }
                .ok_or_else(|| token.expected("an extension"))?,
            );
        }
        self.expect(TokenKind::RParen, "`)`")?;
        Ok(FirstMember { size, extension })
    }

    /// `registers(ty[, ty][, offset N][, complex], align N)`.
    fn result_registers<'s>(&mut self) -> Result<ParamAbi<'s>, ParseError<'t>> {
        _ = self.next();
        self.expect(TokenKind::LParen, "`(`")?;
        let first = self.ty()?;
        self.expect(TokenKind::Comma, "`,`")?;
        let second = if matches!(
            self.peek().kind,
            TokenKind::Ident("align" | "offset" | "complex")
        ) {
            None
        } else {
            let second = self.ty()?;
            self.expect(TokenKind::Comma, "`,`")?;
            Some(second)
        };
        let second_offset = if self.eat(TokenKind::Ident("offset")) {
            let (token, offset) = self.unsigned()?;
            let offset =
                u32::try_from(offset).map_err(|_| token.error(ParseErrorKind::NumberTooLarge))?;
            self.expect(TokenKind::Comma, "`,`")?;
            offset
        } else {
            8
        };
        let complex = self.eat(TokenKind::Ident("complex"));
        if complex {
            self.expect(TokenKind::Comma, "`,`")?;
        }
        let align = self.align()?;
        self.expect(TokenKind::RParen, "`)`")?;
        Ok(ParamAbi::Registers(ResultRegisters {
            first,
            second,
            second_offset,
            complex,
            align,
        }))
    }
}

impl<'t> Token<'t> {
    fn error(&self, kind: ParseErrorKind<'t>) -> ParseError<'t> {
        ParseError {
            line: self.line,
            column: self.column,
            kind,
        }
    }

    fn expected(&self, expected: &'static str) -> ParseError<'t> {
        self.error(ParseErrorKind::Expected {
            expected,
            found: self.text,
        })
    }
}

/// Why the textual form could not be parsed, and where.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ParseError<'t> {
    /// 1-based.
    pub(crate) line:   u32,
    /// 1-based, in bytes.
    pub(crate) column: u32,
    pub(crate) kind:   ParseErrorKind<'t>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ParseErrorKind<'t> {
    UnexpectedCharacter(char),
    UnterminatedString,
    /// A quoted symbol has an unknown escape or invalid Unicode scalar.
    BadSymbolEscape,
    NumberTooLarge,
    Expected {
        expected: &'static str,
        /// The text found instead; empty at the end of input.
        found:    &'t str,
    },
    UnknownOpcode(&'t str),
    UnknownType(&'t str),
    DuplicateSymbol(&'t str),
    UnknownSymbol(&'t str),
    NotAFunction(&'t str),
    NotAGlobal(&'t str),
    /// A block, value or slot is defined twice.
    DuplicateDefinition(&'t str),
    /// A block, value or slot number skips one: numbers run from 0 without
    /// gaps.
    NotDense(&'t str),
    /// A block, value or slot is used but never defined.
    Undefined(&'t str),
    /// An instruction precedes the first block header.
    OutsideBlock,
    /// A result is named for an instruction that defines none.
    UnexpectedResult(Opcode),
    /// An instruction that defines a value has no `vN =`.
    MissingResult(Opcode),
    IntegerOutOfRange(Type),
    BadAlignment,
    /// A global's bytes are not pairs of hexadecimal digits.
    BadHex,
}

impl fmt::Display for ParseError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.kind)
    }
}

impl fmt::Display for ParseErrorKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            | Self::UnexpectedCharacter(character) =>
                write!(f, "unexpected character {character:?}"),
            | Self::UnterminatedString => f.write_str("unterminated string"),
            | Self::BadSymbolEscape => f.write_str("invalid escape in quoted symbol"),
            | Self::NumberTooLarge => f.write_str("number too large"),
            | Self::Expected {
                expected,
                found: "",
            } => write!(f, "expected {expected}, found the end"),
            | Self::Expected { expected, found } =>
                write!(f, "expected {expected}, found `{found}`"),
            | Self::UnknownOpcode(name) => write!(f, "unknown opcode `{name}`"),
            | Self::UnknownType(name) => write!(f, "unknown type `{name}`"),
            | Self::DuplicateSymbol(name) => write!(f, "`{name}` is declared twice"),
            | Self::UnknownSymbol(name) => write!(f, "`{name}` is not declared"),
            | Self::NotAFunction(name) => write!(f, "`{name}` is not a declared function"),
            | Self::NotAGlobal(name) => write!(f, "`{name}` is not a declared global"),
            | Self::DuplicateDefinition(name) => write!(f, "`{name}` is defined twice"),
            | Self::NotDense(name) => write!(
                f,
                "`{name}` skips a number; blocks, values and slots are numbered from 0 without \
                 gaps"
            ),
            | Self::Undefined(name) => write!(f, "`{name}` is never defined"),
            | Self::OutsideBlock => f.write_str("an instruction must follow a block header"),
            | Self::UnexpectedResult(opcode) => write!(f, "`{opcode}` defines no value"),
            | Self::MissingResult(opcode) =>
                write!(f, "`{opcode}` defines a value that needs a name"),
            | Self::IntegerOutOfRange(ty) => write!(f, "the constant does not fit in {ty}"),
            | Self::BadAlignment => f.write_str("an alignment is a power of two up to 2^31"),
            | Self::BadHex => f.write_str("bytes are written as pairs of hexadecimal digits"),
        }
    }
}
