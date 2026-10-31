//! Plain-language explanations of each operation, built from the values the
//! operation used and produced. The text names objects by their source
//! identifiers (`values[1]`), never by host addresses, so it is deterministic.
//!
//! Each builder returns an [`Event`] whose clauses are the rules the
//! operation applied (see `clauses.rs` for the citations).

use std::fmt::Write;

use super::{
    clauses::{
        self,
        Clause,
    },
    machine::Machine,
    model::{
        Diagnostic,
        Event,
        EventKind,
        Op,
        Operand,
        SourceRange,
        Type,
        Value,
    },
};

/// An idea an event's note explains: why the rule it applied matters. Every
/// event of a topic carries its note; the site shows a note the first time
/// the learner sees it, so the many reads, stores and constants of a
/// program stay short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Topic {
    AbstractMachine,
    Scope,
    Lifetime,
    Indeterminate,
    Lvalue,
    LvalueConversion,
    ArrayDecay,
    Promotion,
    Address,
    PointerArithmetic,
    Comparison,
    IntRange,
    Division,
    ElementOrder,
    SequencePoint,
    ImplementationDefined,
    Increment,
    Condition,
    ByValue,
    ExitStatus,
    Undefined,
    IndeterminateRead,
}

impl Topic {
    /// Why the idea matters, in plain words.
    const fn text(self) -> &'static str {
        match self {
            | Self::AbstractMachine =>
                "The C standard does not describe your CPU. It defines what a program means by \
                 what an abstract machine does when it runs it: objects in memory, values read and \
                 stored, and rules for when each effect happens. A compiler may produce any code \
                 that behaves the same way, which is why the compiler pipeline can look so \
                 different from these steps.",
            | Self::Scope =>
                "A block is a scope: names declared inside it exist only up to its closing brace. \
                 Its automatic variables (locals not declared static) are created for this run of \
                 the block and destroyed when execution leaves it, so a loop body gets fresh \
                 locals on every iteration.",
            | Self::Lifetime =>
                "After an object's lifetime ends, a pointer that still holds its address is \
                 dangling. Using it is undefined behavior, the classic dangling-pointer bug: the \
                 memory may already hold something else.",
            | Self::Indeterminate =>
                "C does not set automatic local variables to zero (static ones start at zero). \
                 Until something is stored, the object's value is indeterminate, and reading it is \
                 a bug. Strictly, C creates the object on entry to its block; this machine draws \
                 it when the declaration is reached.",
            | Self::Lvalue =>
                "An lvalue names a place, not a value. The operator around it decides what happens \
                 next: most operators read the value stored there, while the left side of =, the \
                 operand of & and the operand of ++ use the object itself (++ reads it, then \
                 stores into it).",
            | Self::LvalueConversion =>
                "Reading an object (unless it is volatile) is not a side effect: the object keeps \
                 its value. Only stores (=, compound assignments such as +=, ++, --, and calls \
                 that do them) change memory.",
            | Self::ArrayDecay =>
                "Array-to-pointer decay is why passing an array to a function passes only the \
                 address of its first element, and why the function cannot know the array's \
                 length. The exceptions are sizeof and &, which see the whole array.",
            | Self::Promotion =>
                "C never does arithmetic in a type narrower than int. That is why two chars \
                 holding 100 add up to 200, more than a signed char (plain char on x86-64) can \
                 hold: the sum is computed in int.",
            | Self::Address =>
                "& does not read the object, so it is fine to take the address of a variable that \
                 has no value yet. The pointer is only a way to reach the object later.",
            | Self::PointerArithmetic =>
                "An int is 4 bytes on x86-64, so the address grows by 4 per step, but C counts in \
                 elements. Pointer arithmetic is defined only within one array and one past its \
                 end; going further is undefined, even if the pointer is never used.",
            | Self::Comparison =>
                "Comparisons yield the int 0 or 1, so their result can be stored, added or tested \
                 like any other int. An if or a loop then treats 0 as false and anything else as \
                 true.",
            | Self::IntRange =>
                "An int on x86-64 holds -2147483648 to 2147483647. A signed result outside that \
                 range is not wrapped around: it is undefined behavior, and this machine stops on \
                 it.",
            | Self::Division =>
                "Integer division truncates toward zero, so -7 / 2 is -3 and -7 % 2 is -1. \
                 Dividing by zero is undefined behavior.",
            | Self::ElementOrder =>
                "Without designators the list fills the elements from [0] upward, so the third \
                 value always goes to [2]. A designator names an element out of order instead: int \
                 a[5] = {[3] = 7}; starts at [3].",
            | Self::SequencePoint =>
                "Between two sequence points C leaves the order of evaluations and side effects \
                 open. Modifying an object twice, or modifying it and also reading it for another \
                 purpose, with no sequence point between is undefined: i = i++ + 1, i + i++ and \
                 a[i] = i++ all are. Sequence points come at the end of each full expression, \
                 after the first operand of &&, || and ?:, at each comma operator, and just before \
                 a function is called.",
            | Self::ImplementationDefined =>
                "Implementation-defined means each compiler must pick a result and document it. \
                 This one is not portable: where plain char is unsigned (on ARM Linux, for \
                 example), the object would hold 200.",
            | Self::Increment =>
                "As a statement on its own, i++ and ++i do the same thing. They differ only in the \
                 value the expression yields: i++ yields the old value, ++i the new one.",
            | Self::Condition =>
                "Any scalar can be a condition: 0 and a null pointer mean false, everything else \
                 means true. There is no separate boolean test.",
            | Self::ByValue =>
                "C passes every argument by value: the callee gets copies, so assigning to a \
                 parameter never changes the caller's variable. Passing a pointer copies an \
                 address, which still lets the callee reach the caller's objects.",
            | Self::ExitStatus =>
                "The exit status is how a program reports to whoever ran it: 0 means success. A \
                 shell shows it as $?, and on Linux only its low 8 bits get there.",
            | Self::Undefined =>
                "Compilers assume undefined behavior never happens and optimize on that \
                 assumption: they may delete a check like x + 1 > x, or keep a loop running \
                 forever. The same program can then behave differently at -O0 and -O2.",
            | Self::IndeterminateRead =>
                "In practice the read returns whatever was left in that register or stack slot, so \
                 the result can change between runs, compilers and optimization levels.",
        }
    }
}

/// The initial state's event: nothing has run yet.
pub(super) fn start(range: SourceRange) -> Event {
    noted(
        explained(
            Event::new(EventKind::Start, range, String::from("Ready to run main")),
            "Nothing has run yet. Each Step performs one evaluation of the abstract machine and \
             explains the rule of C behind it, starting with main's body.",
            &[clauses::ABSTRACT_MACHINE],
        ),
        Topic::AbstractMachine,
    )
}

/// The event for a diagnostic that stops execution.
pub(super) fn fault(diagnostic: &Diagnostic) -> Event {
    let dangling = diagnostic.clause == Some(&clauses::LIFETIME);
    let (title, explanation, topic) = match diagnostic.kind {
        | "undefined_behavior" => (
            "Undefined behavior",
            String::from(
                "The C standard places no requirements on a program once this happens: the real \
                 program could crash, print garbage, or appear to work. The abstract machine \
                 stops here instead of guessing.",
            ),
            Some(Topic::Undefined),
        ),
        | "indeterminate" if dangling => (
            "Use of a dangling pointer",
            String::from(
                "When an object's lifetime ends, every pointer to it becomes indeterminate. Using \
                 such a pointer is the dangling-pointer bug: the memory may already belong to \
                 another object. The abstract machine stops here instead of guessing.",
            ),
            Some(Topic::Lifetime),
        ),
        | "indeterminate" => (
            "Read of an indeterminate value",
            String::from(
                "An indeterminate value is either an unspecified value or a trap representation, \
                 a bit pattern that is no value at all, and reading a trap representation is \
                 undefined. C11 went further: reading a local that was never initialized and \
                 whose address is never taken is undefined outright. This machine stops rather \
                 than invent a value.",
            ),
            Some(Topic::IndeterminateRead),
        ),
        | "limit" => ("Engine limit reached", sentence(&diagnostic.message), None),
        | _ => (
            "Outside the supported subset",
            sentence(&diagnostic.message),
            None,
        ),
    };
    let mut event = Event::new(
        EventKind::Fault,
        diagnostic.source_range,
        String::from(title),
    );
    event.explanation = explanation;
    event.clauses.extend(diagnostic.clause.copied());
    if diagnostic.kind == "indeterminate" && !dangling {
        event.clauses.push(clauses::TRAP_REPRESENTATION);
    }
    match topic {
        | Some(topic) => noted(event, topic),
        | None => event,
    }
}

impl Machine {
    /// What the next operation will write, for the pending-effects list.
    pub(super) fn pending_effect(&self, op: &Op) -> Option<String> {
        let top = self.activations.last()?;
        let location = |operand: Option<&Operand>| match operand {
            | Some(&Operand::Address { target, offset }) => self.location(target, offset),
            | _ => String::from("an object"),
        };
        match op {
            | Op::Store { .. } => {
                let length = top.operands.len();
                let target = length
                    .checked_sub(2)
                    .and_then(|index| top.operands.get(index));
                Some(format!("Next: store into {}", location(target)))
            },
            | Op::Increment { .. } =>
                Some(format!("Next: update {}", location(top.operands.last()))),
            | _ => None,
        }
    }

    pub(super) fn leave_block(&self, range: SourceRange, ended: Vec<usize>) -> Event {
        let names = self.names(&ended);
        let mut event = if ended.is_empty() {
            explained(
                Event::new(EventKind::LeaveBlock, range, String::from("Leave a block")),
                "The block finishes. It created no objects, so no lifetimes end here.",
                &[clauses::BLOCK_SCOPE],
            )
        } else {
            let title = format!("End the lifetime of {names}");
            let text = if ended.len() == 1 {
                format!(
                    "The block finishes, so {names} reaches the end of its lifetime. Its storage \
                     can be reused for other objects. The next time the block runs, it gets a \
                     fresh object."
                )
            } else {
                format!(
                    "The block finishes, so {names} reach the end of their lifetime. Their \
                     storage can be reused for other objects. The next time the block runs, it \
                     gets fresh objects."
                )
            };
            noted(
                explained(
                    Event::new(EventKind::LeaveBlock, range, title),
                    &text,
                    &[clauses::AUTOMATIC_LIFETIME, clauses::BLOCK_SCOPE],
                ),
                Topic::Lifetime,
            )
        };
        event.objects = ended;
        event
    }

    pub(super) fn declare(&self, range: SourceRange, id: usize, ty: Type) -> Event {
        let object = &self.state.objects[id - 1];
        let function = self.function_name(object.frame_id);
        let what = match ty {
            | Type::Array(count) => format!(
                "an array of {count} ints ({} bytes on x86-64). Each element's value is",
                count.saturating_mul(4)
            ),
            | Type::Pointer =>
                String::from("an int * object, a pointer to int (8 bytes on x86-64). Its value is"),
            | Type::Char => String::from("a char object (1 byte). Its value is"),
            | Type::Int => String::from("an int object (4 bytes on x86-64). Its value is"),
        };
        let text = format!(
            "The declaration of `{name}` is reached, so the machine creates {what} indeterminate \
             until something is stored. The object lives in {function}'s frame: it has automatic \
             storage duration.",
            name = object.name,
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Declare,
                    range,
                    format!("Create `{}` ({})", object.name, object.r#type),
                ),
                &text,
                &[clauses::AUTOMATIC_LIFETIME, clauses::INDETERMINATE],
            ),
            Topic::Indeterminate,
        );
        event.objects.push(id);
        event
    }

    pub(super) fn designate(&self, range: SourceRange, id: usize, operand: Operand) -> Event {
        let name = &self.state.objects[id - 1].name;
        let text = format!(
            "The name `{name}` is an lvalue: it designates the object `{name}`, a place in \
             memory. Nothing is read yet."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Designate,
                    range,
                    format!("`{name}` designates an object"),
                ),
                &text,
                &[clauses::IDENTIFIER, clauses::LVALUE],
            ),
            Topic::Lvalue,
        );
        event.objects.push(id);
        result(event, operand)
    }

    pub(super) fn element(
        &self,
        range: SourceRange,
        id: usize,
        index: i32,
        operand: Operand,
    ) -> Event {
        let location = self.location(id, index);
        let text = format!(
            "An initializer list fills the array's elements in increasing index order, so the \
             next value initializes {location}."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Designate,
                    range,
                    format!("Select {location} to initialize"),
                ),
                &text,
                &[clauses::INITIALIZER_ORDER],
            ),
            Topic::ElementOrder,
        );
        event.objects.push(id);
        result(event, operand)
    }

    pub(super) fn decay(&self, range: SourceRange, id: usize, value: Value) -> Event {
        let object = &self.state.objects[id - 1];
        let text = format!(
            "`{name}` is an array ({ty}). Used in an expression, an array is not copied: it is \
             converted to a pointer to its first element, {name}[0]. This is array-to-pointer \
             decay.",
            name = object.name,
            ty = object.r#type,
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::ArrayDecay,
                    range,
                    format!("`{}` decays to a pointer", object.name),
                ),
                &text,
                &[clauses::ARRAY_DECAY],
            ),
            Topic::ArrayDecay,
        );
        event.objects.push(id);
        result(event, Operand::Value(value))
    }

    pub(super) fn load(
        &self,
        range: SourceRange,
        target: usize,
        offset: i32,
        value: Value,
    ) -> Event {
        let location = self.location(target, offset);
        let shown = self.value_text(&value);
        let text = format!(
            "Lvalue conversion: {location} is used for its value, so the machine reads the value \
             stored in that object: {shown}."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::LvalueConversion,
                    range,
                    format!("Read {location} → {}", self.short_value(&value)),
                ),
                &text,
                &[clauses::LVALUE_CONVERSION],
            ),
            Topic::LvalueConversion,
        );
        event.operands.push(Operand::Address { target, offset });
        event.objects.push(target);
        result(event, Operand::Value(value))
    }

    pub(super) fn promote(&self, range: SourceRange, value: Value) -> Event {
        let shown = self.short_value(&value);
        let text = format!(
            "The char value {shown} is about to take part in arithmetic or a comparison, so the \
             integer promotions first convert it to int. The value stays {shown}; only its type \
             changes, from char to int."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::IntegerPromotion,
                    range,
                    format!("Promote char {shown} to int"),
                ),
                &text,
                &[clauses::INTEGER_PROMOTIONS, clauses::CHAR_SIGNEDNESS],
            ),
            Topic::Promotion,
        );
        event.operands.push(Operand::Value(value.clone()));
        result(event, Operand::Value(value))
    }

    pub(super) fn address_of(
        &self,
        range: SourceRange,
        target: usize,
        offset: i32,
        value: Value,
    ) -> Event {
        let location = self.location(target, offset);
        let text = format!(
            "The & operator produces a pointer to {location}, which other code can later use to \
             read or change it. It does not read {location} itself."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::AddressOf,
                    range,
                    format!("Take the address of {location}"),
                ),
                &text,
                &[clauses::ADDRESS_OPERATOR],
            ),
            Topic::Address,
        );
        event.objects.push(target);
        result(event, Operand::Value(value))
    }

    pub(super) fn indirection(
        &self,
        range: SourceRange,
        source: &str,
        pointer: Value,
        target: usize,
        offset: i32,
    ) -> Event {
        let location = self.location(target, offset);
        let operand = snippet(source, range).trim_start_matches('*');
        let text = format!(
            "Indirection: `{operand}` holds {}, so `*{operand}` is an lvalue designating \
             {location}. Nothing is read or written yet.",
            self.value_text(&pointer),
        );
        let mut event = explained(
            Event::new(
                EventKind::Dereference,
                range,
                format!("`*{operand}` designates {location}"),
            ),
            &text,
            &[clauses::INDIRECTION],
        );
        event.operands.push(Operand::Value(pointer));
        event.objects.push(target);
        result(event, Operand::Address { target, offset })
    }

    pub(super) fn subscript(
        &self,
        range: SourceRange,
        base: Value,
        index: Value,
        target: usize,
        offset: i32,
    ) -> Event {
        let location = self.location(target, offset);
        let steps = match index {
            | Value::Integer(index) => movement(index, "element"),
            | _ => String::from("by the index"),
        };
        let text = format!(
            "Subscripting: a[i] means *(a + i). Starting from {}, the machine moves {steps} and \
             designates {location}.",
            self.value_text(&base),
        );
        let mut event = explained(
            Event::new(EventKind::Subscript, range, format!("Select {location}")),
            &text,
            &[clauses::SUBSCRIPT, clauses::POINTER_ARITHMETIC],
        );
        event
            .operands
            .extend([Operand::Value(base), Operand::Value(index)]);
        event.objects.push(target);
        result(event, Operand::Address { target, offset })
    }

    pub(super) fn unary(
        &self,
        range: SourceRange,
        operator: &str,
        value: Value,
        output: Value,
    ) -> Event {
        let input = self.short_value(&value);
        let shown = self.short_value(&output);
        let (title, text, clause) = match operator {
            | "!" => (
                format!("Logical not: !{input} → {shown}"),
                format!(
                    "! yields 1 if its operand compares equal to 0 and 0 otherwise. The operand \
                     is {input}, so the result is the int {shown}."
                ),
                clauses::LOGICAL_NOT,
            ),
            | "-" => (
                format!("Negate {input} → {shown}"),
                format!("Unary minus negates its int operand: -({input}) is {shown}."),
                clauses::UNARY_MINUS,
            ),
            | _ => (
                format!("Unary plus: {shown}"),
                format!("Unary plus yields its (promoted) operand unchanged: {shown}."),
                clauses::UNARY_PLUS,
            ),
        };
        let mut event = explained(Event::new(EventKind::Unary, range, title), &text, &[clause]);
        event.operands.push(Operand::Value(value));
        result(event, Operand::Value(output))
    }

    /// The event for a binary operator; `&[]` is `&E1[E2]`, which C99
    /// §6.5.3.2p3 evaluates as `E1 + E2` without the indirection.
    pub(super) fn binary(
        &self,
        range: SourceRange,
        source: &str,
        operator: &str,
        left: Value,
        right: Value,
        output: Value,
    ) -> Event {
        let (operator, element) = match operator {
            | "&[]" => ("+", true),
            | _ => (operator, false),
        };
        let (a, b, shown) = (
            self.short_value(&left),
            self.short_value(&right),
            self.short_value(&output),
        );
        let mut event = match (&left, &right) {
            | (Value::Pointer { .. }, Value::Pointer { .. }) => noted(
                explained(
                    Event::new(
                        EventKind::Comparison,
                        range,
                        format!("Compare pointers: {} → {shown}", snippet(source, range)),
                    ),
                    &format!(
                        "Two pointers compare equal exactly when they point to the same place. \
                         The left is {}, the right is {}, so `{operator}` yields the int {shown}.",
                        self.value_text(&left),
                        self.value_text(&right),
                    ),
                    &[clauses::EQUALITY],
                ),
                Topic::Comparison,
            ),
            | (Value::Pointer { .. }, _) | (_, Value::Pointer { .. })
                if matches!(operator, "+" | "-") =>
            {
                let (pointer, count) = if matches!(left, Value::Pointer { .. }) {
                    (&left, &right)
                } else {
                    (&right, &left)
                };
                let written = snippet(source, range);
                let moved = match count {
                    | &Value::Integer(count) if operator == "-" =>
                        movement(count.saturating_neg(), "int"),
                    | &Value::Integer(count) => movement(count, "int"),
                    | _ => String::from("by the count"),
                };
                let sign = if operator == "-" {
                    "subtracting"
                } else {
                    "adding"
                };
                let mut text = if element {
                    format!(
                        "`{written}` asks for an element's address, so the element itself is \
                         never read: C defines &a[i] as a + i, which uses only the pointer and \
                         the index. Starting from {}, {sign} {} moves {moved} and gives {}.",
                        self.value_text(pointer),
                        self.short_value(count),
                        self.value_text(&output),
                    )
                } else {
                    format!(
                        "Pointer arithmetic counts elements, not bytes. Starting from {}, {sign} \
                         {} moves {moved} and gives {}.",
                        self.value_text(pointer),
                        self.short_value(count),
                        self.value_text(&output),
                    )
                };
                let cited: &[Clause] = if element {
                    &[clauses::ADDRESS_OPERATOR, clauses::POINTER_ARITHMETIC]
                } else {
                    &[clauses::POINTER_ARITHMETIC]
                };
                if self.one_past(&output) {
                    text.push_str(
                        " That is the one-past-the-end position: a pointer there may be formed \
                         and compared, but never dereferenced.",
                    );
                }
                let title = if element {
                    format!("Compute the address {written}")
                } else {
                    format!("Pointer arithmetic: {written}")
                };
                noted(
                    explained(
                        Event::new(EventKind::PointerArithmetic, range, title),
                        &text,
                        cited,
                    ),
                    Topic::PointerArithmetic,
                )
            },
            | (Value::Pointer { .. }, _) | (_, Value::Pointer { .. }) => explained(
                Event::new(
                    EventKind::Comparison,
                    range,
                    format!("Compare with a null pointer → {shown}"),
                ),
                &format!(
                    "The constant 0 here is a null pointer constant, so `{operator}` tests \
                     whether the pointer is null. The result is the int {shown}."
                ),
                &[clauses::EQUALITY],
            ),
            | _ if matches!(operator, "<" | "<=" | ">" | ">=" | "==" | "!=") => {
                let truth = if output == Value::Integer(1) {
                    "holds"
                } else {
                    "does not hold"
                };
                let clause = if matches!(operator, "==" | "!=") {
                    clauses::EQUALITY
                } else {
                    clauses::RELATIONAL
                };
                noted(
                    explained(
                        Event::new(
                            EventKind::Comparison,
                            range,
                            format!("Compare {a} {operator} {b} → {shown}"),
                        ),
                        &format!(
                            "A comparison yields an int: 1 when the relation holds and 0 when it \
                             does not. {a} {operator} {b} {truth}, so the result is {shown}."
                        ),
                        &[clause],
                    ),
                    Topic::Comparison,
                )
            },
            | _ => {
                let (verb, clause, topic) = match operator {
                    | "+" => ("adds", clauses::ADDITION, Topic::IntRange),
                    | "-" => ("subtracts", clauses::ADDITION, Topic::IntRange),
                    | "*" => ("multiplies", clauses::MULTIPLICATION, Topic::IntRange),
                    | "/" => (
                        "divides, truncating toward zero,",
                        clauses::DIVISION,
                        Topic::Division,
                    ),
                    | _ => (
                        "takes the remainder of the division",
                        clauses::DIVISION,
                        Topic::Division,
                    ),
                };
                noted(
                    explained(
                        Event::new(
                            EventKind::Arithmetic,
                            range,
                            format!("Compute {a} {operator} {b} = {shown}"),
                        ),
                        &format!(
                            "Both operands are int, so the usual arithmetic conversions leave \
                             them unchanged and the machine {verb} in int: {a} {operator} {b} = \
                             {shown}. The result fits in an int, so it is well defined."
                        ),
                        &[clause, clauses::USUAL_ARITHMETIC],
                    ),
                    topic,
                )
            },
        };
        event
            .operands
            .extend([Operand::Value(left), Operand::Value(right)]);
        result(event, Operand::Value(output))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One store's complete observation."
    )]
    pub(super) fn stored(
        &self,
        range: SourceRange,
        initializer: bool,
        full_expression: bool,
        target: usize,
        offset: i32,
        previous: Option<Value>,
        value: Value,
        stored: Value,
    ) -> Event {
        let location = self.location(target, offset);
        let shown = self.short_value(&stored);
        let (kind, title, clause) = if initializer {
            (
                EventKind::Initialize,
                format!("Initialize {location} = {shown}"),
                clauses::SCALAR_INITIALIZER,
            )
        } else {
            (
                EventKind::Assign,
                format!("Store {shown} into {location}"),
                clauses::SIMPLE_ASSIGNMENT,
            )
        };
        let converted = value != stored;
        let char_conversion = converted && self.object_type(target) == Some(Type::Char);
        let mut text = if char_conversion {
            let operation = if initializer {
                "Initialization"
            } else {
                "Simple assignment"
            };
            format!(
                "The value to store is the int {}. {operation} converts it to {location}'s type, \
                 char, whose range is -128 to 127. It does not fit, so the result is \
                 implementation-defined: bcc, GCC and Clang on x86-64 keep the low 8 bits, so \
                 {location} now holds {shown}.",
                self.short_value(&value)
            )
        } else if initializer {
            format!(
                "The initializer's value, {}, is stored in {location} as its first value.",
                self.value_text(&value)
            )
        } else {
            let before = previous.map_or_else(
                || String::from("its old value"),
                |previous| self.value_text(&previous),
            );
            format!(
                "Simple assignment: {} replaces the value stored in {location} (previously \
                 {before}). Writing the object is the assignment's side effect.",
                self.value_text(&value)
            )
        };
        let mut cited = vec![clause];
        let mut topic = None;
        if converted && self.object_type(target) == Some(Type::Pointer) {
            let _ = write!(
                text,
                " The integer constant 0 is a null pointer constant: storing it into a pointer \
                 converts it to a null pointer, which points to no object, so {location} now \
                 holds NULL."
            );
            cited.push(clauses::NULL_POINTER);
        } else if char_conversion {
            cited.push(clauses::SIGNED_CONVERSION);
            topic = Some(Topic::ImplementationDefined);
        }
        if full_expression {
            text.push_str(" The full expression ends here, a sequence point.");
            cited.push(clauses::FULL_EXPRESSION);
            topic = topic.or(Some(Topic::SequencePoint));
        }
        let mut event = explained(Event::new(kind, range, title), &text, &cited);
        if let Some(topic) = topic {
            event = noted(event, topic);
        }
        event.sequence_point = full_expression;
        event
            .operands
            .extend([Operand::Address { target, offset }, Operand::Value(value)]);
        event.objects.push(target);
        result(event, Operand::Value(stored))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One increment's complete observation."
    )]
    pub(super) fn increment(
        &self,
        range: SourceRange,
        source: &str,
        delta: i32,
        prefix: bool,
        target: usize,
        offset: i32,
        current: Value,
        sum: &Value,
        stored: Value,
    ) -> Event {
        let location = self.location(target, offset);
        let (old, new) = (self.short_value(&current), self.short_value(&stored));
        let verb = if delta > 0 {
            "adds 1 to"
        } else {
            "subtracts 1 from"
        };
        let form = snippet(source, range);
        let mut text = format!(
            "`{form}` reads {location} ({old}), {verb} it, and stores the result back: {location} \
             is now {new}."
        );
        if *sum != stored {
            text.push_str(
                " The addition happens in int; the int result did not fit in a char, so the \
                 implementation-defined conversion kept its low 8 bits.",
            );
        } else if self.object_type(target) == Some(Type::Char) {
            text.push_str(
                " The char is promoted to int for the addition, and the int result, which fits, \
                 is converted back to char when it is stored.",
            );
        }
        text.push_str(" The full expression ends here, a sequence point.");
        let increment = if prefix {
            clauses::PREFIX_INCREMENT
        } else {
            clauses::POSTFIX_INCREMENT
        };
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Increment,
                    range,
                    format!("Update {location}: {old} → {new}"),
                ),
                &text,
                &[increment, clauses::FULL_EXPRESSION],
            ),
            Topic::Increment,
        );
        event.sequence_point = true;
        event
            .operands
            .extend([Operand::Address { target, offset }, Operand::Value(current)]);
        event.objects.push(target);
        result(event, Operand::Value(stored))
    }

    pub(super) fn discard(&self, range: SourceRange, source: &str, value: Value) -> Event {
        let text = format!(
            "`{}` is an expression statement: it is evaluated only for its effects, and its value \
             ({}) is thrown away. The end of the statement is a sequence point.",
            snippet(source, range),
            self.short_value(&value),
        );
        let mut event = explained(
            Event::new(
                EventKind::Discard,
                range,
                String::from("Discard the result"),
            ),
            &text,
            &[clauses::EXPRESSION_STATEMENT, clauses::FULL_EXPRESSION],
        );
        event.sequence_point = true;
        event.operands.push(Operand::Value(value));
        event
    }

    pub(super) fn branch(
        &self,
        range: SourceRange,
        source: &str,
        statement: &str,
        value: Value,
        truth: bool,
    ) -> Event {
        let shown = self.short_value(&value);
        let condition = snippet(source, range);
        let (title, outcome, clause) = match (statement, truth) {
            | ("if", true) => (
                "true: take the if branch",
                "the first branch runs",
                clauses::IF_STATEMENT,
            ),
            | ("if", false) => (
                "false: skip the if branch",
                "the first branch is skipped (and an else branch, if any, runs)",
                clauses::IF_STATEMENT,
            ),
            | (_, true) => (
                "true: run the loop body",
                "the loop body runs",
                clauses::ITERATION,
            ),
            | (_, false) => ("false: leave the loop", "the loop ends", clauses::ITERATION),
        };
        let meaning = match value {
            | Value::Pointer { target: None, .. } => "a null pointer counts as false",
            | Value::Pointer { .. } => "a non-null pointer counts as true",
            | _ if truth => "any nonzero value counts as true",
            | _ => "zero counts as false",
        };
        let text = format!(
            "The controlling expression `{condition}` of this {statement} statement evaluated to \
             {shown}. In C, {meaning}, so {outcome}. The condition is a full expression, so this \
             is a sequence point."
        );
        let mut event = noted(
            explained(
                Event::new(EventKind::Branch, range, format!("Condition is {title}")),
                &text,
                &[clause, clauses::FULL_EXPRESSION],
            ),
            Topic::Condition,
        );
        event.sequence_point = true;
        event.operands.push(Operand::Value(value.clone()));
        result(event, Operand::Value(value))
    }

    pub(super) fn called(
        &self,
        range: SourceRange,
        name: &str,
        frame: usize,
        recursive: bool,
        arguments: &[Value],
        parameters: Vec<usize>,
    ) -> Event {
        let mut bindings = String::new();
        for (index, id) in parameters.iter().enumerate() {
            if index > 0 {
                bindings.push_str(", ");
            }
            let object = &self.state.objects[id - 1];
            let _ = write!(
                bindings,
                "`{}` = {}",
                object.name,
                self.value_text(&object.value)
            );
        }
        let mut text = if arguments.is_empty() {
            format!(
                "The call has no arguments to evaluate. The machine creates a new activation of \
                 {name} (frame #{frame}) on top of the call stack."
            )
        } else {
            format!(
                "All arguments are evaluated before the call; that is a sequence point. The \
                 machine creates a new activation of {name} (frame #{frame}) on top of the call \
                 stack."
            )
        };
        if parameters.is_empty() {
            text.push_str(" It takes no parameters.");
        } else {
            let _ = write!(
                text,
                " Each parameter is a brand-new local object initialized with a copy of its \
                 argument: {bindings}."
            );
        }
        let mut cited = vec![
            clauses::CALL_ARGUMENTS,
            clauses::CALL_SEQUENCE_POINT,
            clauses::PARAMETERS,
        ];
        if recursive {
            text.push_str(
                " This call is recursive: an earlier activation of the same function is still \
                 waiting further down the stack, and this one gets its own separate parameters \
                 and locals.",
            );
            cited.push(clauses::AUTOMATIC_LIFETIME);
        }
        let mut event = explained(
            Event::new(
                EventKind::Call,
                range,
                format!("Call {name} (frame #{frame})"),
            ),
            &text,
            &cited,
        );
        if !parameters.is_empty() {
            event = noted(event, Topic::ByValue);
        }
        event.sequence_point = true;
        event
            .operands
            .extend(arguments.iter().cloned().map(Operand::Value));
        event.objects = parameters;
        event
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One return's complete observation."
    )]
    pub(super) fn returned(
        &self,
        range: SourceRange,
        function: &str,
        frame: usize,
        value: Value,
        ended: Vec<usize>,
        fallthrough: bool,
        call: Option<&str>,
    ) -> Event {
        let shown = self.short_value(&value);
        let names = self.names(&ended);
        let mut text = if fallthrough {
            String::from(
                "Execution reached the closing brace of main. For main only, that is the same as \
                 return 0.",
            )
        } else if call.is_some() {
            format!(
                "return evaluates its expression and hands the value {shown} back to the caller."
            )
        } else {
            format!("return evaluates its expression, {shown}, and main returns it.")
        };
        if let Some(call) = call {
            let _ = write!(
                text,
                " In the caller, the call `{call}` now has the value {shown}."
            );
        } else {
            let _ = write!(
                text,
                " Returning from main ends the program as if exit({shown}) were called: its exit \
                 status is {shown}."
            );
        }
        if !ended.is_empty() {
            let _ = write!(
                text,
                " Frame #{frame} is destroyed, ending the lifetime of {names}."
            );
        }
        let cited = if call.is_some() {
            [clauses::RETURN, clauses::AUTOMATIC_LIFETIME]
        } else {
            [clauses::MAIN_RETURN, clauses::AUTOMATIC_LIFETIME]
        };
        let mut event = explained(
            Event::new(
                EventKind::Return,
                range,
                format!("Return {shown} from {function}"),
            ),
            &text,
            &cited,
        );
        if call.is_none() {
            event = noted(event, Topic::ExitStatus);
        }
        event.sequence_point = !fallthrough;
        event.operands.push(Operand::Value(value.clone()));
        event.objects = ended;
        result(event, Operand::Value(value))
    }

    /// How the source names a location: `x`, `values[1]`, or `values[3]`
    /// (one past), qualified with the owning function when it is not the
    /// innermost activation's.
    pub(super) fn location(&self, target: usize, offset: i32) -> String {
        let Some(object) = target
            .checked_sub(1)
            .and_then(|index| self.state.objects.get(index))
        else {
            return format!("object #{target}");
        };
        let mut text = if matches!(self.object_type(target), Some(Type::Array(_))) {
            format!("{}[{offset}]", object.name)
        } else if offset == 0 {
            format!("`{}`", object.name)
        } else {
            format!("`{}` + {offset}", object.name)
        };
        if self
            .activations
            .last()
            .is_some_and(|activation| activation.id != object.frame_id)
        {
            let _ = write!(text, " (in {})", self.function_name(object.frame_id));
        }
        text
    }

    fn value_text(&self, value: &Value) -> String {
        match value {
            | Value::Indeterminate => String::from("an indeterminate value"),
            | Value::Integer(integer) => format!("{integer}"),
            | Value::Pointer { target: None, .. } => String::from("a null pointer"),
            | &Value::Pointer {
                target: Some(target),
                offset,
            } =>
                if self.one_past(value) {
                    let name = self
                        .state
                        .objects
                        .get(target - 1)
                        .map_or("", |object| object.name.as_str());
                    format!("a pointer one past the end of `{name}`")
                } else {
                    format!("a pointer to {}", self.location(target, offset))
                },
            | Value::Array(elements) => {
                let mut text = String::from("{");
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        text.push_str(", ");
                    }
                    text.push_str(&self.short_value(element));
                }
                text.push('}');
                text
            },
        }
    }

    fn short_value(&self, value: &Value) -> String {
        match value {
            | Value::Indeterminate => String::from("?"),
            | Value::Integer(integer) => format!("{integer}"),
            | Value::Pointer { target: None, .. } => String::from("NULL"),
            | &Value::Pointer {
                target: Some(target),
                offset,
            } => format!("&{}", self.location(target, offset).replace('`', "")),
            | Value::Array(_) => self.value_text(value),
        }
    }

    fn one_past(&self, value: &Value) -> bool {
        let &Value::Pointer {
            target: Some(target),
            offset,
        } = value
        else {
            return false;
        };
        let length = match self.object_type(target) {
            | Some(Type::Array(count)) => count,
            | _ => 1,
        };
        usize::try_from(offset) == Ok(length)
    }

    fn names(&self, ids: &[usize]) -> String {
        let mut text = String::new();
        for (index, id) in ids.iter().enumerate() {
            if index > 0 {
                text.push_str(if index + 1 == ids.len() {
                    " and "
                } else {
                    ", "
                });
            }
            let _ = write!(text, "`{}`", self.state.objects[id - 1].name);
        }
        text
    }

    fn function_name(&self, frame: usize) -> String {
        self.state
            .frames
            .iter()
            .find(|candidate| candidate.id == frame)
            .map_or_else(
                || String::from("a finished call"),
                |frame| frame.function.clone(),
            )
    }
}

pub(super) fn enter_block(range: SourceRange) -> Event {
    noted(
        explained(
            Event::new(EventKind::EnterBlock, range, String::from("Enter a block")),
            "Execution enters a { … } block. Names declared inside it can be used only until its \
             closing brace.",
            &[clauses::COMPOUND_STATEMENT, clauses::BLOCK_SCOPE],
        ),
        Topic::Scope,
    )
}

pub(super) fn constant(range: SourceRange, integer: i32, value: Operand) -> Event {
    let text = format!(
        "The integer constant {integer} has type int, because its value fits in an int. \
         Evaluating a constant just produces its value."
    );
    result(
        explained(
            Event::new(
                EventKind::Constant,
                range,
                format!("Evaluate the constant {integer}"),
            ),
            &text,
            &[clauses::INTEGER_CONSTANT],
        ),
        value,
    )
}

pub(super) fn implicit_zero(range: SourceRange, value: Operand) -> Event {
    result(
        explained(
            Event::new(
                EventKind::Constant,
                range,
                String::from("Supply an implicit 0"),
            ),
            "The initializer list ran out before the array did. Elements without an initializer \
             of their own are initialized to zero.",
            &[clauses::PARTIAL_INITIALIZER],
        ),
        value,
    )
}

pub(super) fn jump(range: SourceRange, backward: bool) -> Event {
    if backward {
        explained(
            Event::new(
                EventKind::Jump,
                range,
                String::from("Jump back to the loop condition"),
            ),
            "The loop body (and a for loop's update expression) has finished, so control returns \
             to the controlling expression, which decides whether to run the body again.",
            &[clauses::ITERATION],
        )
    } else {
        explained(
            Event::new(EventKind::Jump, range, String::from("Skip the else branch")),
            "The if branch has finished, so control jumps over the else branch.",
            &[clauses::IF_STATEMENT],
        )
    }
}

fn explained(mut event: Event, text: &str, cited: &[Clause]) -> Event {
    event.explanation = String::from(text);
    event.clauses.extend_from_slice(cited);
    event
}

/// `event` with the note on `topic`.
fn noted(mut event: Event, topic: Topic) -> Event {
    event.note = String::from(topic.text());
    event
}

fn result(mut event: Event, operand: Operand) -> Event {
    event.result = Some(operand);
    event
}

/// How far an offset of `delta` elements moves a pointer: "0 ints",
/// "1 int forward", "2 elements back".
fn movement(delta: i32, noun: &str) -> String {
    let count = delta.unsigned_abs();
    let plural = if count == 1 { "" } else { "s" };
    let direction = match delta.signum() {
        | 1 => " forward",
        | -1 => " back",
        | _ => "",
    };
    format!("{count} {noun}{plural}{direction}")
}

fn snippet(source: &str, range: SourceRange) -> &str {
    source.get(range.start..range.end).unwrap_or("")
}

/// `text` as a sentence: capitalized, ending in a full stop.
fn sentence(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        let mut output = String::from(first.to_ascii_uppercase());
        output.push_str(characters.as_str());
        output.push('.');
        output
    })
}
