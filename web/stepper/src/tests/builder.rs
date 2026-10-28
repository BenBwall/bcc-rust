//! Test modules: an encoder for the binary format and an assembler for
//! flat instruction text, so a test reads like the listing it checks:
//!
//! ```text
//! let mut module = Builder::new();
//! module.func("add", &[I32, I32], &[I32], &[], "local.get 0 local.get 1 i32.add");
//! ```
//!
//! Instructions are whitespace-separated mnemonics, each followed by its
//! immediates: `block`, `loop` and `if` take an optional result type or
//! `type=N`; memory instructions take `offset=N` and `align=N`; `call`
//! takes a function index or a name given to [`Builder::func`].

use crate::{
    instr::{
        Load,
        Store,
        numeric_name,
        trunc_sat_name,
    },
    module::ValType,
};

#[derive(Default)]
pub struct Builder {
    types:        Vec<(Vec<ValType>, Vec<ValType>)>,
    imports:      Vec<(String, String, u32)>,
    funcs:        Vec<(u32, Vec<ValType>, String)>,
    names:        Vec<String>,
    memory:       Option<(u32, Option<u32>)>,
    table:        Option<u32>,
    globals:      Vec<(ValType, bool, String)>,
    exports:      Vec<(String, u8, u32)>,
    elements:     Vec<(u32, Vec<u32>)>,
    data:         Vec<(Option<u32>, Vec<u8>)>,
    start:        Option<u32>,
    global_names: Vec<(u32, String)>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ty(&mut self, params: &[ValType], results: &[ValType]) -> u32 {
        let entry = (params.to_vec(), results.to_vec());
        if let Some(index) = self.types.iter().position(|ty| *ty == entry) {
            return index as u32;
        }
        self.types.push(entry);
        self.types.len() as u32 - 1
    }

    /// Import a function; imports must come before defined functions.
    pub fn import(
        &mut self,
        module: &str,
        name: &str,
        params: &[ValType],
        results: &[ValType],
    ) -> u32 {
        assert!(self.funcs.is_empty(), "imports come first");
        let ty = self.ty(params, results);
        self.imports.push((module.into(), name.into(), ty));
        self.names.push(name.into());
        self.imports.len() as u32 - 1
    }

    /// Define and export a function; returns its index.
    pub fn func(
        &mut self,
        name: &str,
        params: &[ValType],
        results: &[ValType],
        locals: &[ValType],
        code: &str,
    ) -> u32 {
        let ty = self.ty(params, results);
        let index = (self.imports.len() + self.funcs.len()) as u32;
        self.funcs.push((ty, locals.to_vec(), code.into()));
        self.names.push(name.into());
        self.exports.push((name.into(), 0, index));
        index
    }

    pub fn memory(&mut self, min: u32, max: Option<u32>) -> &mut Self {
        self.memory = Some((min, max));
        self
    }

    pub fn table(&mut self, size: u32) -> &mut Self {
        self.table = Some(size);
        self
    }

    /// A global with an initializer in instruction text; returns its index.
    pub fn global(&mut self, name: &str, ty: ValType, mutable: bool, init: &str) -> u32 {
        self.globals.push((ty, mutable, init.into()));
        let index = self.globals.len() as u32 - 1;
        self.global_names.push((index, name.into()));
        index
    }

    pub fn elements(&mut self, offset: u32, funcs: &[u32]) -> &mut Self {
        self.elements.push((offset, funcs.to_vec()));
        self
    }

    /// An active data segment at `offset`, or a passive one.
    pub fn data(&mut self, offset: Option<u32>, bytes: &[u8]) -> &mut Self {
        self.data.push((offset, bytes.to_vec()));
        self
    }

    pub fn start(&mut self, func: u32) -> &mut Self {
        self.start = Some(func);
        self
    }

    #[expect(clippy::too_many_lines, reason = "One block per section.")]
    pub fn build(&self) -> Vec<u8> {
        let mut out = b"\0asm\x01\0\0\0".to_vec();
        section(
            &mut out,
            1,
            &vector(&self.types, |bytes, (params, results)| {
                bytes.push(0x60);
                bytes.extend(vector(params, |bytes, ty| bytes.push(code(*ty))));
                bytes.extend(vector(results, |bytes, ty| bytes.push(code(*ty))));
            }),
        );
        if !self.imports.is_empty() {
            section(
                &mut out,
                2,
                &vector(&self.imports, |bytes, (module, name, ty)| {
                    name_bytes(bytes, module);
                    name_bytes(bytes, name);
                    bytes.push(0);
                    leb(bytes, u64::from(*ty));
                }),
            );
        }
        section(
            &mut out,
            3,
            &vector(&self.funcs, |bytes, (ty, ..)| {
                leb(bytes, u64::from(*ty));
            }),
        );
        if let Some(size) = self.table {
            section(
                &mut out,
                4,
                &vector(&[size], |bytes, size| {
                    bytes.extend([0x70, 0x00]);
                    leb(bytes, u64::from(*size));
                }),
            );
        }
        if let Some((min, max)) = self.memory {
            let mut bytes = vec![1];
            match max {
                | Some(max) => {
                    bytes.push(1);
                    leb(&mut bytes, u64::from(min));
                    leb(&mut bytes, u64::from(max));
                },
                | None => {
                    bytes.push(0);
                    leb(&mut bytes, u64::from(min));
                },
            }
            section(&mut out, 5, &bytes);
        }
        if !self.globals.is_empty() {
            section(
                &mut out,
                6,
                &vector(&self.globals, |bytes, (ty, mutable, init)| {
                    bytes.push(code(*ty));
                    bytes.push(u8::from(*mutable));
                    bytes.extend(self.assemble(init));
                    bytes.push(0x0B);
                }),
            );
        }
        section(
            &mut out,
            7,
            &vector(&self.exports, |bytes, (name, kind, index)| {
                name_bytes(bytes, name);
                bytes.push(*kind);
                leb(bytes, u64::from(*index));
            }),
        );
        if let Some(start) = self.start {
            let mut bytes = Vec::new();
            leb(&mut bytes, u64::from(start));
            section(&mut out, 8, &bytes);
        }
        if !self.elements.is_empty() {
            section(
                &mut out,
                9,
                &vector(&self.elements, |bytes, (offset, funcs)| {
                    bytes.push(0);
                    bytes.push(0x41);
                    sleb(bytes, i64::from(*offset));
                    bytes.push(0x0B);
                    bytes.extend(vector(funcs, |bytes, func| leb(bytes, u64::from(*func))));
                }),
            );
        }
        if !self.data.is_empty() {
            let mut count = Vec::new();
            leb(&mut count, self.data.len() as u64);
            section(&mut out, 12, &count);
        }
        section(
            &mut out,
            10,
            &vector(&self.funcs, |bytes, (_, locals, text)| {
                let mut body = vector(locals, |bytes, ty| {
                    bytes.push(1);
                    bytes.push(code(*ty));
                });
                body.extend(self.assemble(text));
                body.push(0x0B);
                leb(bytes, body.len() as u64);
                bytes.extend(body);
            }),
        );
        if !self.data.is_empty() {
            section(
                &mut out,
                11,
                &vector(&self.data, |bytes, (offset, data)| {
                    match offset {
                        | Some(offset) => {
                            bytes.extend([0, 0x41]);
                            sleb(bytes, i64::from(*offset));
                            bytes.push(0x0B);
                        },
                        | None => bytes.push(1),
                    }
                    leb(bytes, data.len() as u64);
                    bytes.extend(data);
                }),
            );
        }
        // The name section: functions (1) and globals (7).
        let mut names = Vec::new();
        let functions: Vec<(u32, &String)> = self
            .names
            .iter()
            .enumerate()
            .map(|(index, name)| (index as u32, name))
            .collect();
        subsection(
            &mut names,
            1,
            &vector(&functions, |bytes, (index, name)| {
                leb(bytes, u64::from(*index));
                name_bytes(bytes, name);
            }),
        );
        subsection(
            &mut names,
            7,
            &vector(&self.global_names, |bytes, (index, name)| {
                leb(bytes, u64::from(*index));
                name_bytes(bytes, name);
            }),
        );
        let mut custom = Vec::new();
        name_bytes(&mut custom, "name");
        custom.extend(names);
        section(&mut out, 0, &custom);
        out
    }

    /// Assemble instruction text into code bytes.
    #[expect(clippy::too_many_lines, reason = "One arm per mnemonic.")]
    pub fn assemble(&self, text: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut words = text.split_whitespace().peekable();
        while let Some(word) = words.next() {
            let mut immediate = || {
                words
                    .next()
                    .unwrap_or_else(|| panic!("{word} needs an immediate"))
            };
            match word {
                | "unreachable" => bytes.push(0x00),
                | "nop" => bytes.push(0x01),
                | "block" | "loop" | "if" => {
                    bytes.push(match word {
                        | "block" => 0x02,
                        | "loop" => 0x03,
                        | _ => 0x04,
                    });
                    match words.peek().copied() {
                        | Some(ty @ ("i32" | "i64" | "f32" | "f64")) => {
                            words.next();
                            bytes.push(code(value_type(ty)));
                        },
                        | Some(ty) if ty.starts_with("type=") => {
                            words.next();
                            sleb(&mut bytes, ty[5..].parse().unwrap());
                        },
                        | _ => bytes.push(0x40),
                    }
                },
                | "else" => bytes.push(0x05),
                | "end" => bytes.push(0x0B),
                | "br" | "br_if" => {
                    bytes.push(if word == "br" { 0x0C } else { 0x0D });
                    leb(&mut bytes, immediate().parse().unwrap());
                },
                | "br_table" => {
                    // `br_table 0 1 2` : the last is the default.
                    let mut targets = Vec::new();
                    while let Some(target) = words.peek().and_then(|word| word.parse::<u64>().ok())
                    {
                        targets.push(target);
                        words.next();
                    }
                    let default = targets.pop().expect("a default label");
                    bytes.push(0x0E);
                    leb(&mut bytes, targets.len() as u64);
                    for target in targets {
                        leb(&mut bytes, target);
                    }
                    leb(&mut bytes, default);
                },
                | "return" => bytes.push(0x0F),
                | "call" => {
                    bytes.push(0x10);
                    let target = immediate();
                    let index = target.parse().unwrap_or_else(|_| {
                        self.names
                            .iter()
                            .position(|name| name == target)
                            .unwrap_or_else(|| panic!("no function {target}"))
                            as u64
                    });
                    leb(&mut bytes, index);
                },
                | "call_indirect" => {
                    bytes.push(0x11);
                    leb(&mut bytes, immediate().parse().unwrap());
                    bytes.push(0);
                },
                | "drop" => bytes.push(0x1A),
                | "select" => bytes.push(0x1B),
                | "local.get" | "local.set" | "local.tee" | "global.get" | "global.set" => {
                    bytes.push(match word {
                        | "local.get" => 0x20,
                        | "local.set" => 0x21,
                        | "local.tee" => 0x22,
                        | "global.get" => 0x23,
                        | _ => 0x24,
                    });
                    leb(&mut bytes, immediate().parse().unwrap());
                },
                | "table.get" => {
                    bytes.push(0x25);
                    bytes.push(0);
                },
                | "table.set" => {
                    bytes.push(0x26);
                    bytes.push(0);
                },
                | "memory.size" => bytes.extend([0x3F, 0]),
                | "memory.grow" => bytes.extend([0x40, 0]),
                | "memory.copy" => bytes.extend([0xFC, 10, 0, 0]),
                | "memory.fill" => bytes.extend([0xFC, 11, 0]),
                | "memory.init" => {
                    bytes.extend([0xFC, 8]);
                    leb(&mut bytes, immediate().parse().unwrap());
                    bytes.push(0);
                },
                | "data.drop" => {
                    bytes.extend([0xFC, 9]);
                    leb(&mut bytes, immediate().parse().unwrap());
                },
                | "table.size" => bytes.extend([0xFC, 16, 0]),
                | "table.grow" => bytes.extend([0xFC, 15, 0]),
                | "table.fill" => bytes.extend([0xFC, 17, 0]),
                | "i32.const" => {
                    bytes.push(0x41);
                    sleb(&mut bytes, i64::from(parse_i32(immediate())));
                },
                | "i64.const" => {
                    bytes.push(0x42);
                    sleb(&mut bytes, parse_i64(immediate()));
                },
                | "f32.const" => {
                    bytes.push(0x43);
                    bytes.extend(parse_f32(immediate()).to_le_bytes());
                },
                | "f64.const" => {
                    bytes.push(0x44);
                    bytes.extend(parse_f64(immediate()).to_le_bytes());
                },
                | "ref.null" => {
                    bytes.push(0xD0);
                    bytes.push(if immediate() == "func" { 0x70 } else { 0x6F });
                },
                | "ref.is_null" => bytes.push(0xD1),
                | "ref.func" => {
                    bytes.push(0xD2);
                    leb(&mut bytes, immediate().parse().unwrap());
                },
                | _ =>
                    if let Some(index) = Load::ALL.iter().position(|load| load.name() == word) {
                        bytes.push(0x28 + index as u8);
                        memarg(&mut bytes, &mut words, Load::ALL[index].size());
                    } else if let Some(index) =
                        Store::ALL.iter().position(|store| store.name() == word)
                    {
                        bytes.push(0x36 + index as u8);
                        memarg(&mut bytes, &mut words, Store::ALL[index].size());
                    } else if let Some(op) = (0x45..=0xC4).find(|&op| numeric_name(op) == word) {
                        bytes.push(op);
                    } else if let Some(op) = (0..8).find(|&op| trunc_sat_name(op) == word) {
                        bytes.extend([0xFC, op]);
                    } else {
                        panic!("unknown instruction {word}");
                    },
            }
        }
        bytes
    }
}

fn memarg<'a>(
    bytes: &mut Vec<u8>,
    words: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>,
    size: u32,
) {
    let mut align = size.trailing_zeros();
    let mut offset = 0;
    while let Some(word) = words.peek().copied() {
        if let Some(value) = word.strip_prefix("offset=") {
            offset = value.parse().unwrap();
        } else if let Some(value) = word.strip_prefix("align=") {
            align = value.parse::<u32>().unwrap().trailing_zeros();
        } else {
            break;
        }
        words.next();
    }
    leb(bytes, u64::from(align));
    leb(bytes, offset);
}

fn parse_i32(text: &str) -> i32 {
    text.strip_prefix("0x").map_or_else(
        || text.parse::<i64>().unwrap() as i32,
        |hex| u32::from_str_radix(hex, 16).unwrap() as i32,
    )
}

fn parse_i64(text: &str) -> i64 {
    text.strip_prefix("0x").map_or_else(
        || text.parse::<i128>().unwrap() as i64,
        |hex| u64::from_str_radix(hex, 16).unwrap() as i64,
    )
}

/// A float literal: decimal, `inf`, `-inf`, `nan`, or `bits:0x…`.
fn parse_f32(text: &str) -> u32 {
    if let Some(bits) = text.strip_prefix("bits:0x") {
        return u32::from_str_radix(bits, 16).unwrap();
    }
    match text {
        | "nan" => 0x7FC0_0000,
        | "inf" => f32::INFINITY.to_bits(),
        | "-inf" => f32::NEG_INFINITY.to_bits(),
        | _ => text.parse::<f32>().unwrap().to_bits(),
    }
}

fn parse_f64(text: &str) -> u64 {
    if let Some(bits) = text.strip_prefix("bits:0x") {
        return u64::from_str_radix(bits, 16).unwrap();
    }
    match text {
        | "nan" => 0x7FF8_0000_0000_0000,
        | "inf" => f64::INFINITY.to_bits(),
        | "-inf" => f64::NEG_INFINITY.to_bits(),
        | _ => text.parse::<f64>().unwrap().to_bits(),
    }
}

fn value_type(name: &str) -> ValType {
    match name {
        | "i32" => ValType::I32,
        | "i64" => ValType::I64,
        | "f32" => ValType::F32,
        | _ => ValType::F64,
    }
}

fn code(ty: ValType) -> u8 {
    match ty {
        | ValType::I32 => 0x7F,
        | ValType::I64 => 0x7E,
        | ValType::F32 => 0x7D,
        | ValType::F64 => 0x7C,
        | ValType::FuncRef => 0x70,
        | ValType::ExternRef => 0x6F,
    }
}

pub fn leb(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            bytes.push(byte);
            return;
        }
        bytes.push(byte | 0x80);
    }
}

pub fn sleb(bytes: &mut Vec<u8>, mut value: i64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        if done {
            bytes.push(byte);
            return;
        }
        bytes.push(byte | 0x80);
    }
}

fn name_bytes(bytes: &mut Vec<u8>, name: &str) {
    leb(bytes, name.len() as u64);
    bytes.extend(name.as_bytes());
}

fn vector<T>(items: &[T], mut encode: impl FnMut(&mut Vec<u8>, &T)) -> Vec<u8> {
    let mut bytes = Vec::new();
    leb(&mut bytes, items.len() as u64);
    for item in items {
        encode(&mut bytes, item);
    }
    bytes
}

pub fn section(out: &mut Vec<u8>, id: u8, contents: &[u8]) {
    out.push(id);
    leb(out, contents.len() as u64);
    out.extend(contents);
}

fn subsection(out: &mut Vec<u8>, id: u8, contents: &[u8]) {
    section(out, id, contents);
}
