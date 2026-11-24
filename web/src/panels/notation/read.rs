//! Readers of single lines of output: each takes one line of a view apart
//! into the parts the figure glosses, or declines a line of another shape.
//! A reader accepts only the shapes it can gloss exactly, so a line it
//! accepts from any program is explained correctly.

use super::{
    Example,
    Ink::{
        self,
        Bytes,
        Instruction,
        Muted,
        Number,
        Qualifier,
        Register,
        Type,
        Value,
    },
    Part,
    Reading,
};

/// bcc IR: `v3 = iadd.i32 nsw v1, v2`, a signed `int` add, subtract or
/// multiply (`middle-end.md`, "Undefined behaviour": C lowering sets `nsw`
/// on signed `+ - *`, C99 §6.5p5).
pub fn bcc_ir(line: &str) -> Option<Reading> {
    let (result, rest) = line.trim().split_once(" = ")?;
    let (operation, rest) = rest.split_once(' ')?;
    let (op, ty) = operation.split_once('.')?;
    let (verb, noun) = match op {
        | "iadd" => ("add", "sum"),
        | "isub" => ("subtract", "difference"),
        | "imul" => ("multiply", "product"),
        | _ => return None,
    };
    let (left, right) = rest.strip_prefix("nsw ")?.split_once(", ")?;
    if ty != "i32" || ![result, left, right].into_iter().all(is_bcc_value) {
        return None;
    }
    let reads = match op {
        | "iadd" =>
            format!("Add {left} and {right} as 32-bit integers, and call the sum {result}."),
        | "isub" => format!(
            "Subtract {right} from {left} as 32-bit integers, and call the difference {result}."
        ),
        | _ =>
            format!("Multiply {left} by {right} as 32-bit integers, and call the product {result}."),
    };
    Some(Reading {
        rank:    0,
        example: Example {
            parts: vec![
                part(
                    vec![(result, Value), (" =", Qualifier)],
                    "the result: a new value, assigned only here",
                ),
                part(
                    vec![(op, Instruction)],
                    format!("the operation: integer {verb}"),
                ),
                joined(
                    vec![(".i32", Type)],
                    "its type: a 32-bit integer, as int is",
                ),
                part(
                    vec![("nsw", Qualifier)],
                    format!(
                        "no signed wrap: bcc may assume the {noun} fits, since signed overflow is \
                         undefined"
                    ),
                ),
                part(
                    vec![
                        (left, Value),
                        (",", Qualifier),
                        (&format!(" {right}"), Value),
                    ],
                    "the operands: values made earlier",
                ),
            ],
            reads,
        },
    })
}

/// `v12`
fn is_bcc_value(name: &str) -> bool {
    name.strip_prefix('v').is_some_and(is_number)
}

/// LLVM IR: `%v1 = load i32, ptr %v0, align 4` (LangRef, `load`), or
/// else `%v7 = add nsw i32 %v4, 3`, a signed `int` add, subtract or
/// multiply (LangRef, `add`: with `nsw`, the result is poison on signed
/// overflow).
pub fn llvm_ir(line: &str) -> Option<Reading> {
    let (result, rest) = line.trim().split_once(" = ")?;
    let Some(rest) = rest.strip_prefix("load ") else {
        return llvm_arithmetic(result, rest);
    };
    let mut fields = rest.split(", ");
    let (ty, pointer, align) = (fields.next()?, fields.next()?, fields.next()?);
    let pointer = pointer.strip_prefix("ptr ")?;
    let align = align.strip_prefix("align ")?;
    if fields.next().is_some()
        || ![result, pointer].into_iter().all(is_llvm_local)
        || !is_number(align)
    {
        return None;
    }
    let noun = match ty {
        | "i8" => "8-bit integer",
        | "i16" => "16-bit integer",
        | "i32" => "32-bit integer",
        | "i64" => "64-bit integer",
        | "ptr" => "pointer",
        | "float" => "32-bit floating-point number",
        | "double" => "64-bit floating-point number",
        | _ => return None,
    };
    Some(Reading {
        rank:    0,
        example: Example {
            parts: vec![
                part(
                    vec![(result, Value), (" =", Qualifier)],
                    "the result: % marks a local name, assigned only here",
                ),
                part(vec![("load", Instruction)], "the operation: read memory"),
                part(
                    vec![(ty, Type), (",", Qualifier)],
                    format!("the type it reads: {}", article(noun)),
                ),
                part(
                    vec![
                        ("ptr", Type),
                        (&format!(" {pointer}"), Value),
                        (",", Qualifier),
                    ],
                    format!("from the address the pointer {pointer} holds"),
                ),
                part(
                    vec![("align", Qualifier), (&format!(" {align}"), Number)],
                    format!("a promise that the address is a multiple of {align}"),
                ),
            ],
            reads: format!("Read the {noun} at the address in {pointer}, and call it {result}."),
        },
    })
}

/// `%v3 = add nsw i32 %v1, 2`, read as [`llvm_ir`] says.
fn llvm_arithmetic(result: &str, rest: &str) -> Option<Reading> {
    let (op, rest) = rest.split_once(' ')?;
    let (verb, noun) = match op {
        | "add" => ("add", "sum"),
        | "sub" => ("subtract", "difference"),
        | "mul" => ("multiply", "product"),
        | _ => return None,
    };
    let (left, right) = rest.strip_prefix("nsw i32 ")?.split_once(", ")?;
    let operand = |name: &str| {
        if is_llvm_local(name) {
            Some((name.to_owned(), Value))
        } else {
            let digits = name.strip_prefix('-').unwrap_or(name);
            is_number(digits).then(|| (name.to_owned(), Number))
        }
    };
    let (left_code, right_code) = (operand(left)?, operand(right)?);
    if !is_llvm_local(result) {
        return None;
    }
    let reads = match op {
        | "add" => format!("Add {left} and {right} as 32-bit integers, and call the sum {result}."),
        | "sub" => format!(
            "Subtract {right} from {left} as 32-bit integers, and call the difference {result}."
        ),
        | _ =>
            format!("Multiply {left} by {right} as 32-bit integers, and call the product {result}."),
    };
    Some(Reading {
        rank:    1,
        example: Example {
            parts: vec![
                part(
                    vec![(result, Value), (" =", Qualifier)],
                    "the result: % marks a local name, assigned only here",
                ),
                part(
                    vec![(op, Instruction)],
                    format!("the operation: integer {verb}"),
                ),
                part(
                    vec![("nsw", Qualifier)],
                    format!(
                        "no signed wrap: the result is poison if the {noun} overflows, so LLVM \
                         may assume it fits"
                    ),
                ),
                part(vec![("i32", Type)], "the type: a 32-bit integer, as int is"),
                part(
                    vec![
                        left_code,
                        (",".to_owned(), Qualifier),
                        (format!(" {}", right_code.0), right_code.1),
                    ],
                    "the operands: values made earlier, or constants",
                ),
            ],
            reads,
        },
    })
}

/// `%v1`
fn is_llvm_local(name: &str) -> bool {
    name.strip_prefix("%v").is_some_and(is_number)
}

/// x86-64 assembly in AT&T syntax: `movl %eax, (%rdi)`.
pub fn assembly(line: &str) -> Option<Reading> {
    let instruction = AsmLine::parse(line)?;
    Some(Reading {
        rank:    instruction.rank(),
        example: Example {
            parts: instruction.parts(),
            reads: instruction.reads(),
        },
    })
}

/// The machine-code listing: `5  89 07   movl %eax, (%rdi)`, an offset, the
/// bytes, and the instruction they encode, for the encodings the reader can
/// take apart: an ALU or `mov` opcode whose ModRM byte names a register
/// source and a register or memory destination (SDM Vol. 2, Table 2-2).
pub fn machine_code(line: &str) -> Option<Reading> {
    let (offset, rest) = line.trim_start().split_once("  ")?;
    let (bytes, text) = rest.trim_start().split_once("  ")?;
    if offset.is_empty() || !offset.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = bytes
        .split(' ')
        .map(|byte| {
            (byte.len() == 2)
                .then(|| u8::from_str_radix(byte, 16).ok())
                .flatten()
        })
        .collect::<Option<Vec<u8>>>()?;
    let instruction = AsmLine::parse(text)?;
    let mut parts = vec![part(
        vec![(offset, Muted)],
        "the offset from the function's first byte, in hexadecimal",
    )];
    parts.extend(instruction.encoding(&bytes)?);
    let count = bytes.len();
    parts.push(part(
        instruction.code(" "),
        format!("the assembly the {} bytes encode", number_word(count)),
    ));
    let spelled: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut word = number_word(count).to_owned();
    word[..1].make_ascii_uppercase();
    Some(Reading {
        rank:    instruction.rank(),
        example: Example {
            parts,
            reads: format!(
                "{word} bytes, {}, are the whole of the assembly view's {}.",
                spelled.join(" "),
                instruction.text()
            ),
        },
    })
}

/// The WebAssembly text format: `i32.load offset=4 align=4`, a load or a
/// store (WebAssembly 2.0, §6.5.6 for the text, §4.4.7 for what it does).
pub fn wasm_text(line: &str) -> Option<Reading> {
    let mut fields = line.split_whitespace();
    let (ty, op) = fields.next()?.split_once('.')?;
    let noun = match ty {
        | "i32" => "32-bit integer",
        | "i64" => "64-bit integer",
        | "f32" => "32-bit floating-point number",
        | "f64" => "64-bit floating-point number",
        | _ => return None,
    };
    let load = match op {
        | "load" => true,
        | "store" => false,
        | _ => return None,
    };
    let (mut offset, mut align) = (None, None);
    for field in fields {
        match field.split_once('=') {
            | Some(("offset", value)) if offset.is_none() && align.is_none() && is_number(value) =>
                offset = Some(value),
            | Some(("align", value)) if align.is_none() && is_number(value) => align = Some(value),
            | _ => return None,
        }
    }
    let instruction = if load {
        "the instruction: pop an address off the operand stack, read memory there and push the \
         value"
    } else {
        "the instruction: pop a value, then an address, off the operand stack, and write the value \
         to memory there"
    };
    let mut parts = vec![
        part(vec![(ty, Type)], format!("the type: {}", article(noun))),
        joined(vec![(".", Qualifier), (op, Instruction)], instruction),
    ];
    if let Some(offset) = offset {
        parts.push(part(
            vec![("offset", Qualifier), ("=", Qualifier), (offset, Number)],
            format!("added to the address: the memory {offset} bytes past it"),
        ));
    }
    if let Some(align) = align {
        parts.push(part(
            vec![("align", Qualifier), ("=", Qualifier), (align, Number)],
            format!(
                "a hint that the address is a multiple of {align}; it changes nothing the \
                 instruction does"
            ),
        ));
    }
    let place = offset.map_or_else(
        || "at that address".to_owned(),
        |offset| format!("{offset} bytes past that address"),
    );
    let reads = if load {
        format!("Pop an address, read the {noun} {place}, and push it.")
    } else {
        format!("Pop a {noun} and then an address, and write the {noun} {place}.")
    };
    let rank = match (load, offset.is_some()) {
        | (true, true) => 0,
        | (true, false) => 1,
        | (false, true) => 2,
        | (false, false) => 3,
    };
    Some(Reading {
        rank,
        example: Example { parts, reads },
    })
}

/// An x86-64 instruction of the shape the figure reads: an integer `mov` or
/// two-operand ALU instruction with a size suffix, a register or constant
/// source and a register or memory destination.
struct AsmLine<'a> {
    /// The mnemonic without its suffix: `mov`, `add`.
    base:        &'a str,
    /// 64 bits (`q`) rather than 32 (`l`).
    wide:        bool,
    source:      Operand<'a>,
    destination: Operand<'a>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operand<'a> {
    /// `%eax`, without the `%`.
    Register(&'a str),
    /// `$1`, without the `$`.
    Constant(&'a str),
    /// `-8(%rsp)` or `(%rdi)`: a displacement from the address a register
    /// holds.
    Memory {
        displacement: i64,
        written:      &'a str,
        base:         &'a str,
    },
}

impl<'a> Operand<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        if let Some(name) = text.strip_prefix('%') {
            return is_register(name).then_some(Self::Register(name));
        }
        if let Some(value) = text.strip_prefix('$') {
            let digits = value.strip_prefix('-').unwrap_or(value);
            return is_number(digits).then_some(Self::Constant(value));
        }
        let (written, rest) = text.split_once("(%")?;
        let base = rest.strip_suffix(')')?;
        let displacement = if written.is_empty() {
            0
        } else {
            written.parse().ok()?
        };
        is_register(base).then_some(Self::Memory {
            displacement,
            written,
            base,
        })
    }

    /// The operand as the view colours it.
    fn code(self) -> Vec<(String, Ink)> {
        match self {
            | Self::Register(name) => vec![(format!("%{name}"), Register)],
            | Self::Constant(value) => vec![(format!("${value}"), Number)],
            | Self::Memory { written, base, .. } => {
                let mut code = Vec::new();
                if !written.is_empty() {
                    code.push((written.to_owned(), Number));
                }
                code.extend([
                    ("(".to_owned(), Qualifier),
                    (format!("%{base}"), Register),
                    (")".to_owned(), Qualifier),
                ]);
                code
            },
        }
    }

    /// Where a memory operand is: "the address rdi holds", "8 bytes past
    /// the address rsp holds".
    fn address(displacement: i64, base: &str) -> String {
        match displacement {
            | 0 => format!("the address {base} holds"),
            | 1.. => format!(
                "{} past the address {base} holds",
                bytes(displacement.unsigned_abs())
            ),
            | _ => format!(
                "{} before the address {base} holds",
                bytes(displacement.unsigned_abs())
            ),
        }
    }
}

impl<'a> AsmLine<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let (mnemonic, operands) = text.trim().split_once([' ', '\t'])?;
        let (source, destination) = operands.trim().split_once(", ")?;
        let (base, wide) = if let Some(base) = mnemonic.strip_suffix('q') {
            (base, true)
        } else {
            (mnemonic.strip_suffix('l')?, false)
        };
        if !matches!(base, "mov" | "add" | "sub" | "imul" | "and" | "or" | "xor") {
            return None;
        }
        let source = Operand::parse(source)?;
        let destination = Operand::parse(destination)?;
        let fits = |name: &str| register_bits(name) == Some(if wide { 64 } else { 32 });
        let sized = match (source, destination) {
            | (Operand::Register(source), Operand::Register(destination)) =>
                fits(source) && fits(destination),
            | (Operand::Register(source), Operand::Memory { base, .. }) =>
                fits(source) && register_bits(base) == Some(64),
            | (Operand::Constant(_), Operand::Register(destination)) => fits(destination),
            | (Operand::Constant(_), Operand::Memory { base, .. }) =>
                register_bits(base) == Some(64),
            | _ => false,
        };
        sized.then_some(Self {
            base,
            wide,
            source,
            destination,
        })
    }

    /// How instructive a line is: a store first, as it shows the memory
    /// operand, then arithmetic, then a copy between registers, and last the
    /// prologue's `movq %rsp, %rbp`, which every function has.
    fn rank(&self) -> u8 {
        match (self.source, self.destination) {
            | (Operand::Register(_), Operand::Memory { .. }) => 0,
            | (_, Operand::Memory { .. }) => 1,
            | _ if self.base != "mov" => 2,
            | (Operand::Register("rsp"), Operand::Register("rbp")) => 4,
            | _ => 3,
        }
    }

    const fn bits(&self) -> u32 {
        if self.wide { 64 } else { 32 }
    }

    fn mnemonic(&self) -> String {
        format!("{}{}", self.base, if self.wide { "q" } else { "l" })
    }

    /// The instruction as the listing writes it.
    fn text(&self) -> String {
        self.code(" ").into_iter().map(|(text, _)| text).collect()
    }

    /// The instruction in its colours, its operands after `gap`.
    fn code(&self, gap: &str) -> Vec<(String, Ink)> {
        let mut code = vec![(self.mnemonic(), Instruction)];
        let mut source = self.source.code();
        source[0].0.insert_str(0, gap);
        code.extend(source);
        code.push((", ".to_owned(), Qualifier));
        code.extend(self.destination.code());
        merged(code)
    }

    /// The assembly line in parts.
    fn parts(&self) -> Vec<Part> {
        let verb = match self.base {
            | "mov" => "copy",
            | "add" => "add",
            | "sub" => "subtract",
            | "imul" => "multiply",
            | "and" => "bitwise and",
            | "or" => "bitwise or",
            | _ => "bitwise exclusive or",
        };
        let size = if self.wide {
            "the size: q for quadword, 64 bits"
        } else {
            "the size: l for long, 32 bits"
        };
        let source = match self.source {
            | Operand::Register(name) => format!("the source: register {name}"),
            | Operand::Constant(value) =>
                format!("the source: the constant {value}, written into the instruction"),
            | Operand::Memory { .. } => unreachable!("a source is never memory here"),
        };
        let mut source_code = self.source.code();
        source_code.push((",".to_owned(), Qualifier));
        vec![
            part(
                vec![(self.base, Instruction)],
                format!("the instruction: {verb}"),
            ),
            joined(vec![(if self.wide { "q" } else { "l" }, Instruction)], size),
            Part {
                code:   source_code,
                gloss:  source,
                joined: false,
            },
            Part {
                code:   self.destination.code(),
                gloss:  format!("the destination, last: {}", self.destination_words()),
                joined: false,
            },
        ]
    }

    /// "register ecx", "the memory at the address in register rdi".
    fn destination_words(&self) -> String {
        match self.destination {
            | Operand::Register(name) => format!("register {name}"),
            | Operand::Memory {
                displacement, base, ..
            } => match displacement {
                | 0 => format!("the memory at the address in register {base}"),
                | 1.. => format!(
                    "the memory {} past the address in register {base}",
                    bytes(displacement.unsigned_abs())
                ),
                | _ => format!(
                    "the memory {} before the address in register {base}",
                    bytes(displacement.unsigned_abs())
                ),
            },
            | Operand::Constant(_) => unreachable!("a destination is never a constant"),
        }
    }

    /// What the instruction does, in a sentence.
    fn reads(&self) -> String {
        let bits = self.bits();
        let source = match self.source {
            | Operand::Register(name) => name.to_owned(),
            | Operand::Constant(value) => value.to_owned(),
            | Operand::Memory { .. } => unreachable!("a source is never memory here"),
        };
        let (destination, there) = match self.destination {
            | Operand::Register(name) => (name.to_owned(), name.to_owned()),
            | Operand::Memory {
                displacement, base, ..
            } => (
                format!(
                    "the {bits} bits at {}",
                    Operand::address(displacement, base)
                ),
                "the same place".to_owned(),
            ),
            | Operand::Constant(_) => unreachable!("a destination is never a constant"),
        };
        match (self.base, self.source, self.destination) {
            | (
                "mov",
                Operand::Register(_),
                Operand::Memory {
                    displacement, base, ..
                },
            ) => format!(
                "Store the {bits} bits in {source} at {}.",
                Operand::address(displacement, base)
            ),
            | (
                "mov",
                Operand::Constant(_),
                Operand::Memory {
                    displacement, base, ..
                },
            ) => format!(
                "Store {source} as {bits} bits at {}.",
                Operand::address(displacement, base)
            ),
            | ("mov", Operand::Register(_), _) =>
                format!("Copy the {bits} bits in {source} into {destination}."),
            | ("mov", ..) => format!("Put {source} in {destination}, {bits} bits wide."),
            | ("add", ..) =>
                format!("Add {source} to {destination}, {bits} bits wide; the sum goes in {there}."),
            | ("sub", ..) => format!(
                "Subtract {source} from {destination}, {bits} bits wide; the difference goes in \
                 {there}."
            ),
            | ("imul", ..) => format!(
                "Multiply {destination} by {source}, {bits} bits wide; the product goes in \
                 {there}."
            ),
            | (base, ..) => format!(
                "Take the bitwise {} of {destination} and {source}, {bits} bits wide; the result \
                 goes in {there}.",
                match base {
                    | "and" => "and",
                    | "or" => "or",
                    | _ => "exclusive or",
                }
            ),
        }
    }

    /// `bytes` taken apart as the encoding of this instruction, or `None`
    /// if they are not one the reader knows: an optional REX.W prefix
    /// (`48`), one of the `op r/m, reg` opcodes, and a ModRM byte whose reg
    /// field names the source register, with the SIB byte and one-byte
    /// displacement its r/m and mod fields call for (SDM Vol. 2, §2.1 and
    /// Tables 2-2 and 2-3).
    fn encoding(&self, bytes: &[u8]) -> Option<Vec<Part>> {
        let Operand::Register(source) = self.source else {
            return None;
        };
        let mut parts = Vec::new();
        let mut rest = bytes;
        if self.wide {
            let (&0x48, after) = rest.split_first()? else {
                return None;
            };
            parts.push(byte_part(
                0x48,
                "the REX.W prefix: the operation is 64 bits wide",
            ));
            rest = after;
        }
        let (&opcode, after) = rest.split_first()?;
        let (base, does) = match opcode {
            | 0x89 => ("mov", "copy a register into a register or memory"),
            | 0x01 => ("add", "add a register to a register or memory"),
            | 0x29 => ("sub", "subtract a register from a register or memory"),
            | 0x21 => ("and", "bitwise and a register into a register or memory"),
            | 0x09 => ("or", "bitwise or a register into a register or memory"),
            | 0x31 => (
                "xor",
                "bitwise exclusive or a register into a register or memory",
            ),
            | _ => return None,
        };
        if base != self.base {
            return None;
        }
        let does = if self.wide {
            format!("the opcode: {does}; the prefix makes it 64 bits")
        } else {
            format!(
                "the opcode: {}",
                does.replacen("a register", "a 32-bit register", 1)
            )
        };
        parts.push(byte_part(opcode, does));
        let (&modrm, mut rest) = after.split_first()?;
        let (mode, reg, rm) = (modrm >> 6, (modrm >> 3) & 7, modrm & 7);
        if register_name(reg, self.wide) != source {
            return None;
        }
        let modrm_gloss = match (mode, self.destination) {
            | (0b11, Operand::Register(destination))
                if register_name(rm, self.wide) == destination =>
                format!(
                    "the ModRM byte: register {source} is the source, register {destination} the \
                     destination"
                ),
            | (
                0b00 | 0b01,
                Operand::Memory {
                    displacement, base, ..
                },
            ) => {
                let sib = rm == 0b100;
                if rm == 0b101 && mode == 0b00 || register_name(rm, true) != base && !sib {
                    return None;
                }
                let place = if sib {
                    "memory whose address a SIB byte gives"
                } else {
                    &format!("the memory at the address in {base}")
                };
                let plus = if mode == 0b01 {
                    ", plus a one-byte displacement"
                } else {
                    ""
                };
                let gloss = format!(
                    "the ModRM byte: register {source} is the source, {place}{plus} the \
                     destination"
                );
                parts.push(byte_part(modrm, gloss));
                if sib {
                    // SIB 0x24: scale 1, no index (100), base rsp (100).
                    let (&0x24, after) = rest.split_first()? else {
                        return None;
                    };
                    if base != "rsp" {
                        return None;
                    }
                    parts.push(byte_part(
                        0x24,
                        "the SIB byte: the address is in rsp, with no index",
                    ));
                    rest = after;
                }
                if mode == 0b01 {
                    let (&byte, after) = rest.split_first()?;
                    let value = i64::from(byte.cast_signed());
                    if value != displacement {
                        return None;
                    }
                    parts.push(byte_part(
                        byte,
                        format!("the displacement: {byte:02x} is {value} as a signed byte"),
                    ));
                    rest = after;
                } else if displacement != 0 {
                    return None;
                }
                return rest.is_empty().then_some(parts);
            },
            | _ => return None,
        };
        parts.push(byte_part(modrm, modrm_gloss));
        rest.is_empty().then_some(parts)
    }
}

/// The name of register `number` (0-7) in a ModRM field, at 64 or 32
/// bits (SDM Vol. 2, Table 2-2; no REX.R or REX.B).
fn register_name(number: u8, wide: bool) -> &'static str {
    const WIDE: [&str; 8] = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi"];
    const NARROW: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];
    (if wide { WIDE } else { NARROW })[usize::from(number & 7)]
}

/// The width of a general-purpose register, by its name.
fn register_bits(name: &str) -> Option<u32> {
    let legacy = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];
    if name.len() == 3 && legacy.contains(&&name[1..]) {
        return match &name[..1] {
            | "r" => Some(64),
            | "e" => Some(32),
            | _ => None,
        };
    }
    let number = name.strip_prefix('r')?;
    let (digits, bits) = if let Some(digits) = number.strip_suffix('d') {
        (digits, 32)
    } else {
        (number, 64)
    };
    matches!(digits, "8" | "9" | "10" | "11" | "12" | "13" | "14" | "15").then_some(bits)
}

fn is_register(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// "8 bytes", "1 byte".
fn bytes(count: impl Into<u64>) -> String {
    let count = count.into();
    if count == 1 {
        "1 byte".to_owned()
    } else {
        format!("{count} bytes")
    }
}

/// "a 32-bit integer", "an 8-bit integer".
fn article(noun: &str) -> String {
    if noun.starts_with('8') {
        format!("an {noun}")
    } else {
        format!("a {noun}")
    }
}

fn number_word(count: usize) -> &'static str {
    [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
    ]
    .get(count)
    .copied()
    .unwrap_or("many")
}

/// Neighbouring pieces of one colour as one.
fn merged(code: Vec<(String, Ink)>) -> Vec<(String, Ink)> {
    let mut out: Vec<(String, Ink)> = Vec::new();
    for (text, ink) in code {
        match out.last_mut() {
            | Some((last, last_ink)) if *last_ink == ink => last.push_str(&text),
            | _ => out.push((text, ink)),
        }
    }
    out
}

fn byte_part(byte: u8, gloss: impl Into<String>) -> Part {
    part(vec![(&format!("{byte:02x}"), Bytes)], gloss)
}

fn part<S: AsRef<str>>(code: Vec<(S, Ink)>, gloss: impl Into<String>) -> Part {
    Part {
        code:   code
            .into_iter()
            .map(|(text, ink)| (text.as_ref().to_owned(), ink))
            .collect(),
        gloss:  gloss.into(),
        joined: false,
    }
}

/// A part written against the one before it, as `.i32` follows `iadd`.
fn joined<S: AsRef<str>>(code: Vec<(S, Ink)>, gloss: impl Into<String>) -> Part {
    Part {
        joined: true,
        ..part(code, gloss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spelled(example: &Example) -> String {
        let mut line = String::new();
        for (index, part) in example.parts.iter().enumerate() {
            if index > 0 && !part.joined {
                line.push(' ');
            }
            for (text, _) in &part.code {
                line.push_str(text);
            }
        }
        line
    }

    #[test]
    fn bcc_ir_reads_signed_int_arithmetic_only() {
        let reading = bcc_ir("    v5 = isub.i32 nsw v0, v4").unwrap();
        assert_eq!(spelled(&reading.example), "v5 = isub.i32 nsw v0, v4");
        assert_eq!(
            reading.example.reads,
            "Subtract v4 from v0 as 32-bit integers, and call the difference v5."
        );
        assert!(
            bcc_ir("v17 = imul.i64 nsw v15, v16").is_none(),
            "not an int"
        );
        assert!(bcc_ir("v14 = iadd.i32 v11, v12").is_none(), "no nsw");
    }

    #[test]
    fn llvm_ir_reads_loads() {
        let reading = llvm_ir("  %v5 = load i8, ptr %v4, align 1").unwrap();
        assert_eq!(spelled(&reading.example), "%v5 = load i8, ptr %v4, align 1");
        assert_eq!(
            reading.example.reads,
            "Read the 8-bit integer at the address in %v4, and call it %v5."
        );
        assert_eq!(
            reading.example.parts[2].gloss,
            "the type it reads: an 8-bit integer"
        );
        assert!(llvm_ir("  %v5 = load volatile i8, ptr %v4, align 1").is_none());
        let add = llvm_ir("  %v7 = add nsw i32 %v4, 3").unwrap();
        assert_eq!(spelled(&add.example), "%v7 = add nsw i32 %v4, 3");
        assert_eq!(add.rank, 1);
        assert_eq!(
            add.example.reads,
            "Add %v4 and 3 as 32-bit integers, and call the sum %v7."
        );
        assert!(llvm_ir("  %v14 = add i32 %v11, %v12").is_none(), "no nsw");
    }

    #[test]
    fn assembly_reads_stores_and_arithmetic() {
        let store = assembly("\tmovl\t%ecx, -32(%rsp)").unwrap();
        assert_eq!(
            spelled(&store.example),
            "mov l %ecx, -32(%rsp)".replacen(" l", "l", 1)
        );
        assert_eq!(store.rank, 0);
        assert_eq!(
            store.example.reads,
            "Store the 32 bits in ecx at 32 bytes before the address rsp holds."
        );
        assert_eq!(
            store.example.parts[3].gloss,
            "the destination, last: the memory 32 bytes before the address in register rsp"
        );
        let add = assembly("\taddl\t$1, %eax").unwrap();
        assert_eq!(add.rank, 2);
        assert_eq!(
            add.example.reads,
            "Add 1 to eax, 32 bits wide; the sum goes in eax."
        );
        assert_eq!(assembly("\tmovq\t%rsp, %rbp").unwrap().rank, 4);
        assert!(assembly("\tleaq\t(%rdx,%rax,1), %r8").is_none());
        assert!(assembly("\tmovl\t%eax, %rcx").is_none(), "sizes differ");
        assert!(assembly("\tmovsd\t%xmm0, (%rdi)").is_none());
    }

    #[test]
    fn machine_code_takes_the_bytes_apart() {
        let line = "    1a  89 04 24                 movl %eax, 0(%rsp)";
        let reading = machine_code(line).unwrap();
        assert_eq!(spelled(&reading.example), "1a 89 04 24 movl %eax, 0(%rsp)");
        let glosses: Vec<&str> = reading
            .example
            .parts
            .iter()
            .map(|part| part.gloss.as_str())
            .collect();
        assert_eq!(
            glosses[1],
            "the opcode: copy a 32-bit register into a register or memory"
        );
        assert_eq!(
            glosses[2],
            "the ModRM byte: register eax is the source, memory whose address a SIB byte gives \
             the destination"
        );
        assert_eq!(
            glosses[3],
            "the SIB byte: the address is in rsp, with no index"
        );
        assert_eq!(
            reading.example.reads,
            "Three bytes, 89 04 24, are the whole of the assembly view's movl %eax, 0(%rsp)."
        );
        let wide = machine_code("     9  48 89 7c 24 e0           movq %rdi, -32(%rsp)").unwrap();
        let glosses: Vec<&str> = wide
            .example
            .parts
            .iter()
            .map(|part| part.gloss.as_str())
            .collect();
        assert_eq!(
            glosses[1],
            "the REX.W prefix: the operation is 64 bits wide"
        );
        assert_eq!(glosses[5], "the displacement: e0 is -32 as a signed byte");
        let add = machine_code("    47  01 f2                    addl %esi, %edx").unwrap();
        assert_eq!(
            add.example.parts[2].gloss,
            "the ModRM byte: register esi is the source, register edx the destination"
        );
        // Bytes that do not encode the instruction, and encodings the
        // reader does not know, are declined.
        assert!(machine_code("     9  89 0f                    movl %eax, (%rdi)").is_none());
        assert!(machine_code("    36  41 ff d5                 callq *%r13").is_none());
        assert!(machine_code("    34  (12 bytes of padding)").is_none());
    }

    #[test]
    fn wasm_text_reads_loads_and_stores() {
        let load = wasm_text("    i32.load offset=12 align=4").unwrap();
        assert_eq!(spelled(&load.example), "i32.load offset=12 align=4");
        assert_eq!(
            load.example.reads,
            "Pop an address, read the 32-bit integer 12 bytes past that address, and push it."
        );
        assert_eq!(load.rank, 0);
        let store = wasm_text("    i32.store").unwrap();
        assert_eq!(store.rank, 3);
        assert_eq!(
            store.example.reads,
            "Pop a 32-bit integer and then an address, and write the 32-bit integer at that \
             address."
        );
        assert!(wasm_text("    i32.load8_u align=1").is_none());
        assert!(wasm_text("    i32.add").is_none());
    }
}
