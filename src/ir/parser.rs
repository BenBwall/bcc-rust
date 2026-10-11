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

mod lexer;

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
        ConstId,
        Entity,
        FuncId,
        StackSlot,
        Value,
    },
    function::StackSlotData,
    instructions::{
        Align,
        BlockCall,
        FloatCC,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
    },
    module::{
        GlobalDecl,
        GlobalInit,
        Linkage,
        Relocation,
        Symbol,
    },
    types::Type,
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
            | TokenKind::Ident("global") => {
                let (name, declaration) = parse_global_header(cursor)?;
                if module.symbol(name.text_name()).is_some() {
                    return Err(name.error(ParseErrorKind::DuplicateSymbol(name.text)));
                }
                _ = module.declare_global(name.text_name(), declaration);
                skip_global_init(cursor)?;
            },
            | TokenKind::Ident("function") => {
                let (name, params, result, variadic, linkage) =
                    parse_function_header(cursor, scratch)?;
                if module.symbol(name.text_name()).is_some() {
                    return Err(name.error(ParseErrorKind::DuplicateSymbol(name.text)));
                }
                let signature = module.intern_signature(&params, result, variadic);
                _ = module.declare_function(name.text_name(), signature, linkage);
                if cursor.eat(TokenKind::LBrace) {
                    while !matches!(cursor.peek().kind, TokenKind::RBrace | TokenKind::End) {
                        _ = cursor.next();
                    }
                    cursor.expect(TokenKind::RBrace, "`}`")?;
                }
            },
            | _ => return Err(token.expected("`target`, `global` or `function`")),
        }
    }
}

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
            | TokenKind::Ident("global") => {
                let (name, _) = parse_global_header(cursor)?;
                let Some(Symbol::Global(global)) = module.symbol(name.text_name()) else {
                    unreachable!("the first pass declared every global");
                };
                if cursor.eat(TokenKind::Equals) {
                    parse_global_init(cursor, module, global, scratch)?;
                }
            },
            | TokenKind::Ident("function") => {
                let (name, ..) = parse_function_header(cursor, scratch)?;
                let Some(Symbol::Function(func)) = module.symbol(name.text_name()) else {
                    unreachable!("the first pass declared every function");
                };
                if cursor.eat(TokenKind::LBrace) {
                    parse_body(cursor, module, func, scratch)?;
                }
            },
            | _ => return Err(token.expected("`target`, `global` or `function`")),
        }
    }
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

/// `global @name linkage [constant] size N, align N`, up to the initializer.
fn parse_global_header<'t>(
    cursor: &mut Cursor<'_, 't>,
) -> Result<(Token<'t>, GlobalDecl), ParseError<'t>> {
    _ = cursor.next();
    let name = cursor.symbol()?;
    let linkage = cursor.linkage()?;
    let constant = cursor.eat(TokenKind::Ident("constant"));
    cursor.expect(TokenKind::Ident("size"), "`size`")?;
    let size = cursor.unsigned()?;
    cursor.expect(TokenKind::Comma, "`,`")?;
    let align = cursor.align()?;
    Ok((
        name,
        GlobalDecl {
            linkage,
            size: u64::try_from(size.1)
                .map_err(|_| size.0.error(ParseErrorKind::NumberTooLarge))?,
            align,
            constant,
        },
    ))
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
            let name = cursor.symbol()?;
            let symbol = module
                .symbol(name.text_name())
                .ok_or_else(|| name.error(ParseErrorKind::UnknownSymbol(name.text)))?;
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
                symbol,
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

/// `function @name(params) [-> ty] linkage`, up to the body.
#[expect(
    clippy::type_complexity,
    reason = "Both passes destructure the header in place."
)]
fn parse_function_header<'t, 's>(
    cursor: &mut Cursor<'_, 't>,
    scratch: &'s Bump,
) -> Result<(Token<'t>, ArenaVec<'s, Type>, Option<Type>, bool, Linkage), ParseError<'t>> {
    _ = cursor.next();
    let name = cursor.symbol()?;
    let (params, result, variadic) = cursor.signature(scratch)?;
    let linkage = cursor.linkage()?;
    Ok((name, params, result, variadic, linkage))
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
            | TokenKind::Slot(_) => body.stack_slot()?,
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

/// How many blocks, values and stack slots a body defines.
#[derive(Clone, Copy, Debug)]
struct Definitions {
    blocks: usize,
    values: usize,
    slots:  usize,
}

impl Definitions {
    /// Scans a body's tokens for definitions, checking that each kind is
    /// numbered from 0 without gaps or repeats. A block is defined by its
    /// header, a value by `vN =` at the start of a line or `vN:` in a block
    /// header, and a slot by `slotN =`.
    fn scan<'t>(cursor: &Cursor<'_, 't>, scratch: &Bump) -> Result<Self, ParseError<'t>> {
        let end = cursor.tokens[cursor.pos..]
            .iter()
            .position(|token| matches!(token.kind, TokenKind::RBrace | TokenKind::End))
            .map_or(cursor.tokens.len(), |offset| cursor.pos + offset);
        let tokens = &cursor.tokens[cursor.pos..end];
        let definition = |index: usize| -> Option<(usize, u32)> {
            let token = tokens[index];
            let next = tokens.get(index + 1).map(|next| next.kind);
            match token.kind {
                | TokenKind::Block(number) if token.line_start => Some((0, number)),
                | TokenKind::Value(number)
                    if (token.line_start && next == Some(TokenKind::Equals))
                        || next == Some(TokenKind::Colon) =>
                    Some((1, number)),
                | TokenKind::Slot(number) if token.line_start && next == Some(TokenKind::Equals) =>
                    Some((2, number)),
                | _ => None,
            }
        };
        let mut counts = [0_usize; 3];
        for index in 0..tokens.len() {
            if let Some((kind, _)) = definition(index) {
                counts[kind] += 1;
            }
        }
        let mut seen = counts.map(|count| {
            let mut seen = ArenaVec::with_capacity_in(count, scratch);
            seen.resize(count, false);
            seen
        });
        for (index, &token) in tokens.iter().enumerate() {
            let Some((kind, number)) = definition(index) else {
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
            | _ if opcode.is_int_binary() || opcode.is_float_binary() || opcode == Opcode::PtrAdd =>
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
                || matches!(
                    opcode,
                    Opcode::Fneg | Opcode::Freeze | Opcode::VaArg | Opcode::VaStart | Opcode::VaEnd
                ) =>
                InstData::Unary {
                    opcode,
                    ty,
                    arg: self.value()?,
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
            | Opcode::Null => InstData::Nullary {
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
                match self.builder.module().symbol(token.text_name()) {
                    | Some(Symbol::Global(global)) => InstData::GlobalAddr { global },
                    | _ => return Err(token.error(ParseErrorKind::NotAGlobal(token.text))),
                }
            },
            | Opcode::FuncAddr => InstData::FuncAddr {
                func: self.function()?,
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
            | Opcode::Copy | Opcode::Fill => {
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
                let (params, result, variadic) = self.cursor.signature(self.scratch)?;
                InstData::CallIndirect {
                    sig:  self.builder.intern_signature(&params, result, variadic),
                    args: self.builder.value_list(&args),
                }
            },
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
            | Opcode::Unreachable => InstData::Unreachable,
            | _ => unreachable!("every opcode has an arm above"),
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

    /// `blockN:` or `blockN(vA: ty, ...):`, which makes the block current.
    fn block_header(&mut self) -> Result<(), ParseError<'t>> {
        let token = self.cursor.next();
        let TokenKind::Block(number) = token.kind else {
            unreachable!("the caller saw a block");
        };
        if !token.line_start {
            return Err(token.expected("a block header at the start of a line"));
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

    /// The value a definition names; the scan guarantees it is in range.
    fn defined_value(&mut self) -> Result<Value, ParseError<'t>> {
        let token = self.cursor.next();
        match token.kind {
            | TokenKind::Value(number) => Ok(Value::new(number as usize)),
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
        match self.builder.module().symbol(token.text_name()) {
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
            | TokenKind::Symbol(_) => Ok(token),
            | _ => Err(token.expected("a `@` symbol")),
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
        .ok_or_else(|| token.expected("`external` or `internal`"))
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

    /// `(ty, ..., [...]) [-> ty]`.
    fn signature<'s>(
        &mut self,
        scratch: &'s Bump,
    ) -> Result<(ArenaVec<'s, Type>, Option<Type>, bool), ParseError<'t>> {
        self.expect(TokenKind::LParen, "`(`")?;
        let mut params = ArenaVec::new_in(scratch);
        let mut variadic = false;
        while !self.eat(TokenKind::RParen) {
            if variadic {
                return Err(self.peek().expected("`)` after `...`"));
            }
            if !params.is_empty() {
                self.expect(TokenKind::Comma, "`,` or `)`")?;
            }
            if self.eat(TokenKind::Ellipsis) {
                variadic = true;
            } else {
                params.push(self.ty()?);
            }
        }
        let result = if self.eat(TokenKind::Arrow) {
            Some(self.ty()?)
        } else {
            None
        };
        Ok((params, result, variadic))
    }
}

impl<'t> Token<'t> {
    /// A symbol token's name without its `@`.
    fn text_name(&self) -> &'t str {
        match self.kind {
            | TokenKind::Symbol(name) => name,
            | _ => self.text,
        }
    }

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
            | Self::BadAlignment => f.write_str("an alignment is a power of two up to 2^32"),
            | Self::BadHex => f.write_str("bytes are written as pairs of hexadecimal digits"),
        }
    }
}
