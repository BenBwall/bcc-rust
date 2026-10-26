//! Checks GNU assembly statements inside functions and retains what
//! lowering needs to emit them. Each operand expression is analyzed by the
//! ordinary expression work; once all are typed, [`Analyzer::finish_assembly`]
//! parses each constraint, picks the operand's location (a register, the
//! lvalue's memory, an immediate, a tied output or a flag), records the value
//! conversion of every operand read from a register, and parses the clobbers
//! and the template against the operands and goto labels. A statement with
//! any error retains no facts.
//!
//! For `asm ("addl %1, %0" : "+r" (x) : "g" (5));` operand 0 is `x` read
//! and written in a general register, operand 1 is the immediate 5, and
//! the template is the text `addl `, operand 1, `, ` and operand 0.
//!
//! Read [`Analyzer::assembly_statement`] and [`Analyzer::finish_assembly`],
//! then `constraints.rs`, `template.rs` and `registers.rs`.
//!
//! C99: §J.5.10, p. 514; PDF p. 526 (`asm` as a common extension); an
//! output is a modifiable lvalue as an assignment's left operand is,
//! §6.5.16p2, p. 91; PDF p. 103; `n` operands are integer constant
//! expressions, §6.6p6, p. 95; PDF p. 107; goto labels follow §6.8.6.1p1,
//! p. 137; PDF p. 149.
//! GNU extension: GCC manual, "Extended Asm" and "Constraints for asm
//! Operands". <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>

// Grammars of the statement's strings
mod constraints;
mod registers;
mod template;

use constraints::{
    Constraint,
    ConstraintFault,
    Tie,
};
use template::{
    Names,
    TemplateFault,
};

use super::{
    Analyzer,
    AsmClobbers,
    AsmFacts,
    AsmOperandFacts,
    AsmOperandKind,
    AsmPiece,
    AsmRegisterClass,
    AsmRole,
    ConstantClass,
    ExpressionInfo,
    Scalar,
    SemanticErrorKind,
    TypeId,
    TypeKind,
    ValueCategory,
    Work,
    statements::StatementWork,
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            Asm,
            AsmOperand,
            syntax::Statement,
        },
        preprocessing::{
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Schedules the operand expressions of a statement-level `asm`, with
    /// [`StatementWork::AssemblyDone`] after them, and records each goto
    /// label as one of the statement's jumps.
    /// GNU extension: GCC manual, "Extended Asm", "Goto Labels".
    pub(super) fn assembly_statement(
        &mut self,
        statement: &'tu Statement<'tu>,
        asm: &'tu Asm<'tu>,
    ) {
        if asm.recovered {
            return;
        }
        self.work
            .push(Work::StatementWork(StatementWork::AssemblyDone(
                statement, asm,
            )));
        for operand in asm.operands.iter().rev() {
            self.work.push(Work::Expression(operand.expression));
        }
        if asm.qualifiers.goto {
            for &label in asm.labels {
                self.label(statement, label, false);
            }
        }
    }

    /// Checks a statement-level `asm` whose operands are typed and retains
    /// its facts when it has no error.
    pub(super) fn finish_assembly(&mut self, statement: &'tu Statement<'tu>, asm: &'tu Asm<'tu>) {
        let errors = self.semantic_errors;
        let facts = self.check_assembly(asm);
        if let Some(facts) = facts
            && self.semantic_errors == errors
        {
            _ = self
                .assembly
                .insert(std::ptr::from_ref(statement).addr(), facts);
        }
    }

    fn check_assembly(&mut self, asm: &'tu Asm<'tu>) -> Option<AsmFacts<'tu>> {
        let tu = self.context.tu_arena();
        let text = self.assembly_string(asm.template?)?;
        let basic = asm.sections == 0;
        if asm.operands.len() > MAX_OPERANDS {
            self.assembly_error(AsmFault::TooManyOperands, asm.source_vectors, None);
            return None;
        }
        let mut names = ArenaVec::new_in(self.scratch);
        for (index, operand) in asm.operands.iter().enumerate() {
            let name = operand.name.map(|name| {
                &*self
                    .scratch
                    .alloc_str(self.context.string_cache.at(name.name))
            });
            if let Some(name) = operand.name
                && asm.operands[..index]
                    .iter()
                    .any(|other| other.name.is_some_and(|other| other.name == name.name))
            {
                self.assembly_error(
                    AsmFault::DuplicateName,
                    name.source_vectors,
                    Some(name.name),
                );
                return None;
            }
            names.push(name);
        }
        let outputs = asm
            .operands
            .iter()
            .take_while(|operand| operand.output)
            .count();
        let mut operands = ArenaVec::with_capacity_in(asm.operands.len(), tu);
        let mut valid = true;
        for operand in asm.operands {
            match self.assembly_operand(asm, &names, outputs, operand) {
                | Some(facts) => operands.push(facts),
                | None => valid = false,
            }
        }
        let mut clobbers = AsmClobbers::default();
        for &token in asm.clobbers {
            let name = self.assembly_string(token)?;
            if !registers::clobber(name, &mut clobbers) {
                self.assembly_error(
                    AsmFault::UnknownRegister,
                    token.source_vectors,
                    Some(token.contents),
                );
                valid = false;
            }
        }
        if valid {
            valid = self.check_clobber_conflicts(asm, &operands, clobbers);
        }
        let template = if basic {
            &*tu.alloc_slice_copy(&[AsmPiece::Text(text)])
        } else {
            let mut labels = ArenaVec::new_in(self.scratch);
            for label in asm.labels {
                labels.push(
                    &*self
                        .scratch
                        .alloc_str(self.context.string_cache.at(label.name)),
                );
            }
            let names = Names {
                operands: &names,
                labels:   &labels,
            };
            match template::parse(text, &names, tu) {
                | Ok(pieces) => pieces,
                | Err(fault) => {
                    let token = asm.template?;
                    let fault = match fault {
                        | TemplateFault::Escape => AsmFault::Escape,
                        | TemplateFault::OperandNumber => AsmFault::OperandNumber,
                        | TemplateFault::Name => AsmFault::OperandName,
                        | TemplateFault::Modifier => AsmFault::Modifier,
                    };
                    self.assembly_error(fault, token.source_vectors, None);
                    return None;
                },
            }
        };
        let goto = asm.qualifiers.goto;
        valid.then(|| AsmFacts {
            volatile: asm.qualifiers.volatile || basic || goto || outputs == 0,
            template,
            operands: operands.leak(),
            clobbers,
            labels: u32::try_from(asm.labels.len()).unwrap_or(u32::MAX),
        })
    }

    /// Picks operand `index`'s location from its constraint and expression
    /// and records the value conversion of an operand read from a register.
    /// Outputs are modifiable lvalues and memory operands addressable ones;
    /// immediates are constants of the kind their letter allows.
    fn assembly_operand(
        &mut self,
        asm: &'tu Asm<'tu>,
        names: &[Option<&str>],
        outputs: usize,
        operand: &AsmOperand<'tu>,
    ) -> Option<AsmOperandFacts<'tu>> {
        let text = self.assembly_string(operand.constraint)?;
        let source = operand.expression.source_vectors;
        let constraint = match constraints::parse(text, operand.output) {
            | Ok(constraint) => constraint,
            | Err(fault) => {
                let fault = match fault {
                    | ConstraintFault::MissingOutputModifier => AsmFault::OutputModifier,
                    | ConstraintFault::Invalid if operand.output => AsmFault::OutputConstraint,
                    | ConstraintFault::Invalid => AsmFault::InputConstraint,
                };
                self.assembly_error(
                    fault,
                    operand.constraint.source_vectors,
                    Some(operand.constraint.contents),
                );
                return None;
            },
        };
        let info = self.expression_info(operand.expression);
        if self.types.unanalyzed(info.ty) {
            return None;
        }
        let role = constraint.role?;
        let kind = if role.writes() {
            self.output_location(info, &constraint, source)?
        } else if let Some(tie) = constraint.tied {
            let output = match tie {
                | Tie::Number(number) => usize::try_from(number).ok().filter(|&n| n < outputs),
                | Tie::Name(name) => names[..outputs]
                    .iter()
                    .position(|&known| known == Some(name)),
            };
            let Some(output) = output else {
                self.assembly_error(
                    AsmFault::TiedOperand,
                    operand.constraint.source_vectors,
                    Some(operand.constraint.contents),
                );
                return None;
            };
            self.tied_location(asm, output, info, source)?
        } else {
            self.input_location(info, &constraint, source)?
        };
        // Lowering reads an operand in a register, or a `+` one, through
        // its value conversion; a memory operand stays an lvalue.
        let read = match role {
            | AsmRole::Output => false,
            | AsmRole::Update => kind != AsmOperandKind::Memory,
            | AsmRole::Input => matches!(
                kind,
                AsmOperandKind::Register(_)
                    | AsmOperandKind::Tied(_)
                    | AsmOperandKind::Immediate { symbolic: true }
            ),
        };
        if read {
            _ = self.converted(info);
        }
        Some(AsmOperandFacts {
            expression: operand.expression,
            role,
            kind,
            early_clobber: constraint.early_clobber,
        })
    }

    /// An output's location: a flag, a register when the constraint allows
    /// one and the lvalue's type fits it, else its memory.
    /// C99: §6.5.16p2, p. 91; PDF p. 103 (a modifiable lvalue).
    fn output_location(
        &mut self,
        info: ExpressionInfo<'tu>,
        constraint: &Constraint<'_>,
        source: SourceVectors,
    ) -> Option<AsmOperandKind> {
        if info.category != ValueCategory::ModifiableLvalue {
            self.assembly_error(AsmFault::OutputLvalue, source, None);
            return None;
        }
        if let Some(condition) = constraint.flag {
            if self.integer_type(info.ty).is_none() {
                self.assembly_error(AsmFault::OperandType, source, None);
                return None;
            }
            return Some(AsmOperandKind::Flag(condition));
        }
        if let Some(class) = self.assembly_register_class(info.ty, constraint) {
            return Some(AsmOperandKind::Register(class));
        }
        if constraint.memory {
            if !addressable(info) {
                self.assembly_error(AsmFault::OutputLvalue, source, None);
                return None;
            }
            return Some(AsmOperandKind::Memory);
        }
        self.assembly_error(AsmFault::OperandType, source, None);
        None
    }

    /// An input's location: an immediate for a constant the constraint
    /// allows, else a register, else the lvalue's memory.
    /// C99: §6.6p6, p. 95; PDF p. 107 (integer constant expressions).
    fn input_location(
        &mut self,
        info: ExpressionInfo<'tu>,
        constraint: &Constraint<'_>,
        source: SourceVectors,
    ) -> Option<AsmOperandKind> {
        let integer = info
            .integer
            .filter(|_| info.ice)
            .and_then(super::Integer::to_i128);
        let value = self.value_type(info);
        let register = self.assembly_register_class(value, constraint);
        let generic = constraint.immediate;
        let ranged = constraint.ranges != 0;
        if let Some(value) = integer {
            let in_range = constraints::RANGES
                .iter()
                .enumerate()
                .any(|(bit, (_, allows))| constraint.ranges & (1 << bit) != 0 && allows(value));
            // `g` and `X` also allow a register, which a constant that no
            // instruction encodes as an immediate takes instead.
            let encodable = register.is_none() || i32::try_from(value).is_ok();
            if (generic.is_some() && encodable) || in_range {
                return Some(AsmOperandKind::Immediate { symbolic: false });
            }
        }
        // A function designator decays to an address constant (§6.6p9).
        let address = matches!(info.constant, ConstantClass::Address)
            || info.category == ValueCategory::FunctionDesignator;
        if generic == Some(true) && register.is_none() && address {
            return Some(AsmOperandKind::Immediate { symbolic: true });
        }
        if let Some(class) = register {
            return Some(AsmOperandKind::Register(class));
        }
        if constraint.memory {
            if !addressable(info) {
                self.assembly_error(AsmFault::InputLvalue, source, None);
                return None;
            }
            return Some(AsmOperandKind::Memory);
        }
        if generic.is_some() || ranged {
            let fault = if integer.is_some() {
                AsmFault::ImmediateRange
            } else {
                AsmFault::ImmediateRequired
            };
            self.assembly_error(fault, source, None);
            return None;
        }
        self.assembly_error(AsmFault::OperandType, source, None);
        None
    }

    /// An input tied to `output`, which must be in a register: both are
    /// integers or pointers, whose register holds the input widened or
    /// narrowed to the output's width as in GCC, or both are floating of one
    /// width, or both have one type (as two x87 `long double`s).
    fn tied_location(
        &mut self,
        asm: &'tu Asm<'tu>,
        output: usize,
        info: ExpressionInfo<'tu>,
        source: SourceVectors,
    ) -> Option<AsmOperandKind> {
        let output_info = self.expression_info(asm.operands[output].expression);
        let value = self.value_type(info);
        let input = self.assembly_register_bits(value);
        let target = self.assembly_register_bits(output_info.ty);
        let fits = match (input, target) {
            | (Some((bits, float)), Some((target, target_float))) =>
                float == target_float && (!float || bits == target),
            | _ => value == self.value_type_of(output_info.ty) && self.arithmetic(value),
        };
        if !fits {
            self.assembly_error(AsmFault::TiedType, source, None);
            return None;
        }
        Some(AsmOperandKind::Tied(u32::try_from(output).ok()?))
    }

    /// Rejects a clobber of a register an operand's constraint names alone:
    /// the operand and the clobber would need the same register.
    fn check_clobber_conflicts(
        &mut self,
        asm: &'tu Asm<'tu>,
        operands: &[AsmOperandFacts<'tu>],
        clobbers: AsmClobbers,
    ) -> bool {
        for (operand, facts) in asm.operands.iter().zip(operands) {
            if let AsmOperandKind::Register(AsmRegisterClass::General(mask)) = facts.kind
                && mask.is_power_of_two()
                && clobbers.general & mask != 0
            {
                self.assembly_error(
                    AsmFault::ClobberConflict,
                    operand.constraint.source_vectors,
                    Some(operand.constraint.contents),
                );
                return false;
            }
        }
        true
    }

    /// The register class a value of type `ty` takes under `constraint`, if
    /// the constraint allows registers and the value fits one: an integer or
    /// pointer of at most 64 bits, or a `float` or `double`.
    fn assembly_register_class(
        &self,
        ty: TypeId,
        constraint: &Constraint<'_>,
    ) -> Option<AsmRegisterClass> {
        let ty = self.value_type_of(ty);
        if constraint.general != 0 && self.assembly_register_bits(ty).is_some() {
            return Some(AsmRegisterClass::General(constraint.general));
        }
        if constraint.sse && self.assembly_register_bits(ty).is_some() {
            return Some(AsmRegisterClass::Sse);
        }
        let arithmetic = self.arithmetic(ty) || self.pointer_target(ty).is_some();
        constraint
            .other
            .filter(|_| arithmetic)
            .map(AsmRegisterClass::Other)
    }

    /// The bits of a value a register holds and whether it is floating:
    /// integers and pointers of at most 64 bits, `float` and `double`.
    fn assembly_register_bits(&self, ty: TypeId) -> Option<(u32, bool)> {
        let ty = self.value_type_of(ty);
        if let Some((bits, _)) = self.integer_type(ty) {
            return (bits <= 64).then_some((bits.max(8), false));
        }
        if self.pointer_target(ty).is_some() {
            return Some((64, false));
        }
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Float) => Some((32, true)),
            | TypeKind::Scalar(Scalar::Double) => Some((64, true)),
            | _ => None,
        }
    }

    /// The type of the value an operand's register holds: its lvalue type
    /// without qualifiers and atomicity, or its decayed array or function.
    fn value_type(&mut self, info: ExpressionInfo<'tu>) -> TypeId {
        self.value_conversion(info).0
    }

    fn value_type_of(&self, ty: TypeId) -> TypeId {
        self.types.non_atomic(ty).unqualified()
    }

    /// The text of an ordinary string literal in the statement, or `None`
    /// after reporting a wide or Unicode one.
    fn assembly_string(&mut self, token: Token) -> Option<&'tu str> {
        let TokenType::String(StringTokenType::String(id)) = token.kind else {
            if matches!(token.kind, TokenType::String(_)) {
                self.assembly_error(AsmFault::WideString, token.source_vectors, None);
            }
            return None;
        };
        let tu = self.context.tu_arena();
        self.context.literal_text_in(tu, id, false)
    }

    fn assembly_error(
        &mut self,
        fault: AsmFault,
        source: SourceVectors,
        name: Option<crate::util::string_cache::StringCacheId>,
    ) {
        self.error(SemanticErrorKind::InvalidAsm(fault), source, name, None);
    }
}

/// Whether memory operands can address the lvalue: not a bit-field, a
/// `register` object or a vector element.
/// C99: §6.5.3.2p1, p. 78; PDF p. 90 (the objects `&` may address).
fn addressable(info: ExpressionInfo<'_>) -> bool {
    matches!(
        info.category,
        ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
    ) && info.bit_field.is_none()
        && !info.register
        && !info.vector_element
}

/// GCC's limit on the operands of one statement.
/// GNU extension: GCC manual, "Extended Asm", "Output Operands".
const MAX_OPERANDS: usize = 30;

/// Why a statement-level `asm` is invalid. The messages follow GCC and
/// Clang.
/// GNU extension: GCC manual, "Extended Asm" and "Constraints for asm
/// Operands".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsmFault {
    WideString,
    TooManyOperands,
    DuplicateName,
    OutputModifier,
    OutputConstraint,
    InputConstraint,
    OutputLvalue,
    InputLvalue,
    ImmediateRequired,
    ImmediateRange,
    OperandType,
    TiedOperand,
    TiedType,
    UnknownRegister,
    ClobberConflict,
    Escape,
    OperandNumber,
    OperandName,
    Modifier,
}

impl AsmFault {
    /// The short label of the diagnostic.
    pub(super) const fn label(self) -> &'static str {
        match self {
            | Self::WideString => "wide or Unicode string literal",
            | Self::TooManyOperands => "more than 30 operands",
            | Self::DuplicateName => "operand name used before",
            | Self::OutputModifier
            | Self::OutputConstraint
            | Self::InputConstraint
            | Self::TiedOperand => "invalid constraint",
            | Self::OutputLvalue | Self::InputLvalue => "not an addressable lvalue",
            | Self::ImmediateRequired | Self::ImmediateRange =>
                "not a constant the constraint allows",
            | Self::OperandType | Self::TiedType => "operand type does not fit the constraint",
            | Self::UnknownRegister => "unknown register",
            | Self::ClobberConflict => "register is clobbered",
            | Self::Escape | Self::OperandNumber | Self::OperandName | Self::Modifier =>
                "invalid template",
        }
    }

    /// The message and note of the diagnostic.
    pub(super) const fn explanation(self) -> (&'static str, &'static str) {
        match self {
            | Self::WideString => (
                "cannot use wide or unicode string literal in 'asm'",
                "GNU Extended Asm: templates, constraints and clobbers are ordinary string \
                 literals",
            ),
            | Self::TooManyOperands => (
                "more than 30 operands in 'asm'",
                "GNU Extended Asm: a statement has at most 30 output and input operands",
            ),
            | Self::DuplicateName => (
                "duplicate asm operand name",
                "GNU Extended Asm: each [name] names one operand of the statement",
            ),
            | Self::OutputModifier => (
                "output operand constraint lacks '='",
                "GNU Extended Asm: an output constraint starts with '=' (written) or '+' (read \
                 and written)",
            ),
            | Self::OutputConstraint => (
                "invalid output constraint in asm",
                "GNU Constraints for asm Operands: the constraint names x86-64 registers, memory, \
                 or a flag output",
            ),
            | Self::InputConstraint => (
                "invalid input constraint in asm",
                "GNU Constraints for asm Operands: the constraint names x86-64 registers, memory, \
                 immediates or an output to match, and has no '=', '+' or '&'",
            ),
            | Self::OutputLvalue => (
                "invalid lvalue in asm output",
                "C99 §6.5.16p2: an output is a modifiable lvalue, as an assignment's left \
                 operand; a memory output is not a bit-field or register variable",
            ),
            | Self::InputLvalue => (
                "invalid lvalue in asm input for a memory constraint",
                "GNU Constraints for asm Operands: a memory-only input is an lvalue whose address \
                 can be taken",
            ),
            | Self::ImmediateRequired => (
                "constraint expects a constant expression",
                "C99 §6.6p6: 'n' and the x86 range letters take an integer constant expression; \
                 'i' also takes an address constant",
            ),
            | Self::ImmediateRange => (
                "value is out of range for constraint",
                "GNU Machine Constraints: the x86 letters I, J, K, L, M, N, O, e and Z each allow \
                 a range of integers",
            ),
            | Self::OperandType => (
                "invalid type in asm operand for its constraint",
                "GNU Constraints for asm Operands: a register holds an integer or pointer of at \
                 most 64 bits, a float or a double; other types need a memory constraint",
            ),
            | Self::TiedOperand => (
                "invalid matching constraint",
                "GNU Extended Asm: a digit or [name] in an input constraint names an earlier \
                 output operand in a register",
            ),
            | Self::TiedType => (
                "unsupported inline asm: input does not fit its matching output",
                "GNU Extended Asm: a matching input and its output are both integers or pointers, \
                 or both floating of one width",
            ),
            | Self::UnknownRegister => (
                "unknown register name in asm",
                "GNU Clobbers and Scratch Registers: a clobber names an x86-64 register, 'cc' or \
                 'memory'",
            ),
            | Self::ClobberConflict => (
                "asm operand has impossible constraints",
                "GNU Clobbers and Scratch Registers: a clobbered register cannot hold an operand",
            ),
            | Self::Escape => (
                "invalid % escape in inline assembly string",
                "GNU Extended Asm: '%' is followed by an operand reference, a modifier and a \
                 reference, '%', '=', '{', '|' or '}'",
            ),
            | Self::OperandNumber => (
                "invalid operand number in inline asm string",
                "GNU Extended Asm: '%N' names an operand and '%lN' a goto label, numbered after \
                 the operands",
            ),
            | Self::OperandName => (
                "unknown symbolic operand name in inline asm string",
                "GNU Extended Asm: '%[name]' names an operand and '%l[name]' a goto label",
            ),
            | Self::Modifier => (
                "invalid operand modifier in inline asm string",
                "GNU x86 Operand Modifiers: a, A, b, c, d, g, h, H, k, l, n, p, P, q, t, V, w, x \
                 and z",
            ),
        }
    }
}
