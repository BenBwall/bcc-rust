//! The text format (WebAssembly 2.0 §6), printed from the decoded module
//! one instruction per line, so stepping highlights exactly one line.
//!
//! The listing is linear rather than folded: `block`, `loop`, `if`, `else`
//! and `end` stand on their own lines and indent what they enclose. Each
//! label is numbered by its nesting (`;; label = @2`) and each branch
//! names the label it reaches (`br 1 (;@1;)`), as `wasm2wat` writes them.
//! Names come from the name section. [`Wat::lines`] records which
//! instruction each line shows.

use std::fmt::Write;

use crate::{
    instr::{
        Instr,
        MemArg,
        numeric_name,
        trunc_sat_name,
    },
    module::{
        BlockType,
        ConstExpr,
        ExternKind,
        FuncKind,
        FuncType,
        ImportKind,
        Limits,
        Module,
        SegmentMode,
        ValType,
    },
    value::Value,
};

/// Data segments longer than this are cut short in the listing.
const DATA_PREVIEW: usize = 256;

/// A module's listing and, for each of its lines, what it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wat {
    pub text:  String,
    /// One entry per line of `text`.
    pub lines: Vec<WatLine>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatLine {
    /// The instruction on this line: its function, index and module offset.
    pub instr: Option<(u32, u32, u32)>,
    /// The function whose header or body this line is part of.
    pub func:  Option<u32>,
}

impl Wat {
    /// The 1-based line showing instruction `pc` of `func`.
    pub fn line_of(&self, func: u32, pc: u32) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| matches!(line.instr, Some((f, p, _)) if f == func && p == pc))
            .map(|index| index + 1)
    }

    /// The 1-based line of a function's header.
    pub fn header_of(&self, func: u32) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.func == Some(func))
            .map(|index| index + 1)
    }
}

/// Print `module`.
pub fn print(module: &Module) -> Wat {
    let mut printer = Printer {
        module,
        wat: Wat::default(),
        func: 0,
    };
    printer.module();
    printer.wat
}

struct Printer<'m> {
    module: &'m Module,
    wat:    Wat,
    /// The function being printed, whose locals instructions name.
    func:   u32,
}

impl Printer<'_> {
    fn line(&mut self, text: &str, line: WatLine) {
        self.wat.text.push_str(text);
        self.wat.text.push('\n');
        self.wat.lines.push(line);
    }

    fn plain(&mut self, text: &str) {
        self.line(text, WatLine::default());
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One block per section, in the format's order."
    )]
    fn module(&mut self) {
        let module = self.module;
        match &module.names.module {
            | Some(name) => self.plain(&format!("(module ${name}")),
            | None => self.plain("(module"),
        }
        for (index, ty) in module.types.iter().enumerate() {
            self.plain(&format!("  (type (;{index};) (func{}))", signature(ty)));
        }
        let mut func_index = 0_u32;
        let mut global_index = 0_u32;
        for import in &module.imports {
            let description = match &import.kind {
                | ImportKind::Func(ty) => {
                    let text = format!("(func {} (type {ty}))", self.func_id(func_index));
                    func_index += 1;
                    text
                },
                | ImportKind::Memory(limits) => format!("(memory (;0;) {})", limits_text(*limits)),
                | ImportKind::Table(ty) => format!(
                    "(table {} {})",
                    limits_text(ty.limits),
                    ref_type(ty.element)
                ),
                | ImportKind::Global(ty) => {
                    let text = format!(
                        "(global {} {})",
                        self.global_id(global_index),
                        global_type(ty.value, ty.mutable)
                    );
                    global_index += 1;
                    text
                },
            };
            self.plain(&format!(
                "  (import \"{}\" \"{}\" {description})",
                escape(import.module.as_bytes()),
                escape(import.name.as_bytes())
            ));
        }
        for index in 0..module.funcs.len() as u32 {
            if matches!(module.funcs[index as usize].kind, FuncKind::Defined(_)) {
                self.function(index);
            }
        }
        for table in &module.tables {
            self.plain(&format!(
                "  (table (;0;) {} {})",
                limits_text(table.limits),
                ref_type(table.element)
            ));
        }
        for limits in &module.memories {
            self.plain(&format!("  (memory (;0;) {})", limits_text(*limits)));
        }
        for global in &module.globals {
            let text = format!(
                "  (global {} {} ({}))",
                self.global_id(global_index),
                global_type(global.ty.value, global.ty.mutable),
                self.const_expr(&global.init)
            );
            global_index += 1;
            self.plain(&text);
        }
        for export in &module.exports {
            let target = match export.kind {
                | ExternKind::Func => format!("func {}", self.func_ref(export.index)),
                | ExternKind::Table => format!("table {}", export.index),
                | ExternKind::Memory => format!("memory {}", export.index),
                | ExternKind::Global => format!("global {}", self.global_ref(export.index)),
            };
            self.plain(&format!(
                "  (export \"{}\" ({target}))",
                escape(export.name.as_bytes())
            ));
        }
        if let Some(start) = module.start {
            let text = format!("  (start {})", self.func_ref(start));
            self.plain(&text);
        }
        for (index, element) in module.elements.iter().enumerate() {
            // `func 0 1` when every item names a function, otherwise the
            // element type and each item's expression.
            let functions: Option<Vec<String>> = (element.ty == ValType::FuncRef)
                .then(|| {
                    element
                        .items
                        .iter()
                        .map(|item| match item.code.as_slice() {
                            | [Instr::RefFunc(func)] | [Instr::RefFunc(func), Instr::End] =>
                                Some(self.func_ref(*func)),
                            | _ => None,
                        })
                        .collect()
                })
                .flatten();
            let list = match functions {
                | Some(functions) => format!("func {}", functions.join(" ")),
                | None => {
                    let items: Vec<String> = element
                        .items
                        .iter()
                        .map(|item| format!("({})", self.const_expr(item)))
                        .collect();
                    format!("{} {}", ref_type(element.ty), items.join(" "))
                },
            };
            let mode = match &element.mode {
                | SegmentMode::Active { index: 0, offset } =>
                    format!(" ({})", self.const_expr(offset)),
                | SegmentMode::Active { index, offset } =>
                    format!(" (table {index}) ({})", self.const_expr(offset)),
                | SegmentMode::Passive => String::new(),
                | SegmentMode::Declarative => " declare".to_owned(),
            };
            self.plain(&format!("  (elem (;{index};){mode} {})", list.trim_end()));
        }
        for (index, data) in module.data.iter().enumerate() {
            let name = module
                .names
                .data(index as u32)
                .map_or_else(|| format!("(;{index};)"), |name| format!("${name}"));
            let mode = match &data.mode {
                | SegmentMode::Active { offset, .. } => format!(" ({})", self.const_expr(offset)),
                | _ => String::new(),
            };
            let shown = &data.bytes[..data.bytes.len().min(DATA_PREVIEW)];
            let hidden = data.bytes.len() - shown.len();
            let tail = if hidden > 0 {
                format!(" ;; and {hidden} more bytes")
            } else {
                String::new()
            };
            self.plain(&format!(
                "  (data {name}{mode} \"{}\"){tail}",
                escape(shown)
            ));
        }
        for custom in &module.customs {
            self.plain(&format!(
                "  ;; custom section \"{}\", {} bytes",
                escape(custom.name.as_bytes()),
                custom.bytes.len()
            ));
        }
        self.plain(")");
    }

    fn function(&mut self, func: u32) {
        let module = self.module;
        let FuncKind::Defined(body) = &module.funcs[func as usize].kind else {
            return;
        };
        self.func = func;
        let ty = module.func_type(func).cloned().unwrap_or_default();
        let header = WatLine {
            instr: None,
            func:  Some(func),
        };
        let mut text = format!(
            "  (func {} (type {})",
            self.func_id(func),
            module.funcs[func as usize].ty
        );
        for (index, param) in ty.params.iter().enumerate() {
            match module.names.local(func, index as u32) {
                | Some(name) => write!(text, " (param ${name} {})", param.name()),
                | None => write!(text, " (param {})", param.name()),
            }
            .ok();
        }
        if !ty.results.is_empty() {
            let results: Vec<&str> = ty.results.iter().map(|ty| ty.name()).collect();
            write!(text, " (result {})", results.join(" ")).ok();
        }
        self.line(&text, header);
        if !body.locals.is_empty() {
            let first = ty.params.len() as u32;
            let named = (0..body.locals.len() as u32)
                .any(|index| module.names.local(func, first + index).is_some());
            let text = if named {
                body.locals
                    .iter()
                    .enumerate()
                    .map(
                        |(index, ty)| match module.names.local(func, first + index as u32) {
                            | Some(name) => format!("(local ${name} {})", ty.name()),
                            | None => format!("(local {})", ty.name()),
                        },
                    )
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                let types: Vec<&str> = body.locals.iter().map(|ty| ty.name()).collect();
                format!("(local {})", types.join(" "))
            };
            self.line(&format!("    {text}"), header);
        }
        // The nesting depth of each open label, for `@N` names.
        let mut open: Vec<usize> = Vec::new();
        let last = body.code.len().saturating_sub(1);
        for (pc, instr) in body.code.iter().enumerate() {
            let line = WatLine {
                instr: Some((func, pc as u32, body.offsets[pc])),
                func:  Some(func),
            };
            if pc == last {
                // The body's own `end` closes the function.
                self.line("  )", line);
                break;
            }
            let depth = match instr {
                | Instr::End | Instr::Else { .. } => open.len().saturating_sub(1),
                | _ => open.len(),
            };
            let indent = "  ".repeat(depth + 2);
            let text = match instr {
                | Instr::Block { ty, .. } | Instr::Loop { ty, .. } | Instr::If { ty, .. } => {
                    open.push(open.len() + 1);
                    let name = match instr {
                        | Instr::Block { .. } => "block",
                        | Instr::Loop { .. } => "loop",
                        | _ => "if",
                    };
                    format!(
                        "{name}{}  ;; label = @{}",
                        Self::block_type(*ty),
                        open.len()
                    )
                },
                | Instr::End => {
                    open.pop();
                    "end".to_owned()
                },
                | _ => self.instruction(instr, &open),
            };
            self.line(&format!("{indent}{text}"), line);
        }
    }

    /// One instruction, given the open labels' `@N` names.
    fn instruction(&self, instr: &Instr, open: &[usize]) -> String {
        let label = |depth: u32| {
            open.len()
                .checked_sub(1 + depth as usize)
                .map_or_else(String::new, |index| format!(" (;@{};)", open[index]))
        };
        match instr {
            | Instr::Unreachable => "unreachable".into(),
            | Instr::Nop => "nop".into(),
            | Instr::Else { .. } => "else".into(),
            | Instr::Br(depth) => format!("br {depth}{}", label(*depth)),
            | Instr::BrIf(depth) => format!("br_if {depth}{}", label(*depth)),
            | Instr::BrTable { targets, default } => {
                let mut text = "br_table".to_owned();
                for depth in targets.iter().chain([default]) {
                    write!(text, " {depth}{}", label(*depth)).ok();
                }
                text
            },
            | Instr::Return => "return".into(),
            | Instr::Call(func) => format!("call {}", self.func_ref(*func)),
            | Instr::CallIndirect { ty, table } =>
                if *table == 0 {
                    format!("call_indirect (type {ty})")
                } else {
                    format!("call_indirect {table} (type {ty})")
                },
            | Instr::Drop => "drop".into(),
            | Instr::Select(None) => "select".into(),
            | Instr::Select(Some(ty)) => format!("select (result {})", ty.name()),
            | Instr::LocalGet(index) => format!("local.get {}", self.local_ref(*index)),
            | Instr::LocalSet(index) => format!("local.set {}", self.local_ref(*index)),
            | Instr::LocalTee(index) => format!("local.tee {}", self.local_ref(*index)),
            | Instr::GlobalGet(index) => format!("global.get {}", self.global_ref(*index)),
            | Instr::GlobalSet(index) => format!("global.set {}", self.global_ref(*index)),
            | Instr::TableGet(table) => format!("table.get {table}"),
            | Instr::TableSet(table) => format!("table.set {table}"),
            | Instr::Load(load, memarg) =>
                format!("{}{}", load.name(), memarg_text(*memarg, load.size())),
            | Instr::Store(store, memarg) =>
                format!("{}{}", store.name(), memarg_text(*memarg, store.size())),
            | Instr::MemorySize => "memory.size".into(),
            | Instr::MemoryGrow => "memory.grow".into(),
            | Instr::MemoryInit(segment) => format!("memory.init {segment}"),
            | Instr::DataDrop(segment) => format!("data.drop {segment}"),
            | Instr::MemoryCopy => "memory.copy".into(),
            | Instr::MemoryFill => "memory.fill".into(),
            | Instr::TableInit { elem, table } => format!("table.init {table} {elem}"),
            | Instr::ElemDrop(elem) => format!("elem.drop {elem}"),
            | Instr::TableCopy { dst, src } => format!("table.copy {dst} {src}"),
            | Instr::TableGrow(table) => format!("table.grow {table}"),
            | Instr::TableSize(table) => format!("table.size {table}"),
            | Instr::TableFill(table) => format!("table.fill {table}"),
            | Instr::I32Const(value) => format!("i32.const {value}"),
            | Instr::I64Const(value) => format!("i64.const {value}"),
            | Instr::F32Const(bits) => format!("f32.const {}", Value::F32(*bits)),
            | Instr::F64Const(bits) => format!("f64.const {}", Value::F64(*bits)),
            | Instr::RefNull(ty) => format!(
                "ref.null {}",
                if *ty == ValType::FuncRef {
                    "func"
                } else {
                    "extern"
                }
            ),
            | Instr::RefIsNull => "ref.is_null".into(),
            | Instr::RefFunc(func) => format!("ref.func {}", self.func_ref(*func)),
            | Instr::Numeric(op) => numeric_name(*op).into(),
            | Instr::TruncSat(op) => trunc_sat_name(*op).into(),
            | Instr::Block { .. } | Instr::Loop { .. } | Instr::If { .. } | Instr::End =>
                String::new(),
        }
    }

    fn block_type(ty: BlockType) -> String {
        match ty {
            | BlockType::Empty => String::new(),
            | BlockType::Value(value) => format!(" (result {})", value.name()),
            | BlockType::Type(index) => format!(" (type {index})"),
        }
    }

    fn const_expr(&self, expr: &ConstExpr) -> String {
        let parts: Vec<String> = expr
            .code
            .iter()
            .filter(|instr| **instr != Instr::End)
            .map(|instr| self.instruction(instr, &[]))
            .collect();
        parts.join(") (")
    }

    /// A function's identifier where it is declared: `$name`, or its index
    /// as a comment, since a bare number cannot declare one.
    fn func_id(&self, func: u32) -> String {
        match self.module.names.function(func) {
            | Some(name) => format!("${}", identifier(name)),
            | None => format!("(;{func};)"),
        }
    }

    fn global_id(&self, global: u32) -> String {
        match self.module.global_name(global) {
            | Some(name) => format!("${}", identifier(name)),
            | None => format!("(;{global};)"),
        }
    }

    fn func_ref(&self, func: u32) -> String {
        match self.module.names.function(func) {
            | Some(name) => format!("${}", identifier(name)),
            | None => func.to_string(),
        }
    }

    fn global_ref(&self, global: u32) -> String {
        match self.module.global_name(global) {
            | Some(name) => format!("${}", identifier(name)),
            | None => global.to_string(),
        }
    }

    fn local_ref(&self, local: u32) -> String {
        match self.module.names.local(self.func, local) {
            | Some(name) => format!("${}", identifier(name)),
            | None => local.to_string(),
        }
    }
}

/// ` offset=N` and ` align=N` where they differ from the defaults.
fn memarg_text(memarg: MemArg, size: u32) -> String {
    let mut text = String::new();
    if memarg.offset != 0 {
        write!(text, " offset={}", memarg.offset).ok();
    }
    let natural = size.trailing_zeros();
    if memarg.align != natural {
        write!(text, " align={}", 1_u64 << memarg.align.min(63)).ok();
    }
    text
}

fn signature(ty: &FuncType) -> String {
    let mut text = String::new();
    if !ty.params.is_empty() {
        let params: Vec<&str> = ty.params.iter().map(|ty| ty.name()).collect();
        write!(text, " (param {})", params.join(" ")).ok();
    }
    if !ty.results.is_empty() {
        let results: Vec<&str> = ty.results.iter().map(|ty| ty.name()).collect();
        write!(text, " (result {})", results.join(" ")).ok();
    }
    text
}

fn limits_text(limits: Limits) -> String {
    match limits.max {
        | Some(max) => format!("{} {max}", limits.min),
        | None => limits.min.to_string(),
    }
}

fn ref_type(ty: ValType) -> &'static str {
    if ty == ValType::FuncRef {
        "funcref"
    } else {
        "externref"
    }
}

fn global_type(ty: ValType, mutable: bool) -> String {
    if mutable {
        format!("(mut {})", ty.name())
    } else {
        ty.name().to_owned()
    }
}

/// A name as a text-format identifier: characters an identifier cannot
/// hold become `_`.
fn identifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "!#$%&'*+-./:<=>?@\\^_`|~".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Bytes as a text-format string's contents.
fn escape(bytes: &[u8]) -> String {
    let mut text = String::new();
    for &byte in bytes {
        match byte {
            | b'"' => text.push_str("\\\""),
            | b'\\' => text.push_str("\\\\"),
            | b'\n' => text.push_str("\\n"),
            | b'\t' => text.push_str("\\t"),
            | 0x20..=0x7E => text.push(char::from(byte)),
            | _ => {
                write!(text, "\\{byte:02x}").ok();
            },
        }
    }
    text
}
