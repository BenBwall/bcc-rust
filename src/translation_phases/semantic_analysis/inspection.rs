//! Deterministic non-recursive inspection of semantic declaration types.
//! C99: type-name spelling §6.7.6, p. 122; PDF p. 134; linkage and duration
//! §6.2.2-§6.2.4, pp. 30-32; PDF pp. 42-44. Inspection is
//! implementation-defined.
//!
//! The format follows `--syntax-tree`: one item per line, nested by scope
//! with two-space indentation, lowercase `key=value` fields and inapplicable
//! fields omitted. A value other than a plain word is quoted.

use std::fmt::Write as _;

use super::{
    ArenaVec,
    ArrayBound,
    BindingKind,
    Bump,
    Context,
    Duration,
    Linkage,
    ScopeKind,
    SemanticTranslationUnit,
    TagKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
    expressions::{
        ConstantClass,
        ConversionKind,
        ValueCategory,
    },
};
use crate::util::bump::ArenaString;

#[derive(Clone, Copy)]
enum Part<'tu> {
    Text(&'tu str),
    Type(TypeId),
    Function(&'tu [TypeId], bool, bool),
}

/// One inspected item of a scope, in discovery order within its table.
#[derive(Clone, Copy)]
enum Item {
    Tag(usize),
    Binding(usize),
}

/// Deeper nesting is shown by a `[depth=N]` prefix rather than more spaces,
/// as in syntax inspection, so output stays linear in the item count.
const MAXIMUM_INDENT: usize = 32;

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn inspect<'d>(&self, context: &Context<'tu>, arena: &'d Bump) -> &'d str {
        let mut out = ArenaString::new_in(arena);
        let target = self.types.target;
        let _ = writeln!(
            out,
            "target {} size_t={} ptrdiff_t={} wchar_t={} char={}",
            context.configuration.target().inspection_name(),
            value(arena, target.size_t.spelling()),
            value(arena, target.ptrdiff_t.spelling()),
            value(arena, target.wchar_t.spelling()),
            if target.char_signed {
                "signed"
            } else {
                "unsigned"
            }
        );
        let mut items = ArenaVec::new_in(arena);
        items.resize_with(self.scopes.len(), || ArenaVec::new_in(arena));
        // A tag's members are listed at its last declaration, which follows
        // any forward declaration of it.
        let mut last = ArenaVec::new_in(arena);
        last.resize(self.types.tags.len(), usize::MAX);
        for (index, &(tag, scope)) in self.tag_declarations.iter().enumerate() {
            items[scope].push(Item::Tag(index));
            last[tag] = index;
        }
        for (index, binding) in self.bindings.iter().enumerate() {
            items[binding.scope].push(Item::Binding(index));
        }
        let mut minimums = super::ArenaMap::with_hasher_in(super::FxBuildHasher, arena);
        for (_, parameters) in self.parameters {
            for parameter in *parameters {
                if let (Some(name), Some(ArrayBound::Constant(minimum))) =
                    (parameter.name, parameter.array_minimum)
                {
                    _ = minimums.insert(name.source_vectors, minimum);
                }
            }
        }
        let mut depths = ArenaVec::new_in(arena);
        for scope in self.scopes {
            let depth = scope.parent.map_or(0, |parent: usize| depths[parent] + 1);
            depths.push(depth);
        }
        for (index, scope) in self.scopes.iter().enumerate() {
            if scope.kind == ScopeKind::Prototype {
                continue;
            }
            let depth = depths[index];
            indent(&mut out, depth);
            let _ = writeln!(out, "scope {index} {}", scope_kind(scope.kind));
            for &item in &items[index] {
                match item {
                    | Item::Tag(declaration) => {
                        let (id, _) = self.tag_declarations[declaration];
                        self.write_tag(
                            id,
                            last[id] == declaration,
                            depth + 1,
                            context,
                            arena,
                            &mut out,
                        );
                    },
                    | Item::Binding(binding) => {
                        indent(&mut out, depth + 1);
                        self.write_binding(binding, &minimums, context, arena, &mut out);
                    },
                }
            }
        }
        for &(_, ty) in self.type_names {
            let _ = writeln!(
                out,
                "type-name type={}",
                self.type_value(ty, context, arena)
            );
        }
        let mut identities = super::ArenaMap::with_hasher_in(super::FxBuildHasher, arena);
        for (index, info) in self.expressions.iter().enumerate() {
            _ = identities.insert(std::ptr::from_ref(info.expression).addr(), index);
            let _ = write!(
                out,
                "expression {index} type={} category={}",
                self.type_value(info.ty, context, arena),
                match info.category {
                    | ValueCategory::Lvalue => "lvalue",
                    | ValueCategory::ModifiableLvalue => "modifiable-lvalue",
                    | ValueCategory::FunctionDesignator => "function-designator",
                    | ValueCategory::Rvalue => "rvalue",
                }
            );
            if let Some(binding) = info.binding {
                let _ = write!(
                    out,
                    " binding={}",
                    context.string_cache.at(self.bindings[binding].name.name)
                );
            }
            if let Some(ty) = info.operation_type {
                let _ = write!(out, " operation={}", self.type_value(ty, context, arena));
            }
            if let Some(width) = info.bit_field {
                let _ = write!(out, " bit-field={width}");
            }
            if let Some(value) = info.floating {
                let _ = write!(out, " real={} imag={}", value.real, value.imag);
            }
            if let Some(value) = info.integer {
                let _ = write!(out, " value={}", value.value);
            }
            if info.ice {
                out.push_str(" ice");
            }
            match info.constant {
                | ConstantClass::None => {},
                | ConstantClass::Arithmetic => out.push_str(" constant=arithmetic"),
                | ConstantClass::Address => out.push_str(" constant=address"),
            }
            out.push('\n');
        }
        for conversion in self.conversions {
            let index = identities
                .get(&std::ptr::from_ref(conversion.expression).addr())
                .copied()
                .unwrap_or(usize::MAX);
            let _ = writeln!(
                out,
                "convert expression {index} kind={} type={}",
                match conversion.kind {
                    | ConversionKind::Lvalue => "lvalue",
                    | ConversionKind::ArrayDecay => "array-decay",
                    | ConversionKind::FunctionDecay => "function-decay",
                    | ConversionKind::Arithmetic => "arithmetic",
                    | ConversionKind::Assignment => "assignment",
                    | ConversionKind::DefaultArgument => "default-argument",
                },
                self.type_value(conversion.ty, context, arena)
            );
        }
        out.into_str()
    }

    fn write_tag<'d>(
        &self,
        id: usize,
        members: bool,
        depth: usize,
        context: &Context<'tu>,
        arena: &'d Bump,
        out: &mut ArenaString<'d>,
    ) {
        let tag = self.types.tags[id];
        indent(out, depth);
        let _ = write!(
            out,
            "tag {} {} {}",
            tag_kind(tag.kind),
            tag.name
                .map_or("<anonymous>", |n| context.string_cache.at(n)),
            if tag.complete.get() {
                "complete"
            } else {
                "incomplete"
            }
        );
        if tag.tainted.get() {
            out.push_str(" unanalyzed");
        }
        if let Some(layout) = tag.layout.get() {
            let _ = write!(out, " size={} align={}", layout.size, layout.align);
        }
        if tag.kind == TagKind::Enum && tag.complete.get() && !tag.tainted.get() {
            let _ = write!(
                out,
                " compatible={}",
                value(arena, tag.compatible.get().spelling())
            );
        }
        out.push('\n');
        if !members || tag.kind == TagKind::Enum {
            return;
        }
        for member in tag.members.get() {
            indent(out, depth + 1);
            let _ = write!(
                out,
                "member {} type={} offset={}",
                member.name.map_or(
                    if member.anonymous {
                        "<anonymous>"
                    } else {
                        "<unnamed>"
                    },
                    |n| context.string_cache.at(n.name)
                ),
                self.type_value(member.ty, context, arena),
                member.offset
            );
            if let Some(width) = member.width {
                let _ = write!(out, " bit-offset={} width={width}", member.bit_offset);
            }
            out.push('\n');
        }
        // Names that anonymous members contribute, with offsets from this
        // record's start.
        for field in tag.fields.get().iter().filter(|f| f.path.len() > 1) {
            indent(out, depth + 1);
            let _ = write!(
                out,
                "field {} type={} offset={}",
                context.string_cache.at(field.name.name),
                self.type_value(field.ty.qualified(field.qualifiers), context, arena),
                field.offset
            );
            if let Some(width) = field.width {
                let _ = write!(out, " bit-offset={} width={width}", field.bit_offset);
            }
            out.push('\n');
        }
    }

    fn write_binding<'d>(
        &self,
        index: usize,
        minimums: &super::ArenaMap<'d, super::SourceVectors, u64>,
        context: &Context<'tu>,
        arena: &'d Bump,
        out: &mut ArenaString<'d>,
    ) {
        let binding = self.bindings[index];
        let _ = write!(
            out,
            "{} {} type={}",
            match binding.kind {
                | BindingKind::Object => "object",
                | BindingKind::Function => "function",
                | BindingKind::Typedef => "typedef",
                | BindingKind::Enumerator => "enumerator",
                | BindingKind::Parameter => "parameter",
            },
            context.string_cache.at(binding.name.name),
            self.type_value(binding.ty, context, arena)
        );
        if let TypeKind::Function {
            prototype: true, ..
        } = self.types.nodes[binding.ty.index]
            && binding.kind == BindingKind::Function
        {
            out.push_str(" prototype");
        }
        if let Some(value) = binding.value {
            let _ = write!(out, " value={}", value.value);
        }
        if let Some(minimum) = minimums.get(&binding.name.source_vectors) {
            let _ = write!(out, " static-minimum={minimum}");
        }
        match binding.linkage {
            | Linkage::None => {},
            | Linkage::Internal => out.push_str(" linkage=internal"),
            | Linkage::External => out.push_str(" linkage=external"),
        }
        if binding.kind == BindingKind::Object {
            match binding.duration {
                | Duration::None => {},
                | Duration::Automatic => out.push_str(" duration=automatic"),
                | Duration::Static => out.push_str(" duration=static"),
            }
        }
        if matches!(binding.kind, BindingKind::Object | BindingKind::Parameter)
            && let Some(layout) = self.types.layout(binding.ty)
        {
            let _ = write!(out, " size={} align={}", layout.size, layout.align);
        }
        out.push('\n');
    }

    /// A type spelling as a field value.
    fn type_value<'d>(&self, ty: TypeId, context: &Context<'tu>, arena: &'d Bump) -> &'d str {
        let mut spelling = ArenaString::new_in(arena);
        self.write_type(ty, context, arena, &mut spelling);
        value(arena, spelling.into_str())
    }

    fn write_type<'d>(
        &self,
        ty: TypeId,
        context: &Context<'tu>,
        arena: &'d Bump,
        out: &mut ArenaString<'d>,
    ) {
        let mut work = ArenaVec::new_in(arena);
        work.push(Part::Type(ty));
        while let Some(part) = work.pop() {
            match part {
                | Part::Text(text) => out.push_str(text),
                | Part::Function(parameters, prototype, variadic) => {
                    work.push(Part::Text(")"));
                    if variadic {
                        work.push(Part::Text(if parameters.is_empty() {
                            "..."
                        } else {
                            ", ..."
                        }));
                    }
                    if prototype && parameters.is_empty() && !variadic {
                        work.push(Part::Text("void"));
                    }
                    for (index, &p) in parameters.iter().enumerate().rev() {
                        work.push(Part::Type(p));
                        if index != 0 {
                            work.push(Part::Text(", "));
                        }
                    }
                    work.push(Part::Text("("));
                },
                | Part::Type(mut ty) => {
                    // Derivations are met outermost first and printed
                    // innermost first; a pointer's qualifiers need a
                    // separating space only before a later derivation.
                    let mut left = ArenaVec::new_in(arena);
                    let mut right = ArenaVec::new_in(arena);
                    let mut pointer = false;
                    let base = loop {
                        match self.types.nodes[ty.index] {
                            | TypeKind::Pointer(next) => {
                                let words = qualifiers(ty.qualifiers);
                                if !words.is_empty() {
                                    left.push(Part::Text(if left.is_empty() {
                                        words
                                    } else {
                                        crate::diagnostics::format_in!(arena, "{words} ")
                                    }));
                                }
                                left.push(Part::Text("*"));
                                pointer = true;
                                ty = next;
                            },
                            | TypeKind::Array(next, bound) => {
                                if pointer {
                                    left.push(Part::Text("("));
                                    right.push(Part::Text(")"));
                                }
                                let text = match bound {
                                    | ArrayBound::Incomplete => "[]",
                                    | ArrayBound::Variable => "[*runtime*]",
                                    | ArrayBound::Star => "[*]",
                                    | ArrayBound::Constant(n) =>
                                        crate::diagnostics::format_in!(arena, "[{n}]"),
                                };
                                right.push(Part::Text(text));
                                pointer = false;
                                ty = next;
                            },
                            | TypeKind::Function {
                                result,
                                parameters,
                                prototype,
                                variadic,
                            } => {
                                if pointer {
                                    left.push(Part::Text("("));
                                    right.push(Part::Text(")"));
                                }
                                right.push(Part::Function(parameters, prototype, variadic));
                                pointer = false;
                                ty = result;
                            },
                            | TypeKind::Unknown => break "<unanalyzed>",
                            | TypeKind::Scalar(s) => break s.spelling(),
                            | TypeKind::Tag(id) => {
                                let tag = self.types.tags[id];
                                break crate::diagnostics::format_in!(
                                    arena,
                                    "{} {}",
                                    tag_kind(tag.kind),
                                    tag.name.map_or("<anonymous>", |name| context
                                        .string_cache
                                        .at(name))
                                );
                            },
                        }
                    };
                    let derived = !left.is_empty() || !right.is_empty();
                    for part in right.into_iter().rev() {
                        work.push(part);
                    }
                    for part in left {
                        work.push(part);
                    }
                    if derived {
                        work.push(Part::Text(" "));
                    }
                    work.push(Part::Text(base));
                    let words = qualifiers(ty.qualifiers);
                    if !words.is_empty() {
                        work.push(Part::Text(" "));
                        work.push(Part::Text(words));
                    }
                },
            }
        }
    }
}

fn indent(out: &mut ArenaString<'_>, depth: usize) {
    for _ in 0..depth.min(MAXIMUM_INDENT) {
        out.push_str("  ");
    }
    if depth > MAXIMUM_INDENT {
        let _ = write!(out, "[depth={depth}] ");
    }
}

/// Plain words stay bare; anything else is quoted.
fn value<'d>(arena: &'d Bump, text: &str) -> &'d str {
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        arena.alloc_str(text)
    } else {
        crate::diagnostics::format_in!(arena, "\"{text}\"")
    }
}

const fn scope_kind(kind: ScopeKind) -> &'static str {
    match kind {
        | ScopeKind::File => "file",
        | ScopeKind::Function => "function",
        | ScopeKind::Block => "block",
        | ScopeKind::Prototype => "prototype",
    }
}

const fn tag_kind(kind: TagKind) -> &'static str {
    match kind {
        | TagKind::Struct => "struct",
        | TagKind::Union => "union",
        | TagKind::Enum => "enum",
    }
}

const fn qualifiers(q: TypeQualifiers) -> &'static str {
    match q.bits() & 7 {
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
