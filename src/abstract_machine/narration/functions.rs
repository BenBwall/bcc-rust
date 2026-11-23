//! Pointers to functions: a function's name becoming a pointer, `&f` and
//! `*fp`, a call through a pointer, and comparing two pointers to
//! functions. A pointer to a function holds which function's code a call
//! runs, so the explanations name the function, its type and the line that
//! defines it.
//!
//! C99: §6.3.2.1 paragraph 4, p. 46; PDF p. 58.
//! C99: §6.5.2.2 paragraphs 1 and 3, p. 71; PDF p. 83.
//! C99: §6.5.9 paragraph 6, p. 87; PDF p. 99.

use super::{
    Topic,
    enclosing_parenthesis,
    explained,
    noted,
    result,
    snippet,
};
use crate::abstract_machine::{
    clauses,
    machine::Machine,
    model::{
        Event,
        EventKind,
        Operand,
        PointerSource,
        PointerSourceKind,
        Program,
        SourceRange,
        Value,
    },
};

/// `text` without the parentheses that enclose all of it: `*f` for `(*f)`.
fn unparenthesized(text: &str) -> String {
    let mut text = text.trim();
    while text.starts_with('(') && enclosing_parenthesis(text.as_bytes()) {
        text = text[1..text.len() - 1].trim();
    }
    String::from(text)
}

impl Machine {
    /// The function's name, its type as C writes it (`int (int)`) and the
    /// line of its definition.
    fn function_facts(&self, program: &Program, function: usize) -> (String, String, usize) {
        let defined = &program.functions[function];
        let ty = self
            .signatures
            .get(defined.signature)
            .map_or_else(String::new, |signature| {
                signature.spelling(&self.structures, &self.signatures)
            });
        (defined.name.clone(), ty, defined.range.line)
    }

    /// The function `function`'s name as an operand designated it and
    /// became a pointer to it, or, when `address`, `&` and the name made
    /// that pointer without the conversion.
    ///
    /// C99: §6.3.2.1 paragraph 4, p. 46; PDF p. 58.
    /// C99: §6.5.3.2 paragraph 3, pp. 78-79; PDF pp. 90-91.
    pub(in crate::abstract_machine) fn function_designator(
        &self,
        range: SourceRange,
        program: &Program,
        function: usize,
        address: bool,
        value: Value,
    ) -> Event {
        let (name, ty, line) = self.function_facts(program, function);
        let event = if address {
            explained(
                Event::new(
                    EventKind::AddressOf,
                    range,
                    format!("Take the address of the function {name}"),
                ),
                &format!(
                    "`&{name}` yields a pointer to the function {name}, the same pointer `{name}` \
                     alone would become."
                ),
                &format!(
                    "`{name}` designates the function {name}, defined on line {line}, of type \
                     {ty}. The & operator applied to a function yields a pointer to it, so \
                     `&{name}` and a plain `{name}` give the same value: the & only says so \
                     explicitly, and the name is not converted first. A function is code, not an \
                     object, so nothing is read."
                ),
                &[clauses::ADDRESS_OPERATOR, clauses::FUNCTION_DESIGNATOR],
            )
        } else {
            explained(
                Event::new(
                    EventKind::FunctionDesignator,
                    range,
                    format!("`{name}` becomes a pointer to the function"),
                ),
                &format!(
                    "A function's name used as an operand designates the function, and C converts \
                     it to a pointer to it: `{name}` yields a pointer to the function {name}."
                ),
                &format!(
                    "`{name}` names the function {name}, defined on line {line}, of type {ty}. A \
                     function is code, not an object: it has no value to read and cannot be \
                     copied, so wherever its name is an operand, other than after & or sizeof, C \
                     converts it to a pointer to the function. The pointer records which \
                     function's code a call through it runs. A call written `{name}(…)` makes the \
                     same conversion before it calls; this machine shows it as a step only where \
                     the pointer is kept or passed on."
                ),
                &[clauses::FUNCTION_DESIGNATOR, clauses::FUNCTION_TYPE],
            )
        };
        result(noted(event, Topic::FunctionPointer), Operand::Value(value))
    }

    /// `*e` of a pointer to the function `function`, where `operand` is
    /// the source of `e` and how it gives the pointer: the function
    /// designator `*` yields converts straight back to the same pointer.
    /// The explanation is built from that structure, so each spelling it
    /// quotes is C: `(**f)(…)` and `(*f)(…)` call the same function, where
    /// `*f(…)` would apply `*` to what `f` returns.
    ///
    /// C99: §6.5.3.2 paragraph 4, p. 79; PDF p. 91.
    /// C99: §6.3.2.1 paragraph 4, p. 46; PDF p. 58.
    pub(in crate::abstract_machine) fn function_indirection(
        &self,
        range: SourceRange,
        (operand, form): (&str, PointerSource),
        program: &Program,
        function: usize,
        pointer: Value,
    ) -> Event {
        let (name, _, _) = self.function_facts(program, function);
        // `*` and its operand as written: the operand is a cast expression
        // there, so this is the expression the step evaluates.
        let written = format!("*{operand}");
        // A call's parentheses follow a postfix expression; any other
        // operand needs parentheses of its own (C99 §6.5.2p1).
        let callee = if form.postfix {
            String::from(operand)
        } else {
            format!("({operand})")
        };
        let gives = match form.kind {
            | PointerSourceKind::Name => format!(
                "`{operand}` designates the function {name}, and like any function designator it \
                 converts to a pointer to that function."
            ),
            | PointerSourceKind::Indirection => format!(
                "`{operand}` designates the function {name} too, and converts to a pointer to it."
            ),
            | PointerSourceKind::Object =>
                format!("`{operand}` holds a pointer to the function {name}."),
            | PointerSourceKind::Value =>
                format!("`{operand}` yields a pointer to the function {name}."),
        };
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Dereference,
                    range,
                    format!("`{written}` designates the function {name}"),
                ),
                &format!(
                    "`{written}` follows the pointer to the function {name}, and that function \
                     designator converts straight back to a pointer to it."
                ),
                &format!(
                    "{gives} The unary * applied to a pointer to a function yields a function \
                     designator, {name} itself, and a function designator in an expression is \
                     converted to a pointer to the function. The two steps cancel: `{written}` \
                     yields the same pointer as `{operand}`, so `({written})(…)` and \
                     `{callee}(…)` call the same function. However many * a program writes, each \
                     only shows the reader that a pointer is being followed."
                ),
                &[clauses::INDIRECTION, clauses::FUNCTION_DESIGNATOR],
            ),
            Topic::FunctionPointer,
        );
        event.operands.push(Operand::Value(pointer.clone()));
        result(event, Operand::Value(pointer))
    }

    /// `event`, a call's, retold as a call through the pointer the
    /// expression `callee` evaluated to, which points to the function
    /// `function`; `kind` says how `callee` gave that pointer.
    ///
    /// C99: §6.5.2.2 paragraphs 1 and 3, p. 71; PDF p. 83.
    pub(in crate::abstract_machine) fn called_through(
        &self,
        mut event: Event,
        program: &Program,
        function: usize,
        (callee, kind): (&str, PointerSourceKind),
    ) -> Event {
        let (name, ty, line) = self.function_facts(program, function);
        let gives = match kind {
            | PointerSourceKind::Name | PointerSourceKind::Indirection => format!(
                "`{callee}` designates {name} and converts to a pointer to it, so the call runs"
            ),
            | PointerSourceKind::Object => {
                format!("`{callee}` holds a pointer to {name}, so the call runs")
            },
            | PointerSourceKind::Value => {
                format!("`{callee}` yields a pointer to {name}, so the call runs")
            },
        };
        event.title = event.title.replacen(
            &format!("Call {name}"),
            &format!("Call {name} through `{callee}`"),
            1,
        );
        event.summary = event.summary.replacen("The call runs", &gives, 1);
        event.explanation = format!(
            "This call goes through a pointer: the expression before the parentheses, `{callee}`, \
             gave a pointer to the function {name} (defined on line {line}), and the call runs \
             the function it points to, exactly as `{name}(…)` would. C leaves the order of \
             evaluating that expression and the arguments unspecified; this machine evaluates it \
             first. In x86-64 code the pointer holds the address where {name}'s machine code \
             starts, and the call jumps there rather than to an address fixed when the program \
             was linked; in WebAssembly it holds an index into a table of functions. The \
             pointer's type promised a function of type {ty}, which {name} has, so the arguments \
             convert to {name}'s parameters as that type says. {}",
            event.explanation
        );
        let mut cited = vec![clauses::CALLED_FUNCTION, clauses::FUNCTION_CALL];
        cited.append(&mut event.clauses);
        event.clauses = cited;
        if event.note.is_empty() {
            event = noted(event, Topic::FunctionPointer);
        }
        // The arguments stay the event's operands, one per parameter; the
        // pointer is named in the explanation.
        event
    }

    /// `left == right` or `!=` where one operand points to a function and
    /// the other to a function or nowhere.
    ///
    /// C99: §6.5.9 paragraph 6, p. 87; PDF p. 99.
    pub(in crate::abstract_machine) fn function_comparison(
        &self,
        range: SourceRange,
        (source, constant): (&str, Option<&str>),
        operator: &str,
        (left, right): (&Value, &Value),
        shown: &str,
    ) -> Event {
        let null = |value: &Value| !matches!(value, Value::Function(_));
        // An integer 0 here is a null pointer constant, which converts to
        // a null pointer (C99 §6.3.2.3p3).
        let side = |value: &Value| match value {
            | Value::Integer(0) => format!(
                "the null pointer constant `{}`, which converts to a null pointer",
                constant.unwrap_or("0")
            ),
            | value => self.value_text(value),
        };
        let (summary, text) = if null(left) || null(right) {
            (
                format!(
                    "A null pointer points to no function, so it equals no pointer to one: \
                     `{operator}` yields {shown}."
                ),
                format!(
                    "A null pointer is guaranteed to compare unequal to a pointer to any \
                     function. The left is {}, the right is {}, so `{operator}` yields the int \
                     {shown}. A pointer that points to a function never compares equal to a null \
                     pointer, so comparing a pointer with null tells whether there is a function \
                     to call through it.",
                    side(left),
                    side(right),
                ),
            )
        } else {
            (
                format!(
                    "Two pointers to functions are equal exactly when they point to the same \
                     function: `{operator}` yields {shown}."
                ),
                format!(
                    "Two pointers to functions compare equal exactly when both point to the same \
                     function, or both are null. The left is {}, the right is {}, so `{operator}` \
                     yields the int {shown}.",
                    self.value_text(left),
                    self.value_text(right),
                ),
            )
        };
        noted(
            explained(
                Event::new(
                    EventKind::Comparison,
                    range,
                    format!(
                        "Compare pointers to functions: {} → {shown}",
                        unparenthesized(&snippet(source, range))
                    ),
                ),
                &summary,
                &text,
                &if null(left) || null(right) {
                    vec![
                        clauses::POINTER_EQUALITY,
                        clauses::NULL_POINTER,
                        clauses::EVALUATION_ORDER,
                    ]
                } else {
                    vec![clauses::POINTER_EQUALITY, clauses::EVALUATION_ORDER]
                },
            ),
            Topic::FunctionPointer,
        )
    }
}
