//! What a failed check passes the run-time library: the static data, whose
//! layout GCC's and Clang's libraries share, and the operands, each encoded
//! as a `ValueHandle`. compiler-rt `lib/ubsan/ubsan_handlers.h` and
//! `ubsan_value.h` define both:
//!
//! ```text
//! SourceLocation { const char *file; u32 line; u32 column; }
//! TypeDescriptor { u16 kind; u16 info; char name[]; }   kind 0: integer,
//!                  info log2(bits) << 1 | signed; kind 1: floating, info
//!                  bits; 0xffff: other
//! OverflowData   { SourceLocation; TypeDescriptor *type; }  and so on
//! ```
//!
//! An integer or floating operand of at most 64 bits is passed in the
//! handle's bits, zero-extended; a wider one by its address; a pointer as
//! itself. The type descriptor says how the library prints it.

use super::{
    super::{
        FunctionLowerer,
        module_items::Unit,
        types::{
            self,
            Repr,
        },
    },
    Report,
};
use crate::{
    ir::{
        Align,
        FuncId,
        GlobalDecl,
        GlobalId,
        GlobalInit,
        InstData,
        InstFlags,
        Linkage,
        Opcode,
        Relocation,
        RelocationTarget,
        Signature,
        Symbol,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::declaration_syntax::TypeQualifiers,
        semantic_analysis::{
            ArrayBound,
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

/// A run-time library function that reports one kind of check.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::lowering) enum Handler {
    AddOverflow,
    SubOverflow,
    MulOverflow,
    NegateOverflow,
    DivremOverflow,
    ShiftOutOfBounds,
    OutOfBounds,
    TypeMismatch,
    BuiltinUnreachable,
    VlaBoundNotPositive,
    FloatCastOverflow,
    LoadInvalidValue,
    PointerOverflow,
    InvalidBuiltin,
    NonnullArg,
    NonnullReturn,
}

impl Handler {
    /// The name after `__ubsan_handle_`.
    const fn name(self) -> &'static str {
        match self {
            | Self::AddOverflow => "add_overflow",
            | Self::SubOverflow => "sub_overflow",
            | Self::MulOverflow => "mul_overflow",
            | Self::NegateOverflow => "negate_overflow",
            | Self::DivremOverflow => "divrem_overflow",
            | Self::ShiftOutOfBounds => "shift_out_of_bounds",
            | Self::OutOfBounds => "out_of_bounds",
            | Self::TypeMismatch => "type_mismatch_v1",
            | Self::BuiltinUnreachable => "builtin_unreachable",
            | Self::VlaBoundNotPositive => "vla_bound_not_positive",
            | Self::FloatCastOverflow => "float_cast_overflow",
            | Self::LoadInvalidValue => "load_invalid_value",
            | Self::PointerOverflow => "pointer_overflow",
            | Self::InvalidBuiltin => "invalid_builtin",
            | Self::NonnullArg => "nonnull_arg",
            | Self::NonnullReturn => "nonnull_return_v1",
        }
    }

    /// Whether the handler never returns, so that it has no `_abort` form.
    pub(in crate::lowering) const fn always_ends(self) -> bool {
        matches!(self, Self::BuiltinUnreachable)
    }

    /// The handler's operands, as a check builds them.
    pub(in crate::lowering) fn report<'s>(
        self,
        scratch: &'s crate::util::bump::Bump,
        fields: &[Field],
        arguments: &[Value],
    ) -> Report<'s> {
        Report {
            handler:   self,
            fields:    scratch.alloc_slice_copy(fields),
            arguments: scratch.alloc_slice_copy(arguments),
            site:      super::DataSite::Check,
        }
    }
}

/// A field of a check's static data after its source location.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::lowering) enum Field {
    /// A pointer to a type descriptor.
    Type(GlobalId),
    /// A byte: an alignment's logarithm, an access kind, a builtin's kind.
    Byte(u8),
    /// A 32-bit integer: an argument's position.
    Int(u32),
    /// Another source location, or an unknown one.
    Location(Option<SourceVectors>),
}

impl Unit<'_, '_, '_> {
    /// A fresh writable global holding the source location of `site` (zero
    /// for an unknown one), then `fields`, each at its natural alignment.
    pub(in crate::lowering) fn check_data(
        &mut self,
        site: Option<SourceVectors>,
        fields: &[Field],
    ) -> GlobalId {
        let mut bytes = ArenaVec::with_capacity_in(16 + 8 * fields.len(), self.tables);
        let mut relocations = ArenaVec::new_in(self.tables);
        self.push_location(site, &mut bytes, &mut relocations);
        for &field in fields {
            match field {
                | Field::Type(descriptor) => {
                    pad(&mut bytes, 8);
                    relocations.push(Relocation {
                        offset: bytes.len() as u64,
                        target: RelocationTarget::Symbol(Symbol::Global(descriptor)),
                        addend: 0,
                    });
                    bytes.extend_from_slice(&[0; 8]);
                },
                | Field::Byte(byte) => bytes.push(byte),
                | Field::Int(value) => {
                    pad(&mut bytes, 4);
                    bytes.extend_from_slice(&value.to_le_bytes());
                },
                | Field::Location(site) => {
                    pad(&mut bytes, 8);
                    self.push_location(site, &mut bytes, &mut relocations);
                },
            }
        }
        pad(&mut bytes, 8);
        let name = self.unique_name(".ubsan.data");
        let global = self.module.declare_global(
            name,
            GlobalDecl {
                linkage:      Linkage::Internal,
                size:         bytes.len() as u64,
                align:        Align::from_bytes(8).unwrap_or(Align::BYTE),
                constant:     false,
                thread_local: false,
            },
        );
        self.module.define_global(
            global,
            GlobalInit::Bytes {
                bytes:       &bytes,
                relocations: &relocations,
            },
        );
        self.note_generated_global(global);
        global
    }

    /// Appends a `SourceLocation` for `site`, whose file is named by a
    /// relocation, or zeros for an unknown one.
    fn push_location(
        &mut self,
        site: Option<SourceVectors>,
        bytes: &mut ArenaVec<'_, u8>,
        relocations: &mut ArenaVec<'_, Relocation>,
    ) {
        let Some(site) = site else {
            bytes.extend_from_slice(&[0; 16]);
            return;
        };
        let vector = self.context.file_vector(site);
        let (line, column, index) = (vector.line, vector.column, vector.source_file_index);
        let file = self.source_file_name(index);
        relocations.push(Relocation {
            offset: bytes.len() as u64,
            target: RelocationTarget::Symbol(Symbol::Global(file)),
            addend: 0,
        });
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&line.to_le_bytes());
        bytes.extend_from_slice(&column.to_le_bytes());
    }

    /// The name of the source file `index` as `__FILE__` gives it, which is
    /// what both compilers report.
    fn source_file_name(&mut self, index: u32) -> GlobalId {
        if let Some(&global) = self.sanitizer.files.get(&index) {
            return global;
        }
        let path = self.context.get_source_file(index).to_string_lossy();
        let mut bytes = ArenaVec::with_capacity_in(path.len() + 1, self.tables);
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        let global = self.internal_constant(".ubsan.file", &bytes, 1);
        _ = self.sanitizer.files.insert(index, global);
        global
    }

    /// The type descriptor of the kind `kind`, information `info` and quoted
    /// name `name`, shared by every check that names it.
    fn descriptor(&mut self, kind: u16, info: u16, name: &str) -> GlobalId {
        let name = &*self.tables.alloc_str(name);
        if let Some(&global) = self.sanitizer.descriptors.get(&(kind, info, name)) {
            return global;
        }
        let mut bytes = ArenaVec::with_capacity_in(name.len() + 7, self.tables);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&info.to_le_bytes());
        bytes.push(b'\'');
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'\'');
        bytes.push(0);
        let global = self.internal_constant(".ubsan.type", &bytes, 2);
        _ = self
            .sanitizer
            .descriptors
            .insert((kind, info, name), global);
        global
    }

    /// The handler for `handler` taking the data and `operands` handles, or
    /// its `_abort` form unless `recover`.
    pub(in crate::lowering) fn ub_handler(
        &mut self,
        handler: Handler,
        operands: usize,
        recover: bool,
    ) -> FuncId {
        let suffix = if recover || handler.always_ends() {
            ""
        } else {
            "_abort"
        };
        let name = crate::diagnostics::format_in!(
            self.tables,
            "__ubsan_handle_{}{suffix}",
            handler.name()
        );
        if let Some(&function) = self.sanitizer.handlers.get(name) {
            return function;
        }
        let mut params = ArenaVec::with_capacity_in(operands + 1, self.tables);
        params.push(Type::Ptr);
        params.extend(std::iter::repeat_n(Type::I64, operands));
        let signature = self.module.intern(Signature {
            params:           &params,
            aggregates:       &[],
            noundef:          &[],
            result:           None,
            variadic:         false,
            variadic_fixed:   None,
            extensions:       &[],
            result_extension: None,
            result_noundef:   false,
            variadic_noundef: false,
            chain:            false,
        });
        let function = match self.module.symbol(name) {
            | Some(Symbol::Function(function)) => function,
            // A unit that defines an object of the name keeps it, and the
            // report calls a handler of another name the library lacks.
            | Some(Symbol::Global(_)) => {
                let name = self.unique_name(name);
                self.module
                    .declare_function(name, signature, Linkage::External)
            },
            | None => self
                .module
                .declare_function(name, signature, Linkage::External),
        };
        _ = self.sanitizer.handlers.insert(name, function);
        function
    }
}

impl<'s> FunctionLowerer<'_, '_, '_, '_, 's> {
    /// The type descriptor of `ty`, as the compiler bcc answers as writes it:
    /// a scalar's kind and width, and its name as that compiler spells it in
    /// the report it is for.
    pub(in crate::lowering) fn type_descriptor(&mut self, ty: TypeId, style: Style) -> GlobalId {
        let gcc = self.gcc_reports();
        let (kind, info) = match types::repr(&self.unit.sema.types, ty) {
            | Ok(Repr::Int { ty, signed, .. }) => (
                0,
                u16::try_from(ty.bits().trailing_zeros() << 1).unwrap_or(0) | u16::from(signed),
            ),
            // GCC describes `_Bool` by its one bit of precision, Clang by its
            // eight bits of storage.
            | Ok(Repr::Bool) => (0, if gcc { 0 } else { 3 << 1 }),
            // GCC gives `long double` its 80 bits of precision, Clang its 128
            // bits of storage; the library reads either as `long double`.
            | Ok(Repr::Float { ty: Type::F80 }) => (1, if gcc { 80 } else { 128 }),
            | Ok(Repr::Float { ty }) => (1, u16::try_from(ty.bits()).unwrap_or(0)),
            | _ => (0xFFFF, 0),
        };
        let name = if gcc {
            self.gcc_type_name(ty, style)
        } else {
            self.type_name(ty, false)
        };
        self.unit.descriptor(kind, info, &name)
    }

    /// `value` as the library's `ValueHandle`: an integer or floating value
    /// of at most 64 bits in its bits, zero-extended, a pointer as its
    /// address, and anything wider by the address of a copy. Built where the
    /// check failed.
    pub(in crate::lowering) fn value_handle(&mut self, value: Value) -> Value {
        let ty = self.draft.value_type(value);
        match ty {
            | Type::Ptr => self.convert(Opcode::Ptrtoint, Type::I64, value),
            | Type::I64 => value,
            | Type::F32 => {
                let bits = self.convert(Opcode::Bitcast, Type::I32, value);
                self.extend(bits, false, Type::I64)
            },
            | Type::F64 => self.convert(Opcode::Bitcast, Type::I64, value),
            | _ if ty.is_int() && ty.bits() < 64 => self.extend(value, false, Type::I64),
            | _ => {
                let bytes = u64::from(ty.bits().div_ceil(8));
                let align = Align::from_bytes(16).unwrap_or(Align::BYTE);
                let slot = self.object_slot(u32::try_from(bytes).unwrap_or(16), align);
                let address = self.inst(InstData::StackAddr { slot }, Type::Ptr);
                _ = self.draft.push(
                    self.memory_op(InstData::Store {
                        ty,
                        flags: InstFlags::empty(),
                        align,
                        tag: None,
                        args: [value, address],
                    }),
                    None,
                );
                self.convert(Opcode::Ptrtoint, Type::I64, address)
            },
        }
    }

    /// `ty` as GCC (`gcc`) or Clang names it in a type descriptor. bcc keeps no
    /// typedef names, so a type is spelled from its canonical form where both
    /// compilers name a typedef an array's elements have. GCC leaves out the
    /// qualifiers, and the keyword of a tag; Clang writes `int[4]` where GCC
    /// writes `int [4]`.
    pub(in crate::lowering) fn type_name(&self, ty: TypeId, gcc: bool) -> ArenaString<'s> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Last {
            None,
            Pointer,
            Other,
        }
        let types = &self.unit.sema.types;
        let mut declarator = ArenaString::new_in(self.scratch);
        let mut last = Last::None;
        let mut ty = ty;
        let base = loop {
            match types.kind(ty) {
                | TypeKind::Pointer(next) => {
                    let mut text = ArenaString::new_in(self.scratch);
                    text.push('*');
                    if !gcc {
                        text.push_str(qualifier_words(ty.qualifiers));
                    }
                    text.push_str(&declarator);
                    declarator = text;
                    last = Last::Pointer;
                    ty = next;
                },
                | TypeKind::Array(next, bound) => {
                    if last == Last::Pointer {
                        declarator = parenthesized(self.scratch, &declarator);
                    }
                    match bound {
                        | ArrayBound::Constant(n) => declarator
                            .push_str(crate::diagnostics::format_in!(self.scratch, "[{n}]")),
                        | ArrayBound::Incomplete => declarator.push_str("[]"),
                        | ArrayBound::Variable(_) | ArrayBound::Star => declarator.push_str("[*]"),
                    }
                    last = Last::Other;
                    ty = next;
                },
                | TypeKind::Function {
                    result,
                    parameters,
                    variadic,
                    prototype,
                } => {
                    if last == Last::Pointer {
                        declarator = parenthesized(self.scratch, &declarator);
                    }
                    declarator.push('(');
                    for (index, &parameter) in parameters.iter().enumerate() {
                        if index != 0 {
                            declarator.push_str(", ");
                        }
                        declarator.push_str(&self.type_name(parameter, gcc));
                    }
                    if variadic {
                        declarator.push_str(if parameters.is_empty() {
                            "..."
                        } else {
                            ", ..."
                        });
                    } else if prototype && parameters.is_empty() {
                        declarator.push_str("void");
                    }
                    declarator.push(')');
                    last = Last::Other;
                    ty = result;
                },
                | TypeKind::Atomic(inner) => ty = inner,
                | kind => break kind,
            }
        };
        let mut text = ArenaString::new_in(self.scratch);
        if !gcc {
            let words = qualifier_words(ty.qualifiers).trim_start();
            if !words.is_empty() {
                text.push_str(words);
                text.push(' ');
            }
        }
        match base {
            | TypeKind::Scalar(scalar) => text.push_str(scalar_name(scalar, gcc)),
            | TypeKind::BitInt { width, signed } => {
                if !signed {
                    text.push_str("unsigned ");
                }
                text.push_str(crate::diagnostics::format_in!(
                    self.scratch,
                    "_BitInt({width})"
                ));
            },
            | TypeKind::Tag(id) => {
                let tag = types.tags[id];
                if !gcc {
                    text.push_str(match tag.kind {
                        | TagKind::Struct => "struct ",
                        | TagKind::Union => "union ",
                        | TagKind::Enum => "enum ",
                    });
                }
                match tag.name {
                    | Some(name) => text.push_str(self.unit.context.string_cache.at(name)),
                    | None => text.push_str(if gcc { "<anonymous>" } else { "(unnamed)" }),
                }
            },
            | TypeKind::Nullptr => text.push_str("nullptr_t"),
            | _ => text.push_str("<unknown>"),
        }
        if !declarator.is_empty() {
            if gcc || !declarator.starts_with('[') {
                text.push(' ');
            }
            text.push_str(&declarator);
        }
        text
    }
}

/// How GCC spells a type, which depends on the report (GCC's
/// `ubsan_print_style`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::lowering) enum Style {
    /// An operand's type: its name alone.
    Plain,
    /// The type of an object accessed through a pointer: with its
    /// qualifiers, a record's keyword and a star for each pointer.
    Pointer,
    /// An array's type: its element's name, stars and bounds.
    Array,
}

impl<'s> FunctionLowerer<'_, '_, '_, '_, 's> {
    /// `ty` as GCC names it in a report of `style`: the name of the type
    /// left once arrays and pointers are taken off, which for a record is
    /// its tag, then for `Pointer` a star for each pointer, after the
    /// qualifiers and the record's keyword, and for `Array` the stars and
    /// each bound, without either. GCC 13 `ubsan.cc`, `ubsan_type_descriptor`.
    fn gcc_type_name(&self, ty: TypeId, style: Style) -> ArenaString<'s> {
        let types = &self.unit.sema.types;
        let mut base = ty;
        let mut bounds = ArenaString::new_in(self.scratch);
        while let TypeKind::Array(element, bound) = types.kind(base) {
            match bound {
                | ArrayBound::Constant(n) =>
                    bounds.push_str(crate::diagnostics::format_in!(self.scratch, "[{n}]")),
                | _ => bounds.push_str("[*]"),
            }
            base = element;
        }
        let style = if bounds.is_empty() {
            style
        } else {
            Style::Array
        };
        let mut stars = 0;
        if style != Style::Plain {
            while let TypeKind::Pointer(target) = types.kind(base) {
                stars += 1;
                base = target;
            }
        }
        let mut text = ArenaString::new_in(self.scratch);
        let (name, keyword) = match types.kind(base) {
            | TypeKind::Scalar(scalar) => (scalar_name(scalar, true), ""),
            | TypeKind::Tag(id) => {
                let tag = types.tags[id];
                let keyword = match tag.kind {
                    | TagKind::Struct => "struct ",
                    | TagKind::Union => "union ",
                    | TagKind::Enum => "",
                };
                let name = tag
                    .name
                    .map_or("<unknown>", |name| self.unit.context.string_cache.at(name));
                (name, keyword)
            },
            | _ => ("<unknown>", ""),
        };
        if style == Style::Pointer {
            let qualifiers = base.qualifiers;
            for (qualifier, word) in [
                (TypeQualifiers::VOLATILE, "volatile "),
                (TypeQualifiers::CONST, "const "),
                (TypeQualifiers::RESTRICT, "restrict "),
            ] {
                if qualifiers.contains(qualifier) {
                    text.push_str(word);
                }
            }
            text.push_str(keyword);
        }
        text.push_str(name);
        if style == Style::Array || stars != 0 {
            text.push(' ');
        }
        for _ in 0..stars {
            text.push('*');
        }
        text.push_str(&bounds);
        text
    }
}

/// Pads `bytes` with zeros to a multiple of `align`.
fn pad(bytes: &mut ArenaVec<'_, u8>, align: usize) {
    while !bytes.len().is_multiple_of(align) {
        bytes.push(0);
    }
}

/// `text` in parentheses.
fn parenthesized<'s>(scratch: &'s crate::util::bump::Bump, text: &str) -> ArenaString<'s> {
    let mut grouped = ArenaString::new_in(scratch);
    grouped.push('(');
    grouped.push_str(text);
    grouped.push(')');
    grouped
}

/// A scalar type's keywords: GCC's, which spell out `int` and put `unsigned`
/// after the size, or Clang's.
fn scalar_name(scalar: crate::target::Scalar, gcc: bool) -> &'static str {
    use crate::target::Scalar as S;
    match (scalar, gcc) {
        | (S::Short, true) => "short int",
        | (S::UnsignedShort, true) => "short unsigned int",
        | (S::Long, true) => "long int",
        | (S::UnsignedLong, true) => "long unsigned int",
        | (S::LongLong, true) => "long long int",
        | (S::UnsignedLongLong, true) => "long long unsigned int",
        | (S::UnsignedInt128, true) => "__int128 unsigned",
        | (S::ComplexFloat, true) => "complex float",
        | (S::ComplexDouble, true) => "complex double",
        | (S::ComplexLongDouble, true) => "complex long double",
        | (S::ComplexFloat, false) => "_Complex float",
        | (S::ComplexDouble, false) => "_Complex double",
        | (S::ComplexLongDouble, false) => "_Complex long double",
        | _ => scalar.spelling(),
    }
}

/// The qualifier keywords, each after a space, in the order Clang writes them.
const fn qualifier_words(qualifiers: TypeQualifiers) -> &'static str {
    match qualifiers.bits() & 7 {
        | 0 => "",
        | 1 => "const",
        | 2 => "volatile",
        | 3 => "const volatile",
        | 4 => "restrict",
        | 5 => "const restrict",
        | 6 => "volatile restrict",
        | _ => "const volatile restrict",
    }
}
