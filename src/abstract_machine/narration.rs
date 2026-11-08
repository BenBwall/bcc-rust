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
    layout,
    machine::Machine,
    model::{
        Diagnostic,
        DirectiveKind,
        Event,
        EventKind,
        Macro,
        Op,
        Operand,
        Program,
        SourceRange,
        Type,
        Undefined,
        Value,
    },
};

/// The sentence that ends a binary operator's details: the machine's
/// left-to-right order is its own choice.
const UNORDERED: &str = " C leaves the order of evaluating the two operands unspecified; this \
                         machine evaluated the left one first.";

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
    StaticStorage,
    StaticLocal,
    Member,
    StructureCopy,
    Padding,
    ExitStatus,
    Undefined,
    IndeterminateRead,
    Preprocessor,
    StringLiteral,
    StringInitializer,
    CharacterConstant,
    Format,
    VariadicPromotion,
    Macro,
    Concatenation,
    Const,
    Cast,
    CompoundAssign,
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
                 next: most operators read the value stored there, while the left side of `=`, the \
                 operand of `&` and the operand of `++` use the object itself (`++` reads it, then \
                 stores into it).",
            | Self::LvalueConversion =>
                "Reading an object (unless it is volatile) is not a side effect: the object keeps \
                 its value. Only stores (`=`, compound assignments such as `+=`, `++`, `--`, and \
                 calls that do them) change memory.",
            | Self::ArrayDecay =>
                "Array-to-pointer decay is why passing an array to a function passes only the \
                 address of its first element, and why the function cannot know the array's \
                 length. The exceptions are `sizeof` and `&`, which see the whole array.",
            | Self::Promotion =>
                "C never does arithmetic in a type narrower than int. That is why two chars \
                 holding 100 add up to 200, more than a signed char (plain char on x86-64) can \
                 hold: the sum is computed in int.",
            | Self::Address =>
                "`&` does not read the object, so it is fine to take the address of a variable \
                 that has no value yet. The pointer is only a way to reach the object later.",
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
                "Integer division truncates toward zero, so `-7 / 2` is -3 and `-7 % 2` is -1. \
                 Dividing by zero is undefined behavior.",
            | Self::ElementOrder =>
                "Without designators the list fills the elements from [0] upward, so the third \
                 value always goes to `[2]`. A designator names an element out of order instead: \
                 `int a[5] = {[3] = 7};` starts at `[3]`.",
            | Self::SequencePoint =>
                "Between two sequence points C leaves the order of evaluations and side effects \
                 open. Modifying an object twice, or modifying it and also reading it for another \
                 purpose, with no sequence point between is undefined: `i = i++ + 1`, `i + i++` \
                 and `a[i] = i++` all are. Sequence points come at the end of each full \
                 expression, after the first operand of `&&`, `||` and `?:`, at each comma \
                 operator, and just before a function is called.",
            | Self::ImplementationDefined =>
                "Implementation-defined means each compiler must pick a result and document it. \
                 This one is not portable: where plain char is unsigned (on ARM Linux, for \
                 example), the object would hold 200.",
            | Self::Increment =>
                "As a statement on its own, `i++` and `++i` do the same thing. They differ only in \
                 the value the expression yields: `i++` yields the old value, `++i` the new one.",
            | Self::Condition =>
                "Any scalar can be a condition: 0 and a null pointer mean false, everything else \
                 means true. There is no separate boolean test.",
            | Self::ByValue =>
                "C passes every argument by value: the callee gets copies, so assigning to a \
                 parameter never changes the caller's variable. Passing a pointer copies an \
                 address, which still lets the callee reach the caller's objects.",
            | Self::StaticStorage =>
                "C has two lifetimes for ordinary variables. Automatic objects (locals) are \
                 created each time their block runs and start indeterminate. Static objects \
                 (declared outside every function, or with `static`) exist once, for the whole \
                 run, and start at zero unless initialized: no code runs to set them, because the \
                 value is already in the program file the system loads.",
            | Self::StaticLocal =>
                "A `static` local keeps its value from one call to the next, which makes it useful \
                 for counters and caches, but also makes the function depend on its history: two \
                 calls with the same arguments can return different results.",
            | Self::Member =>
                "A member is an object in its own right, at a fixed offset inside the structure. \
                 `p->x` is shorthand for `(*p).x`: follow the pointer, then select the member.",
            | Self::StructureCopy =>
                "Unlike an array, a structure is copied whole by `=`, by passing it to a function \
                 and by returning it. A large structure is often passed by pointer instead, to \
                 avoid the copy and to let the callee change the caller's object.",
            | Self::Padding =>
                "On x86-64 an int starts on a multiple of 4 bytes and a pointer on a multiple of \
                 8, so a char followed by an int leaves 3 unused bytes between them. Ordering \
                 members from largest to smallest alignment often removes the padding. Padding \
                 holds unspecified bytes, which is why comparing structures with memcmp is unsafe.",
            | Self::ExitStatus =>
                "The exit status is how a program reports to whoever ran it: 0 means success. A \
                 shell shows it as `$?`, and on Linux only its low 8 bits get there.",
            | Self::Undefined =>
                "Compilers assume undefined behavior never happens and optimize on that \
                 assumption: they may delete a check like `x + 1 > x`, or keep a loop running \
                 forever. The same program can then behave differently at -O0 and -O2.",
            | Self::IndeterminateRead =>
                "In practice the read returns whatever was left in that register or stack slot, so \
                 the result can change between runs, compilers and optimization levels.",
            | Self::Preprocessor =>
                "#include pastes text in before the compiler reads the program, which is why it \
                 comes first: C needs a function declared before a call to it. The header only \
                 declares printf; the code that prints is in the C library, which the linker adds \
                 to the program.",
            | Self::StringLiteral =>
                "C has no string type: a string is an array of chars ended by a null character, \
                 and its length is stored nowhere. printf, puts and every other string function \
                 find the end by looking for the null, so an array without one is not a string, \
                 and reading it as one runs past its end.",
            | Self::StringInitializer =>
                "`char s[] = \"hi\";` makes an array of your own, a copy you may change. `char *p \
                 = \"hi\";` only points at the literal's array, which you must not change: storing \
                 through p is undefined, and on most systems it crashes, because literals live in \
                 read-only memory.",
            | Self::CharacterConstant =>
                "A char is a small integer: 'A' is just another way to write 65 in ASCII, and 'A' \
                 + 1 is 'B'. That is why a loop can compare a char with '\\0' or count from 'a' to \
                 'z'.",
            | Self::Format =>
                "printf trusts its format: nothing at run time checks that each conversion matches \
                 the argument it takes. A %d given a pointer, or a conversion with no argument \
                 left, is undefined, and in a real program prints garbage or crashes. Compilers \
                 warn about it when the format is a literal (-Wformat).",
            | Self::Macro =>
                "A macro is text substitution, not a variable: it has no type, no storage and no \
                 address, and the compiler never sees its name. `#define N 10` costs no memory, \
                 and a debugger usually cannot show N, because by the time the program runs every \
                 N has become 10.",
            | Self::Concatenation =>
                "Adjacent literals are how C writes a long string across several lines, or builds \
                 one from macros: `\"bcc \" VERSION`. The joining happens while the program is \
                 translated; C has no operator that joins strings while it runs.",
            | Self::Const =>
                "const is a promise the compiler checks: an assignment to a const object does not \
                 compile. It is not a lock on memory: a cast can remove the const from a pointer, \
                 and storing through that pointer into the object is undefined behavior, which a \
                 real program may or may not survive.",
            | Self::Cast =>
                "A cast tells the compiler that a conversion is intended, so it no longer warns. \
                 It does not check that the value fits: (char)300 quietly becomes 44, and a cast \
                 that removes const compiles even though storing through the result can be \
                 undefined.",
            | Self::CompoundAssign =>
                "`x += 2` is shorthand for `x = x + 2`, except that `x` is evaluated once. That \
                 matters when the left side does work: `a[next()] += 1` calls next once, while \
                 `a[next()] = a[next()] + 1` calls it twice and may change two different elements.",
            | Self::VariadicPromotion =>
                "A function with `...` cannot say what types its extra arguments have, so C widens \
                 them to a few standard types first. printf can then read every small integer as \
                 an int, which is why %c takes an int and why there is no conversion for a char.",
        }
    }
}

/// What the operator around a name will do with the object it designates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Designation {
    /// Read its value (lvalue conversion comes next).
    Read,
    /// Store into it: the left operand of `=`.
    Assigned,
    /// Give it its first value: the declarator of an initialized
    /// declaration, which is not an expression.
    Initialized,
    /// Take its address: the operand of `&`.
    Address,
    /// Read and store it: the operand of `++` or `--`.
    Updated,
    /// Select one of its members: the left operand of `.`.
    Member,
    /// Read, combine and store it: the left operand of a compound
    /// assignment such as `+=`.
    Compound,
}

/// The initial state's event: nothing has run yet. A program with
/// `statics` objects of static storage duration initializes them first.
pub(super) fn start(range: SourceRange, program: &Program) -> Event {
    let statics = program.statics.len();
    let defines = program
        .directives
        .iter()
        .any(|directive| matches!(directive.kind, DirectiveKind::Define { .. }));
    if defines {
        return noted(
            explained(
                Event::new(
                    EventKind::Start,
                    range,
                    String::from("Ready to translate and run the program"),
                ),
                "Nothing has run yet: first the preprocessor runs the lines that start with #, \
                 then the static objects get their values, and then each Step performs one \
                 evaluation.",
                "Nothing has run yet. Before the program can run, it is translated: in \
                 translation phase 4 the preprocessor executes the directives, the lines that \
                 start with #, one step each here. A #define makes its name a macro, and the \
                 preprocessor replaces every later use of the name, so the program that runs \
                 never mentions it. Then, before main is called, every object with static storage \
                 duration gets its initial value, and each Step performs one evaluation of the \
                 abstract machine, explaining the rule of C behind it.",
                &[
                    clauses::ABSTRACT_MACHINE,
                    clauses::TRANSLATION_PHASES,
                    clauses::STARTUP,
                ],
            ),
            Topic::AbstractMachine,
        );
    }
    if !program.directives.is_empty() {
        return noted(
            explained(
                Event::new(
                    EventKind::Start,
                    range,
                    String::from("Ready to translate and run the program"),
                ),
                "Nothing has run yet: first the preprocessor runs the #include line, then the \
                 static objects get their values, and then each Step performs one evaluation.",
                "Nothing has run yet. Before the program can run, it is translated: the \
                 preprocessor executes the #include directive in translation phase 4. Then, \
                 before main is called, every object with static storage duration (each string \
                 literal's array among them) gets its initial value, and each Step performs one \
                 evaluation of the abstract machine, explaining the rule of C behind it.",
                &[
                    clauses::ABSTRACT_MACHINE,
                    clauses::TRANSLATION_PHASES,
                    clauses::STARTUP,
                ],
            ),
            Topic::AbstractMachine,
        );
    }
    if statics == 0 {
        return noted(
            explained(
                Event::new(EventKind::Start, range, String::from("Ready to run main")),
                "Nothing has run yet: each Step performs one evaluation and says which rule of C \
                 it followed.",
                "Nothing has run yet. Each Step performs one evaluation of the abstract machine \
                 and explains the rule of C behind it, starting with main's body.",
                &[clauses::ABSTRACT_MACHINE],
            ),
            Topic::AbstractMachine,
        );
    }
    let objects = if statics == 1 {
        String::from("its object with static storage duration gets its initial value")
    } else {
        format!("its {statics} objects with static storage duration get their initial values")
    };
    noted(
        explained(
            Event::new(
                EventKind::Start,
                range,
                String::from("Ready to start the program"),
            ),
            &format!(
                "Nothing has run yet: before main is called, {objects}; then each Step performs \
                 one evaluation."
            ),
            &format!(
                "Nothing has run yet. Before the program starts, that is before main is called, \
                 {objects}. Then each Step performs one evaluation of the abstract machine and \
                 explains the rule of C behind it, starting with main's body."
            ),
            &[clauses::ABSTRACT_MACHINE, clauses::STARTUP],
        ),
        Topic::AbstractMachine,
    )
}

/// Translation phase 4 runs the program's directive `index`: `#include
/// <stdio.h>`, whose declarations replace the line, or an object-like
/// `#define`.
pub(super) fn directive(program: &Program, index: usize) -> Event {
    let directive = &program.directives[index];
    let range = directive.range;
    if let DirectiveKind::Define { index, again } = directive.kind {
        return define(&program.macros[index], range, again);
    }
    let index = program.directives[..index]
        .iter()
        .filter(|earlier| earlier.kind == DirectiveKind::Include)
        .count();
    let again = if index == 0 {
        ""
    } else {
        " This file already included <stdio.h>; a standard header included again has no further \
         effect."
    };
    let mut cited = vec![
        clauses::TRANSLATION_PHASES,
        clauses::SOURCE_INCLUSION,
        clauses::LIBRARY_HEADERS,
    ];
    if index > 0 {
        cited.push(clauses::HEADER_ORDER);
    }
    noted(
        explained(
            Event::new(
                EventKind::Directive,
                range,
                String::from("#include <stdio.h>: declare printf, puts and putchar"),
            ),
            "Before the program runs, the preprocessor replaces this line with the header \
             <stdio.h>, whose declarations of printf, puts and putchar let the code below call \
             them.",
            &format!(
                "C translates a source file in phases, and in phase 4 the preprocessor executes \
                 directives, the lines that start with #. #include <stdio.h> is replaced by the \
                 entire contents of the standard header <stdio.h>, which declares the library's \
                 input and output functions, each with a prototype. This machine declares the \
                 three it runs: printf, puts and putchar, which write to standard output, shown \
                 in the Output panel. The header holds only declarations: the functions \
                 themselves are compiled code in the C library. The compiler pipeline's \
                 Preprocessor stage shows what the real preprocessor makes of this file.{again}"
            ),
            &cited,
        ),
        Topic::Preprocessor,
    )
}

/// Translation phase 4 runs `#define`, defining the object-like macro
/// `defined` (or repeating its identical definition, when `again`).
///
/// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
fn define(defined: &Macro, range: SourceRange, again: bool) -> Event {
    let (name, replacement, kind) = (&defined.name, &defined.replacement, defined.kind);
    if again {
        return noted(
            explained(
                Event::new(
                    EventKind::Directive,
                    range,
                    format!("#define {name} again: nothing changes"),
                ),
                &format!(
                    "{name} is already a macro for `{replacement}`, and C allows defining it \
                     again with exactly the same replacement, which changes nothing."
                ),
                &format!(
                    "In translation phase 4 the preprocessor executes this #define, but {name} is \
                     already defined with the replacement list `{replacement}`. A macro may be \
                     defined again only with an identical replacement list; this one is, so the \
                     directive has no effect. A different replacement would be an error the \
                     compiler must report."
                ),
                &[
                    clauses::TRANSLATION_PHASES,
                    clauses::MACRO_REDEFINITION,
                    clauses::OBJECT_LIKE_MACRO,
                ],
            ),
            Topic::Macro,
        );
    }
    noted(
        explained(
            Event::new(
                EventKind::Directive,
                range,
                format!("#define {name}: {name} stands for {replacement}"),
            ),
            &format!(
                "Before the program runs, the preprocessor makes {name} a macro: from here to the \
                 end of the file, every {name} is replaced by `{replacement}`."
            ),
            &format!(
                "C translates a source file in phases, and in phase 4 the preprocessor executes \
                 directives, the lines that start with #. `#define {name} {replacement}` defines \
                 an object-like macro: each later instance of the name {name} is replaced by the \
                 replacement list, here the {kind} `{replacement}`, before the compiler proper \
                 reads the program, and the directive line itself is then deleted. {name} is not \
                 a variable: it has no type, no storage and no address, and nothing of it exists \
                 when the program runs. The machine marks each replaced use when it evaluates it, \
                 and the compiler pipeline's Preprocessor stage shows the real preprocessor doing \
                 the replacement."
            ),
            &[clauses::TRANSLATION_PHASES, clauses::OBJECT_LIKE_MACRO],
        ),
        Topic::Macro,
    )
}

/// `event`, which used the replacement of the macro `used` where the source
/// spells its name. Phase 4 replaced the name before the program ran, so an
/// evaluation of the replacement leads with that; a declaration that used it
/// (in an array size or a static initializer) says so in its details.
///
/// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
pub(super) fn macro_use(mut event: Event, used: &Macro, source: &str, lead: bool) -> Event {
    let (name, replacement) = (&used.name, &used.replacement);
    let line = source[..used.range.start]
        .bytes()
        .filter(|&byte| byte == b'\n')
        .count()
        + 1;
    let replaced = format!(
        "`{name}` is not a variable: the #define on line {line} made it a macro, and in \
         translation phase 4, before the program was compiled, the preprocessor replaced this \
         {name} with its replacement list, `{replacement}`. The compiler, and this machine, see \
         only `{replacement}`."
    );
    if !event.clauses.contains(&clauses::OBJECT_LIKE_MACRO) {
        event.clauses.insert(0, clauses::OBJECT_LIKE_MACRO);
    }
    if !lead {
        let _ = write!(event.explanation, " In the declaration, {replaced}");
        return event;
    }
    let summary = lowered(&event.summary);
    event.summary = format!(
        "`{name}` is a macro the preprocessor replaced with `{replacement}` before the program \
         ran: {summary}"
    );
    event.explanation = format!("{replaced} {}", event.explanation);
    event.title = if event.title.starts_with("Before main") {
        format!("{}, from the macro {name}", event.title)
    } else {
        format!("Macro {name}: {}", lowered(&event.title))
    };
    noted(event, Topic::Macro)
}

/// `text` with a leading "The" or a capitalized verb in lower case, to
/// continue a sentence.
fn lowered(text: &str) -> String {
    let first_word = text.split(' ').next().unwrap_or_default();
    let common = matches!(
        first_word,
        "The" | "Evaluate" | "Before" | "Convert" | "Decay" | "Each" | "This" | "A" | "An"
    );
    if !common {
        return String::from(text);
    }
    let mut lowered = String::from(text);
    lowered[..1].make_ascii_lowercase();
    lowered
}

/// The event for a diagnostic that stops execution. Undefined behavior is
/// titled by what went wrong, from the paragraph that makes it undefined.
pub(super) fn fault(diagnostic: &Diagnostic) -> Event {
    let dangling = diagnostic.clause == Some(&clauses::LIFETIME);
    let (title, summary, explanation, topic) = match diagnostic.kind {
        | "undefined_behavior" => {
            let (title, summary) = undefined(diagnostic);
            (
                title,
                String::from(summary),
                String::from(
                    "The C standard places no requirements on a program once this happens: the \
                     real program could crash, print garbage, or appear to work. The abstract \
                     machine stops here instead of guessing.",
                ),
                Some(Topic::Undefined),
            )
        },
        | "indeterminate" if dangling => (
            "Use of a dangling pointer",
            String::from(
                "The object this pointer pointed to no longer exists, so the pointer's value is \
                 indeterminate and cannot be used.",
            ),
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
                "The read would use a value the program never set, so the result could differ \
                 from run to run and between compilers.",
            ),
            String::from(
                "An indeterminate value is either an unspecified value or a trap representation, \
                 a bit pattern that is no value at all, and reading a trap representation is \
                 undefined. C11 went further: reading a local that was never initialized and \
                 whose address is never taken is undefined outright. This machine stops rather \
                 than invent a value.",
            ),
            Some(Topic::IndeterminateRead),
        ),
        | "limit" => (
            "Engine limit reached",
            sentence(&diagnostic.message),
            String::from(
                "The browser's engine bounds every run so that a long loop cannot freeze the \
                 page. The program itself may be fine.",
            ),
            None,
        ),
        | _ => (
            "Outside the supported subset",
            sentence(&diagnostic.message),
            String::from(
                "The abstract machine runs a small subset of C: int and char objects, pointers to \
                 them, int and char arrays, string literals, structures, file-scope and static \
                 objects, functions and prototypes, if, while and for, and printf, puts and \
                 putchar from <stdio.h>. The compiler pipeline and WebAssembly views compile the \
                 whole language.",
            ),
            None,
        ),
    };
    let mut event = Event::new(
        EventKind::Fault,
        diagnostic.source_range,
        String::from(title),
    );
    event.summary = summary;
    event.explanation = explanation;
    event.clauses.extend(diagnostic.clause.copied());
    if diagnostic.kind == "indeterminate" && !dangling {
        event.clauses.push(clauses::TRAP_REPRESENTATION);
    }
    match diagnostic.undefined {
        | Some(Undefined::QuotientOverflow) => event.clauses.push(clauses::QUOTIENT),
        | Some(Undefined::NullSubscript) => event.clauses.push(clauses::POINTER_ARITHMETIC),
        | Some(Undefined::Unterminated) => event.clauses.push(clauses::CONVERSIONS),
        | Some(Undefined::WrongArgument) => event
            .clauses
            .extend([clauses::CONVERSIONS, clauses::VARIADIC_TYPES]),
        | Some(Undefined::StringLiteralWrite) => event.clauses.push(clauses::STRING_LITERAL),
        | _ => {},
    }
    match topic {
        | Some(topic) => noted(event, topic),
        | None => event,
    }
}

/// The lead for a subscript of a null pointer, quoting the program's own
/// base and index.
pub(super) fn null_subscript(base: &str, index: &str) -> String {
    format!(
        "`{base}[{index}]` means `*({base} + {index})`, and a null pointer points into no array, \
         so even the addition has no meaning."
    )
}

/// The title and the one-sentence reason for undefined behavior, from its
/// cause. The fault callout states the facts; this says why C gives them
/// no meaning. A null subscript's lead is generic here; the machine quotes
/// the program's own (`null_subscript`).
fn undefined(diagnostic: &Diagnostic) -> (&'static str, &'static str) {
    match diagnostic.undefined {
        | Some(Undefined::SignedOverflow) => (
            "Undefined behavior: signed overflow",
            "The exact result does not fit in an int, and C defines no behavior for signed \
             overflow.",
        ),
        | Some(Undefined::QuotientOverflow) => (
            "Undefined behavior: the quotient does not fit",
            "The quotient would be one more than the largest int; C11 makes both `/` and `%` \
             undefined then, where C99 left the `%` case unsettled.",
        ),
        | Some(Undefined::DivisionByZero) => (
            "Undefined behavior: division by zero",
            "C defines no quotient or remainder for an integer division by zero.",
        ),
        | Some(Undefined::NullIndirection) => (
            "Undefined behavior: null pointer dereference",
            "A null pointer points to no object, so there is nothing for `*` to designate.",
        ),
        | Some(Undefined::NullSubscript) => (
            "Undefined behavior: subscript of a null pointer",
            "`p[i]` means `*(p + i)`, and a null pointer points into no array, so even the \
             addition has no meaning.",
        ),
        | Some(Undefined::NullArithmetic) => (
            "Undefined behavior: arithmetic on a null pointer",
            "Pointer arithmetic moves within an array, and a null pointer points into none, so C \
             defines no result.",
        ),
        | Some(Undefined::AfterLifetime) => (
            "Undefined behavior: use after lifetime",
            "The object no longer exists, and C gives no meaning to using it; its memory may \
             already hold another object.",
        ),
        | Some(Undefined::OnePastEnd) => (
            "Undefined behavior: access one past the end",
            "C lets a pointer stop just past the last element, so a loop can end there, but \
             reading or writing there has no meaning.",
        ),
        | Some(Undefined::PointerOutside) => (
            "Undefined behavior: pointer outside its array",
            "Pointer arithmetic may move only within an array and to one past its end; this \
             result would leave both.",
        ),
        | Some(Undefined::StringLiteralWrite) => (
            "Undefined behavior: modifying a string literal",
            "A string literal's array belongs to the program's constants, and C gives no meaning \
             to changing it; real programs usually crash here, as literals sit in read-only \
             memory.",
        ),
        | Some(Undefined::ConstWrite) => (
            "Undefined behavior: modifying a const object",
            "The object was defined const, and C gives no meaning to changing it through a \
             pointer whose const a cast removed; a real program may crash, or keep using the old \
             value.",
        ),
        | Some(Undefined::Unterminated) => (
            "Undefined behavior: a string without its null",
            "A string ends at its first null character, and this array has none from here on, so \
             reading the string would run past the end of the array.",
        ),
        | Some(Undefined::NullString) => (
            "Undefined behavior: a null pointer for a string",
            "A library function that reads a string needs a pointer to one; a null pointer points \
             to no characters at all.",
        ),
        | Some(Undefined::MissingArgument) => (
            "Undefined behavior: printf has too few arguments",
            "Each conversion takes the next argument, and this one has none left to take, so \
             printf would read whatever happens to be where an argument would be.",
        ),
        | Some(Undefined::WrongArgument) => (
            "Undefined behavior: printf argument of the wrong type",
            "printf reads each argument as the type its conversion names, and this argument has \
             another type, so C gives the result no meaning.",
        ),
        | Some(Undefined::InvalidConversion) => (
            "Undefined behavior: invalid conversion specification",
            "printf's format must spell each conversion as the standard defines it, and this one \
             is not one of them.",
        ),
        | None => (
            "Undefined behavior",
            "C places no requirements on what the program does from here.",
        ),
    }
}

impl Machine {
    /// What the next operation will write, for the pending-effects list.
    pub(super) fn pending_effect(&self, op: &Op) -> Option<String> {
        let top = self.activations.last()?;
        let location = |operand: Option<&Operand>| match operand {
            | Some(&Operand::Address {
                target,
                offset,
                structure,
            }) => self.location_of(target, offset, structure),
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
            | Op::Compound(_) => {
                let length = top.operands.len();
                let target = length
                    .checked_sub(2)
                    .and_then(|index| top.operands.get(index));
                Some(format!("Next: update {}", location(target)))
            },
            | Op::InitializeString { .. } => Some(format!(
                "Next: store the string's characters into {}",
                location(top.operands.last())
            )),
            | Op::Library(library, _) => Some(format!(
                "Next: {} writes to standard output",
                library.name()
            )),
            | _ => None,
        }
    }

    pub(super) fn leave_block(&self, range: SourceRange, ended: Vec<usize>) -> Event {
        let names = self.names(&ended);
        let mut event = if ended.is_empty() {
            explained(
                Event::new(EventKind::LeaveBlock, range, String::from("Leave a block")),
                "The block ends; it declared no objects, so nothing ceases to exist.",
                "The block finishes. It created no objects, so no lifetimes end here.",
                &[clauses::BLOCK_SCOPE],
            )
        } else {
            let title = format!("End the lifetime of {names}");
            let summary = if ended.len() == 1 {
                format!("The block ends, so {names}, declared inside it, ceases to exist.")
            } else {
                format!("The block ends, so {names}, declared inside it, cease to exist.")
            };
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
                    &summary,
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
            | Type::Pointer(_) => format!(
                "an {} object, a pointer to int (8 bytes on x86-64). Its value is",
                object.r#type
            ),
            | Type::StructPointer(_) => format!(
                "a {} object, a pointer to a structure (8 bytes on x86-64). Its value is",
                object.r#type
            ),
            | Type::Struct(id) => format!(
                "a {} object, {}. Each member's value is",
                object.r#type,
                self.layout_text(id)
            ),
            | Type::Char => String::from("a char object (1 byte). Its value is"),
            | Type::Int => String::from("an int object (4 bytes on x86-64). Its value is"),
            | Type::CharArray(count) => format!(
                "an array of {count} chars ({count} byte{}). Each element's value is",
                if count == 1 { "" } else { "s" }
            ),
            | Type::CharPointer(_) => format!(
                "a {} object, a pointer to char (8 bytes on x86-64). Its value is",
                object.r#type
            ),
        };
        let kind = match ty {
            | Type::Array(count) => format!("an array of {count} ints"),
            | Type::Pointer(_) => String::from("a pointer to int"),
            | Type::StructPointer(_) => String::from("a pointer to a structure"),
            | Type::Struct(_) => format!("a {}", object.r#type),
            | Type::Char => String::from("a char object"),
            | Type::Int => String::from("an int object"),
            | Type::CharArray(count) => format!("an array of {count} chars"),
            | Type::CharPointer(_) => String::from("a pointer to char"),
        };
        let summary = format!(
            "The machine draws `{name}` here, {kind} in {function}'s frame; C has it exist from \
             the start of its block, with no value until one is stored.",
            name = object.name,
        );
        let mut text = format!(
            "The declaration of `{name}` is reached, so the machine creates {what} indeterminate \
             until something is stored. The object lives in {function}'s frame: it has automatic \
             storage duration.",
            name = object.name,
        );
        let mut cited = vec![clauses::AUTOMATIC_LIFETIME, clauses::INDETERMINATE];
        let constant = self.metadata[id - 1].constant;
        if constant {
            let _ = write!(
                text,
                " It is defined const: its initializer gives it its value, and after that the \
                 program may not change it. An assignment to it, ++ or -- does not compile, since \
                 a const object is not a modifiable lvalue."
            );
            cited.push(clauses::MODIFIABLE_LVALUE);
        }
        if matches!(ty, Type::Struct(_)) {
            cited.extend([
                clauses::STRUCTURE_TYPE,
                clauses::MEMBER_ORDER,
                clauses::MEMBER_ALIGNMENT,
                clauses::TRAILING_PADDING,
            ]);
        }
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Declare,
                    range,
                    format!("Create `{}` ({})", object.name, object.r#type),
                ),
                &summary,
                &text,
                &cited,
            ),
            if matches!(ty, Type::Struct(_)) && self.padded(ty) {
                Topic::Padding
            } else if constant {
                Topic::Const
            } else {
                Topic::Indeterminate
            },
        );
        event.objects.push(id);
        event
    }

    /// Before `main`: static object `index` of the program, now object
    /// `id`, exists with its initial value.
    pub(super) fn static_storage(&self, program: &Program, index: usize, id: usize) -> Event {
        let declared = &program.statics[index];
        let object = &self.state.objects[id - 1];
        if declared.literal {
            return self.string_literal(declared.range, id, declared.parts);
        }
        let name = &object.name;
        let shown = self.short_value(&object.value);
        let pointer = declared.ty.is_pointer();
        let aggregate = matches!(
            declared.ty,
            Type::Array(_) | Type::Struct(_) | Type::CharArray(_)
        );
        let (place, scope) = declared.function.as_ref().map_or_else(
            || {
                (
                    String::from("declared outside every function"),
                    clauses::FILE_SCOPE,
                )
            },
            |function| {
                (
                    format!("declared `static` inside {function}"),
                    clauses::BLOCK_SCOPE,
                )
            },
        );
        let lifetime = format!(
            "`{name}` is {place}, so it has static storage duration: it exists for the whole run \
             of the program, not just during one call, and belongs to no frame."
        );
        let (title, summary, how, mut cited) = if declared.explicit {
            (
                format!("Before main: `{name}` = {shown}"),
                format!(
                    "`{name}` has static storage duration: it gets its constant initializer's \
                     value, {shown}, before main runs, and keeps existing until the program ends."
                ),
                format!(
                    "Its initializer must be a constant expression, known before the program \
                     runs, so no code runs to compute it: the value {} is simply there when the \
                     program starts.{}",
                    self.value_text(&object.value),
                    if aggregate {
                        " Every element or member the initializer does not mention starts at zero."
                    } else {
                        ""
                    }
                ),
                vec![clauses::STATIC_STORAGE, clauses::STATIC_INITIALIZER],
            )
        } else {
            let zero = if aggregate {
                format!(
                    "every member and element zero, every pointer in it null: {}",
                    self.value_text(&object.value)
                )
            } else if pointer {
                String::from("a null pointer")
            } else {
                String::from("zero")
            };
            (
                format!("Before main: `{name}` = {shown} (static storage)"),
                format!(
                    "`{name}` has static storage duration, so without an initializer it starts as \
                     {}, not indeterminate, before main runs.",
                    if aggregate {
                        "all zeros"
                    } else if pointer {
                        "a null pointer"
                    } else {
                        "zero"
                    }
                ),
                format!(
                    "It has no initializer, and unlike an automatic local it does not start \
                     indeterminate: C initializes it to {zero}."
                ),
                vec![clauses::STATIC_STORAGE, clauses::STATIC_ZERO],
            )
        };
        cited.extend([clauses::STARTUP, scope]);
        let mut text = format!("{lifetime} {how} All of this happens before main is called.");
        if let Type::Struct(structure) = declared.ty {
            let _ = write!(
                text,
                " It is a {}, {}",
                object.r#type,
                self.layout_text(structure)
            );
            cited.extend([clauses::MEMBER_ORDER, clauses::MEMBER_ALIGNMENT]);
        }
        let topic = if declared.constant {
            let _ = write!(
                text,
                " It is defined const, so the program may never change it: an assignment to it \
                 does not compile."
            );
            cited.push(clauses::MODIFIABLE_LVALUE);
            Topic::Const
        } else {
            Topic::StaticStorage
        };
        let mut event = noted(
            explained(
                Event::new(EventKind::StaticStorage, declared.range, title),
                &summary,
                &text,
                &cited,
            ),
            topic,
        );
        event.objects.push(id);
        event.result = Some(Operand::Value(object.value.clone()));
        event
    }

    /// Before `main`: the array of the string literal at `range`, object
    /// `id`, exists with its characters and a null.
    fn string_literal(&self, range: SourceRange, id: usize, parts: usize) -> Event {
        let object = &self.state.objects[id - 1];
        let name = &object.name;
        let characters = self.characters(&object.value);
        let count = layout::count(&object.value);
        if parts > 1 {
            return self.joined_literal(range, id, parts);
        }
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::StaticStorage,
                    range,
                    format!("Before main: the string literal {name} ({})", object.r#type),
                ),
                &format!(
                    "The string literal {name} is an array of {count} chars with static storage \
                     duration, existing before main runs: {characters}."
                ),
                &format!(
                    "In translation phase 7 a null character, '\\0', is added to the end of each \
                     string literal's characters, and they initialize an unnamed array of static \
                     storage duration just long enough to hold them: here {} = {characters}. Like \
                     every static object it exists before main is called and lasts for the whole \
                     run, and wherever the literal appears in an expression it stands for this \
                     array. The program must not modify it: a store into it is undefined, which \
                     is why a pointer to one is usually declared `const char *`.",
                    object.r#type
                ),
                &[
                    clauses::STRING_LITERAL,
                    clauses::STRING_LITERAL_WRITE,
                    clauses::STARTUP,
                ],
            ),
            Topic::StringLiteral,
        );
        event.objects.push(id);
        event.result = Some(Operand::Value(object.value.clone()));
        event
    }

    /// Before `main`: the array of a string literal the source writes as
    /// `parts` adjacent literals, which translation phase 6 joined.
    ///
    /// C99: §5.1.1.2 paragraph 1 item 6, p. 10; PDF p. 22.
    /// C99: §6.4.5 paragraph 4, p. 62; PDF p. 74.
    fn joined_literal(&self, range: SourceRange, id: usize, parts: usize) -> Event {
        let object = &self.state.objects[id - 1];
        let name = &object.name;
        let characters = self.characters(&object.value);
        let count = layout::count(&object.value);
        let mut joined = Vec::new();
        if let Value::Array(elements) = &object.value {
            for element in elements {
                if let &Value::Integer(character) = element {
                    joined.push(character.to_le_bytes()[0]);
                }
            }
        }
        let _null = joined.pop();
        let literal = super::library::quoted(&joined);
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::StaticStorage,
                    range,
                    format!("Before main: {parts} literals joined into {literal}"),
                ),
                &format!(
                    "Translation phase 6 joined the {parts} adjacent string literals {name} into \
                     one, {literal}: a single array of {count} chars with one null at its end."
                ),
                &format!(
                    "The source writes {parts} string literals side by side, {name}. Before the \
                     program runs, translation phase 6 concatenates adjacent string literals into \
                     one, after phase 5 has replaced their escape sequences, so this is one \
                     literal, {literal}, not {parts}. Phase 7 then adds a single null character, \
                     '\\0', at the end, and the characters initialize one unnamed array of static \
                     storage duration: here {} = {characters}. Like every static object it exists \
                     before main is called, and the program must not modify it.",
                    object.r#type
                ),
                &[
                    clauses::LITERAL_CONCATENATION_PHASE,
                    clauses::STRING_CONCATENATION,
                    clauses::STRING_LITERAL,
                    clauses::STARTUP,
                ],
            ),
            Topic::Concatenation,
        );
        event.objects.push(id);
        event.result = Some(Operand::Value(object.value.clone()));
        event
    }

    /// `char s[] = "...";`: `bytes`, then zeros, were stored into the
    /// char array at leaf `offset` of `target`; `stored` is their values.
    #[expect(
        clippy::too_many_arguments,
        reason = "One string initialization's complete observation."
    )]
    pub(super) fn initialized_string(
        &self,
        range: SourceRange,
        target: usize,
        offset: i32,
        bytes: &[u8],
        stored: Value,
        full_expression: bool,
        parts: usize,
    ) -> Event {
        let length = layout::count(&stored);
        let whole = self.array_name(target, offset);
        let literal = super::library::quoted(bytes);
        let characters = self.characters(&stored);
        let null = if bytes.len() < length {
            let zeros = length - bytes.len() - 1;
            if zeros == 0 {
                String::from(" and its null character")
            } else {
                format!(
                    " and its null character, and the {zeros} element{} after them are zero",
                    if zeros == 1 { "" } else { "s" }
                )
            }
        } else {
            String::from(
                ", filling the array exactly: there is no room for the null, so the array holds \
                 characters but no string",
            )
        };
        let mut text = format!(
            "A string literal may initialize a char array: its characters{null}. Here {whole} \
             becomes {characters}. The array is the program's own, so it may be changed later. \
             The literal only spells the initial characters: C still describes it as an array of \
             static storage duration, but nothing can refer to that array, so the machine does \
             not draw it."
        );
        let mut cited = vec![clauses::STRING_INITIALIZER, clauses::PARTIAL_INITIALIZER];
        if parts > 1 {
            let _ = write!(
                text,
                " The source writes the literal as {parts} adjacent string literals; translation \
                 phase 6 joined them into this one before the program ran."
            );
            cited.extend([
                clauses::LITERAL_CONCATENATION_PHASE,
                clauses::STRING_CONCATENATION,
            ]);
        }
        if full_expression {
            text.push_str(" The initializer is a full expression: a sequence point follows.");
        }
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Initialize,
                    range,
                    format!("Initialize {whole} from {literal}"),
                ),
                &format!("The characters of {literal}{null} initialize {whole}, one element each."),
                &text,
                &cited,
            ),
            if parts > 1 {
                Topic::Concatenation
            } else {
                Topic::StringInitializer
            },
        );
        event.sequence_point = full_expression;
        event.operands.push(Operand::Address {
            target,
            offset,
            structure: None,
        });
        event.operands.push(Operand::Value(stored.clone()));
        event.objects.push(target);
        result(event, Operand::Value(stored))
    }

    /// A char passed through `...` gets the default argument promotions.
    pub(super) fn default_promotion(&self, range: SourceRange, value: Value) -> Event {
        let shown = self.short_value(&value);
        let character = match value {
            | Value::Integer(integer) => format!(" ({})", char_text(integer)),
            | _ => String::new(),
        };
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::IntegerPromotion,
                    range,
                    format!("Promote char {shown} to int for `...`"),
                ),
                &format!(
                    "This argument goes through printf's `...`, so the default argument \
                     promotions apply: the char {shown}{character} arrives as the int {shown}."
                ),
                &format!(
                    "printf's prototype, int printf(const char *format, ...), gives a type only \
                     to the format. Every later argument matches the `...`, and C applies the \
                     default argument promotions to it: the integer promotions, which turn a char \
                     into an int with the same value. So the char {shown}{character} reaches \
                     printf as the int {shown}."
                ),
                &[
                    clauses::VARIADIC_ARGUMENTS,
                    clauses::DEFAULT_PROMOTIONS,
                    clauses::INTEGER_PROMOTIONS,
                ],
            ),
            Topic::VariadicPromotion,
        );
        event.operands.push(Operand::Value(value.clone()));
        result(event, Operand::Value(value))
    }

    /// The elements of a char array value as character constants:
    /// `{'h', 'i', '\0'}`.
    fn characters(&self, value: &Value) -> String {
        let Value::Array(elements) = value else {
            return self.value_text(value);
        };
        let mut text = String::from("{");
        for (index, element) in elements.iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            match element {
                | Value::Integer(integer) => text.push_str(&char_text(*integer)),
                | other => text.push_str(&self.short_value(other)),
            }
        }
        text.push('}');
        text
    }

    /// Execution reaches the declaration of the block-scope static `id`.
    pub(super) fn static_local(&self, range: SourceRange, id: usize) -> Event {
        let object = &self.state.objects[id - 1];
        let name = &object.name;
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::StaticLocal,
                    range,
                    format!("`{name}` is static: nothing to create"),
                ),
                &format!(
                    "`{name}` is declared `static`, so it already exists and keeps its value, {}, \
                     from earlier calls: this declaration does nothing at run time.",
                    self.short_value(&object.value)
                ),
                &format!(
                    "An automatic local would be created here, indeterminate, every time the \
                     block runs. `{name}` has static storage duration instead: it was created and \
                     initialized once, before main, and its initializer does not run again. The \
                     declaration only makes the name `{name}` usable in this block. It holds {}.",
                    self.value_text(&object.value)
                ),
                &[clauses::STATIC_STORAGE, clauses::BLOCK_SCOPE],
            ),
            Topic::StaticLocal,
        );
        event.objects.push(id);
        event
    }

    /// Lvalue conversion of a whole structure: a copy of every member.
    pub(super) fn load_structure(
        &self,
        range: SourceRange,
        target: usize,
        offset: i32,
        structure: usize,
        value: Value,
    ) -> Event {
        let location = self.location_of(target, offset, Some(structure));
        let shown = self.value_text(&value);
        let mut text = format!(
            "{location} is used for its value, so the machine reads the whole structure, every \
             member at once: {shown}. C calls this lvalue conversion; for a structure, the value \
             is a copy of all its members."
        );
        let mut cited = vec![clauses::LVALUE_CONVERSION];
        if indeterminate_member(&value) {
            text.push_str(
                " Some members are still indeterminate. That is not an error here: a structure's \
                 value is never a trap representation, so copying it copies those members as they \
                 are, still indeterminate.",
            );
            cited.push(clauses::STRUCTURE_VALUE);
        }
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::LvalueConversion,
                    range,
                    format!("Read {location} → {}", self.short_value(&value)),
                ),
                &format!(
                    "The expression needs the value of the structure {location}, so the machine \
                     reads every member: {shown}."
                ),
                &text,
                &cited,
            ),
            Topic::StructureCopy,
        );
        event.operands.push(Operand::Address {
            target,
            offset,
            structure: Some(structure),
        });
        event.objects.push(target);
        result(event, Operand::Value(value))
    }

    /// `.m` or `->m` designated a member of the structure `whole` at leaf
    /// `base`. `pointer` is the pointer `->` followed.
    #[expect(
        clippy::too_many_arguments,
        reason = "One member access's complete observation."
    )]
    pub(super) fn member(
        &self,
        range: SourceRange,
        source: &str,
        target: usize,
        base: i32,
        whole: Option<usize>,
        pointer: Option<Value>,
        selected: Operand,
    ) -> Event {
        let written = snippet(source, range);
        let (member, offset, decayed) = match &selected {
            | &Operand::Address {
                offset, structure, ..
            } => (self.location_of(target, offset, structure), offset, None),
            | Operand::Value(value @ Value::Pointer { first, .. }) =>
                (self.array_name(target, *first), *first, Some(value)),
            | Operand::Value(_) => (String::from("the member"), base, None),
        };
        let whole = self.location_of(target, base, whole);
        let byte = self.byte_offset(target, base, offset);
        let at = byte.map_or_else(String::new, |byte| format!(", {byte} bytes into it"));
        let (summary, mut text, mut cited) = if let Some(pointer) = &pointer {
            (
                format!(
                    "`{written}` follows the pointer to the structure {whole} and designates its \
                     member {member}{at}."
                ),
                format!(
                    "The left operand holds {}, so `->` designates the member of the structure it \
                     points to: {member}{at}. `p->m` means `(*p).m`. Nothing is read or written \
                     yet.",
                    self.value_text(pointer)
                ),
                vec![clauses::MEMBER_POINTER, clauses::MEMBER_ORDER],
            )
        } else {
            (
                format!("`{written}` designates the member {member} inside {whole}{at}."),
                format!(
                    "The `.` operator designates a member of the structure {whole}: {member}, an \
                     object of its own at a fixed offset{at}. It is an lvalue, like the \
                     structure; nothing is read or written yet."
                ),
                vec![clauses::MEMBER, clauses::MEMBER_ORDER],
            )
        };
        if let Some(decayed) = decayed {
            let _ = write!(
                text,
                " The member is an array, so, used in an expression, it becomes {} at once.",
                self.value_text(decayed)
            );
            cited.push(clauses::ARRAY_DECAY);
        }
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Member,
                    range,
                    if decayed.is_some() {
                        format!("`{written}` decays to a pointer")
                    } else {
                        format!("`{written}` designates {member}")
                    },
                ),
                &summary,
                &text,
                &cited,
            ),
            Topic::Member,
        );
        if let Some(pointer) = pointer {
            event.operands.push(Operand::Value(pointer));
        }
        event.objects.push(target);
        result(event, selected)
    }

    /// A whole structure stored by `=` or an initializer: a copy of every
    /// member.
    #[expect(
        clippy::too_many_arguments,
        reason = "One copy's complete observation."
    )]
    pub(super) fn copied(
        &self,
        range: SourceRange,
        initializer: bool,
        full_expression: bool,
        target: usize,
        offset: i32,
        structure: usize,
        previous: &Value,
        stored: Value,
    ) -> Event {
        let location = self.location_of(target, offset, Some(structure));
        let shown = self.short_value(&stored);
        let size = self
            .structures
            .get(structure)
            .map_or(0, |layout| layout.size);
        let members = self
            .structures
            .get(structure)
            .map_or(0, |layout| layout.members.len());
        let (kind, title, mut cited) = if initializer {
            (
                EventKind::Initialize,
                format!("Initialize {location} = {shown}"),
                vec![clauses::STRUCTURE_INITIALIZER],
            )
        } else {
            (
                EventKind::Assign,
                format!("Copy {shown} into {location}"),
                vec![clauses::SIMPLE_ASSIGNMENT, clauses::STRUCTURE_ASSIGNMENT],
            )
        };
        let mut summary = if initializer {
            format!(
                "The initializer copies a whole structure into {location}: all {members} members, \
                 {shown}."
            )
        } else {
            format!(
                "The assignment copies the whole structure into {location}, all {members} \
                 members: {shown}."
            )
        };
        let mut text = if initializer {
            format!(
                "{location} is initialized from a structure of the same type, so it gets a copy \
                 of that structure's value, member by member: {}. The two are separate objects \
                 afterwards; changing one does not change the other.",
                self.value_text(&stored)
            )
        } else {
            format!(
                "Simple assignment works on whole structures: {} replaces the value of {location} \
                 (previously {}), member by member, {size} bytes in all on x86-64. The two are \
                 separate objects afterwards; changing one does not change the other.",
                self.value_text(&stored),
                self.value_text(previous)
            )
        };
        if indeterminate_member(&stored) {
            text.push_str(
                " Indeterminate members are copied as they are: a structure's value is never a \
                 trap representation.",
            );
            cited.push(clauses::STRUCTURE_VALUE);
        }
        if full_expression {
            summary.push_str(" That ends the full expression: a sequence point.");
            text.push_str(" The full expression ends here, a sequence point.");
            cited.push(clauses::FULL_EXPRESSION);
        }
        let mut event = noted(
            explained(Event::new(kind, range, title), &summary, &text, &cited),
            Topic::StructureCopy,
        );
        event.sequence_point = full_expression;
        event.operands.extend([
            Operand::Address {
                target,
                offset,
                structure: Some(structure),
            },
            Operand::Value(stored.clone()),
        ]);
        event.objects.push(target);
        result(event, Operand::Value(stored))
    }

    /// The byte offset of leaf `offset` from the structure at leaf `base` of
    /// `target`, as laid out on x86-64.
    fn byte_offset(&self, target: usize, base: i32, offset: i32) -> Option<usize> {
        let mut ty = self.object_type(target)?;
        let mut within = usize::try_from(offset).ok()?;
        let base = usize::try_from(base).ok()?;
        let mut bytes = 0;
        let mut start = None;
        let mut consumed = 0;
        loop {
            match ty {
                | Type::Struct(id) => {
                    if consumed == base && start.is_none() {
                        start = Some(bytes);
                    }
                    let (member, rest) = layout::member_at(self.structures.get(id)?, within)?;
                    bytes += member.offset;
                    consumed += within - rest;
                    within = rest;
                    ty = member.ty;
                },
                | Type::Array(_) => {
                    bytes += within * 4;
                    break;
                },
                | Type::CharArray(_) => {
                    bytes += within;
                    break;
                },
                | _ => break,
            }
        }
        Some(bytes - start?)
    }

    /// The event for a name evaluated to the object it designates, told by
    /// what the operator around it will do with the object.
    pub(super) fn designate(
        &self,
        range: SourceRange,
        id: usize,
        operand: Operand,
        role: Designation,
    ) -> Event {
        let name = &self.state.objects[id - 1].name;
        let lvalue = format!(
            "The name `{name}` is an lvalue: it designates the object `{name}`, a place in memory."
        );
        let (title, summary, text, cited): (String, String, String, &[Clause]) = match role {
            | Designation::Read => (
                format!("`{name}` names an object to read"),
                format!(
                    "The name `{name}` designates its object, a place in memory; the next step \
                     reads the value stored there."
                ),
                format!(
                    "{lvalue} The expression needs a value, so the next step reads the object, \
                     but a name on its own only says where the object is."
                ),
                &[clauses::IDENTIFIER, clauses::LVALUE],
            ),
            | Designation::Assigned => (
                format!("`{name}` is where the result goes"),
                format!(
                    "The left side of `=` says which object receives the result, so `{name}` is \
                     located, not read."
                ),
                format!(
                    "{lvalue} The left operand of an assignment must be such a place: the \
                     assignment stores into it and never reads its old value. C leaves the order \
                     of evaluating the two operands of `=` unspecified; this machine does the \
                     left one first."
                ),
                &[
                    clauses::IDENTIFIER,
                    clauses::LVALUE,
                    clauses::ASSIGNMENT_ORDER,
                ],
            ),
            | Designation::Initialized => (
                format!("`{name}` receives its initial value"),
                format!(
                    "The value after `=` becomes the first value of `{name}`, the object this \
                     declaration just created."
                ),
                format!(
                    "In a declaration, `{name}` is not an expression that is evaluated: the \
                     initializer gives the new object its first value, converted to its type as \
                     if by assignment. The machine next evaluates the initializer."
                ),
                &[clauses::SCALAR_INITIALIZER],
            ),
            | Designation::Address => (
                format!("`{name}` is the operand of `&`"),
                format!(
                    "`&` needs the object itself, not its value, so `{name}` is located but not \
                     read."
                ),
                format!("{lvalue} `&` turns that place into a pointer; nothing is read."),
                &[clauses::IDENTIFIER, clauses::LVALUE],
            ),
            | Designation::Member => (
                format!("`{name}` holds the member to select"),
                format!(
                    "`.` selects a member inside `{name}`, so the structure is located, not read \
                     as a whole."
                ),
                format!(
                    "{lvalue} The `.` operator next designates one of its members, an object at a \
                     fixed offset inside it; nothing is read yet."
                ),
                &[clauses::IDENTIFIER, clauses::LVALUE, clauses::MEMBER],
            ),
            | Designation::Updated => (
                format!("`{name}` is the object to update"),
                format!(
                    "`++` and `--` work on the object itself: they read `{name}` and store the \
                     new value back."
                ),
                format!("{lvalue} The next step reads it and stores the updated value back."),
                &[clauses::IDENTIFIER, clauses::LVALUE],
            ),
            | Designation::Compound => (
                format!("`{name}` is the object to update"),
                format!(
                    "A compound assignment works on the object itself: `{name}` is located once \
                     here, and read, combined and stored into in one later step."
                ),
                format!(
                    "{lvalue} A compound assignment evaluates its left operand only once, so this \
                     is the only step that locates `{name}`. Once the right operand has its \
                     value, one step reads `{name}`, applies the operator and stores the result \
                     back. C leaves the order of evaluating the two operands unspecified; this \
                     machine does the left one first."
                ),
                &[
                    clauses::IDENTIFIER,
                    clauses::LVALUE,
                    clauses::COMPOUND_ASSIGNMENT,
                ],
            ),
        };
        let mut event = explained(
            Event::new(EventKind::Designate, range, title),
            &summary,
            &text,
            cited,
        );
        if role != Designation::Initialized {
            event = noted(event, Topic::Lvalue);
        }
        event.objects.push(id);
        result(event, operand)
    }

    pub(super) fn element(
        &self,
        range: SourceRange,
        id: usize,
        offset: i32,
        operand: Operand,
    ) -> Event {
        let structure = match operand {
            | Operand::Address { structure, .. } => structure,
            | Operand::Value(_) => None,
        };
        let location = self.location_of(id, offset, structure);
        let mut event = if self.object_type(id).is_some_and(Type::is_array) {
            let text = format!(
                "An initializer list fills the array's elements in increasing index order, so the \
                 next value initializes {location}."
            );
            noted(
                explained(
                    Event::new(
                        EventKind::Designate,
                        range,
                        format!("Select {location} to initialize"),
                    ),
                    &format!(
                        "An initializer list fills an array from [0] upward, so the next value \
                         goes to {location}."
                    ),
                    &text,
                    &[clauses::INITIALIZER_ORDER],
                ),
                Topic::ElementOrder,
            )
        } else {
            explained(
                Event::new(
                    EventKind::Designate,
                    range,
                    format!("Select {location} to initialize"),
                ),
                &format!(
                    "An initializer list fills a structure's members in declaration order, or \
                     jumps to the one a `.member =` names, so the next value goes to {location}."
                ),
                &format!(
                    "Each brace-enclosed list initializes one object, member after member in \
                     declaration order; a designator `.member =` moves to that member and the \
                     list continues after it. The next value initializes {location}."
                ),
                &[
                    clauses::MEMBER_INITIALIZER_ORDER,
                    clauses::MEMBER_DESIGNATOR,
                ],
            )
        };
        event.objects.push(id);
        result(event, operand)
    }

    pub(super) fn decay(&self, range: SourceRange, id: usize, value: Value) -> Event {
        let object = &self.state.objects[id - 1];
        if object.literal {
            let mut event = noted(
                explained(
                    Event::new(
                        EventKind::ArrayDecay,
                        range,
                        format!("{} becomes a pointer to its first char", object.name),
                    ),
                    &format!(
                        "The string literal {name} is an array of chars, so in an expression it \
                         becomes a pointer to its first element, {name}[0].",
                        name = object.name
                    ),
                    &format!(
                        "A string literal stands for its array, {ty}, which has existed since \
                         before main. Like every array used in an expression, it is not copied: \
                         it is converted to a pointer to its first element. That pointer is all a \
                         function such as printf receives; it finds the end of the string by its \
                         null character.",
                        ty = object.r#type
                    ),
                    &[clauses::ARRAY_DECAY, clauses::STRING_LITERAL],
                ),
                Topic::ArrayDecay,
            );
            event.objects.push(id);
            return result(event, Operand::Value(value));
        }
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
                &format!(
                    "An array is never copied in an expression: `{name}` becomes a pointer to its \
                     first element, {name}[0].",
                    name = object.name,
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
        let mut shown = self.value_text(&value);
        if let (Some(Type::Char), Value::Integer(integer)) =
            (self.leaf_type(target, offset), &value)
        {
            let _ = write!(shown, " ({})", char_text(*integer));
        }
        let summary = format!(
            "The expression needs the value of {location}, so the machine reads it: {shown}."
        );
        let text = format!(
            "{location} is used for its value, not as a place, so the machine reads the value \
             stored in that object: {shown}. C calls this lvalue conversion."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::LvalueConversion,
                    range,
                    format!("Read {location} → {}", self.short_value(&value)),
                ),
                &summary,
                &text,
                &[clauses::LVALUE_CONVERSION],
            ),
            Topic::LvalueConversion,
        );
        event.operands.push(Operand::Address {
            target,
            offset,
            structure: None,
        });
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
                &format!(
                    "C does no arithmetic in char, so the char {shown} becomes the int {shown} \
                     first: same value, wider type."
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
        structure: Option<usize>,
        value: Value,
    ) -> Event {
        let location = self.location_of(target, offset, structure);
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
                &format!(
                    "`&` makes a pointer to {location}, a way to reach it later, without reading \
                     it."
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
        structure: Option<usize>,
    ) -> Event {
        let location = self.location_of(target, offset, structure);
        let operand = snippet(source, range).trim_start_matches('*');
        let summary =
            format!("`*{operand}` follows the pointer to the object it points to, {location}.");
        let text = format!(
            "`{operand}` holds {}, so `*{operand}` (indirection) is an lvalue designating \
             {location}. Nothing is read or written yet: the operator around it decides which.",
            self.value_text(&pointer),
        );
        let mut event = explained(
            Event::new(
                EventKind::Dereference,
                range,
                format!("`*{operand}` designates {location}"),
            ),
            &summary,
            &text,
            &[clauses::INDIRECTION],
        );
        event.operands.push(Operand::Value(pointer));
        event.objects.push(target);
        result(
            event,
            Operand::Address {
                target,
                offset,
                structure,
            },
        )
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
            &format!(
                "Subscripting counts elements from the pointer: it moves {steps} and designates \
                 {location}."
            ),
            &text,
            &[clauses::SUBSCRIPT, clauses::POINTER_ARITHMETIC],
        );
        event
            .operands
            .extend([Operand::Value(base), Operand::Value(index)]);
        event.objects.push(target);
        result(
            event,
            Operand::Address {
                target,
                offset,
                structure: None,
            },
        )
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
        let (title, summary, text, clause) = match operator {
            | "!" => (
                format!("Logical not: !{input} → {shown}"),
                format!("`!` turns 0 into 1 and anything else into 0, so `!{input}` is {shown}."),
                format!(
                    "! yields 1 if its operand compares equal to 0 and 0 otherwise. The operand \
                     is {input}, so the result is the int {shown}."
                ),
                clauses::LOGICAL_NOT,
            ),
            | "-" => (
                format!("Negate {input} → {shown}"),
                format!("Unary minus negates the value: `-({input})` is {shown}."),
                format!("Unary minus negates its int operand: -({input}) is {shown}."),
                clauses::UNARY_MINUS,
            ),
            | _ => (
                format!("Unary plus: {shown}"),
                format!("Unary plus leaves the value as it is: {shown}."),
                format!("Unary plus yields its (promoted) operand unchanged: {shown}."),
                clauses::UNARY_PLUS,
            ),
        };
        let mut event = explained(
            Event::new(EventKind::Unary, range, title),
            &summary,
            &text,
            &[clause],
        );
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
                        "Pointers are equal only when they point to the same place, so \
                         `{operator}` yields the int {shown}."
                    ),
                    &format!(
                        "Two pointers compare equal exactly when they point to the same place. \
                         The left is {}, the right is {}, so `{operator}` yields the int \
                         {shown}.{UNORDERED}",
                        self.value_text(&left),
                        self.value_text(&right),
                    ),
                    &[clauses::EQUALITY, clauses::EVALUATION_ORDER],
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
                let noun = match *pointer {
                    | Value::Pointer {
                        target: Some(target),
                        first,
                        ..
                    } if self.leaf_type(target, first) == Some(Type::Char) => "char",
                    | _ => "int",
                };
                let moved = match count {
                    | &Value::Integer(count) if operator == "-" =>
                        movement(count.saturating_neg(), noun),
                    | &Value::Integer(count) => movement(count, noun),
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
                    &[
                        clauses::ADDRESS_OPERATOR,
                        clauses::POINTER_ARITHMETIC,
                        clauses::EVALUATION_ORDER,
                    ]
                } else {
                    &[clauses::POINTER_ARITHMETIC, clauses::EVALUATION_ORDER]
                };
                if one_past(&output) {
                    text.push_str(
                        " That is the one-past-the-end position: a pointer there may be formed \
                         and compared, but never dereferenced.",
                    );
                }
                text.push_str(UNORDERED);
                let title = if element {
                    format!("Compute the address {written}")
                } else {
                    format!("Pointer arithmetic: {written}")
                };
                let summary = if element {
                    format!(
                        "`{written}` computes an address from the pointer and the index; the \
                         element is never read."
                    )
                } else if one_past(&output) {
                    format!(
                        "Pointer arithmetic counts elements: {sign} {} gives {}, which may be \
                         formed and compared but not read through.",
                        self.short_value(count),
                        self.value_text(&output),
                    )
                } else {
                    format!(
                        "Pointer arithmetic counts elements, not bytes: {sign} {} moves {moved}.",
                        self.short_value(count),
                    )
                };
                noted(
                    explained(
                        Event::new(EventKind::PointerArithmetic, range, title),
                        &summary,
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
                &format!("`{operator}` tests whether the pointer is null: the result is {shown}."),
                &format!(
                    "The constant 0 here is a null pointer constant, so `{operator}` tests \
                     whether the pointer is null. The result is the int {shown}.{UNORDERED}"
                ),
                &[clauses::EQUALITY, clauses::EVALUATION_ORDER],
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
                            "{a} {operator} {b} {truth}, so the comparison yields the int {shown}."
                        ),
                        &format!(
                            "A comparison yields an int: 1 when the relation holds and 0 when it \
                             does not. {a} {operator} {b} {truth}, so the result is \
                             {shown}.{UNORDERED}"
                        ),
                        &[clause, clauses::EVALUATION_ORDER],
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
                            "{a} {operator} {b} = {shown}, computed in int; the result fits in an \
                             int, so it is well defined."
                        ),
                        &format!(
                            "Both operands are int, so the usual arithmetic conversions leave \
                             them unchanged and the machine {verb} in int: {a} {operator} {b} = \
                             {shown}. The result fits in an int, so it is well defined.{UNORDERED}"
                        ),
                        &[clause, clauses::USUAL_ARITHMETIC, clauses::EVALUATION_ORDER],
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
        char_value: bool,
        target: usize,
        offset: i32,
        previous: Option<&Value>,
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
        let leaf = self.leaf_type(target, offset);
        let char_conversion = converted && leaf == Some(Type::Char);
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
                |previous| self.value_text(previous),
            );
            format!(
                "Simple assignment: {} replaces the value stored in {location} (previously \
                 {before}). Writing the object is the assignment's side effect.",
                self.value_text(&value)
            )
        };
        let mut summary = if char_conversion {
            format!(
                "{} does not fit in a char, so {location} gets {shown}; C leaves that result to \
                 the implementation.",
                self.short_value(&value)
            )
        } else if converted {
            format!("Storing the constant 0 into a pointer makes {location} a null pointer.")
        } else if initializer {
            format!("The initializer gives {location} its first value, {shown}.")
        } else {
            let before = previous.map_or_else(
                || String::from("its old value"),
                |previous| self.short_value(previous),
            );
            format!("The assignment stores {shown} into {location}, replacing {before}.")
        };
        if !converted && !char_value && leaf == Some(Type::Char) {
            let _ = write!(
                text,
                " The int {shown} is converted to {location}'s type, char; it fits in -128 to \
                 127, so the value is unchanged."
            );
        }
        let mut cited = vec![clause];
        let mut topic = None;
        if converted && leaf.is_some_and(Type::is_pointer) {
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
        if full_expression {
            summary.push_str(" That ends the full expression: a sequence point.");
        }
        let mut event = explained(Event::new(kind, range, title), &summary, &text, &cited);
        if let Some(topic) = topic {
            event = noted(event, topic);
        }
        event.sequence_point = full_expression;
        event.operands.extend([
            Operand::Address {
                target,
                offset,
                structure: None,
            },
            Operand::Value(value),
        ]);
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
        let arithmetic = if delta > 0 { "addition" } else { "subtraction" };
        if *sum != stored {
            let _ = write!(
                text,
                " The {arithmetic} happens in int; the int result did not fit in a char, so the \
                 implementation-defined conversion kept its low 8 bits."
            );
        } else if self.leaf_type(target, offset) == Some(Type::Char) {
            let _ = write!(
                text,
                " The char is promoted to int for the {arithmetic}, and the int result, which \
                 fits, is converted back to char when it is stored."
            );
        }
        text.push_str(" The full expression ends here, a sequence point.");
        let summary = format!("`{form}` {verb} {location}, so it changes from {old} to {new}.");
        let increment = match (prefix, delta > 0) {
            | (true, true) => clauses::PREFIX_INCREMENT,
            | (true, false) => clauses::PREFIX_DECREMENT,
            | (false, true) => clauses::POSTFIX_INCREMENT,
            | (false, false) => clauses::POSTFIX_DECREMENT,
        };
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Increment,
                    range,
                    format!("Update {location}: {old} → {new}"),
                ),
                &summary,
                &text,
                &[increment, clauses::FULL_EXPRESSION],
            ),
            Topic::Increment,
        );
        event.sequence_point = true;
        event.operands.extend([
            Operand::Address {
                target,
                offset,
                structure: None,
            },
            Operand::Value(current),
        ]);
        event.objects.push(target);
        result(event, Operand::Value(stored))
    }

    /// `(to) value`: the cast at `range` converted `value`, of type
    /// `from`, to `converted`.
    ///
    /// C99: §6.5.4 paragraph 4, p. 81; PDF p. 93.
    /// C99: §6.3.1.3 paragraphs 1 and 3, p. 43; PDF p. 55.
    pub(super) fn cast(
        &self,
        range: SourceRange,
        source: &str,
        from: Type,
        to: Type,
        value: Value,
        converted: Value,
    ) -> Event {
        let form = snippet(source, range);
        let name = super::expressions::cast_spelling(to);
        let old = self.short_value(&value);
        let new = self.short_value(&converted);
        let source_type = super::expressions::cast_spelling(from);
        let (title, summary, text, cited, topic): (String, String, String, Vec<Clause>, Topic) =
            if from == to {
                (
                    format!("Cast to {name}: no conversion, {new}"),
                    format!(
                        "`({name})` names the type the value already has, so the cast changes \
                         nothing: the value stays {new}."
                    ),
                    format!(
                        "A cast converts its operand's value to the type it names. The operand is \
                         already {}, so this cast specifies no conversion, and C says such a cast \
                         has no effect on the type or the value: {new}.",
                        article(source_type)
                    ),
                    vec![clauses::CAST],
                    Topic::Cast,
                )
            } else if from.is_pointer() {
                let target = self.value_text(&converted);
                if to.points_to_const() {
                    (
                        format!("Cast to {name}: add const"),
                        format!(
                            "`{form}` makes a pointer to const from a plain pointer: it still \
                             points to the same place, but nothing may be changed through it."
                        ),
                        format!(
                            "A cast converts its operand's value to the type it names, here from \
                             {source_type} to {name}. A pointer to a type converts to a pointer \
                             to the const version of that type and still points to the same \
                             place: {target}. Through the new pointer the object may be read but \
                             not changed."
                        ),
                        vec![clauses::CAST, clauses::QUALIFIED_POINTER],
                        Topic::Cast,
                    )
                } else {
                    (
                        format!("Cast to {name}: remove const"),
                        format!(
                            "`{form}` removes the const from the pointer; it still points to the \
                             same place, but storing through it into an object defined const \
                             would be undefined."
                        ),
                        format!(
                            "A cast converts its operand's value to the type it names, here from \
                             {source_type} to {name}. The pointer still points to the same place, \
                             {target}, and C lets a cast drop the const, so the compiler no \
                             longer stops a store through it. Reading through it is fine. But if \
                             the object it points to was defined const, an attempt to modify it \
                             through this pointer is undefined behavior: the const was a promise \
                             about the object, not about the pointer."
                        ),
                        vec![clauses::CAST, clauses::CONST_WRITE],
                        Topic::Const,
                    )
                }
            } else if converted == value {
                (
                    format!("Cast {old} to {name}: the value fits"),
                    format!(
                        "The cast converts the {source_type} {old} to {name}; {old} fits in {}, \
                         so the value is unchanged.",
                        range_text(to)
                    ),
                    format!(
                        "A cast converts its operand's value to the type it names, here from \
                         {source_type} to {name}. When the new type can represent the value, the \
                         conversion keeps it: {old} fits in {}, so the result is the {name} \
                         {new}.{}",
                        range_text(to),
                        if to == Type::Int {
                            " Every char value fits in an int."
                        } else {
                            ""
                        }
                    ),
                    vec![clauses::CAST, clauses::INTEGER_UNCHANGED],
                    Topic::Cast,
                )
            } else {
                (
                    format!("Cast {old} to char → {new}"),
                    format!(
                        "{old} does not fit in a char, so the result is implementation-defined; \
                         bcc and GCC wrap it to {new}."
                    ),
                    format!(
                        "A cast converts its operand's value to the type it names, here from \
                         {source_type} to char. Plain char is signed on x86-64 and holds -128 to \
                         127, and {old} is outside that range. Converting a value that does not \
                         fit into a signed type gives an implementation-defined result (or raises \
                         an implementation-defined signal): each compiler must choose and \
                         document one. bcc and GCC wrap the value modulo 256, keeping its low 8 \
                         bits, so {old} becomes {new}. Where plain char is unsigned, as on ARM \
                         Linux, the cast would give {} instead.",
                        match value {
                            | Value::Integer(integer) => integer.rem_euclid(256),
                            | _ => 0,
                        }
                    ),
                    vec![
                        clauses::CAST,
                        clauses::SIGNED_CONVERSION,
                        clauses::CHAR_SIGNEDNESS,
                    ],
                    Topic::Cast,
                )
            };
        let mut event = noted(
            explained(
                Event::new(EventKind::Cast, range, title),
                &summary,
                &text,
                &cited,
            ),
            topic,
        );
        event.operands.push(Operand::Value(value));
        if let Value::Pointer {
            target: Some(target),
            ..
        } = converted
        {
            event.objects.push(target);
        }
        result(event, Operand::Value(converted))
    }

    /// `E1 op= E2` read `current` from the object at leaf `offset` of
    /// `target`, combined it with `right` into `combined`, and stored
    /// `stored`, its conversion to the object's type.
    ///
    /// C99: §6.5.16.2 paragraph 3, p. 93; PDF p. 105.
    #[expect(
        clippy::too_many_arguments,
        reason = "One compound assignment's complete observation."
    )]
    pub(super) fn compound(
        &self,
        range: SourceRange,
        source: &str,
        operator: &str,
        target: usize,
        offset: i32,
        current: Value,
        right: Value,
        combined: &Value,
        stored: Value,
    ) -> Event {
        let location = self.location(target, offset);
        let form = snippet(source, range);
        let (old, operand, new) = (
            self.short_value(&current),
            self.short_value(&right),
            self.short_value(&stored),
        );
        let (left_text, right_text) = form
            .split_once(&format!("{operator}="))
            .map_or((location.as_str(), ""), |(left, right)| {
                (left.trim(), right.trim())
            });
        let meaning = format!("{left_text} = {left_text} {operator} ({right_text})");
        let pointer = matches!(current, Value::Pointer { .. });
        let (verb, clause) = match operator {
            | _ if pointer => ("moves it", clauses::POINTER_ARITHMETIC),
            | "+" => ("adds", clauses::ADDITION),
            | "-" => ("subtracts", clauses::ADDITION),
            | "*" => ("multiplies by", clauses::MULTIPLICATION),
            | "/" => ("divides, truncating toward zero, by", clauses::DIVISION),
            | _ => ("takes the remainder of dividing by", clauses::DIVISION),
        };
        let action = if pointer {
            let count = match right {
                | Value::Integer(count) if operator == "-" => count.saturating_neg(),
                | Value::Integer(count) => count,
                | _ => 0,
            };
            let noun = if matches!(self.leaf_type(target, offset), Some(Type::CharPointer(_))) {
                "char"
            } else {
                "int"
            };
            format!("moves it {}", movement(count, noun))
        } else {
            format!("{verb} {operand}")
        };
        let mut text = format!(
            "`{form}` means `{meaning}`, except that {left_text} is evaluated only once: an \
             earlier step located {location}, and this step reads it ({old}), {action}, and \
             stores the result back. {location} is now {new}."
        );
        let mut cited = vec![clauses::COMPOUND_ASSIGNMENT, clause];
        if *combined != stored {
            let _ = write!(
                text,
                " The arithmetic happens in int, giving {}; that does not fit in a char, so the \
                 implementation-defined conversion back to char keeps its low 8 bits, as bcc and \
                 GCC do.",
                self.short_value(combined)
            );
            cited.push(clauses::SIGNED_CONVERSION);
        } else if self.leaf_type(target, offset) == Some(Type::Char) {
            let _ = write!(
                text,
                " The char is promoted to int for the arithmetic, and the int result, which fits, \
                 is converted back to char when it is stored."
            );
        } else if pointer {
            text.push_str(" Pointer arithmetic counts elements, not bytes.");
        }
        text.push_str(" The full expression ends here, a sequence point.");
        cited.push(clauses::FULL_EXPRESSION);
        let summary = format!(
            "`{form}` is `{meaning}` with {left_text} evaluated once: {location} changes from \
             {old} to {new}."
        );
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::CompoundAssign,
                    range,
                    format!("Update {location}: {old} → {new}"),
                ),
                &summary,
                &text,
                &cited,
            ),
            Topic::CompoundAssign,
        );
        event.sequence_point = true;
        event.operands.extend([
            Operand::Address {
                target,
                offset,
                structure: None,
            },
            Operand::Value(current),
            Operand::Value(right),
        ]);
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
            &format!(
                "The statement's value, {}, is not used: the statement ran only for its effects.",
                self.short_value(&value),
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
        has_else: bool,
    ) -> Event {
        let shown = self.short_value(&value);
        let condition = snippet(source, range);
        let (title, outcome, short, clause) = match (statement, truth) {
            | ("if", true) => (
                "true: take the if branch",
                "the first branch runs",
                "the if branch runs",
                clauses::IF_STATEMENT,
            ),
            | ("if", false) if has_else => (
                "false: take the else branch",
                "the first branch is skipped and the else branch runs",
                "the else branch runs",
                clauses::IF_STATEMENT,
            ),
            | ("if", false) => (
                "false: skip the if branch",
                "the first branch is skipped; with no else, the if statement is done",
                "the if branch is skipped",
                clauses::IF_STATEMENT,
            ),
            | (_, true) => (
                "true: run the loop body",
                "the loop body runs",
                "the loop body runs",
                clauses::ITERATION,
            ),
            | (_, false) => (
                "false: leave the loop",
                "the loop ends",
                "the loop ends",
                clauses::ITERATION,
            ),
        };
        let counts = if truth { "true" } else { "false" };
        // An omitted for condition is a nonzero constant; its range is the
        // keyword.
        let omitted = statement == "for" && condition == "for";
        let summary = if omitted {
            format!("This for statement has no condition, which counts as true, so {short}.")
        } else {
            format!("`{condition}` is {shown}, which counts as {counts}, so {short}.")
        };
        let meaning = match value {
            | Value::Pointer { target: None, .. } => "a null pointer counts as false",
            | Value::Pointer { .. } => "a non-null pointer counts as true",
            | _ if truth => "any nonzero value counts as true",
            | _ => "zero counts as false",
        };
        let text = if omitted {
            format!(
                "This for statement omits its controlling expression, and C replaces an omitted \
                 one with a nonzero constant, so {outcome}: the loop ends only by a return or a \
                 jump out of it."
            )
        } else {
            format!(
                "The controlling expression `{condition}` of this {statement} statement evaluated \
                 to {shown}. In C, {meaning}, so {outcome}. The condition is a full expression, \
                 so this is a sequence point."
            )
        };
        let cited: &[Clause] = if omitted {
            &[clauses::OMITTED_CONDITION]
        } else {
            &[clause, clauses::FULL_EXPRESSION]
        };
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Branch,
                    range,
                    if omitted {
                        String::from("No condition: run the loop body")
                    } else {
                        format!("Condition is {title}")
                    },
                ),
                &summary,
                &text,
                cited,
            ),
            Topic::Condition,
        );
        // An omitted condition is no expression, so no sequence point.
        event.sequence_point = !omitted;
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
                "All arguments are evaluated before the call, in an order C leaves unspecified \
                 (this machine goes left to right); a sequence point follows. The machine creates \
                 a new activation of {name} (frame #{frame}) on top of the call stack."
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
            if arguments
                .iter()
                .any(|argument| matches!(argument, Value::Struct { .. }))
            {
                text.push_str(
                    " A structure argument is copied whole, every member: the callee works on its \
                     own copy, and changing it leaves the caller's structure alone.",
                );
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
        let summary = match parameters.as_slice() {
            | [] =>
                format!("The call runs {name} in a new frame of its own; it takes no arguments."),
            | [only] => format!(
                "The call runs {name} in a new frame of its own, and its parameter {} gets a copy \
                 of the argument.",
                self.names(&[*only])
            ),
            | all => format!(
                "The call runs {name} in a new frame of its own, and its parameters {} get copies \
                 of the arguments.",
                self.names(all)
            ),
        };
        let mut event = explained(
            Event::new(
                EventKind::Call,
                range,
                format!("Call {name} (frame #{frame})"),
            ),
            &summary,
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
            if matches!(value, Value::Struct { .. }) {
                text.push_str(
                    " A structure is returned by value: the caller gets a copy of every member, \
                     and the function's own objects can end safely.",
                );
            }
        } else {
            let _ = write!(
                text,
                " Returning from main ends the program as if exit({shown}) were called: its exit \
                 status is {shown}{}.",
                shell_status(&value)
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
        let summary = if fallthrough {
            String::from(
                "Reaching main's closing brace returns 0, so the program ends with exit status 0.",
            )
        } else if call.is_some() {
            format!(
                "The return hands {shown} back to the caller of {function}, and the objects of \
                 its frame cease to exist."
            )
        } else {
            format!(
                "Returning {shown} from main ends the program with exit status {shown}{}.",
                shell_status(&value)
            )
        };
        let mut event = explained(
            Event::new(
                EventKind::Return,
                range,
                format!("Return {shown} from {function}"),
            ),
            &summary,
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

    /// How the source names a location: `x`, `values[1]`, `p.x`, or
    /// `values[3]` (one past), qualified with the owning function when it is
    /// not the innermost activation's.
    pub(super) fn location(&self, target: usize, offset: i32) -> String {
        self.location_of(target, offset, None)
    }

    /// [`Self::location`] for the whole structure `structure` at `offset`
    /// when given, rather than its first scalar.
    pub(super) fn location_of(
        &self,
        target: usize,
        offset: i32,
        structure: Option<usize>,
    ) -> String {
        let Some(object) = target
            .checked_sub(1)
            .and_then(|index| self.state.objects.get(index))
        else {
            return format!("object #{target}");
        };
        let path = layout::path(&object.value, offset, structure, &self.structures);
        let mut text = if path.ends_with(']') {
            format!("{}{path}", object.name)
        } else if let Some(past) = path.strip_prefix('+') {
            format!("`{}` + {past}", object.name)
        } else {
            format!("`{}{path}`", object.name)
        };
        if object.frame_id != 0
            && self
                .activations
                .last()
                .is_some_and(|activation| activation.id != object.frame_id)
        {
            let _ = write!(text, " (in {})", self.function_name(object.frame_id));
        }
        text
    }

    /// The name of the array whose element 0 is leaf `first` of `target`:
    /// `values`, `s.a`, or the object itself when it is no array.
    pub(super) fn array_name(&self, target: usize, first: i32) -> String {
        let Some(object) = target
            .checked_sub(1)
            .and_then(|index| self.state.objects.get(index))
        else {
            return format!("object #{target}");
        };
        let path = layout::path(&object.value, first, None, &self.structures);
        let path = path.strip_suffix("[0]").unwrap_or(&path);
        format!("`{}{path}`", object.name)
    }

    fn value_text(&self, value: &Value) -> String {
        match value {
            | Value::Indeterminate => String::from("an indeterminate value"),
            | Value::Integer(integer) => format!("{integer}"),
            | Value::Pointer { target: None, .. } => String::from("a null pointer"),
            | &Value::Pointer {
                target: Some(target),
                offset,
                first,
                structure,
                ..
            } =>
                if one_past(value) {
                    format!(
                        "a pointer one past the end of {}",
                        self.array_name(target, first)
                    )
                } else {
                    format!(
                        "a pointer to {}",
                        self.location_of(target, offset, structure)
                    )
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
            | Value::Struct { structure, members } => {
                let mut text = String::from("{");
                let names = self
                    .structures
                    .get(*structure)
                    .map(|layout| &layout.members);
                for (index, member) in members.iter().enumerate() {
                    if index > 0 {
                        text.push_str(", ");
                    }
                    if let Some(name) = names.and_then(|names| names.get(index)) {
                        let _ = write!(text, ".{} = ", name.name);
                    }
                    text.push_str(&self.short_value(member));
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
                structure,
                ..
            } => format!(
                "&{}",
                self.location_of(target, offset, structure).replace('`', "")
            ),
            | Value::Array(_) | Value::Struct { .. } => self.value_text(value),
        }
    }

    /// Structure `id`'s layout in words: its size and each member's offset,
    /// with the padding between them.
    fn layout_text(&self, id: usize) -> String {
        let Some(layout) = self.structures.get(id) else {
            return String::from("a structure");
        };
        let mut text = format!(
            "{} bytes on x86-64, aligned to {}: ",
            layout.size, layout.align
        );
        for (index, member) in layout.members.iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            let _ = write!(
                text,
                "`{}` ({}) at byte {}",
                member.name, member.r#type, member.offset
            );
            if member.padding > 0 {
                let after = if index + 1 == layout.members.len() {
                    "at the end"
                } else {
                    "before the next member"
                };
                let _ = write!(
                    text,
                    " followed by {} byte{} of padding {after}",
                    member.padding,
                    if member.padding == 1 { "" } else { "s" }
                );
            }
        }
        text.push('.');
        text
    }

    /// Whether structure type `ty` has padding.
    fn padded(&self, ty: Type) -> bool {
        match ty {
            | Type::Struct(id) => self.structures.get(id).is_some_and(|layout| {
                layout
                    .members
                    .iter()
                    .any(|member| member.padding > 0 || self.padded(member.ty))
            }),
            | _ => false,
        }
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
            "Execution enters a { … } block, a new scope: names declared inside belong to it.",
            "A { … } compound statement is a block, and a block is a scope: a name declared in it \
             can be used from its declaration up to the closing brace, and nowhere outside it.",
            &[clauses::COMPOUND_STATEMENT, clauses::BLOCK_SCOPE],
        ),
        Topic::Scope,
    )
}

/// A character constant, spelled `spelling`, evaluates to `integer`.
pub(super) fn character(range: SourceRange, integer: i32, spelling: &str, value: Operand) -> Event {
    let text = format!(
        "{spelling} is a character constant. Its type is int, not char, and its value is the \
         character's code in the execution character set (ASCII on x86-64): {integer}."
    );
    result(
        noted(
            explained(
                Event::new(
                    EventKind::Constant,
                    range,
                    format!("Evaluate the character constant {spelling}"),
                ),
                &format!("The character constant {spelling} is the int {integer}, its code."),
                &text,
                &[clauses::CHARACTER_CONSTANT],
            ),
            Topic::CharacterConstant,
        ),
        value,
    )
}

/// A char value as a character constant: `'A'`, `'\n'`, or its code when
/// it is no single byte.
fn char_text(value: i32) -> String {
    i8::try_from(value).map_or_else(
        |_| format!("{value}"),
        |byte| super::library::character_name(byte.to_ne_bytes()[0]),
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
            &format!("The constant {integer} is simply the int value {integer}."),
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
            "The list has no value for this element or member, so C initializes it to zero.",
            "The initializer list gives no value for this element or member. Every element or \
             member without an initializer of its own is initialized as an object with static \
             storage duration would be: to zero, or to a null pointer for a pointer.",
            &[
                clauses::PARTIAL_INITIALIZER,
                clauses::IMPLICIT_INITIALIZATION,
            ],
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
            "One pass through the loop is done, so control goes back to test the condition again.",
            "The loop body (and a for loop's update expression) has finished, so control returns \
             to the controlling expression, which decides whether to run the body again.",
            &[clauses::ITERATION],
        )
    } else {
        explained(
            Event::new(EventKind::Jump, range, String::from("Skip the else branch")),
            "The if branch is done, so the else branch is skipped.",
            "The if branch has finished, so control jumps over the else branch.",
            &[clauses::IF_STATEMENT],
        )
    }
}

/// Whether a structure value has an indeterminate member.
fn indeterminate_member(value: &Value) -> bool {
    let mut leaves = Vec::new();
    layout::flatten(value, &mut leaves);
    leaves.contains(&Value::Indeterminate)
}

/// Whether a pointer points one past the end of its array.
const fn one_past(value: &Value) -> bool {
    matches!(
        *value,
        Value::Pointer {
            target: Some(_),
            offset,
            first,
            length,
            ..
        } if offset == first + length
    )
}

/// How a shell on Linux sees main's return `value` when it is outside 0 to
/// 255: only its low 8 bits reach it.
fn shell_status(value: &Value) -> String {
    match *value {
        | Value::Integer(status) if !(0..=255).contains(&status) =>
            format!(", which a shell on Linux sees as {}", status & 0xFF),
        | _ => String::new(),
    }
}

/// `event` with its one-sentence `summary`, the details `text`, and the
/// rules it applied.
fn explained(mut event: Event, summary: &str, text: &str, cited: &[Clause]) -> Event {
    event.summary = String::from(summary);
    event.explanation = String::from(text);
    event.clauses.extend_from_slice(cited);
    event
}

/// `event` with the note on `topic`.
pub(super) fn noted(mut event: Event, topic: Topic) -> Event {
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

/// A type with its article: "an int", "a char", "an int *".
fn article(spelling: &str) -> String {
    if spelling.starts_with(['a', 'e', 'i', 'o', 'u']) {
        format!("an {spelling}")
    } else {
        format!("a {spelling}")
    }
}

/// The range of values an integer type holds, as the narration says it.
const fn range_text(ty: Type) -> &'static str {
    match ty {
        | Type::Char => "a char (-128 to 127)",
        | _ => "an int",
    }
}
