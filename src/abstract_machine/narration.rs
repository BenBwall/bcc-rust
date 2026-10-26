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

/// The initial state's event: nothing has run yet.
pub(super) fn start(range: SourceRange) -> Event {
    explained(
        Event::new(EventKind::Start, range, String::from("Ready to run main")),
        "The C standard defines a program by what an abstract machine does when it runs it: one \
         evaluation at a time, with named objects instead of registers. Each Step below performs \
         one of those evaluations and explains the rule behind it.",
        &[clauses::ABSTRACT_MACHINE],
    )
}

/// The event for a diagnostic that stops execution.
pub(super) fn fault(diagnostic: &Diagnostic) -> Event {
    let title = match diagnostic.kind {
        | "undefined_behavior" => "Undefined behavior",
        | "indeterminate" => "Read of an indeterminate value",
        | "limit" => "Engine limit reached",
        | _ => "Outside the supported subset",
    };
    let mut event = Event::new(
        EventKind::Fault,
        diagnostic.source_range,
        String::from(title),
    );
    let mut text = capitalized(&diagnostic.message);
    text.push('.');
    match diagnostic.kind {
        | "undefined_behavior" => text.push_str(
            " The C standard places no requirements on a program after this point: a compiler may \
             assume it never happens, so the program could crash, print garbage, or appear to \
             work. The abstract machine stops here instead of guessing.",
        ),
        | "indeterminate" => text.push_str(
            " An indeterminate value is not a number you can rely on: it might be anything left \
             in memory, or even a trap representation. This machine stops rather than invent a \
             value.",
        ),
        | _ => {},
    }
    event.explanation = text;
    event.clauses.extend(diagnostic.clause.copied());
    event
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
            let text = format!(
                "The block finishes, so {names} reach the end of their lifetime. Their storage \
                 can be reused for other objects, and any pointer still holding their address \
                 becomes indeterminate. The next time the block runs, it gets fresh objects."
            );
            explained(
                Event::new(EventKind::LeaveBlock, range, title),
                &text,
                &[clauses::AUTOMATIC_LIFETIME, clauses::BLOCK_SCOPE],
            )
        };
        event.objects = ended;
        event
    }

    pub(super) fn declare(&self, range: SourceRange, id: usize, ty: Type) -> Event {
        let object = &self.state.objects[id - 1];
        let function = self.function_name(object.frame_id);
        let what = match ty {
            | Type::Array(count) => format!("an array of {count} ints, each element"),
            | Type::Pointer => String::from("an int * (pointer to int) object, whose value is"),
            | Type::Char => String::from("a char object (one byte), whose value is"),
            | Type::Int => String::from("an int object (four bytes on x86-64), whose value is"),
        };
        let text = format!(
            "The declaration of `{name}` is reached, and the machine creates {what} \
             indeterminate: nothing has been stored yet, and C does not set locals to zero. It \
             lives in {function}'s frame with automatic storage duration. (Strictly, C begins \
             this lifetime on entry to the enclosing block; this machine shows the object once \
             its declaration is reached.)",
            name = object.name,
        );
        let mut event = explained(
            Event::new(
                EventKind::Declare,
                range,
                format!("Create `{}` ({})", object.name, object.r#type),
            ),
            &text,
            &[clauses::AUTOMATIC_LIFETIME, clauses::INDETERMINATE],
        );
        event.objects.push(id);
        event
    }

    pub(super) fn designate(&self, range: SourceRange, id: usize, operand: Operand) -> Event {
        let name = &self.state.objects[id - 1].name;
        let text = format!(
            "The name `{name}` is an lvalue: it designates the object `{name}`, a location in \
             memory, not yet its value. The surrounding expression decides whether that object is \
             read or written."
        );
        let mut event = explained(
            Event::new(
                EventKind::Designate,
                range,
                format!("`{name}` designates an object"),
            ),
            &text,
            &[clauses::IDENTIFIER, clauses::LVALUE],
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
            "The array initializer lists its elements in order, so the next value initializes \
             {location}."
        );
        let mut event = explained(
            Event::new(
                EventKind::Designate,
                range,
                format!("Select {location} to initialize"),
            ),
            &text,
            &[clauses::SUBSCRIPT],
        );
        event.objects.push(id);
        result(event, operand)
    }

    pub(super) fn decay(&self, range: SourceRange, id: usize, value: Value) -> Event {
        let object = &self.state.objects[id - 1];
        let text = format!(
            "`{name}` is an array ({ty}). Used in an expression, an array is not copied: it is \
             converted to a pointer to its first element, {name}[0]. This is called \
             array-to-pointer decay.",
            name = object.name,
            ty = object.r#type,
        );
        let mut event = explained(
            Event::new(
                EventKind::ArrayDecay,
                range,
                format!("`{}` decays to a pointer", object.name),
            ),
            &text,
            &[clauses::ARRAY_DECAY],
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
             currently stored in that object: {shown}."
        );
        let mut event = explained(
            Event::new(
                EventKind::LvalueConversion,
                range,
                format!("Read {location} → {}", self.short_value(&value)),
            ),
            &text,
            &[clauses::LVALUE_CONVERSION],
        );
        event.operands.push(Operand::Address { target, offset });
        event.objects.push(target);
        result(event, Operand::Value(value))
    }

    pub(super) fn promote(&self, range: SourceRange, value: Value) -> Event {
        let shown = self.short_value(&value);
        let text = format!(
            "C never computes in a type narrower than int. Before the char value {shown} takes \
             part in arithmetic or comparison, the integer promotions convert it to an int with \
             the same value. That is why adding two chars can produce a result a char could not \
             hold."
        );
        let mut event = explained(
            Event::new(
                EventKind::IntegerPromotion,
                range,
                format!("Promote char {shown} to int"),
            ),
            &text,
            &[clauses::INTEGER_PROMOTIONS, clauses::CHAR_SIGNEDNESS],
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
            "The & operator does not read {location}. It produces a pointer to {location}'s \
             object, which other code can later use to read or change it."
        );
        let mut event = explained(
            Event::new(
                EventKind::AddressOf,
                range,
                format!("Take the address of {location}"),
            ),
            &text,
            &[clauses::ADDRESS_OPERATOR],
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
        let steps = self.short_value(&index);
        let text = format!(
            "Subscripting: a[i] means *(a + i). Starting from {}, the machine moves {steps} \
             element(s) forward and designates {location}.",
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
                clauses::UNARY_MINUS,
            ),
        };
        let mut event = explained(Event::new(EventKind::Unary, range, title), &text, &[clause]);
        event.operands.push(Operand::Value(value));
        result(event, Operand::Value(output))
    }

    pub(super) fn binary(
        &self,
        range: SourceRange,
        source: &str,
        operator: &str,
        left: Value,
        right: Value,
        output: Value,
    ) -> Event {
        let (a, b, shown) = (
            self.short_value(&left),
            self.short_value(&right),
            self.short_value(&output),
        );
        let mut event = match (&left, &right) {
            | (Value::Pointer { .. }, Value::Pointer { .. }) => explained(
                Event::new(
                    EventKind::Comparison,
                    range,
                    format!("Compare pointers: {} → {shown}", snippet(source, range)),
                ),
                &format!(
                    "Two pointers compare equal exactly when they point to the same place. The \
                     left is {}, the right is {}, so `{operator}` yields the int {shown}.",
                    self.value_text(&left),
                    self.value_text(&right),
                ),
                &[clauses::EQUALITY],
            ),
            | (Value::Pointer { .. }, _) | (_, Value::Pointer { .. })
                if matches!(operator, "+" | "-") =>
            {
                let (pointer, count) = if matches!(left, Value::Pointer { .. }) {
                    (&left, &right)
                } else {
                    (&right, &left)
                };
                let mut text = format!(
                    "Pointer arithmetic counts elements, not bytes: {} {operator} {} moves {} \
                     int(s) and gives {}.",
                    self.value_text(pointer),
                    self.short_value(count),
                    self.short_value(count).trim_start_matches('-'),
                    self.value_text(&output),
                );
                if self.one_past(&output) {
                    text.push_str(
                        " That is the one-past-the-end position: a pointer there may be formed \
                         and compared, but never dereferenced.",
                    );
                }
                explained(
                    Event::new(
                        EventKind::PointerArithmetic,
                        range,
                        format!("Pointer arithmetic: {}", snippet(source, range)),
                    ),
                    &text,
                    &[clauses::POINTER_ARITHMETIC],
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
                explained(
                    Event::new(
                        EventKind::Comparison,
                        range,
                        format!("Compare {a} {operator} {b} → {shown}"),
                    ),
                    &format!(
                        "A comparison yields an int, not a separate boolean type: 1 when the \
                         relation holds and 0 when it does not. {a} {operator} {b} {truth}, so \
                         the result is {shown}."
                    ),
                    &[clause],
                )
            },
            | _ => {
                let (verb, clause) = match operator {
                    | "+" => ("adds", clauses::ADDITION),
                    | "-" => ("subtracts", clauses::ADDITION),
                    | "*" => ("multiplies", clauses::MULTIPLICATION),
                    | "/" => ("divides, truncating toward zero,", clauses::DIVISION),
                    | _ => ("takes the remainder of", clauses::DIVISION),
                };
                explained(
                    Event::new(
                        EventKind::Arithmetic,
                        range,
                        format!("Compute {a} {operator} {b} = {shown}"),
                    ),
                    &format!(
                        "Both operands are int, so the usual arithmetic conversions leave them \
                         unchanged and the machine {verb} in int: {a} {operator} {b} = {shown}. \
                         The result fits in an int, so it is well defined."
                    ),
                    &[clause, clauses::USUAL_ARITHMETIC],
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
        let mut text = if initializer {
            format!(
                "The initializer's value, {}, becomes the first value stored in {location}.",
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
        if value != stored {
            let _ = write!(
                text,
                " But {} does not fit in a char, whose range is -128 to 127. That conversion is \
                 implementation-defined; like bcc on x86-64, this machine keeps the low 8 bits, \
                 so {location} now holds {shown}.",
                self.short_value(&value)
            );
            cited.push(clauses::SIGNED_CONVERSION);
        }
        if full_expression {
            text.push_str(
                " The full expression ends here, which is a sequence point: this store is \
                 complete before the next statement begins.",
            );
            cited.push(clauses::FULL_EXPRESSION);
        }
        let mut event = explained(Event::new(kind, range, title), &text, &cited);
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
                " The int result did not fit in a char, so the implementation-defined conversion \
                 kept its low 8 bits.",
            );
        }
        text.push_str(
            " The statement uses only this side effect, and it is complete at the end of the full \
             expression.",
        );
        let increment = if prefix {
            clauses::PREFIX_INCREMENT
        } else {
            clauses::POSTFIX_INCREMENT
        };
        let mut event = explained(
            Event::new(
                EventKind::Increment,
                range,
                format!("Update {location}: {old} → {new}"),
            ),
            &text,
            &[increment, clauses::FULL_EXPRESSION],
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
             {shown}. In C, {meaning}, so {outcome}. A condition is a full expression: its \
             evaluation, including any side effects, is complete before the next step."
        );
        let mut event = explained(
            Event::new(EventKind::Branch, range, format!("Condition is {title}")),
            &text,
            &[clause, clauses::FULL_EXPRESSION],
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
        let mut text = format!(
            "All arguments are evaluated before the call; that is a sequence point. The machine \
             creates a new activation of {name} (frame #{frame}) on top of the call stack."
        );
        if parameters.is_empty() {
            text.push_str(" It takes no parameters.");
        } else {
            let _ = write!(
                text,
                " Each parameter is a brand-new local object initialized with a copy of its \
                 argument: {bindings}. Changing a parameter never changes the caller's variables"
            );
            if arguments
                .iter()
                .any(|argument| matches!(argument, Value::Pointer { .. }))
            {
                text.push_str(
                    ", but through a copied pointer the callee can reach the caller's objects.",
                );
            } else {
                text.push('.');
            }
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
        } else {
            format!(
                "return evaluates its expression and hands the value {shown} back to the caller."
            )
        };
        if let Some(call) = call {
            let _ = write!(
                text,
                " In the caller, the call `{call}` now has the value {shown}."
            );
        } else {
            let _ = write!(
                text,
                " main has returned, so the program ends with exit status {shown}."
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
    explained(
        Event::new(EventKind::EnterBlock, range, String::from("Enter a block")),
        "Execution enters a block. Names declared inside it are visible only until the block \
         ends, and the automatic objects it creates are destroyed when execution leaves it.",
        &[clauses::COMPOUND_STATEMENT, clauses::BLOCK_SCOPE],
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

fn result(mut event: Event, operand: Operand) -> Event {
    event.result = Some(operand);
    event
}

fn snippet(source: &str, range: SourceRange) -> &str {
    source.get(range.start..range.end).unwrap_or("")
}

fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        let mut output = String::from(first.to_ascii_uppercase());
        output.push_str(characters.as_str());
        output
    })
}
