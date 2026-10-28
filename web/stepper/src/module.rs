//! A decoded module: what `decode.rs` reads from the binary format and
//! `validate.rs` checks. Function bodies are decoded once into [`Instr`]
//! lists with each instruction's byte offset, so the interpreter steps over
//! an array index and the page can map that index back to the binary.

use crate::instr::Instr;

/// A WebAssembly module (WebAssembly 2.0 §2.5), with the custom sections the
/// stepper reads: names and DWARF line tables.
#[derive(Clone, Debug, Default)]
pub struct Module {
    pub types:     Vec<FuncType>,
    /// Imported functions come first in `funcs`, in import order; this keeps
    /// the other imports (which the stepper cannot satisfy) for reporting.
    pub imports:   Vec<Import>,
    pub funcs:     Vec<Func>,
    pub tables:    Vec<TableType>,
    pub memories:  Vec<Limits>,
    pub globals:   Vec<Global>,
    pub exports:   Vec<Export>,
    pub start:     Option<u32>,
    pub elements:  Vec<Element>,
    pub data:      Vec<Data>,
    pub names:     Names,
    /// The byte offset of the Code section's contents, which DWARF addresses
    /// are relative to.
    pub code_base: u32,
    /// Custom sections other than `name`, with their contents.
    pub customs:   Vec<Custom>,
}

impl Module {
    pub fn func_type(&self, func: u32) -> Option<&FuncType> {
        let func = self.funcs.get(func as usize)?;
        self.types.get(func.ty as usize)
    }

    /// The function's name from the name section, its export name, or
    /// `$<index>`.
    pub fn func_name(&self, func: u32) -> String {
        if let Some(name) = self.names.function(func) {
            return name.to_owned();
        }
        if let Some(Func {
            kind: FuncKind::Import { name, .. },
            ..
        }) = self.funcs.get(func as usize)
        {
            return name.clone();
        }
        self.exports
            .iter()
            .find(|export| export.kind == ExternKind::Func && export.index == func)
            .map_or_else(|| format!("${func}"), |export| export.name.clone())
    }

    pub fn global_name(&self, global: u32) -> Option<&str> {
        self.names.global(global).or_else(|| {
            self.exports
                .iter()
                .find(|export| export.kind == ExternKind::Global && export.index == global)
                .map(|export| export.name.as_str())
        })
    }

    /// The index of the exported item `name` of `kind`.
    pub fn export(&self, name: &str, kind: ExternKind) -> Option<u32> {
        self.exports
            .iter()
            .find(|export| export.kind == kind && export.name == name)
            .map(|export| export.index)
    }

    /// The body of a defined function.
    pub fn body(&self, func: u32) -> Option<&Body> {
        match &self.funcs.get(func as usize)?.kind {
            | FuncKind::Defined(body) => Some(body),
            | FuncKind::Import { .. } => None,
        }
    }

    pub fn custom(&self, name: &str) -> Option<&[u8]> {
        self.customs
            .iter()
            .find(|custom| custom.name == name)
            .map(|custom| custom.bytes.as_slice())
    }

    /// The defined function and instruction index at `offset`, the byte
    /// offset of an instruction's opcode.
    pub fn locate(&self, offset: u32) -> Option<(u32, u32)> {
        self.funcs.iter().enumerate().find_map(|(index, func)| {
            let FuncKind::Defined(body) = &func.kind else {
                return None;
            };
            let pc = body.offsets.binary_search(&offset).ok()?;
            Some((index as u32, pc as u32))
        })
    }
}

/// A value type (§2.3.4); `v128` is not supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    FuncRef,
    ExternRef,
}

impl ValType {
    pub fn name(self) -> &'static str {
        match self {
            | Self::I32 => "i32",
            | Self::I64 => "i64",
            | Self::F32 => "f32",
            | Self::F64 => "f64",
            | Self::FuncRef => "funcref",
            | Self::ExternRef => "externref",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, Self::I32 | Self::I64 | Self::F32 | Self::F64)
    }

    pub fn is_reference(self) -> bool {
        matches!(self, Self::FuncRef | Self::ExternRef)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuncType {
    pub params:  Vec<ValType>,
    pub results: Vec<ValType>,
}

/// A structured instruction's type (§2.4.8): no values, one result, or a
/// function type's parameters and results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockType {
    Empty,
    Value(ValType),
    Type(u32),
}

#[derive(Clone, Debug)]
pub struct Import {
    pub module: String,
    pub name:   String,
    pub kind:   ImportKind,
}

#[derive(Clone, Debug)]
pub enum ImportKind {
    Func(u32),
    Table(TableType),
    Memory(Limits),
    Global(GlobalType),
}

#[derive(Clone, Debug)]
pub struct Func {
    /// Its type's index.
    pub ty:   u32,
    pub kind: FuncKind,
}

#[derive(Clone, Debug)]
pub enum FuncKind {
    Import { module: String, name: String },
    Defined(Body),
}

/// A defined function's code: its locals after the parameters, and its
/// instructions with each one's byte offset in the module.
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub locals:  Vec<ValType>,
    pub code:    Vec<Instr>,
    pub offsets: Vec<u32>,
    /// Where the body (its size field) starts and its contents end.
    pub start:   u32,
    pub end:     u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub min: u32,
    pub max: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableType {
    pub element: ValType,
    pub limits:  Limits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalType {
    pub value:   ValType,
    pub mutable: bool,
}

#[derive(Clone, Debug)]
pub struct Global {
    pub ty:   GlobalType,
    pub init: ConstExpr,
}

/// A constant expression (§3.3.10): a short instruction sequence evaluated
/// once at instantiation. Extended constants (`i32.add` and friends) are
/// accepted.
#[derive(Clone, Debug, Default)]
pub struct ConstExpr {
    pub code: Vec<Instr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternKind {
    Func,
    Table,
    Memory,
    Global,
}

#[derive(Clone, Debug)]
pub struct Export {
    pub name:  String,
    pub kind:  ExternKind,
    pub index: u32,
}

#[derive(Clone, Debug)]
pub struct Element {
    pub ty:    ValType,
    /// Each element's initializer: `ref.func`, `ref.null` or `global.get`.
    pub items: Vec<ConstExpr>,
    pub mode:  SegmentMode,
}

#[derive(Clone, Debug)]
pub enum SegmentMode {
    Passive,
    Active { index: u32, offset: ConstExpr },
    Declarative,
}

#[derive(Clone, Debug)]
pub struct Data {
    pub bytes:  Vec<u8>,
    pub mode:   SegmentMode,
    /// Where the bytes start in the module.
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct Custom {
    pub name:  String,
    pub bytes: Vec<u8>,
}

/// The `name` custom section (§7.4): debug names for functions, locals,
/// globals and data segments.
#[derive(Clone, Debug, Default)]
pub struct Names {
    pub module:    Option<String>,
    pub functions: Vec<(u32, String)>,
    /// Per function: each local's index and name.
    pub locals:    Vec<(u32, Vec<(u32, String)>)>,
    pub labels:    Vec<(u32, Vec<(u32, String)>)>,
    pub globals:   Vec<(u32, String)>,
    pub data:      Vec<(u32, String)>,
}

impl Names {
    pub fn function(&self, func: u32) -> Option<&str> {
        find(&self.functions, func)
    }

    pub fn local(&self, func: u32, local: u32) -> Option<&str> {
        self.locals
            .iter()
            .find(|(index, _)| *index == func)
            .and_then(|(_, names)| find(names, local))
    }

    pub fn global(&self, global: u32) -> Option<&str> {
        find(&self.globals, global)
    }

    pub fn data(&self, segment: u32) -> Option<&str> {
        find(&self.data, segment)
    }
}

fn find(names: &[(u32, String)], index: u32) -> Option<&str> {
    names
        .iter()
        .find(|(known, _)| *known == index)
        .map(|(_, name)| name.as_str())
}
