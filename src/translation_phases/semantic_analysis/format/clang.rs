//! Clang's reading of a format, as Clang 23's format-string analysis makes
//! it on a Linux target: the string literals `checkFormatStringExpr` finds
//! (through parentheses and casts, a conditional's arms unless its
//! condition is constant, a constant variable's initializer, a `format_arg`
//! call's argument and a constant offset), each specification as
//! `ParsePrintfSpecifier` and `ParseScanfSpecifier` read it, and each data
//! argument against the type `ArgType::matchesType` wants. Clang reports a
//! mismatch at the argument and a fault of the string within the string,
//! and stops reading a string at a fault it cannot read past.
//! C99: §7.19.6.1p4-p17, pp. 275-281; PDF pp. 287-293;
//! §7.19.6.2p3-p13, pp. 282-287; PDF pp. 294-299.

use Scalar as S;

use super::{
    super::{
        Analyzer,
        ArrayBound,
        BindingKind,
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
        TypeQualifiers,
        facts::FormatKind,
    },
    FormatCall,
    FormatNote,
    literal::{
        Leaf,
        Spelled,
    },
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator,
            Constant,
            Expression,
            ExpressionType as E,
            UnaryOperator,
        },
        preprocessing::KeywordTokenType,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

/// What `checkFormatStringExpr` found, ordered as Clang compares them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Found {
    NotALiteral,
    Unchecked,
    Checked,
}

/// The first data argument no string covered (`UncoveredArgHandler`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Uncovered {
    Unknown,
    AllCovered,
    First(usize),
}

/// A length modifier (`LengthModifier::Kind`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Length {
    None,
    Char,
    Short,
    Long,
    LongLong,
    Quad,
    IntMax,
    SizeT,
    PtrDiff,
    LongDouble,
    Int64,
    Int32,
    Int3264,
    Wide,
    MAllocate,
}

/// A conversion specifier's kind (`ConversionSpecifier::Kind`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Conversion {
    Invalid,
    Percent,
    Signed,
    Unsigned,
    Binary,
    Octal,
    Hex,
    Double,
    Char,
    WideChar,
    String,
    WideString,
    Pointer,
    Written,
    Errno,
    ScanList,
    /// `%P`, which only `os_log` takes.
    Os,
    /// `%@`, which only Objective-C takes.
    Object,
    /// `%Z`, the Microsoft C runtime's counted string.
    Counted,
}

impl Conversion {
    const fn consumes(self) -> bool {
        !matches!(self, Self::Invalid | Self::Percent | Self::Errno)
    }
}

/// An optional amount: a field width or a precision (`OptionalAmount`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Amount {
    NotSpecified,
    /// A number, its first byte and its length.
    Constant {
        value:  u32,
        start:  usize,
        length: usize,
    },
    /// A data argument, `*` or `*n$`, and where Clang points for it.
    Argument {
        index: usize,
        start: usize,
    },
}

/// One conversion specification as parsed.
#[derive(Clone, Copy)]
struct Spec {
    start:       usize,
    end:         usize,
    positional:  bool,
    index:       usize,
    thousands:   Option<usize>,
    left:        Option<usize>,
    plus:        Option<usize>,
    space:       Option<usize>,
    alternative: Option<usize>,
    zeros:       Option<usize>,
    suppressed:  bool,
    width:       Amount,
    precision:   Amount,
    length:      Length,
    length_at:   usize,
    length_len:  usize,
    conversion:  Conversion,
    position:    usize,
    letter:      u8,
}

/// The type a specification wants (`ArgType`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Invalid,
    Unknown,
    Specific(Ty),
    AnyChar,
    CString,
    WideString,
    WideInt,
    CPointer,
}

/// A specific type, with the name Clang gives it when it has one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Scalar(Scalar),
    Size,
    SignedSize,
    PtrDiff,
    UnsignedPtrDiff,
    IntMax,
    UintMax,
    WideChar,
    Int64,
    UnsignedInt64,
    Int32,
    UnsignedInt32,
}

#[derive(Clone, Copy)]
struct ArgType {
    want:    Want,
    pointer: bool,
}

const fn arg(want: Want) -> ArgType {
    ArgType {
        want,
        pointer: false,
    }
}

const fn pointer_to(want: Want) -> ArgType {
    ArgType {
        want,
        pointer: true,
    }
}

const fn specific(scalar: Scalar) -> Want {
    Want::Specific(Ty::Scalar(scalar))
}

/// How well an argument's type fits (`ArgType::MatchKind`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fit {
    Mismatch,
    Exact,
    Promotion,
    PromotionConfusion,
    Pedantic,
    Signedness,
    Confusion,
}

/// The state of one string's reading (`CheckFormatHandler`).
struct Reading<'a, 'tu> {
    call:       &'a FormatCall<'a, 'tu>,
    /// Whether the target's C library is the Microsoft C runtime.
    msvcrt:     bool,
    leaf:       Leaf<'tu>,
    bytes:      &'tu [u8],
    covered:    ArenaVec<'a, bool>,
    at_first:   bool,
    positional: bool,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks a call's format as Clang does (`checkFormatArguments`).
    pub(super) fn clang_check_format(&mut self, call: &FormatCall<'_, 'tu>) {
        let mut uncovered = Uncovered::Unknown;
        let found = self.clang_format_expression(call, call.format, true, 0, &mut uncovered, 0);
        if let Uncovered::First(index) = uncovered
            && let Some(&argument) = call.data().get(index)
        {
            self.format_warning(
                call.call.source_vectors,
                G::FormatExtraArgs,
                "data argument not used by format string",
                argument.source_vectors,
                None,
            );
        }
        if found != Found::NotALiteral {
            return;
        }
        match self.clang_caller_format(call) {
            | Caller::Attributed => return,
            | Caller::Suggest(caller, format, first) => {
                // Clang gives the caller the attribute it suggests, so that
                // calls to the caller are checked in turn.
                if self
                    .context
                    .configuration_at(call.call.source_vectors)
                    .diagnostic_severity(G::MissingFormatAttribute)
                    .is_none()
                {
                    return;
                }
                // Clang adds it to the first declaration, which calls, the
                // caller's own later ones included, see only when the
                // definition is that declaration.
                let declaration = self.first_declaration(caller);
                if declaration == caller {
                    self.infer_format_attribute(caller, call.kind, format, first);
                }
                let kind = if call.kind == FormatKind::Scanf {
                    "scanf"
                } else {
                    "printf"
                };
                let name = self
                    .context
                    .string_cache
                    .at(self.bindings[caller].name.name);
                let words = format_in!(
                    self.scratch,
                    "diagnostic behavior may be improved by adding the 'format({kind}, {format}, \
                     {first})' attribute to the declaration of '{name}'"
                );
                let declared = self.bindings[declaration].name.source_vectors;
                let site = crate::translation_phases::Context::last_source_vector(
                    call.call.source_vectors,
                );
                self.format_warning(
                    call.call.source_vectors,
                    G::MissingFormatAttribute,
                    words,
                    site,
                    Some((
                        FormatNote::DeclaredHere(self.bindings[caller].name.name),
                        declared,
                    )),
                );
                return;
            },
            | Caller::Other => {},
        }
        let site = call.format.source_vectors;
        if call.data().is_empty() && !call.takes_va_list() {
            let note =
                (call.kind == FormatKind::Printf).then_some((FormatNote::TreatAsArgument, site));
            self.format_warning(
                call.call.source_vectors,
                G::FormatSecurity,
                "format string is not a string literal (potentially insecure)",
                site,
                note,
            );
        } else {
            self.format_warning(
                call.call.source_vectors,
                G::FormatNonliteral,
                "format string is not a string literal",
                site,
                None,
            );
        }
    }

    /// What Clang makes of a format that is the caller's own parameter: its
    /// callers' arguments are checked against the caller's `format`
    /// attribute, so the format is not reported, and without one Clang 23
    /// suggests the attribute where the call passes the caller's data on:
    /// its `va_list`, or its parameters after the format, in their order.
    fn clang_caller_format(&mut self, call: &FormatCall<'_, 'tu>) -> Caller {
        let mut e = call.format;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Cast {
                    operand_expression, ..
                } => e = operand_expression,
                | _ => break,
            }
        }
        let Some(binding) = self.expression_info(e).binding else {
            return Caller::Other;
        };
        if self.bindings[binding].kind != BindingKind::Parameter {
            return Caller::Other;
        }
        let Some(function) = self.functions.current else {
            return Caller::Other;
        };
        // The body names the definition's parameters, which the function's
        // parameter list knows by name.
        let named = |analyzer: &Self, binding: usize, parameter: &super::super::Parameter<'tu>| {
            analyzer.bindings[binding].kind == BindingKind::Parameter
                && parameter.name.map(|name| name.name)
                    == Some(analyzer.bindings[binding].name.name)
        };
        let Some(position) = function
            .parameters
            .iter()
            .position(|parameter| named(self, binding, parameter))
        else {
            return Caller::Other;
        };
        let Some(caller) = function.binding else {
            return Caller::Other;
        };
        if let Some(attribute) = self.format_attributes(caller).0 {
            return if attribute.format as usize == position + 1 && attribute.kind == call.kind {
                Caller::Attributed
            } else {
                Caller::Other
            };
        }
        let format = position + 1;
        if call.takes_va_list() {
            let variadic = matches!(
                self.types.nodes[self.bindings[caller].ty.index],
                TypeKind::Function { variadic: true, .. }
            );
            let first = if variadic {
                function.parameters.len() + 1
            } else {
                0
            };
            return Caller::Suggest(caller, format, first);
        }
        let passed = &call.arguments[call.format_index..];
        let forwards = !call.data().is_empty()
            && passed.len() == function.parameters.len() - position
            && passed
                .iter()
                .zip(&function.parameters[position..])
                .all(|(&argument, parameter)| {
                    let mut argument = argument;
                    while let E::Parenthesized { expression } = argument.kind {
                        argument = expression;
                    }
                    matches!(argument.kind, E::Identifier(_))
                        && self
                            .expression_info(argument)
                            .binding
                            .is_some_and(|binding| named(self, binding, parameter))
                });
        if forwards {
            Caller::Suggest(caller, format, format + 1)
        } else {
            Caller::Other
        }
    }

    /// The first declaration of the function `binding` declares.
    fn first_declaration(&mut self, binding: usize) -> usize {
        let entity = self.entity_of(binding);
        let name = self.bindings[binding].name.name;
        (0..binding)
            .find(|&earlier| {
                self.bindings[earlier].kind == BindingKind::Function
                    && self.bindings[earlier].name.name == name
                    && self.entity_of(earlier) == entity
            })
            .unwrap_or(binding)
    }

    /// Follows the format expression `e` to its string literals, checking
    /// each (`checkFormatStringExpr`).
    fn clang_format_expression(
        &mut self,
        call: &FormatCall<'_, 'tu>,
        e: &'tu Expression<'tu>,
        in_call: bool,
        offset: i128,
        uncovered: &mut Uncovered,
        depth: u32,
    ) -> Found {
        if depth > 64 {
            return Found::NotALiteral;
        }
        let mut e = e;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Cast {
                    operand_expression, ..
                } => e = operand_expression,
                // `IgnoreParenCasts` looks through the operand a selection
                // chooses.
                | E::Generic(_) => match self.expression_info(e).selected_expression {
                    | Some(selected) => e = selected,
                    | None => break,
                },
                | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                    match self.expression_info(e).selected_expression {
                        | Some(selected) => e = selected,
                        | None => break,
                    },
                | _ => break,
            }
        }
        if self.null_pointer_constant(self.expression_info(e)) {
            return Found::Unchecked;
        }
        match e.kind {
            | E::Conditional(conditional) => {
                let then_value = conditional.then_expression;
                let condition = self.expression_info(conditional.condition_expression);
                let truth = condition
                    .ice
                    .then(|| Self::constant_truth(condition))
                    .flatten();
                let left = if truth == Some(false) {
                    Found::Unchecked
                } else {
                    let left = self.clang_format_expression(
                        call,
                        then_value,
                        in_call,
                        offset,
                        uncovered,
                        depth + 1,
                    );
                    if left == Found::NotALiteral || truth == Some(true) {
                        return left;
                    }
                    left
                };
                let right = self.clang_format_expression(
                    call,
                    conditional.else_expression,
                    in_call,
                    offset,
                    uncovered,
                    depth + 1,
                );
                if truth.is_none() && left < right {
                    left
                } else {
                    right
                }
            },
            | E::Identifier(_) => {
                let Some(binding) = self.expression_info(e).binding else {
                    return Found::NotALiteral;
                };
                let ty = self.bindings[binding].ty;
                let initializer = match self.types.nodes[ty.index] {
                    | TypeKind::Array(element, _)
                        if element.qualifiers.contains(TypeQualifiers::CONST) =>
                        self.constant_aggregate(binding)
                            .and_then(super::initializer_expression),
                    | TypeKind::Pointer(target)
                        if ty.qualifiers.contains(TypeQualifiers::CONST)
                            && target.qualifiers.contains(TypeQualifiers::CONST) =>
                        self.constant_scalar(binding),
                    | _ => None,
                };
                // An array's string keeps the array's extent, which may
                // leave out its null character.
                let extent = match self.types.nodes[ty.index] {
                    | TypeKind::Array(_, ArrayBound::Constant(count)) =>
                        usize::try_from(count).ok(),
                    | _ => None,
                };
                if let Some((literal, _)) = initializer.and_then(Self::string_literal)
                    && extent.is_some()
                {
                    let Some(mut leaf) = usize::try_from(offset)
                        .ok()
                        .and_then(|offset| self.leaf(literal, literal, offset, false, false))
                    else {
                        return Found::NotALiteral;
                    };
                    leaf.extent = extent;
                    self.clang_check_string(call, leaf, uncovered);
                    return Found::Checked;
                }
                match initializer {
                    | Some(initializer) => self.clang_format_expression(
                        call,
                        initializer,
                        false,
                        offset,
                        uncovered,
                        depth + 1,
                    ),
                    | None => Found::NotALiteral,
                }
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                let formats = self.format_arguments_of(function_expression, arguments);
                let mut found = None;
                for &argument in &formats {
                    let result = self.clang_format_expression(
                        call,
                        argument,
                        in_call,
                        offset,
                        uncovered,
                        depth + 1,
                    );
                    _ = found.get_or_insert(result);
                }
                found.unwrap_or(Found::NotALiteral)
            },
            | E::StringLiteral(_) => {
                // A negative offset, or one past the string, is no string.
                let Some(leaf) = usize::try_from(offset)
                    .ok()
                    .and_then(|offset| self.leaf(e, e, offset, in_call, in_call))
                else {
                    return Found::NotALiteral;
                };
                let length = self.format_bytes(&Leaf { offset: 0, ..leaf }).len();
                if leaf.offset > length {
                    return Found::NotALiteral;
                }
                self.clang_check_string(call, leaf, uncovered);
                Found::Checked
            },
            // A constant offset from a literal is still the literal
            // (`sumOffsets`): `"..." + N`, `N + "..."` and `"..." - N`.
            | E::Binary {
                operator: operator @ (BinaryOperator::Addition | BinaryOperator::Subtraction),
                left_expression,
                right_expression,
            } => {
                let constant = |analyzer: &Self, e: &'tu Expression<'tu>| {
                    let info = analyzer.expression_info(e);
                    info.integer
                        .filter(|_| info.ice)
                        .and_then(super::super::Integer::to_i128)
                };
                let (pointer, number) = match (
                    constant(self, left_expression),
                    constant(self, right_expression),
                ) {
                    | (Some(value), None) if operator == BinaryOperator::Addition =>
                        (right_expression, value),
                    | (None, Some(value)) if operator == BinaryOperator::Addition =>
                        (left_expression, value),
                    | (None, Some(value)) => (left_expression, -value),
                    | _ => return Found::NotALiteral,
                };
                match offset.checked_add(number) {
                    | Some(offset) => self.clang_format_expression(
                        call,
                        pointer,
                        in_call,
                        offset,
                        uncovered,
                        depth + 1,
                    ),
                    | None => Found::NotALiteral,
                }
            },
            | E::Unary {
                operator: UnaryOperator::AddressOf,
                operand_expression,
            } => match Self::without_parentheses(operand_expression).kind {
                | E::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    let info = self.expression_info(right_expression);
                    match info
                        .integer
                        .filter(|_| info.ice)
                        .and_then(super::super::Integer::to_i128)
                        .and_then(|value| offset.checked_add(value))
                    {
                        | Some(offset) => self.clang_format_expression(
                            call,
                            left_expression,
                            in_call,
                            offset,
                            uncovered,
                            depth + 1,
                        ),
                        | None => Found::NotALiteral,
                    }
                },
                | _ => Found::NotALiteral,
            },
            | _ => Found::NotALiteral,
        }
    }

    /// Checks one string literal (`CheckFormatString`).
    fn clang_check_string(
        &mut self,
        call: &FormatCall<'_, 'tu>,
        leaf: Leaf<'tu>,
        uncovered: &mut Uncovered,
    ) {
        let mut bytes = self.format_bytes(&leaf);
        let data = call.data().len();
        // An array too short for the null character, with none of its own
        // within it, holds no string; a longer one ends where it does.
        if let Some(extent) = leaf.extent {
            if extent <= bytes.len() && !bytes[..extent].contains(&0) {
                self.clang_string_warning_at(
                    call,
                    &leaf,
                    G::Format,
                    "format string is not null-terminated",
                    leaf.literal.source_vectors,
                );
                return;
            }
            bytes = &bytes[..bytes.len().min(extent.max(1) - 1)];
        }
        if bytes.is_empty() && data > 0 {
            self.clang_string_warning_at(
                call,
                &leaf,
                G::FormatZeroLength,
                "format string is empty",
                leaf.literal.source_vectors,
            );
            return;
        }
        let scratch = self.scratch;
        let mut reading = Reading {
            call,
            msvcrt: matches!(
                self.context.configuration.target(),
                crate::target::Target::WindowsGnu | crate::target::Target::WindowsMsvc
            ),
            leaf,
            bytes,
            covered: ArenaVec::new_in(scratch),
            at_first: true,
            positional: false,
        };
        for _ in 0..data {
            reading.covered.push(false);
        }
        let completed = if call.kind == FormatKind::Scanf {
            self.clang_scanf(&mut reading)
        } else {
            self.clang_printf(&mut reading)
        };
        if !completed || call.takes_va_list() {
            return;
        }
        let first = reading.covered.iter().position(|&covered| !covered);
        match (first, *uncovered) {
            | (_, Uncovered::AllCovered) => {},
            | (None, _) => *uncovered = Uncovered::AllCovered,
            | (Some(index), Uncovered::Unknown) => *uncovered = Uncovered::First(index),
            | (Some(index), Uncovered::First(old)) =>
                if index > old {
                    *uncovered = Uncovered::First(index);
                },
        }
    }

    /// Reads a printf string; `false` when the reading stopped
    /// (`ParsePrintfString`).
    fn clang_printf(&mut self, r: &mut Reading<'_, 'tu>) -> bool {
        let mut i = 0;
        let mut next_index = 0;
        while i < r.bytes.len() {
            match self.clang_parse_printf(r, &mut i, &mut next_index) {
                | Parsed::Stop => return false,
                | Parsed::Nothing => {},
                | Parsed::Spec(spec) =>
                    if !self.clang_printf_spec(r, spec) {
                        return false;
                    },
            }
        }
        true
    }

    /// Reads a scanf string; `false` when the reading stopped
    /// (`ParseScanfString`).
    fn clang_scanf(&mut self, r: &mut Reading<'_, 'tu>) -> bool {
        let mut i = 0;
        let mut next_index = 0;
        while i < r.bytes.len() {
            match self.clang_parse_scanf(r, &mut i, &mut next_index) {
                | Parsed::Stop => return false,
                | Parsed::Nothing => {},
                | Parsed::Spec(spec) =>
                    if !self.clang_scanf_spec(r, spec) {
                        return false;
                    },
            }
        }
        true
    }

    /// Finds the next `%` and reads the specification it begins
    /// (`ParsePrintfSpecifier`).
    fn clang_parse_printf(
        &mut self,
        r: &mut Reading<'_, 'tu>,
        i: &mut usize,
        next_index: &mut usize,
    ) -> Parsed {
        let bytes = r.bytes;
        let end = bytes.len();
        let mut start = None;
        while *i < end {
            let c = bytes[*i];
            if c == 0 {
                self.clang_string_warning(
                    r,
                    G::Format,
                    "format string contains '\\0' within the string body",
                    *i,
                );
                *i = end;
                return Parsed::Stop;
            }
            *i += 1;
            if c == b'%' {
                start = Some(*i - 1);
                break;
            }
        }
        let Some(start) = start else {
            return Parsed::Nothing;
        };
        let incomplete = |analyzer: &mut Self, r: &Reading<'_, 'tu>| {
            analyzer.clang_string_warning(r, G::Format, "incomplete format specifier", start);
            Parsed::Stop
        };
        if *i == end {
            return incomplete(self, r);
        }
        let mut spec = Spec {
            start,
            end: start,
            positional: false,
            index: 0,
            thousands: None,
            left: None,
            plus: None,
            space: None,
            alternative: None,
            zeros: None,
            suppressed: false,
            width: Amount::NotSpecified,
            precision: Amount::NotSpecified,
            length: Length::None,
            length_at: 0,
            length_len: 0,
            conversion: Conversion::Invalid,
            position: 0,
            letter: 0,
        };
        // An argument position, `n$`.
        match self.clang_position(r, &mut spec, start, i) {
            | Some(true) => return Parsed::Stop,
            | Some(false) => {},
            | None => return incomplete(self, r),
        }
        if *i == end {
            return incomplete(self, r);
        }
        // Flags.
        while *i < end {
            match bytes[*i] {
                | b'\'' => spec.thousands = Some(*i),
                | b'-' => spec.left = Some(*i),
                | b'+' => spec.plus = Some(*i),
                | b' ' => spec.space = Some(*i),
                | b'#' => spec.alternative = Some(*i),
                | b'0' => spec.zeros = Some(*i),
                | _ => break,
            }
            *i += 1;
        }
        if *i == end {
            return incomplete(self, r);
        }
        // The field width.
        match self.clang_amount(r, &spec, start, i, next_index, false) {
            | Some(amount) => spec.width = amount,
            | None => return Parsed::Stop,
        }
        if *i == end {
            return incomplete(self, r);
        }
        // The precision.
        if bytes[*i] == b'.' {
            *i += 1;
            if *i == end {
                return incomplete(self, r);
            }
            match self.clang_amount(r, &spec, start, i, next_index, true) {
                | Some(Amount::Constant {
                    value,
                    start,
                    length,
                }) =>
                    spec.precision = Amount::Constant {
                        value,
                        start: start - 1,
                        length,
                    },
                | Some(Amount::Argument { index, start }) =>
                    spec.precision = Amount::Argument {
                        index,
                        start: start - 1,
                    },
                | Some(Amount::NotSpecified) => {},
                | None => return Parsed::Stop,
            }
            if *i == end {
                return incomplete(self, r);
            }
        }
        // The length modifier.
        if Self::clang_length(r, &mut spec, i, false) && *i == end {
            return incomplete(self, r);
        }
        if bytes[*i] == 0 {
            self.clang_string_warning(
                r,
                G::Format,
                "format string contains '\\0' within the string body",
                *i,
            );
            return Parsed::Stop;
        }
        // The conversion specifier.
        spec.position = *i;
        spec.letter = bytes[*i];
        *i += 1;
        spec.conversion = match spec.letter {
            | b'%' => Conversion::Percent,
            | b'd' | b'i' => Conversion::Signed,
            | b'u' => Conversion::Unsigned,
            | b'b' | b'B' => Conversion::Binary,
            | b'o' => Conversion::Octal,
            | b'x' | b'X' => Conversion::Hex,
            | b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => Conversion::Double,
            | b'c' => Conversion::Char,
            | b'C' => Conversion::WideChar,
            | b's' => Conversion::String,
            | b'S' => Conversion::WideString,
            | b'p' => Conversion::Pointer,
            | b'P' => Conversion::Os,
            | b'@' => Conversion::Object,
            | b'n' => Conversion::Written,
            | b'm' => Conversion::Errno,
            | b'Z' if r.msvcrt => Conversion::Counted,
            | _ => Conversion::Invalid,
        };
        if spec.conversion.consumes() && !spec.positional {
            spec.index = *next_index;
            *next_index += 1;
        }
        spec.end = *i;
        if spec.conversion == Conversion::Invalid {
            *i = utf8_end(bytes, spec.position);
            spec.end = *i;
            return if self.clang_invalid_specifier(r, &spec) {
                Parsed::Nothing
            } else {
                Parsed::Stop
            };
        }
        Parsed::Spec(spec)
    }

    /// Reads `n$` at `i` (`ParseArgPosition`): `Some(true)` when the
    /// reading stops, `None` for an incomplete specifier.
    fn clang_position(
        &mut self,
        r: &Reading<'_, 'tu>,
        spec: &mut Spec,
        start: usize,
        i: &mut usize,
    ) -> Option<bool> {
        let bytes = r.bytes;
        let (value, j, _) = amount_digits(bytes, *i);
        if j == bytes.len() {
            return None;
        }
        if j > *i && bytes[j] == b'$' {
            if value == 0 {
                self.clang_string_warning(
                    r,
                    G::Format,
                    "position arguments in format strings start counting at 1 (not 0)",
                    start,
                );
                return Some(true);
            }
            spec.index = (value - 1) as usize;
            spec.positional = true;
            *i = j + 1;
        }
        Some(false)
    }

    /// Reads a field width or a precision at `i` (`ParseFieldWidth`,
    /// `ParsePrecision`); `None` when the reading stops.
    fn clang_amount(
        &mut self,
        r: &Reading<'_, 'tu>,
        spec: &Spec,
        start: usize,
        i: &mut usize,
        next_index: &mut usize,
        precision: bool,
    ) -> Option<Amount> {
        let bytes = r.bytes;
        let digits = |j: usize| amount_digits(bytes, j);
        if bytes[*i] != b'*' {
            let (value, j, length) = digits(*i);
            if length == 0 {
                return Some(Amount::NotSpecified);
            }
            let amount = Amount::Constant {
                value,
                start: *i,
                length,
            };
            *i = j;
            return Some(amount);
        }
        if !spec.positional {
            *i += 1;
            let amount = Amount::Argument {
                index: *next_index,
                start: *i,
            };
            *next_index += 1;
            return Some(amount);
        }
        let star = *i;
        let (value, j, length) = digits(*i + 1);
        let invalid = |analyzer: &mut Self| {
            let words = if precision {
                "invalid position specified for field precision"
            } else {
                "invalid position specified for field width"
            };
            analyzer.clang_string_warning(r, G::Format, words, star);
            None
        };
        if length == 0 {
            return invalid(self);
        }
        if j == bytes.len() {
            self.clang_string_warning(r, G::Format, "incomplete format specifier", start);
            return None;
        }
        if bytes[j] != b'$' {
            return invalid(self);
        }
        if value == 0 {
            self.clang_string_warning(
                r,
                G::Format,
                "position arguments in format strings start counting at 1 (not 0)",
                star,
            );
            return None;
        }
        *i = j + 1;
        Some(Amount::Argument {
            index: (value - 1) as usize,
            // Clang points at the `*` of `*n$`, and past a plain `*`.
            start: star,
        })
    }

    /// Reads a length modifier at `i` (`ParseLengthModifier`); whether
    /// there was one.
    fn clang_length(r: &Reading<'_, 'tu>, spec: &mut Spec, i: &mut usize, scanf: bool) -> bool {
        let bytes = r.bytes;
        let at = |j: usize| bytes.get(j).copied();
        let start = *i;
        let length = match at(*i) {
            | Some(b'h') =>
                if at(*i + 1) == Some(b'h') {
                    *i += 2;
                    Length::Char
                } else {
                    *i += 1;
                    Length::Short
                },
            | Some(b'l') =>
                if at(*i + 1) == Some(b'l') {
                    *i += 2;
                    Length::LongLong
                } else {
                    *i += 1;
                    Length::Long
                },
            | Some(b'j') => {
                *i += 1;
                Length::IntMax
            },
            | Some(b'z') => {
                *i += 1;
                Length::SizeT
            },
            | Some(b't') => {
                *i += 1;
                Length::PtrDiff
            },
            | Some(b'L') => {
                *i += 1;
                Length::LongDouble
            },
            | Some(b'q') => {
                *i += 1;
                Length::Quad
            },
            | Some(b'm') if scanf => {
                *i += 1;
                Length::MAllocate
            },
            | Some(b'I') => {
                if *i + 2 < bytes.len() + 1 && at(*i + 1).is_some() && at(*i + 2).is_some() {
                    if at(*i + 1) == Some(b'6') && at(*i + 2) == Some(b'4') {
                        *i += 3;
                        spec.length = Length::Int64;
                        spec.length_at = start;
                        spec.length_len = 3;
                        return true;
                    }
                    if scanf {
                        return false;
                    }
                    if at(*i + 1) == Some(b'3') && at(*i + 2) == Some(b'2') {
                        *i += 3;
                        spec.length = Length::Int32;
                        spec.length_at = start;
                        spec.length_len = 3;
                        return true;
                    }
                }
                *i += 1;
                Length::Int3264
            },
            | Some(b'w') => {
                *i += 1;
                Length::Wide
            },
            | _ => return false,
        };
        spec.length = length;
        spec.length_at = start;
        spec.length_len = *i - start;
        true
    }

    /// Reports an invalid conversion specifier and covers its argument;
    /// whether the reading goes on (`HandleInvalidConversionSpecifier`).
    fn clang_invalid_specifier(&mut self, r: &mut Reading<'_, 'tu>, spec: &Spec) -> bool {
        let keep_going = if spec.index < r.covered.len() {
            r.covered[spec.index] = true;
            true
        } else {
            false
        };
        let text = &r.bytes[spec.position..spec.end];
        let mut out = ArenaString::new_in(self.scratch);
        let letter = spec.letter;
        if letter.is_ascii_graphic() || letter == b' ' {
            out.push(letter as char);
        } else {
            use std::fmt::Write;
            let code = std::str::from_utf8(text)
                .ok()
                .and_then(|text| text.chars().next())
                .map_or(u32::from(letter), u32::from);
            let _ = if code < 256 {
                write!(out, "\\x{code:02x}")
            } else if code <= 0xFFFF {
                write!(out, "\\u{code:04x}")
            } else {
                write!(out, "\\U{code:08x}")
            };
        }
        let words = format_in!(
            self.scratch,
            "invalid conversion specifier '{}'",
            out.as_str()
        );
        self.clang_string_warning(r, G::FormatInvalidSpecifier, words, spec.position);
        keep_going
    }

    /// Checks a printf specification; whether the reading goes on
    /// (`CheckPrintfHandler::HandlePrintfSpecifier`).
    fn clang_printf_spec(&mut self, r: &mut Reading<'_, 'tu>, spec: Spec) -> bool {
        if spec.conversion.consumes() {
            if r.at_first {
                r.at_first = false;
                r.positional = spec.positional;
            } else if r.positional != spec.positional {
                self.clang_string_warning(
                    r,
                    G::Format,
                    "cannot mix positional and non-positional arguments in format string",
                    spec.position,
                );
                return false;
            }
        }
        if !self.clang_star(r, spec.width, false) || !self.clang_star(r, spec.precision, true) {
            return false;
        }
        if !spec.conversion.consumes() {
            return true;
        }
        if spec.index < r.covered.len() {
            r.covered[spec.index] = true;
        }
        if matches!(spec.conversion, Conversion::Os | Conversion::Object) {
            return self.clang_invalid_specifier(r, &spec);
        }
        let letter = spec.letter as char;
        // Widths and precisions that mean nothing here.
        if spec.conversion == Conversion::Written && spec.width != Amount::NotSpecified {
            self.clang_amount_warning(r, spec.width, "field width", letter);
        }
        if spec.precision != Amount::NotSpecified && !valid_precision(spec.conversion) {
            self.clang_amount_warning(r, spec.precision, "precision", letter);
        }
        // Flags.
        let flags: [Flag; 6] = [
            (spec.thousands, '\'', |c| {
                matches!(c, Conversion::Signed | Conversion::Unsigned) || c == Conversion::Double
            }),
            (spec.zeros, '0', valid_zeros),
            (spec.plus, '+', valid_sign),
            (spec.space, ' ', valid_sign),
            (spec.alternative, '#', valid_alternative),
            (spec.left, '-', |c| c != Conversion::Written),
        ];
        for (at, flag, valid) in flags {
            if let Some(at) = at
                && !(valid(spec.conversion)
                    && (flag != '\''
                        || spec.letter != b'a'
                            && spec.letter != b'A'
                            && spec.letter != b'e'
                            && spec.letter != b'E'))
            {
                let words = format_in!(
                    self.scratch,
                    "flag '{flag}' results in undefined behavior with '{letter}' conversion \
                     specifier"
                );
                self.clang_string_warning(r, G::Format, words, at);
            }
        }
        if let (Some(space), Some(_)) = (spec.space, spec.plus) {
            self.clang_string_warning(
                r,
                G::Format,
                "flag ' ' is ignored when flag '+' is present",
                space,
            );
        }
        if let (Some(zeros), Some(_)) = (spec.zeros, spec.left) {
            self.clang_string_warning(
                r,
                G::Format,
                "flag '0' is ignored when flag '-' is present",
                zeros,
            );
        }
        if !valid_length(spec.length, spec.conversion, false, r.msvcrt) {
            self.clang_length_warning(r, &spec);
        }
        if r.call.takes_va_list() {
            return true;
        }
        if !self.clang_enough(r, &spec) {
            return false;
        }
        let Some(&argument) = r.call.data().get(spec.index) else {
            return true;
        };
        let want = printf_type(&spec, r.msvcrt);
        if want.want == Want::Invalid || want.want == Want::Unknown {
            return true;
        }
        self.clang_printf_argument(r, &spec, want, argument);
        true
    }

    /// Checks a `*` width or precision's argument (`HandleAmount`);
    /// whether the reading goes on.
    fn clang_star(&mut self, r: &mut Reading<'_, 'tu>, amount: Amount, precision: bool) -> bool {
        let Amount::Argument { index, start } = amount else {
            return true;
        };
        if r.call.takes_va_list() {
            return true;
        }
        let what = if precision { "precision" } else { "width" };
        let Some(&argument) = r.call.data().get(index) else {
            let star = if precision { ".*" } else { "*" };
            let words = format_in!(
                self.scratch,
                "'{star}' specified field {what} is missing a matching 'int' argument"
            );
            self.clang_string_warning(r, G::Format, words, start);
            return false;
        };
        r.covered[index] = true;
        let ty = match r.call.fixed_parameter(index) {
            | Some(ty) => ty,
            | None => self.clang_value_type(argument),
        };
        if self.clang_matches(arg(specific(S::Int)), ty) == Fit::Mismatch {
            let Some(spelling) = self.clang_type_name(argument, ty) else {
                return false;
            };
            let words = format_in!(
                self.scratch,
                "field {what} should have type 'int', but argument has type {spelling}"
            );
            self.clang_string_warning(r, G::Format, words, start);
            return false;
        }
        true
    }

    /// Whether the specification's argument exists (`CheckNumArgs`).
    fn clang_enough(&mut self, r: &mut Reading<'_, 'tu>, spec: &Spec) -> bool {
        let count = r.call.data().len();
        if spec.index < count {
            return true;
        }
        let words = if spec.positional {
            format_in!(
                self.scratch,
                "data argument position '{}' exceeds the number of data arguments ({count})",
                spec.index + 1
            )
        } else {
            "more '%' conversions than data arguments"
        };
        let group = if spec.positional {
            G::Format
        } else {
            G::FormatInsufficientArgs
        };
        self.clang_string_warning(r, group, words, spec.position);
        false
    }

    /// Checks a printf argument against the type its specification wants
    /// (`checkFormatExpr`).
    fn clang_printf_argument(
        &mut self,
        r: &Reading<'_, 'tu>,
        spec: &Spec,
        want: ArgType,
        argument: &'tu Expression<'tu>,
    ) {
        let info = self.expression_info(argument);
        // A truth value printed as a character makes no sense.
        if spec.conversion == Conversion::Char
            && let Some(site) = self.clang_boolean(argument)
        {
            let text = std::str::from_utf8(&r.bytes[spec.start..spec.end]).unwrap_or("%c");
            let words = format_in!(
                self.scratch,
                "using '{text}' format specifier, but argument has boolean value"
            );
            let leaf = r.leaf;
            let note = (!leaf.in_call)
                .then(|| (FormatNote::DefinedHere, self.clang_byte(&leaf, spec.start)));
            self.format_warning(r.call.call.source_vectors, G::Format, words, site, note);
            return;
        }
        // A parameter's argument has the parameter's type, not promoted.
        let fixed = r.call.fixed_parameter(spec.index);
        let value_type = fixed.unwrap_or_else(|| self.decayed_type(argument));
        let promoted = match fixed {
            | Some(ty) => ty,
            | None => self.clang_promoted(argument, value_type),
        };
        let mut matched = self.clang_matches(want, promoted);
        let original = matched;
        if matched == Fit::Signedness {
            matched = Fit::Exact;
        }
        if matched == Fit::Exact {
            return;
        }
        let mut shown = promoted;
        let mut implicit = Fit::Mismatch;
        let promotion =
            fixed.is_none() && promoted != value_type.unqualified() && self.arithmetic(value_type);
        if promotion {
            shown = value_type;
            let int = self.types.scalar(S::Int);
            let unsigned = self.types.scalar(S::UnsignedInt);
            if promoted == int || promoted == unsigned {
                implicit = self.clang_matches(want, value_type);
                if original == Fit::Signedness && implicit != Fit::Signedness {
                    return;
                }
                if implicit == Fit::Signedness {
                    implicit = Fit::Exact;
                }
                if implicit == Fit::Exact {
                    return;
                }
            }
        } else if fixed.is_none()
            && let E::Constant(Constant::Char(_)) = Self::without_parentheses(argument).kind
            && info.ty == self.types.scalar(S::Int)
            && spec.length != Length::Char
            && info
                .integer
                .and_then(super::super::Integer::to_i128)
                .is_some_and(|value| (0..256).contains(&value))
        {
            shown = self.types.scalar(S::Char);
            if matched == Fit::Promotion {
                matched = Fit::Mismatch;
            }
        }
        if matched == Fit::Promotion {
            if implicit != Fit::PromotionConfusion && implicit != Fit::Confusion {
                return;
            }
            matched = Fit::Mismatch;
        }
        if implicit == Fit::Pedantic || implicit == Fit::Confusion {
            matched = implicit;
        }
        let group = match matched {
            | Fit::Mismatch => G::Format,
            | Fit::Pedantic => G::FormatPedantic,
            | Fit::Confusion => G::FormatTypeConfusion,
            | _ => return,
        };
        let (shown, underlying) = match self.types.nodes[shown.index] {
            | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum => {
                let compatible = self.types.tags[id].compatible.get();
                (self.types.scalar(compatible), true)
            },
            | _ => (shown, false),
        };
        let Some(wanted) = self.clang_want_name(want) else {
            return;
        };
        let Some(spelling) = self.clang_type_name(argument, shown) else {
            return;
        };
        let words = format_in!(
            self.scratch,
            "format specifies type {wanted} but the argument has {} {spelling}",
            if underlying {
                "underlying type"
            } else {
                "type"
            }
        );
        self.clang_argument_warning(r, group, argument, spec.start, words);
    }

    /// Where Clang reports an argument known to be a truth value: one of
    /// type `_Bool`, a comparison or a logical operation, at its operator.
    fn clang_boolean(&self, argument: &'tu Expression<'tu>) -> Option<SourceVectors> {
        let e = Self::without_parentheses(argument);
        let operator = e.operator_source_vectors.unwrap_or(e.source_vectors);
        match e.kind {
            | E::Binary {
                operator:
                    BinaryOperator::LessThan
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::LessThanOrEqual
                    | BinaryOperator::GreaterThanOrEqual
                    | BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::LogicalAnd
                    | BinaryOperator::LogicalOr,
                ..
            }
            | E::Unary {
                operator: UnaryOperator::LogicalNot,
                ..
            } => Some(operator),
            | _ if self.types.nodes[self.types.non_atomic(self.expression_info(e).ty).index]
                == TypeKind::Scalar(S::Bool) =>
                Some(e.source_vectors),
            | _ => None,
        }
    }

    /// Reads the next scanf specification (`ParseScanfSpecifier`).
    fn clang_parse_scanf(
        &mut self,
        r: &mut Reading<'_, 'tu>,
        i: &mut usize,
        next_index: &mut usize,
    ) -> Parsed {
        let bytes = r.bytes;
        let end = bytes.len();
        let mut start = None;
        while *i < end {
            let c = bytes[*i];
            if c == 0 {
                self.clang_string_warning(
                    r,
                    G::Format,
                    "format string contains '\\0' within the string body",
                    *i,
                );
                *i = end;
                return Parsed::Stop;
            }
            *i += 1;
            if c == b'%' {
                start = Some(*i - 1);
                break;
            }
        }
        let Some(start) = start else {
            return Parsed::Nothing;
        };
        let incomplete = |analyzer: &mut Self, r: &Reading<'_, 'tu>| {
            analyzer.clang_string_warning(r, G::Format, "incomplete format specifier", start);
            Parsed::Stop
        };
        if *i == end {
            return incomplete(self, r);
        }
        let mut spec = Spec {
            start,
            end: start,
            positional: false,
            index: 0,
            thousands: None,
            left: None,
            plus: None,
            space: None,
            alternative: None,
            zeros: None,
            suppressed: false,
            width: Amount::NotSpecified,
            precision: Amount::NotSpecified,
            length: Length::None,
            length_at: 0,
            length_len: 0,
            conversion: Conversion::Invalid,
            position: 0,
            letter: 0,
        };
        match self.clang_position(r, &mut spec, start, i) {
            | Some(true) => return Parsed::Stop,
            | Some(false) => {},
            | None => return incomplete(self, r),
        }
        if *i == end {
            return incomplete(self, r);
        }
        if bytes[*i] == b'*' {
            spec.suppressed = true;
            *i += 1;
            if *i == end {
                return incomplete(self, r);
            }
        }
        let first = *i;
        let (value, after, _) = amount_digits(bytes, first);
        *i = after;
        if *i > first {
            spec.width = Amount::Constant {
                value,
                start: first,
                length: *i - first,
            };
            if *i == end {
                return incomplete(self, r);
            }
        }
        if Self::clang_length(r, &mut spec, i, true) && *i == end {
            return incomplete(self, r);
        }
        if bytes[*i] == 0 {
            self.clang_string_warning(
                r,
                G::Format,
                "format string contains '\\0' within the string body",
                *i,
            );
            return Parsed::Stop;
        }
        spec.position = *i;
        spec.letter = bytes[*i];
        *i += 1;
        spec.conversion = match spec.letter {
            | b'%' => Conversion::Percent,
            | b'd' | b'i' => Conversion::Signed,
            | b'b' => Conversion::Binary,
            | b'u' => Conversion::Unsigned,
            | b'o' => Conversion::Octal,
            | b'x' | b'X' => Conversion::Hex,
            | b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => Conversion::Double,
            | b'c' => Conversion::Char,
            | b'C' => Conversion::WideChar,
            | b's' => Conversion::String,
            | b'S' => Conversion::WideString,
            | b'[' => Conversion::ScanList,
            | b'p' => Conversion::Pointer,
            | b'n' => Conversion::Written,
            | _ => Conversion::Invalid,
        };
        if spec.conversion == Conversion::ScanList {
            // `ParseScanList`.
            let list = *i - 1;
            let incomplete_list = |analyzer: &mut Self, r: &Reading<'_, 'tu>, at: usize| {
                let _ = list;
                analyzer.clang_string_warning(
                    r,
                    G::Format,
                    "no closing ']' for '%[' in scanf format string",
                    at,
                );
                Parsed::Stop
            };
            if *i == end {
                return incomplete_list(self, r, *i - 1);
            }
            if bytes[*i] == b']' {
                *i += 1;
                if *i == end {
                    return incomplete_list(self, r, *i - 1);
                }
            }
            if *i + 1 < end && bytes[*i] == b'^' && bytes[*i + 1] == b']' {
                *i += 2;
                if *i == end {
                    return incomplete_list(self, r, *i - 1);
                }
            }
            while bytes[*i] != b']' {
                *i += 1;
                if *i == end {
                    return incomplete_list(self, r, *i - 1);
                }
            }
            *i += 1;
        }
        if spec.conversion.consumes() && !spec.suppressed && !spec.positional {
            spec.index = *next_index;
            *next_index += 1;
        }
        spec.end = *i;
        if spec.conversion == Conversion::Invalid {
            *i = utf8_end(bytes, spec.position);
            spec.end = *i;
            return if self.clang_invalid_specifier(r, &spec) {
                Parsed::Nothing
            } else {
                Parsed::Stop
            };
        }
        Parsed::Spec(spec)
    }

    /// Checks a scanf specification; whether the reading goes on
    /// (`CheckScanfHandler::HandleScanfSpecifier`).
    fn clang_scanf_spec(&mut self, r: &mut Reading<'_, 'tu>, spec: Spec) -> bool {
        let consumes = spec.conversion.consumes() && !spec.suppressed;
        if consumes {
            if r.at_first {
                r.at_first = false;
                r.positional = spec.positional;
            } else if r.positional != spec.positional {
                self.clang_string_warning(
                    r,
                    G::Format,
                    "cannot mix positional and non-positional arguments in format string",
                    spec.position,
                );
                return false;
            }
        }
        if let Amount::Constant {
            value: 0, start, ..
        } = spec.width
        {
            self.clang_string_warning(
                r,
                G::Format,
                "zero field width in scanf format string is unused",
                start,
            );
        }
        if !consumes {
            return true;
        }
        if spec.index < r.covered.len() {
            r.covered[spec.index] = true;
        }
        if !valid_length(spec.length, spec.conversion, true, r.msvcrt) {
            self.clang_length_warning(r, &spec);
        }
        if r.call.takes_va_list() {
            return true;
        }
        if !self.clang_enough(r, &spec) {
            return false;
        }
        let Some(&argument) = r.call.data().get(spec.index) else {
            return true;
        };
        let want = scanf_type(&spec, r.msvcrt);
        if matches!(want.want, Want::Invalid | Want::Unknown) {
            return true;
        }
        let ty = r
            .call
            .fixed_parameter(spec.index)
            .unwrap_or_else(|| self.decayed_type(argument));
        let mut matched = self.clang_matches(want, ty);
        if matched == Fit::Signedness {
            matched = Fit::Exact;
        }
        // Clang's scanf check has no words for type confusion: it is a
        // mismatch like any other.
        let group = match matched {
            | Fit::Exact => return true,
            | Fit::Pedantic => G::FormatPedantic,
            | _ => G::Format,
        };
        let (Some(wanted), Some(spelling)) = (
            self.clang_want_name(want),
            self.clang_type_name(argument, ty),
        ) else {
            return true;
        };
        let words = format_in!(
            self.scratch,
            "format specifies type {wanted} but the argument has type {spelling}"
        );
        self.clang_argument_warning(r, group, argument, spec.start, words);
        true
    }

    /// Reports a length modifier that means nothing with its conversion
    /// (`HandleInvalidLengthModifier`).
    fn clang_length_warning(&mut self, r: &Reading<'_, 'tu>, spec: &Spec) {
        let length =
            std::str::from_utf8(&r.bytes[spec.length_at..spec.length_at + spec.length_len])
                .unwrap_or("");
        let words = format_in!(
            self.scratch,
            "length modifier '{length}' results in undefined behavior or no effect with '{}' \
             conversion specifier",
            spec.letter as char
        );
        // Clang corrects `L` and `q` on an integer conversion to `ll`.
        let corrects = matches!(spec.length, Length::LongDouble | Length::Quad)
            && matches!(
                spec.conversion,
                Conversion::Signed
                    | Conversion::Unsigned
                    | Conversion::Binary
                    | Conversion::Octal
                    | Conversion::Hex
                    | Conversion::Written
            );
        if corrects && r.leaf.in_call {
            let place = self.clang_byte(&r.leaf, spec.length_at);
            self.format_warning(
                r.call.call.source_vectors,
                G::Format,
                words,
                place,
                Some((FormatNote::UseLongLong, place)),
            );
            return;
        }
        self.clang_string_warning(r, G::Format, words, spec.length_at);
    }

    /// Reports a width or precision a conversion does not take
    /// (`HandleInvalidAmount`).
    fn clang_amount_warning(
        &mut self,
        r: &Reading<'_, 'tu>,
        amount: Amount,
        what: &str,
        letter: char,
    ) {
        let at = match amount {
            | Amount::Constant { start, .. } | Amount::Argument { start, .. } => start,
            | Amount::NotSpecified => return,
        };
        let words = format_in!(
            self.scratch,
            "{what} used with '{letter}' conversion specifier, resulting in undefined behavior"
        );
        self.clang_string_warning(r, G::Format, words, at);
    }

    /// The type of a printf argument after the default argument
    /// promotions, unqualified.
    fn clang_promoted(&mut self, argument: &'tu Expression<'tu>, ty: TypeId) -> TypeId {
        let info = self.expression_info(argument);
        if self.integer_type(ty).is_some() || matches!(self.types.nodes[ty.index], TypeKind::Tag(_))
        {
            return self.promote(info, ty).unqualified();
        }
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(S::Float | S::Float16 | S::BFloat16) => self.types.scalar(S::Double),
            | _ => ty.unqualified(),
        }
    }

    /// The type an argument has before its promotion, without recording
    /// a conversion.
    fn clang_value_type(&mut self, argument: &'tu Expression<'tu>) -> TypeId {
        let ty = self.decayed_type(argument);
        self.clang_promoted(argument, ty)
    }

    /// How well `ty` matches `want` (`ArgType::matchesType`).
    fn clang_matches(&mut self, want: ArgType, ty: TypeId) -> Fit {
        let mut ty = self.types.non_atomic(ty);
        if want.pointer {
            let Some(target) = self.pointer_target(ty) else {
                return Fit::Mismatch;
            };
            if target.qualifiers.contains(TypeQualifiers::CONST) {
                return Fit::Mismatch;
            }
            ty = target;
        }
        let pointer = want.pointer;
        let underlying = |analyzer: &Self, ty: TypeId| match analyzer.types.nodes[ty.index] {
            | TypeKind::Tag(id) if analyzer.types.tags[id].kind == TagKind::Enum =>
                analyzer.types.tags[id]
                    .complete
                    .get()
                    .then(|| analyzer.types.tags[id].compatible.get()),
            | TypeKind::Scalar(scalar) => Some(scalar),
            | _ => None,
        };
        match want.want {
            | Want::Invalid | Want::Unknown => Fit::Exact,
            | Want::AnyChar => {
                let Some(scalar) = underlying(self, ty) else {
                    return Fit::Mismatch;
                };
                match scalar {
                    | S::Char | S::SignedChar | S::UnsignedChar => Fit::Exact,
                    | S::Bool if !pointer => Fit::Exact,
                    | S::Int | S::UnsignedInt if !pointer => Fit::Promotion,
                    | S::Short | S::UnsignedShort if !pointer => Fit::PromotionConfusion,
                    | _ => Fit::Mismatch,
                }
            },
            | Want::Specific(t) => {
                let wanted = self.clang_scalar(t);
                let Some(have) = (match self.types.nodes[ty.index] {
                    | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum =>
                        Some(if self.types.tags[id].complete.get() {
                            self.types.tags[id].compatible.get()
                        } else {
                            S::Int
                        }),
                    | TypeKind::Scalar(scalar) => Some(scalar),
                    | _ => None,
                }) else {
                    return Fit::Mismatch;
                };
                if have == wanted {
                    return Fit::Exact;
                }
                let char_signed = self.types.target.char_signed;
                // Plain `char` is `Char_S` or `Char_U` by the target.
                let signed_char = matches!(have, S::SignedChar) || (have == S::Char && char_signed);
                let unsigned_char =
                    matches!(have, S::UnsignedChar) || (have == S::Char && !char_signed);
                if have == S::Bool && pointer && matches!(wanted, S::UnsignedChar | S::SignedChar) {
                    return Fit::Mismatch;
                }
                if have == S::Bool || signed_char {
                    if matches!(wanted, S::UnsignedShort | S::Short) {
                        return Fit::Confusion;
                    }
                    if wanted == S::UnsignedChar {
                        return Fit::Signedness;
                    }
                    if wanted == S::SignedChar {
                        return Fit::Exact;
                    }
                } else if unsigned_char {
                    if matches!(wanted, S::UnsignedShort | S::Short) {
                        return Fit::Confusion;
                    }
                    if wanted == S::UnsignedChar {
                        return Fit::Exact;
                    }
                    if wanted == S::SignedChar {
                        return Fit::Signedness;
                    }
                }
                let pairs = [
                    (S::Short, S::UnsignedShort),
                    (S::Int, S::UnsignedInt),
                    (S::Long, S::UnsignedLong),
                    (S::LongLong, S::UnsignedLongLong),
                ];
                if pairs
                    .iter()
                    .any(|&(a, b)| (have == a && wanted == b) || (have == b && wanted == a))
                {
                    return Fit::Signedness;
                }
                if !pointer {
                    match have {
                        | S::Bool if matches!(wanted, S::Int | S::UnsignedInt) =>
                            return Fit::Promotion,
                        | S::Int | S::UnsignedInt
                            if matches!(
                                wanted,
                                S::SignedChar | S::UnsignedChar | S::Short | S::UnsignedShort
                            ) || wanted == self.types.target.wchar_t =>
                            return Fit::Promotion,
                        | S::Char if !char_signed && wanted == S::UnsignedInt =>
                            return Fit::Promotion,
                        | S::Char if !char_signed && wanted == S::UnsignedShort =>
                            return Fit::PromotionConfusion,
                        | S::Char if char_signed && wanted == S::Int => return Fit::Promotion,
                        | S::Char if char_signed && wanted == S::Short =>
                            return Fit::PromotionConfusion,
                        | S::Float | S::Float16 if wanted == S::Double => return Fit::Promotion,
                        | S::Short | S::UnsignedShort
                            if matches!(wanted, S::SignedChar | S::UnsignedChar) =>
                            return Fit::PromotionConfusion,
                        | _ => {},
                    }
                }
                Fit::Mismatch
            },
            | Want::CString => {
                let Some(target) = self.pointer_target(ty) else {
                    return Fit::Mismatch;
                };
                match self.types.nodes[target.index] {
                    | TypeKind::Scalar(S::Char | S::SignedChar | S::UnsignedChar) => Fit::Exact,
                    | _ => Fit::Mismatch,
                }
            },
            | Want::WideString => {
                let Some(target) = self.pointer_target(ty) else {
                    return Fit::Mismatch;
                };
                if self.types.nodes[target.index] == TypeKind::Scalar(self.types.target.wchar_t) {
                    Fit::Exact
                } else {
                    Fit::Mismatch
                }
            },
            | Want::WideInt => {
                let wide_int = self.types.target.wint_t;
                let Some(have) = underlying(self, ty) else {
                    return Fit::Mismatch;
                };
                if have == wide_int {
                    return Fit::Exact;
                }
                let promoted = match have {
                    | S::Bool
                    | S::Char
                    | S::SignedChar
                    | S::UnsignedChar
                    | S::Short
                    | S::UnsignedShort => S::Int,
                    | other => other,
                };
                if super::gcc::unsigned_of(promoted) == wide_int || promoted == wide_int {
                    Fit::Exact
                } else {
                    Fit::Mismatch
                }
            },
            | Want::CPointer => {
                if let Some(target) = self.pointer_target(ty) {
                    let target_kind = self.types.nodes[target.index];
                    if target_kind == TypeKind::Scalar(S::Void)
                        || (!pointer
                            && matches!(
                                target_kind,
                                TypeKind::Scalar(S::Char | S::SignedChar | S::UnsignedChar)
                            ))
                    {
                        return Fit::Exact;
                    }
                    return Fit::Pedantic;
                }
                if self.nullptr_type(ty) {
                    return Fit::Exact;
                }
                Fit::Mismatch
            },
        }
    }

    /// The scalar a specific wanted type is on the target.
    fn clang_scalar(&self, ty: Ty) -> Scalar {
        let target = self.types.target;
        match ty {
            | Ty::Scalar(scalar) => scalar,
            | Ty::Size => target.size_t,
            | Ty::SignedSize => super::gcc::signed_of(target.size_t),
            | Ty::PtrDiff => target.ptrdiff_t,
            | Ty::UnsignedPtrDiff => super::gcc::unsigned_of(target.ptrdiff_t),
            | Ty::IntMax => target.intmax_t,
            | Ty::UintMax => target.uintmax_t,
            | Ty::WideChar => target.wchar_t,
            | Ty::Int64 => S::LongLong,
            | Ty::UnsignedInt64 => S::UnsignedLongLong,
            | Ty::Int32 => S::Int,
            | Ty::UnsignedInt32 => S::UnsignedInt,
        }
    }

    /// Clang's words for a wanted type (`getRepresentativeTypeName`).
    fn clang_want_name(&mut self, want: ArgType) -> Option<&'tu str> {
        let (base, name): (Scalar, Option<&str>) = match want.want {
            | Want::Invalid | Want::Unknown => return None,
            | Want::AnyChar => (S::Char, None),
            | Want::CString => {
                let text = if want.pointer {
                    "'char **'"
                } else {
                    "'char *'"
                };
                return Some(text);
            },
            | Want::WideString => {
                let wchar = self.types.scalar(self.types.target.wchar_t);
                let pointer = self.types.intern(TypeKind::Pointer(wchar));
                let spelling = self.type_spelling(pointer, false)?;
                let star = if want.pointer { "*" } else { "" };
                return Some(format_in!(
                    self.types.tu,
                    "'wchar_t *{star}' (aka '{spelling}{star}')"
                ));
            },
            | Want::WideInt => (self.types.target.wint_t, Some("wint_t")),
            | Want::CPointer => {
                return Some(if want.pointer {
                    "'void **'"
                } else {
                    "'void *'"
                });
            },
            | Want::Specific(t) => (
                self.clang_scalar(t),
                match t {
                    | Ty::Scalar(_) => None,
                    | Ty::Size => Some("size_t"),
                    | Ty::SignedSize => Some("signed size_t"),
                    | Ty::PtrDiff => Some("ptrdiff_t"),
                    | Ty::UnsignedPtrDiff => Some("unsigned ptrdiff_t"),
                    | Ty::IntMax => Some("intmax_t"),
                    | Ty::UintMax => Some("uintmax_t"),
                    | Ty::WideChar => Some("wchar_t"),
                    | Ty::Int64 => Some("__int64"),
                    | Ty::UnsignedInt64 => Some("unsigned __int64"),
                    | Ty::Int32 => Some("__int32"),
                    | Ty::UnsignedInt32 => Some("unsigned __int32"),
                },
            ),
        };
        let mut ty = self.types.scalar(base);
        if want.pointer {
            ty = self.types.intern(TypeKind::Pointer(ty));
        }
        let spelling = self.type_spelling(ty, false)?;
        Some(match name {
            | Some(name) => {
                let alias = if want.pointer {
                    format_in!(self.types.tu, "{name} *")
                } else {
                    name
                };
                if alias == spelling {
                    format_in!(self.types.tu, "'{spelling}'")
                } else {
                    format_in!(self.types.tu, "'{alias}' (aka '{spelling}')")
                }
            },
            | None => format_in!(self.types.tu, "'{spelling}'"),
        })
    }

    /// Clang's words for an argument's type: its spelling, and for a
    /// `sizeof` or a pointer difference the typedef Clang 23 gives it.
    fn clang_type_name(&mut self, argument: &'tu Expression<'tu>, ty: TypeId) -> Option<&'tu str> {
        let spelling = self.type_spelling(ty, false)?;
        let sugar = match Self::without_parentheses(argument).kind {
            | E::SizeofType(_) | E::SizeofExpr(_) | E::AlignofType(_) | E::AlignofExpr(_)
                if ty.unqualified() == self.types.scalar(self.types.target.size_t) =>
                Some("__size_t"),
            | E::Binary {
                operator: BinaryOperator::Subtraction,
                left_expression,
                right_expression,
            } if self
                .pointer_target(self.expression_info(left_expression).ty)
                .is_some()
                && self
                    .pointer_target(self.expression_info(right_expression).ty)
                    .is_some() =>
                Some("__ptrdiff_t"),
            | _ => None,
        };
        Some(match sugar {
            | Some(name) => format_in!(self.types.tu, "'{name}' (aka '{spelling}')"),
            | None => format_in!(self.types.tu, "'{spelling}'"),
        })
    }

    /// Reports `words` about the byte `offset` of the string, as Clang's
    /// `EmitFormatDiagnostic` with a string location places it.
    fn clang_string_warning(&mut self, r: &Reading<'_, 'tu>, group: G, words: &str, offset: usize) {
        let leaf = r.leaf;
        let place = self.clang_byte(&leaf, offset);
        self.clang_string_warning_at(r.call, &leaf, group, words, place);
    }

    fn clang_string_warning_at(
        &mut self,
        call: &FormatCall<'_, 'tu>,
        leaf: &Leaf<'tu>,
        group: G,
        words: &str,
        place: SourceVectors,
    ) {
        if leaf.in_call {
            self.format_warning(call.call.source_vectors, group, words, place, None);
        } else {
            self.format_warning(
                call.call.source_vectors,
                group,
                words,
                call.format.source_vectors,
                Some((FormatNote::DefinedHere, place)),
            );
        }
    }

    /// Reports `words` about a data argument, noting the specification
    /// that begins at `start` when the string is not the call's.
    fn clang_argument_warning(
        &mut self,
        r: &Reading<'_, 'tu>,
        group: G,
        argument: &'tu Expression<'tu>,
        start: usize,
        words: &str,
    ) {
        let leaf = r.leaf;
        let note =
            (!leaf.in_call).then(|| (FormatNote::DefinedHere, self.clang_byte(&leaf, start)));
        self.format_warning(
            r.call.call.source_vectors,
            group,
            words,
            argument.source_vectors,
            note,
        );
    }

    /// Where Clang places the byte `offset` of the string: there, unless a
    /// macro wrote the string or its places cannot be told.
    fn clang_byte(&mut self, leaf: &Leaf<'tu>, offset: usize) -> SourceVectors {
        if leaf.spelled == Spelled::MacroBody {
            return leaf.literal.source_vectors;
        }
        match self
            .byte_positions(leaf.literal)
            .and_then(|positions| positions.get(leaf.offset + offset).copied())
        {
            | Some(position) => self.byte_location(position),
            | None => leaf.literal.source_vectors,
        }
    }
}

/// A format the caller passes on.
enum Caller {
    /// The caller's own format parameter, which its `format` attribute
    /// covers.
    Attributed,
    /// Clang's suggestion of `format(kind, format, first)` for the caller.
    Suggest(usize, usize, usize),
    Other,
}

/// A flag's place in the specification, its character, and the
/// conversions it means something with.
type Flag = (Option<usize>, char, fn(Conversion) -> bool);

/// What reading one specification gave.
enum Parsed {
    Nothing,
    Spec(Spec),
    Stop,
}

/// The number written at `at`, its end and its length, accumulated as
/// Clang's `ParseAmount` does in an `unsigned`, which wraps.
fn amount_digits(bytes: &[u8], at: usize) -> (u32, usize, usize) {
    let mut value: u32 = 0;
    let mut end = at;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        value = value
            .wrapping_mul(10)
            .wrapping_add(u32::from(bytes[end] - b'0'));
        end += 1;
    }
    (value, end, end - at)
}

/// The end of the UTF-8 sequence beginning at `at`, as
/// `ParseUTF8InvalidSpecifier` takes an invalid specifier's length.
fn utf8_end(bytes: &[u8], at: usize) -> usize {
    let length = match bytes[at] {
        | 0xF0.. => 4,
        | 0xE0.. => 3,
        | 0xC0.. => 2,
        | _ => 1,
    };
    (at + length).min(bytes.len())
}

/// Whether a conversion takes a precision (`hasValidPrecision`).
const fn valid_precision(conversion: Conversion) -> bool {
    matches!(
        conversion,
        Conversion::Signed
            | Conversion::Unsigned
            | Conversion::Binary
            | Conversion::Octal
            | Conversion::Hex
            | Conversion::Double
            | Conversion::String
            | Conversion::WideString
            | Conversion::Os
    )
}

/// `hasValidLeadingZeros`.
const fn valid_zeros(conversion: Conversion) -> bool {
    matches!(
        conversion,
        Conversion::Signed
            | Conversion::Unsigned
            | Conversion::Binary
            | Conversion::Octal
            | Conversion::Hex
            | Conversion::Double
    )
}

/// `hasValidPlusPrefix` and `hasValidSpacePrefix`.
const fn valid_sign(conversion: Conversion) -> bool {
    matches!(conversion, Conversion::Signed | Conversion::Double)
}

/// `hasValidAlternativeForm`.
const fn valid_alternative(conversion: Conversion) -> bool {
    matches!(
        conversion,
        Conversion::Binary | Conversion::Octal | Conversion::Hex | Conversion::Double
    )
}

/// `hasValidLengthModifier` on a Linux target, or with `msvcrt` on a
/// Windows one, whose C runtime has Microsoft's modifiers (`isOSMSVCRT`,
/// which MinGW is too) and no `L` on an integer (`isOSWindows`).
const fn valid_length(length: Length, conversion: Conversion, scanf: bool, msvcrt: bool) -> bool {
    use Conversion as C;
    let integer = matches!(
        conversion,
        C::Signed | C::Unsigned | C::Binary | C::Octal | C::Hex | C::Written
    );
    let wide = matches!(
        conversion,
        C::Char | C::WideChar | C::String | C::WideString | C::Counted
    );
    match length {
        | Length::None => true,
        | Length::Short => integer || (msvcrt && wide),
        | Length::Char
        | Length::LongLong
        | Length::Quad
        | Length::IntMax
        | Length::SizeT
        | Length::PtrDiff => integer,
        | Length::Long =>
            integer
                || matches!(
                    conversion,
                    C::Double | C::Char | C::String | C::ScanList | C::Counted
                ),
        | Length::LongDouble =>
            matches!(conversion, C::Double)
                || (!msvcrt && matches!(conversion, C::Signed | C::Unsigned | C::Octal | C::Hex)),
        | Length::MAllocate =>
            scanf
                && matches!(
                    conversion,
                    C::Char | C::WideChar | C::String | C::WideString | C::ScanList
                ),
        | Length::Int64 | Length::Int32 | Length::Int3264 =>
            msvcrt && matches!(conversion, C::Signed | C::Unsigned | C::Octal | C::Hex),
        | Length::Wide => msvcrt && wide,
    }
}

/// The type a printf specification wants (`PrintfSpecifier::getArgType`).
fn printf_type(spec: &Spec, msvcrt: bool) -> ArgType {
    use Conversion as C;
    use Length as L;
    let length = spec.length;
    match spec.conversion {
        | C::Char => match length {
            | L::None => arg(specific(S::Int)),
            | L::Long | L::Wide => arg(Want::WideInt),
            | L::Short if msvcrt => arg(specific(S::Int)),
            | _ => arg(Want::Invalid),
        },
        | C::Signed => arg(match length {
            | L::LongDouble | L::LongLong | L::Quad => specific(S::LongLong),
            | L::None => specific(S::Int),
            | L::Int32 => Want::Specific(Ty::Int32),
            | L::Char => Want::AnyChar,
            | L::Short => specific(S::Short),
            | L::Long => specific(S::Long),
            | L::Int64 | L::Int3264 => Want::Specific(Ty::Int64),
            | L::IntMax => Want::Specific(Ty::IntMax),
            | L::SizeT => Want::Specific(Ty::SignedSize),
            | L::PtrDiff => Want::Specific(Ty::PtrDiff),
            | L::Wide | L::MAllocate => Want::Invalid,
        }),
        | C::Unsigned | C::Binary | C::Octal | C::Hex => arg(match length {
            | L::LongDouble | L::LongLong | L::Quad => specific(S::UnsignedLongLong),
            | L::None => specific(S::UnsignedInt),
            | L::Int32 => Want::Specific(Ty::UnsignedInt32),
            | L::Char => specific(S::UnsignedChar),
            | L::Short => specific(S::UnsignedShort),
            | L::Long => specific(S::UnsignedLong),
            | L::Int64 | L::Int3264 => Want::Specific(Ty::UnsignedInt64),
            | L::IntMax => Want::Specific(Ty::UintMax),
            | L::SizeT => Want::Specific(Ty::Size),
            | L::PtrDiff => Want::Specific(Ty::UnsignedPtrDiff),
            | L::Wide | L::MAllocate => Want::Invalid,
        }),
        | C::Double => arg(if length == L::LongDouble {
            specific(S::LongDouble)
        } else {
            specific(S::Double)
        }),
        | C::Written => match length {
            | L::None => pointer_to(specific(S::Int)),
            | L::Char => pointer_to(specific(S::SignedChar)),
            | L::Short => pointer_to(specific(S::Short)),
            | L::Long => pointer_to(specific(S::Long)),
            | L::LongLong | L::Quad => pointer_to(specific(S::LongLong)),
            | L::IntMax => pointer_to(Want::Specific(Ty::IntMax)),
            | L::SizeT => pointer_to(Want::Specific(Ty::SignedSize)),
            | L::PtrDiff => pointer_to(Want::Specific(Ty::PtrDiff)),
            | L::LongDouble => arg(Want::Unknown),
            | _ => arg(Want::Invalid),
        },
        | C::String => match length {
            | L::Long | L::Wide => arg(Want::WideString),
            | _ => arg(Want::CString),
        },
        | C::WideString if msvcrt && length == L::Short => arg(Want::CString),
        | C::WideString => arg(Want::WideString),
        | C::WideChar if msvcrt && length == L::Short => arg(specific(S::Int)),
        | C::WideChar => arg(Want::Specific(Ty::WideChar)),
        | C::Pointer | C::Os => arg(Want::CPointer),
        | _ => arg(Want::Unknown),
    }
}

/// The type a scanf specification wants (`ScanfSpecifier::getArgType`).
fn scanf_type(spec: &Spec, msvcrt: bool) -> ArgType {
    use Conversion as C;
    use Length as L;
    let length = spec.length;
    match spec.conversion {
        | C::Signed => match length {
            | L::None => pointer_to(specific(S::Int)),
            | L::Char => pointer_to(Want::AnyChar),
            | L::Short => pointer_to(specific(S::Short)),
            | L::Long => pointer_to(specific(S::Long)),
            | L::LongLong | L::Quad | L::LongDouble => pointer_to(specific(S::LongLong)),
            | L::Int64 => pointer_to(Want::Specific(Ty::Int64)),
            | L::IntMax => pointer_to(Want::Specific(Ty::IntMax)),
            | L::SizeT => pointer_to(Want::Specific(Ty::SignedSize)),
            | L::PtrDiff => pointer_to(Want::Specific(Ty::PtrDiff)),
            | _ => arg(Want::Invalid),
        },
        | C::Unsigned | C::Binary | C::Octal | C::Hex => match length {
            | L::None => pointer_to(specific(S::UnsignedInt)),
            | L::Char => pointer_to(specific(S::UnsignedChar)),
            | L::Short => pointer_to(specific(S::UnsignedShort)),
            | L::Long => pointer_to(specific(S::UnsignedLong)),
            | L::LongLong | L::Quad | L::LongDouble => pointer_to(specific(S::UnsignedLongLong)),
            | L::Int64 => pointer_to(Want::Specific(Ty::UnsignedInt64)),
            | L::IntMax => pointer_to(Want::Specific(Ty::UintMax)),
            | L::SizeT => pointer_to(Want::Specific(Ty::Size)),
            | L::PtrDiff => pointer_to(Want::Specific(Ty::UnsignedPtrDiff)),
            | _ => arg(Want::Invalid),
        },
        | C::Double => match length {
            | L::None => pointer_to(specific(S::Float)),
            | L::Long => pointer_to(specific(S::Double)),
            | L::LongDouble => pointer_to(specific(S::LongDouble)),
            | _ => arg(Want::Invalid),
        },
        | C::Char | C::String | C::ScanList => match length {
            | L::None => pointer_to(Want::AnyChar),
            | L::Long | L::Wide => pointer_to(Want::Specific(Ty::WideChar)),
            | L::MAllocate => pointer_to(Want::CString),
            | L::Short if msvcrt => pointer_to(Want::AnyChar),
            | _ => arg(Want::Invalid),
        },
        | C::WideChar | C::WideString => match length {
            | L::None | L::Wide => pointer_to(Want::Specific(Ty::WideChar)),
            | L::MAllocate => pointer_to(Want::WideString),
            | L::Short if msvcrt => pointer_to(Want::AnyChar),
            | _ => arg(Want::Invalid),
        },
        | C::Pointer => pointer_to(Want::CPointer),
        | C::Written => match length {
            | L::None => pointer_to(specific(S::Int)),
            | L::Char => pointer_to(specific(S::SignedChar)),
            | L::Short => pointer_to(specific(S::Short)),
            | L::Long => pointer_to(specific(S::Long)),
            | L::LongLong | L::Quad | L::Int64 => pointer_to(specific(S::LongLong)),
            | L::IntMax => pointer_to(Want::Specific(Ty::IntMax)),
            | L::SizeT => pointer_to(Want::Specific(Ty::SignedSize)),
            | L::PtrDiff => pointer_to(Want::Specific(Ty::PtrDiff)),
            | L::LongDouble => arg(Want::Unknown),
            | _ => arg(Want::Invalid),
        },
        | _ => arg(Want::Unknown),
    }
}
