//! The study's closed, source-semantic contract; no LLVM values or attributes.

use std::fmt;

use crate::{
    target::Scalar,
    translation_phases::{
        Context,
        SourceVector,
        SourceVectors,
        semantic_analysis::{
            FunctionRecord,
            SemanticTranslationUnit,
            TypeId,
            TypeKind,
        },
    },
    util::bump::Bump,
};

/// A complete snapshot. IDs and names remain usable after the frontend dies.
/// Copied source segments remain available through their original handles.
#[derive(Debug)]
pub(super) struct Module<'ir> {
    pub(super) functions: &'ir [Function<'ir>],
    pub(super) origins:   &'ir [Origin<'ir>],
}

/// File IDs refer to the host's source catalog; offsets/lines/columns survive
/// frontend teardown, including all segments of macro-expanded operations.
#[derive(Debug, Clone, Copy)]
pub(super) struct Origin<'ir> {
    pub(super) handle: SourceVectors,
    pub(super) spans:  &'ir [SourceVector],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Function<'ir> {
    pub(super) signature:   Signature<'ir>,
    pub(super) locals:      &'ir [Local<'ir>],
    pub(super) expressions: &'ir [Expression<'ir>],
    pub(super) code:        &'ir [Instruction],
    pub(super) labels:      &'ir [usize],
    pub(super) scopes:      usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Signature<'ir> {
    pub(super) entity:     usize,
    pub(super) name:       &'ir str,
    pub(super) result:     IntType,
    pub(super) parameters: &'ir [IntType],
    pub(super) origin:     SourceVectors,
}

/// C rank/identity survives even when two types have equal widths.
/// C99: §6.2.5, pp. 33-37; PDF pp. 45-49.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IntType {
    pub(super) scalar: Scalar,
    pub(super) bits:   u32,
    pub(super) signed: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Local<'ir> {
    pub(super) binding:   usize,
    pub(super) name:      &'ir str,
    pub(super) ty:        IntType,
    pub(super) scope:     usize,
    pub(super) parameter: bool,
    pub(super) origin:    SourceVectors,
}

/// An acyclic expression graph. Child order is the selected schedule;
/// `Sequence`/`Logical` retain required C sequence points separately.
#[derive(Debug, Clone, Copy)]
pub(super) struct Expression<'ir> {
    pub(super) kind:   ExpressionKind<'ir>,
    pub(super) ty:     IntType,
    pub(super) origin: SourceVectors,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ExpressionKind<'ir> {
    Constant(u64),
    Read(usize),
    Convert(usize),
    Unary(Unary, usize),
    Binary(Binary, usize, usize),
    Set {
        local:     usize,
        value:     usize,
        operation: Option<(Binary, IntType)>,
    },
    Update {
        local:     usize,
        increment: bool,
        postfix:   bool,
        promoted:  IntType,
    },
    Sequence(usize, usize),
    Logical {
        left:  usize,
        right: usize,
        and:   bool,
    },
    Conditional {
        condition: usize,
        then:      usize,
        otherwise: usize,
    },
    Call {
        function:  usize,
        arguments: &'ir [usize],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unary {
    Negate,
    Invert,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftLeft,
    ShiftRight,
    And,
    Or,
    Xor,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

/// Statement CFG with explicit lexical entry/exit; labels index `code`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Instruction {
    pub(super) kind:   InstructionKind,
    pub(super) origin: SourceVectors,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum InstructionKind {
    Enter(usize),
    Exit,
    Declare {
        local:       usize,
        initializer: Option<usize>,
    },
    Evaluate(usize),
    Branch {
        condition: usize,
        then:      usize,
        otherwise: usize,
    },
    Jump {
        label: usize,
        keep:  usize,
    },
    Return(usize),
    Fallthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Undefined {
    SignedOverflow,
    DivisionByZero,
    InvalidShift,
    UnsequencedAccess,
}

/// Unsupported/policy-dependent execution is deliberately distinct from UB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    Unsupported(&'static str),
    Undefined(Undefined),
    IndeterminateRead { local: usize },
    Limit(&'static str),
    InvalidIr,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Error {
    pub(super) failure: Failure,
    pub(super) origin:  Option<SourceVectors>,
}

impl Error {
    pub(super) const fn new(failure: Failure, origin: Option<SourceVectors>) -> Self {
        Self { failure, origin }
    }
}

impl IntType {
    pub(super) fn valid(self) -> bool {
        self.scalar.integer() && (1..=64).contains(&self.bits)
    }

    pub(super) fn snapshot(sema: &SemanticTranslationUnit<'_>, ty: TypeId) -> Result<Self, Error> {
        if !ty.qualifiers.is_empty() {
            return Err(Error::new(
                Failure::Unsupported("qualified integer type"),
                None,
            ));
        }
        let TypeKind::Scalar(scalar) = sema.types.kind(ty) else {
            return Err(Error::new(Failure::Unsupported("non-integer type"), None));
        };
        let Some((bits, signed)) = sema.types.target.integer(scalar) else {
            return Err(Error::new(Failure::Unsupported("non-integer scalar"), None));
        };
        if bits > 64 {
            return Err(Error::new(
                Failure::Unsupported("integer wider than 64 bits"),
                None,
            ));
        }
        Ok(Self {
            scalar,
            bits,
            signed,
        })
    }

    pub(super) const fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits)
    }

    pub(super) const fn minimum(self) -> i128 {
        -(1_i128 << (self.bits - 1))
    }

    pub(super) const fn maximum(self) -> i128 {
        (1_i128 << (self.bits - 1)) - 1
    }
}

impl<'ir> Signature<'ir> {
    pub(super) fn snapshot(
        context: &Context<'_>,
        sema: &SemanticTranslationUnit<'_>,
        record: &FunctionRecord<'_>,
        arena: &'ir Bump,
    ) -> Result<Self, Error> {
        let fail = || {
            Error::new(
                Failure::Unsupported("function signature or extension"),
                Some(record.syntax.source_vectors),
            )
        };
        let binding = record.binding.ok_or_else(fail)?;
        let data = sema.bindings[binding];
        let TypeKind::Function {
            prototype: true,
            variadic: false,
            parameters,
            ..
        } = sema.types.kind(data.ty)
        else {
            return Err(fail());
        };
        if record.old_style
            || record.assembly
            || parameters.len() != record.parameter_facts.len()
            || record.syntax.declaration_specifiers.extensions.is_some()
            || sema
                .symbols
                .entity(sema.correspondence.entity_of(binding, data))
                .is_some()
            || sema.function_extensions.function(binding).is_some()
        {
            return Err(fail());
        }
        let result = IntType::snapshot(sema, record.result)?;
        let parameters =
            arena.alloc_slice_fill_iter(parameters.iter().map(|&ty| IntType::snapshot(sema, ty)));
        let mut copied = crate::util::bump::ArenaVec::new_in(arena);
        for parameter in parameters {
            copied.push((*parameter)?);
        }
        Ok(Self {
            entity: sema.correspondence.entity_of(binding, data).0,
            name: arena.alloc_str(context.string_cache.at(data.name.name)),
            result,
            parameters: arena.alloc_slice_copy(&copied),
            origin: record.syntax.source_vectors,
        })
    }
}

impl Module<'_> {
    pub(super) fn source_spans(&self, handle: SourceVectors) -> Option<&[SourceVector]> {
        self.origins
            .iter()
            .find(|o| o.handle == handle)
            .map(|o| o.spans)
    }

    /// Checks executable references before the evaluator trusts this graph.
    pub(super) fn verify(&self) -> Result<(), Error> {
        let bad = || Error::new(Failure::InvalidIr, None);
        for f in self.functions {
            if f.code.is_empty()
                || f.locals.len() > 128
                || !f.signature.result.valid()
                || f.signature.parameters.iter().any(|ty| !ty.valid())
                || f.signature.parameters.len() > f.locals.len()
                || f.labels.iter().any(|&pc| pc >= f.code.len())
            {
                return Err(bad());
            }
            for (id, local) in f.locals.iter().enumerate() {
                if local.scope >= f.scopes
                    || !local.ty.valid()
                    || local.parameter != (id < f.signature.parameters.len())
                    || (local.parameter
                        && (local.scope != 0 || local.ty != f.signature.parameters[id]))
                {
                    return Err(bad());
                }
            }
            for (id, e) in f.expressions.iter().enumerate() {
                if !e.ty.valid() {
                    return Err(bad());
                }
                let child = |n| if n < id { Ok(()) } else { Err(bad()) };
                let local = |n| {
                    if n < f.locals.len() {
                        Ok(())
                    } else {
                        Err(bad())
                    }
                };
                match e.kind {
                    | ExpressionKind::Constant(_) => {},
                    | ExpressionKind::Read(n) => local(n)?,
                    | ExpressionKind::Update {
                        local: n, promoted, ..
                    } => {
                        local(n)?;
                        if !promoted.valid() {
                            return Err(bad());
                        }
                    },
                    | ExpressionKind::Convert(n) | ExpressionKind::Unary(_, n) => child(n)?,
                    | ExpressionKind::Binary(_, a, b)
                    | ExpressionKind::Sequence(a, b)
                    | ExpressionKind::Logical {
                        left: a, right: b, ..
                    } => {
                        child(a)?;
                        child(b)?;
                    },
                    | ExpressionKind::Set {
                        local: n,
                        value,
                        operation,
                    } => {
                        local(n)?;
                        child(value)?;
                        if operation.is_some_and(|(_, ty)| !ty.valid()) {
                            return Err(bad());
                        }
                    },
                    | ExpressionKind::Conditional {
                        condition,
                        then,
                        otherwise,
                    } => {
                        child(condition)?;
                        child(then)?;
                        child(otherwise)?;
                    },
                    | ExpressionKind::Call {
                        function,
                        arguments,
                    } => {
                        let callee = self.functions.get(function).ok_or_else(bad)?;
                        if callee.signature.parameters.len() != arguments.len() {
                            return Err(bad());
                        }
                        for &arg in arguments {
                            child(arg)?;
                        }
                    },
                }
            }
            for op in f.code {
                let expr = |n| {
                    if n < f.expressions.len() {
                        Ok(())
                    } else {
                        Err(bad())
                    }
                };
                let label = |n| {
                    if n < f.labels.len() {
                        Ok(())
                    } else {
                        Err(bad())
                    }
                };
                match op.kind {
                    | InstructionKind::Enter(n) if n >= f.scopes => return Err(bad()),
                    | InstructionKind::Declare { local, initializer } => {
                        if local >= f.locals.len() {
                            return Err(bad());
                        }
                        if let Some(n) = initializer {
                            expr(n)?;
                        }
                    },
                    | InstructionKind::Evaluate(n) | InstructionKind::Return(n) => expr(n)?,
                    | InstructionKind::Branch {
                        condition,
                        then,
                        otherwise,
                    } => {
                        expr(condition)?;
                        label(then)?;
                        label(otherwise)?;
                    },
                    | InstructionKind::Jump { label: n, .. } => label(n)?,
                    | _ => {},
                }
            }
            super::verify::scope_flow(f)?;
        }
        Ok(())
    }
}

impl fmt::Display for Module<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for function in self.functions {
            writeln!(
                f,
                "c.function {} {:?} -> {:?}",
                function.signature.name, function.signature.parameters, function.signature.result
            )?;
            for (id, local) in function.locals.iter().enumerate() {
                writeln!(
                    f,
                    "  object {id} {} {:?} scope={} binding={} parameter={}",
                    local.name, local.ty, local.scope, local.binding, local.parameter
                )?;
            }
            for (id, e) in function.expressions.iter().enumerate() {
                writeln!(f, "  e{id} {:?}: {:?}", e.kind, e.ty)?;
            }
            for (id, op) in function.code.iter().enumerate() {
                writeln!(f, "  pc{id} {:?}", op.kind)?;
            }
            writeln!(f, "  labels {:?}", function.labels)?;
        }
        Ok(())
    }
}
