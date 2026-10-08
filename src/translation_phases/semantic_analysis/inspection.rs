//! Deterministic non-recursive inspection of semantic declaration types.
//! C99: type-name spelling §6.7.6, p. 122; PDF p. 134; linkage and duration
//! §6.2.2-§6.2.4, pp. 30-32; PDF pp. 42-44. Inspection is
//! implementation-defined.

use std::fmt::Write as _;

use super::{
    ArenaVec,
    ArrayBound,
    Bump,
    Context,
    ScopeKind,
    SemanticTranslationUnit,
    TagKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
};
use crate::util::bump::ArenaString;

#[derive(Clone, Copy)]
enum Part<'tu> {
    Text(&'tu str),
    Type(TypeId),
    Function(&'tu [TypeId], bool, bool),
}

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn inspect<'d>(&self, context: &Context<'tu>, arena: &'d Bump) -> &'d str {
        let mut out = ArenaString::new_in(arena);
        let target = self.types.target;
        let _ = writeln!(
            out,
            "target: x86-64 System V LP64; size_t={}, ptrdiff_t={}, wchar_t={}; {} resolved type \
             names",
            target.size_t.spelling(),
            target.ptrdiff_t.spelling(),
            target.wchar_t.spelling(),
            self.type_names.len()
        );
        let minimums = self
            .parameters
            .iter()
            .flat_map(|(_, params)| params.iter())
            .filter(|p| p.array_minimum.is_some())
            .count();
        let _ = writeln!(out, "parameter arrays with static minimum: {minimums}");
        // Tag declarations have their own name space and table, including
        // declarations with no ordinary declarator.
        for &(id, scope_id) in self.tag_declarations {
            let scope = self.scopes[scope_id];
            if scope.kind == ScopeKind::Prototype {
                continue;
            }
            let tag = self.types.tags[id];
            let _ = write!(
                out,
                "scope {scope_id} ({:?}): tag {}: ",
                scope.kind,
                tag.name
                    .map_or("<anonymous>", |n| context.string_cache.at(n))
            );
            out.push_str(match tag.kind {
                | TagKind::Struct => "struct ",
                | TagKind::Union => "union ",
                | TagKind::Enum => "enum ",
            });
            out.push_str(
                tag.name
                    .map_or("<anonymous>", |n| context.string_cache.at(n)),
            );
            if let Some(layout) = tag.layout.get() {
                let _ = write!(out, " [size={}, align={}]", layout.size, layout.align);
            }
            let _ = writeln!(out, "; Tag, None, None");
        }
        for binding in self.bindings {
            let scope = self.scopes[binding.scope];
            if scope.kind == ScopeKind::Prototype {
                continue;
            }
            let _ = write!(
                out,
                "scope {} ({:?}): {}: ",
                binding.scope,
                scope.kind,
                context.string_cache.at(binding.name.name)
            );
            self.write_type(binding.ty, context, arena, &mut out);
            if let Some(layout) = self.types.layout(binding.ty) {
                let _ = write!(out, " [size={}, align={}]", layout.size, layout.align);
            }
            let _ = writeln!(
                out,
                "; {:?}, {:?}, {:?}",
                binding.kind, binding.linkage, binding.duration
            );
        }
        let _ = writeln!(out, "expressions: {}", self.expressions.len());
        for (index, info) in self.expressions.iter().enumerate() {
            let _ = write!(out, "expression {index}: ");
            self.write_type(info.ty, context, arena, &mut out);
            let _ = write!(out, "; {:?}", info.category);
            if let Some(ty) = info.operation_type {
                out.push_str("; operation=");
                self.write_type(ty, context, arena, &mut out);
            }
            if let Some(value) = info.floating {
                let _ = write!(out, "; real={}, imag={}", value.real, value.imag);
            }
            if let Some(value) = info.integer {
                let _ = write!(out, "; integer={}", value.value);
            }
            let _ = writeln!(out, "; ICE={}, constant={:?}", info.ice, info.constant);
        }
        let mut identities = super::ArenaMap::with_hasher_in(super::FxBuildHasher, arena);
        for (index, info) in self.expressions.iter().enumerate() {
            _ = identities.insert(std::ptr::from_ref(info.expression).addr(), index);
        }
        for conversion in self.conversions {
            let index = identities
                .get(&std::ptr::from_ref(conversion.expression).addr())
                .copied()
                .unwrap_or(usize::MAX);
            let _ = write!(out, "convert expression {index}: {:?} to ", conversion.kind);
            self.write_type(conversion.ty, context, arena, &mut out);
            out.push_str("\n");
        }
        out.into_str()
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
                    if prototype && parameters.is_empty() {
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
                    let mut left = ArenaVec::new_in(arena);
                    let mut right = ArenaVec::new_in(arena);
                    let mut pointer = false;
                    let base = loop {
                        match self.types.nodes[ty.index] {
                            | TypeKind::Pointer(next) => {
                                left.push(Part::Text(qualifiers(ty.qualifiers)));
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
                                    | ArrayBound::Variable => "[vla]",
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
                                let kind = match tag.kind {
                                    | TagKind::Struct => "struct",
                                    | TagKind::Union => "union",
                                    | TagKind::Enum => "enum",
                                };
                                break crate::diagnostics::format_in!(
                                    arena,
                                    "{kind} {}",
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
                    work.push(Part::Text(qualifiers(ty.qualifiers)));
                },
            }
        }
    }
}

fn qualifiers(q: TypeQualifiers) -> &'static str {
    match q.bits() & 7 {
        | 0 => "",
        | 1 => "const ",
        | 2 => "volatile ",
        | 3 => "const volatile ",
        | 4 => "restrict ",
        | 5 => "const restrict ",
        | 6 => "volatile restrict ",
        | _ => "const volatile restrict ",
    }
}
