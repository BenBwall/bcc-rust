//! The words of the C standard the narration uses, each defined in plain
//! language with the paragraph that defines it. The narration marks the
//! first use of each term in a step's text, and the page lists them all in
//! its glossary.

use leptos::prelude::*;

use crate::{
    pipeline::Stage,
    snapshot::N1256_URL,
};

/// One defined term.
#[derive(Debug, PartialEq, Eq)]
pub struct Term {
    /// The anchor: the glossary entry is `#term-{id}`.
    pub id:         &'static str,
    pub name:       &'static str,
    /// Spellings the narration uses, matched without regard to case.
    pub phrases:    &'static [&'static str],
    pub definition: &'static str,
    /// The defining paragraph of C99 and its page in N1256.
    pub clause:     &'static str,
    pub page:       u16,
}

impl Term {
    pub fn link(&self) -> String {
        format!("{N1256_URL}#page={}", self.page)
    }
}

/// Every term, in the glossary's order: from what a program is made of to
/// what can go wrong. Longer phrases come before the shorter ones they
/// contain (`lvalue conversion` before `lvalue`).
pub static TERMS: [Term; 62] = [
    Term {
        id:         "abstract-machine",
        name:       "Abstract machine",
        phrases:    &["abstract machine"],
        definition: "The idealized computer the C standard uses to say what a program means: \
                     objects in memory, values read and stored, and rules for when each effect \
                     happens. A compiler may produce any code that behaves the same way.",
        clause:     "5.1.2.3",
        page:       25,
    },
    Term {
        id:         "translation-phase",
        name:       "Translation phase",
        phrases:    &["translation phases", "translation phase"],
        definition: "One of the eight steps that turn a source file into a program, before it \
                     runs: phase 4 executes directives and expands macros, phase 6 joins adjacent \
                     string literals, phase 7 compiles.",
        clause:     "5.1.1.2",
        page:       21,
    },
    Term {
        id:         "macro",
        name:       "Macro",
        phrases:    &["macros", "macro"],
        definition: "A name #define gives to a replacement list. The preprocessor replaces each \
                     later use of the name before the program is compiled, so a macro has no type \
                     and no storage.",
        clause:     "6.10.3p9",
        page:       164,
    },
    Term {
        id:         "header",
        name:       "Header",
        phrases:    &["headers", "header"],
        definition: "A set of declarations the C library provides, such as <stdio.h>. #include \
                     pastes it into the program in translation phase 4, so the program can call \
                     the functions it declares; the functions' code is in the library itself.",
        clause:     "7.1.2p1",
        page:       177,
    },
    Term {
        id:         "prototype",
        name:       "Prototype",
        phrases:    &["prototypes", "prototype"],
        definition: "A declaration of a function that gives its parameters' types: int \
                     length(const char *text) says that length takes a pointer to const char. \
                     With one in sight, a call is checked and each argument converted to its \
                     parameter's type; headers are mostly prototypes.",
        clause:     "6.2.1p2",
        page:       41,
    },
    Term {
        id:         "conversion-specification",
        name:       "Conversion specification",
        phrases:    &["conversion specifications", "conversion specification"],
        definition: "A directive in printf's format that starts with %, such as %d or %.3f: it \
                     takes the next argument, turns it into text as its letter says, and writes \
                     that text in its place.",
        clause:     "7.19.6.1p4",
        page:       287,
    },
    Term {
        id:         "object",
        name:       "Object",
        phrases:    &["objects", "object"],
        definition: "A region of memory that holds a value of some type: a variable, or one \
                     element of an array.",
        clause:     "3.14",
        page:       17,
    },
    Term {
        id:         "string-literal",
        name:       "String literal",
        phrases:    &["string literals", "string literal"],
        definition: "Characters between double quotes, such as \"Ada\". Each becomes an unnamed \
                     char array of static storage duration, one element longer than the \
                     characters for the null character at its end, which the program must not \
                     modify.",
        clause:     "6.4.5p5",
        page:       74,
    },
    Term {
        id:         "null-character",
        name:       "Null character",
        phrases:    &["null character"],
        definition: "The char whose bits are all 0, written '\\0'. It ends a string: printf, puts \
                     and the other library functions read characters until they reach it.",
        clause:     "5.2.1p2",
        page:       29,
    },
    Term {
        id:         "lvalue-conversion",
        name:       "Lvalue conversion",
        phrases:    &["lvalue conversion"],
        definition: "Reading the value stored in the object an lvalue designates. It happens \
                     whenever an lvalue is used for its value rather than as a place.",
        clause:     "6.3.2.1p2",
        page:       58,
    },
    Term {
        id:         "modifiable-lvalue",
        name:       "Modifiable lvalue",
        phrases:    &["modifiable lvalue"],
        definition: "An lvalue the program may store into: not an array and not const-qualified. \
                     The left side of = and the operand of ++ must be one.",
        clause:     "6.3.2.1p1",
        page:       58,
    },
    Term {
        id:         "lvalue",
        name:       "Lvalue",
        phrases:    &["lvalue"],
        definition: "An expression that designates an object, a place in memory: a variable's \
                     name, *p or a[i]. The operator around it decides whether the object is read \
                     or written.",
        clause:     "6.3.2.1p1",
        page:       58,
    },
    Term {
        id:         "side-effect",
        name:       "Side effect",
        phrases:    &["side effects", "side effect"],
        definition: "A change to the program's state, such as storing into an object. An \
                     expression can have side effects as well as a value: x = 5 has the value 5 \
                     and the side effect of storing it.",
        clause:     "5.1.2.3p2",
        page:       25,
    },
    Term {
        id:         "sequence-point",
        name:       "Sequence point",
        phrases:    &["sequence point"],
        definition: "A point in execution where every earlier side effect is complete and no \
                     later one has begun: the end of a full expression, after the first operand \
                     of &&, || and ?:, at a comma operator, and just before a function is called. \
                     Modifying an object twice between two of them, or modifying it and reading \
                     it for something else (i + i++), is undefined.",
        clause:     "5.1.2.3p2",
        page:       25,
    },
    Term {
        id:         "short-circuit",
        name:       "Short-circuit evaluation",
        phrases:    &["short-circuit evaluation"],
        definition: "What && and || do: they evaluate their left operand first and stop when it \
                     settles the result, so the right operand of a && whose left is 0, or of a || \
                     whose left is nonzero, is never evaluated. p != 0 && *p > 0 relies on it.",
        clause:     "6.5.13p4",
        page:       101,
    },
    Term {
        id:         "constant-expression",
        name:       "Constant expression",
        phrases:    &["constant expression"],
        definition: "An expression the compiler can work out while it translates the program, \
                     such as 1 << 3 or sizeof(int) * 4: no call, no assignment and no read of an \
                     object. A static object's initializer must be one, since its value is fixed \
                     before the program starts.",
        clause:     "6.6p2",
        page:       107,
    },
    Term {
        id:         "full-expression",
        name:       "Full expression",
        phrases:    &["full expression"],
        definition: "An expression that is not part of a larger one: an expression statement, an \
                     initializer, a condition or a return value. Its end is a sequence point.",
        clause:     "6.8p4",
        page:       143,
    },
    Term {
        id:         "compound-assignment",
        name:       "Compound assignment",
        phrases:    &["compound assignments", "compound assignment"],
        definition: "An assignment such as x += y, which means x = x + (y), except that x is \
                     evaluated only once.",
        clause:     "6.5.16.2p3",
        page:       105,
    },
    Term {
        id:         "static-storage",
        name:       "Static storage duration",
        phrases:    &["static storage duration", "static storage"],
        definition: "How long objects declared outside every function, or with static, last: the \
                     whole run of the program. They are initialized once, before main is called, \
                     to their constant initializer or to zero.",
        clause:     "6.2.4p3",
        page:       44,
    },
    Term {
        id:         "storage-duration",
        name:       "Automatic storage duration",
        phrases:    &["automatic storage duration", "storage duration"],
        definition: "How long an object's storage lasts. Local variables not declared static have \
                     automatic storage duration: they exist from entry into their block until the \
                     block ends, and each entry gets fresh ones.",
        clause:     "6.2.4p5",
        page:       44,
    },
    Term {
        id:         "lifetime",
        name:       "Lifetime",
        phrases:    &["lifetimes", "lifetime"],
        definition: "The part of execution during which an object exists and keeps its value. \
                     Using an object, or a pointer to it, after its lifetime has ended is \
                     undefined.",
        clause:     "6.2.4p2",
        page:       44,
    },
    Term {
        id:         "scope",
        name:       "Scope",
        phrases:    &["scope"],
        definition: "The part of the program's text where a name can be used. A name declared \
                     inside a block can be used up to that block's closing brace.",
        clause:     "6.2.1p4",
        page:       41,
    },
    Term {
        id:         "frame",
        name:       "Frame (activation)",
        phrases:    &["activation", "frame"],
        definition: "The objects that one call of a function owns: its parameters and local \
                     variables. Every call, including a recursive one, gets its own. The standard \
                     describes this as each call's own automatic objects.",
        clause:     "6.2.4p5",
        page:       44,
    },
    Term {
        id:         "by-value",
        name:       "Pass by value",
        phrases:    &["by value"],
        definition: "How C passes every argument: the parameter is a new object initialized with \
                     a copy of the argument's value.",
        clause:     "6.5.2.2p4",
        page:       83,
    },
    Term {
        id:         "structure",
        name:       "Structure",
        phrases:    &["structures", "structure"],
        definition: "A type made of named member objects, each with its own type, laid out one \
                     after another in declaration order. Unlike an array, a structure is copied \
                     whole by =, by a call and by a return.",
        clause:     "6.2.5p20",
        page:       47,
    },
    Term {
        id:         "member",
        name:       "Member",
        phrases:    &["members", "member"],
        definition: "One named object inside a structure, at a fixed offset from its start. s.m \
                     and p->m designate member m of the structure s, or of the one p points to.",
        clause:     "6.5.2.3p3",
        page:       85,
    },
    Term {
        id:         "alignment",
        name:       "Alignment",
        phrases:    &["alignment", "aligned"],
        definition: "Where an object of a type may start: at an address that is a multiple of its \
                     alignment. On x86-64 Linux an int is aligned to 4 and a pointer or a double \
                     to 8, so the processor can load each in one access.",
        clause:     "3.2",
        page:       15,
    },
    Term {
        id:         "padding",
        name:       "Padding",
        phrases:    &["padding"],
        definition: "Unnamed bytes the compiler leaves inside or at the end of a structure so \
                     that each member is aligned for its type. They belong to no member and hold \
                     unspecified values.",
        clause:     "6.7.2.1p13",
        page:       115,
    },
    Term {
        id:         "indeterminate",
        name:       "Indeterminate value",
        phrases:    &["indeterminate"],
        definition: "A value you cannot rely on: either an unspecified value or a trap \
                     representation. Automatic local variables are indeterminate until something \
                     is stored in them; static ones start at zero.",
        clause:     "3.17.2",
        page:       18,
    },
    Term {
        id:         "trap-representation",
        name:       "Trap representation",
        phrases:    &["trap representation"],
        definition: "A bit pattern that is not a value of its type at all. Reading one as that \
                     type is undefined; a character type such as unsigned char may read any byte.",
        clause:     "6.2.6.1p5",
        page:       49,
    },
    Term {
        id:         "integer-promotions",
        name:       "Integer promotions",
        phrases:    &["integer promotions"],
        definition: "The rule that converts values of types narrower than int, such as char, to \
                     int before arithmetic or a comparison.",
        clause:     "6.3.1.1p2",
        page:       54,
    },
    Term {
        id:         "usual-arithmetic-conversions",
        name:       "Usual arithmetic conversions",
        phrases:    &["usual arithmetic conversions"],
        definition: "The rule that converts both operands of an arithmetic operator or a \
                     comparison to one common type before computing: the lower-ranked operand \
                     converts, and a signed one converts to unsigned when the unsigned type ranks \
                     at least as high, so -1 < 1u is false. Two ints need no conversion.",
        clause:     "6.3.1.8p1",
        page:       56,
    },
    Term {
        id:         "integer-conversion-rank",
        name:       "Integer conversion rank",
        phrases:    &["integer conversion rank", "rank"],
        definition: "The order of the integer types that the conversions use: char, short, int, \
                     long and long long, from lowest to highest. An unsigned type has the rank of \
                     its signed one.",
        clause:     "6.3.1.1p1",
        page:       54,
    },
    Term {
        id:         "unsigned-type",
        name:       "Unsigned type",
        phrases:    &["unsigned types", "unsigned type"],
        definition: "An integer type written with unsigned, the same size as its signed type, \
                     that holds only values from 0 up: unsigned int holds 0 to 4294967295 on \
                     x86-64 Linux. Its arithmetic never overflows.",
        clause:     "6.2.5p6",
        page:       45,
    },
    Term {
        id:         "modulo-reduction",
        name:       "Reduction modulo 2^N",
        phrases:    &[
            "reduced modulo",
            "reduces it modulo",
            "reduces the value modulo",
        ],
        definition: "What an unsigned type does with a value it cannot hold: it adds or subtracts \
                     2^N, one more than its largest value, until the value fits, so -1 becomes \
                     the largest value. This is defined, unlike signed overflow.",
        clause:     "6.2.5p9",
        page:       46,
    },
    Term {
        id:         "bitwise-operator",
        name:       "Bitwise operator",
        phrases:    &["bitwise operators", "bitwise operator"],
        definition: "&, ^, | and ~, which work on an integer's bits one position at a time: a bit \
                     of a & b is 1 where both operands' bits are 1, of a | b where either is, of \
                     a ^ b where exactly one is, and ~a flips every bit.",
        clause:     "6.5.10p4",
        page:       99,
    },
    Term {
        id:         "twos-complement",
        name:       "Two's complement",
        phrases:    &["two's complement"],
        definition: "How x86-64, like every machine bcc targets, stores a negative integer: -x is \
                     the bits of x flipped, plus 1, so -1 is all 1 bits and ~x is -x - 1. C99 \
                     also allows sign and magnitude and ones' complement; C23 requires two's \
                     complement.",
        clause:     "6.2.6.2p2",
        page:       50,
    },
    Term {
        id:         "size-t",
        name:       "size_t",
        phrases:    &["size_t"],
        definition: "The unsigned integer type of sizeof's result, defined by <stddef.h> and \
                     <stdio.h>: unsigned long on x86-64 Linux. printf prints it with %zu.",
        clause:     "7.17p2",
        page:       266,
    },
    Term {
        id:         "floating-type",
        name:       "Floating type",
        phrases:    &["real floating types", "floating types", "floating type"],
        definition: "A type that holds numbers with a fraction, approximately: float, double and \
                     long double, each holding every value of the one before. On x86-64 Linux \
                     they are IEEE 754 binary32 and binary64 and the x87 80-bit extended format.",
        clause:     "6.2.5p10",
        page:       46,
    },
    Term {
        id:         "significand",
        name:       "Significand",
        phrases:    &["significand"],
        definition: "The digits of a floating value, a binary fraction such as 1.1001100… that \
                     the exponent scales by a power of two. Its width, 53 bits for a double, is \
                     the type's precision; a value that needs more bits is rounded.",
        clause:     "5.2.4.2.2p2",
        page:       35,
    },
    Term {
        id:         "round-to-nearest",
        name:       "Rounding to nearest",
        phrases:    &[
            "rounds to the nearest",
            "rounded to the nearest",
            "rounding",
        ],
        definition: "What a floating operation does with an exact result its type cannot hold: it \
                     takes the nearest value the type has, and of two equally near the one whose \
                     last bit is 0. Every program starts in this mode.",
        clause:     "F.7.3p1",
        page:       461,
    },
    Term {
        id:         "truncation",
        name:       "Truncation toward zero",
        phrases:    &["truncating toward zero", "truncates toward zero"],
        definition: "What converting a floating value to an integer does: it drops the fraction, \
                     so 2.99 becomes 2 and -2.99 becomes -2. If what is left does not fit in the \
                     integer type, the behavior is undefined.",
        clause:     "6.3.1.4p1",
        page:       55,
    },
    Term {
        id:         "nan",
        name:       "NaN",
        phrases:    &["not a number", "a NaN"],
        definition: "Not a Number: the value a floating operation gives when it has no meaningful \
                     result, such as 0.0 / 0.0. A NaN compares unequal to everything, itself \
                     included, and spreads through arithmetic.",
        clause:     "5.2.4.2.2p3",
        page:       35,
    },
    Term {
        id:         "infinity",
        name:       "Infinity",
        phrases:    &["an infinity", "infinity"],
        definition: "A floating value beyond every finite one, written inf by printf: the result \
                     of dividing a nonzero value by zero, or of a result too large for its type, \
                     under Annex F.",
        clause:     "5.2.4.2.2p3",
        page:       35,
    },
    Term {
        id:         "annex-f",
        name:       "Annex F",
        phrases:    &["Annex F", "IEC 60559", "IEEE 754"],
        definition: "The part of the C standard that binds the floating types and operators to \
                     IEEE 754 (IEC 60559). An implementation that defines __STDC_IEC_559__, as \
                     bcc does, follows it: division by zero gives an infinity or a NaN instead of \
                     being undefined.",
        clause:     "F.1p1",
        page:       456,
    },
    Term {
        id:         "array-decay",
        name:       "Array-to-pointer decay",
        phrases:    &["array-to-pointer decay"],
        definition: "Using an array in an expression gives a pointer to its first element; the \
                     array is not copied. sizeof and & are the exceptions.",
        clause:     "6.3.2.1p3",
        page:       58,
    },
    Term {
        id:         "function-designator",
        name:       "Function designator",
        phrases:    &["function designator"],
        definition: "An expression that names a function, such as its name or *fp. Used as an \
                     operand it becomes a pointer to the function, as an array becomes a pointer \
                     to its first element; sizeof and & are the exceptions.",
        clause:     "6.3.2.1p4",
        page:       58,
    },
    Term {
        id:         "function-pointer",
        name:       "Pointer to a function",
        phrases:    &["pointer to a function", "pointers to functions"],
        definition: "A value that records which function a call runs. A function's name becomes \
                     one, and fp(x) or (*fp)(x) calls the function it points to; its type spells \
                     the parameters and return type, so int (*)(int) points to a function that \
                     takes an int and returns one.",
        clause:     "6.2.5p20",
        page:       47,
    },
    Term {
        id:         "enumeration-constant",
        name:       "Enumeration constant",
        phrases:    &["enumeration constant", "enumeration constants"],
        definition: "A name an enumeration declares for an int value, such as RED in enum color { \
                     RED, GREEN }. It is a constant, not an object: it takes no storage, and the \
                     compiler puts its value wherever the name appears.",
        clause:     "6.4.4.3p2",
        page:       71,
    },
    Term {
        id:         "typedef-name",
        name:       "Typedef name",
        phrases:    &["typedef name", "typedef names"],
        definition: "A second name for a type, declared with typedef, such as celsius in typedef \
                     double celsius;. It makes no new type and no object: a declaration written \
                     with it means exactly what one written with the type would.",
        clause:     "6.7.7p3",
        page:       135,
    },
    Term {
        id:         "case-label",
        name:       "Case label",
        phrases:    &["case label", "case labels"],
        definition: "A place in a switch's body, such as case 3:, that the switch can jump to: it \
                     jumps to the case label whose constant equals its value, or else to the \
                     default: label, which has no constant. Execution then runs on past later \
                     labels until a break.",
        clause:     "6.8.4.2p5",
        page:       146,
    },
    Term {
        id:         "goto",
        name:       "goto",
        phrases:    &["goto"],
        definition: "A statement, goto name;, that jumps to the statement labeled name: anywhere \
                     in the same function, earlier or later. Leaving a block by the jump ends its \
                     objects' lifetimes; jumping into one begins them, but runs no initializer.",
        clause:     "6.8.6.1p2",
        page:       149,
    },
    Term {
        id:         "continue",
        name:       "continue",
        phrases:    &["continue statement", "continue"],
        definition: "A statement, continue;, that ends the current pass through a loop's body: a \
                     while or do statement tests its condition next, and a for statement runs its \
                     third clause first. Unlike break, it does not leave the loop.",
        clause:     "6.8.6.2p2",
        page:       150,
    },
    Term {
        id:         "pointer-arithmetic",
        name:       "Pointer arithmetic",
        phrases:    &["pointer arithmetic"],
        definition: "Adding an integer n to a pointer moves it n elements, not n bytes, within \
                     one array.",
        clause:     "6.5.6p8",
        page:       95,
    },
    Term {
        id:         "one-past-the-end",
        name:       "One past the end",
        phrases:    &["one-past-the-end", "one past the end"],
        definition: "The position just after an array's last element. A pointer may point there \
                     and be compared, but must not be dereferenced.",
        clause:     "6.5.6p8",
        page:       95,
    },
    Term {
        id:         "indirection",
        name:       "Indirection",
        phrases:    &["indirection"],
        definition: "The unary * operator: *p is the object p points to, not a copy of it, so \
                     reading *p reads that object and storing into *p changes it.",
        clause:     "6.5.3.2p4",
        page:       91,
    },
    Term {
        id:         "null-pointer",
        name:       "Null pointer",
        phrases:    &["null pointer"],
        definition: "A pointer that points to no object. Converting the integer constant 0 to a \
                     pointer type gives one.",
        clause:     "6.3.2.3p3",
        page:       59,
    },
    Term {
        id:         "dangling-pointer",
        name:       "Dangling pointer",
        phrases:    &["dangling pointer", "dangling"],
        definition: "A pointer to an object whose lifetime has ended, such as an automatic \
                     (non-static) local of a function that has returned. Its value is \
                     indeterminate, and using it is undefined.",
        clause:     "6.2.4p2",
        page:       44,
    },
    Term {
        id:         "cast",
        name:       "Cast",
        phrases:    &["casts", "cast"],
        definition: "A parenthesized type name before an expression, such as (char)300, which \
                     converts the expression's value to that type.",
        clause:     "6.5.4p4",
        page:       93,
    },
    Term {
        id:         "constraint",
        name:       "Constraint",
        phrases:    &["constraints", "constraint"],
        definition: "A rule the compiler must check: a program that breaks one, such as assigning \
                     to a const object, must get a diagnostic, an error or a warning. C does not \
                     require the compiler to reject it. GCC 14, Clang and bcc refuse to compile \
                     many, such as int *p = 1, but only warn about others, such as int a[1] = {1, \
                     2}.",
        clause:     "3.8",
        page:       17,
    },
    Term {
        id:         "undefined-behavior",
        name:       "Undefined behavior",
        phrases:    &["undefined behavior", "undefined"],
        definition: "Behavior the standard places no requirements on, such as signed overflow. \
                     Compilers assume it never happens, so a program that does it can do anything.",
        clause:     "3.4.3",
        page:       16,
    },
    Term {
        id:         "implementation-defined",
        name:       "Implementation-defined",
        phrases:    &["implementation-defined"],
        definition: "Behavior each compiler must choose and document, such as the result of \
                     storing 200 in a char.",
        clause:     "3.4.1",
        page:       15,
    },
];

/// A word the compiler pipeline uses, which the narration never marks. The
/// C standard defines a few of them; the rest are compilers' own words.
#[derive(Debug, PartialEq, Eq)]
pub struct CompilerTerm {
    /// The anchor: the glossary entry is `#term-{id}`.
    pub id:         &'static str,
    pub name:       &'static str,
    pub definition: &'static str,
    /// The defining paragraph of C99 and its page in N1256, for a word the
    /// standard defines.
    pub clause:     Option<(&'static str, u16)>,
    /// The stage that shows it.
    pub stage:      Stage,
}

/// The compiler pipeline's words, in the order of its stages.
pub static COMPILER_TERMS: [CompilerTerm; 15] = [
    CompilerTerm {
        id:         "logical-line",
        name:       "Logical source line",
        definition: "A line of source as translation phase 2 leaves it: each backslash that ends \
                     a physical line is deleted with its new-line, so the next physical line \
                     continues this one.",
        clause:     Some(("5.1.1.2p1", 22)),
        stage:      Stage::Lines,
    },
    CompilerTerm {
        id:         "preprocessing-token",
        name:       "Preprocessing token",
        definition: "The unit translation phases 3 and 4 work in: a header name, an identifier, a \
                     pp-number, a character constant, a string literal, a punctuator, or any \
                     other single character. There are no keywords yet: int is still an \
                     identifier.",
        clause:     Some(("6.4p3", 61)),
        stage:      Stage::PpTokens,
    },
    CompilerTerm {
        id:         "translation-unit",
        name:       "Translation unit",
        definition: "A source file together with every header and file it includes, after \
                     preprocessing: what one run of the compiler translates. Separately \
                     translated units are linked into one program.",
        clause:     Some(("5.1.1.1p1", 21)),
        stage:      Stage::Preprocessor,
    },
    CompilerTerm {
        id:         "token",
        name:       "Token",
        definition: "A keyword, an identifier, a constant, a string literal or a punctuator: the \
                     units the parser reads. Translation phase 7 converts each preprocessing \
                     token into one.",
        clause:     Some(("6.4p1", 61)),
        stage:      Stage::Tokens,
    },
    CompilerTerm {
        id:         "syntax-tree",
        name:       "Syntax tree",
        definition: "The program's structure as a tree: each declaration, statement and \
                     expression is a node whose children are its parts, so x + y * 2 is an \
                     addition whose right operand is a multiplication.",
        clause:     None,
        stage:      Stage::SyntaxTree,
    },
    CompilerTerm {
        id:         "ir",
        name:       "Intermediate representation (IR)",
        definition: "The form a compiler translates a program into on the way from source to \
                     machine code: a few simple operations that depend neither on C's syntax nor \
                     on one processor. The optimizer works on it. bcc has its own IR; LLVM has \
                     LLVM IR.",
        clause:     None,
        stage:      Stage::Ir,
    },
    CompilerTerm {
        id:         "target-triple",
        name:       "Target triple",
        definition: "The name of the machine a compiler makes code for, in parts joined by \
                     dashes: the processor, the vendor, the operating system and, often, the \
                     environment. x86_64-unknown-linux-gnu is the x86-64 processor, no vendor in \
                     particular, Linux and the GNU environment. bcc IR and LLVM IR name it on \
                     their first lines.",
        clause:     None,
        stage:      Stage::Ir,
    },
    CompilerTerm {
        id:         "data-layout",
        name:       "Data layout",
        definition: "A line of bcc IR and LLVM IR that records how the target stores data, in \
                     entries joined by dashes: e for little-endian (a value's least significant \
                     byte stored first), i64:64 for 64-bit integers aligned to 64 bits, S128 for \
                     a stack aligned to 128 bits, and the sizes and alignments of other types.",
        clause:     None,
        stage:      Stage::Ir,
    },
    CompilerTerm {
        id:         "ssa",
        name:       "Static single assignment (SSA)",
        definition: "An IR in which each value is assigned exactly once, by the instruction that \
                     defines it. Where control flow joins, a block parameter (in bcc IR) or a phi \
                     instruction (in LLVM IR) chooses among the values that arrive.",
        clause:     None,
        stage:      Stage::Ir,
    },
    CompilerTerm {
        id:         "basic-block",
        name:       "Basic block",
        definition: "A run of instructions entered only at its top and left only at its bottom, \
                     by its last instruction: a jump, a branch or a return. A function's control \
                     flow is the jumps between its blocks.",
        clause:     None,
        stage:      Stage::Ir,
    },
    CompilerTerm {
        id:         "optimizer",
        name:       "Optimizer",
        definition: "The part of a compiler that rewrites the IR into a faster or smaller program \
                     that behaves the same as far as the C standard can tell: it computes \
                     constants ahead of time, deletes work whose result is unused, and copies \
                     small functions into their callers. bcc runs its optimizer at -O2.",
        clause:     None,
        stage:      Stage::Ir,
    },
    CompilerTerm {
        id:         "abi",
        name:       "ABI (application binary interface)",
        definition: "The rules a platform fixes so that separately compiled code fits together: \
                     where each argument and result is passed, in a register or on the stack, \
                     which registers a call may change, and each type's size and alignment. \
                     x86-64 Linux follows the System V ABI.",
        clause:     None,
        stage:      Stage::IrAbi,
    },
    CompilerTerm {
        id:         "register",
        name:       "Register",
        definition: "A storage place inside the processor, much faster than memory but few: \
                     x86-64 has 16 general-purpose registers, %rax to %r15, and the %xmm \
                     registers for floating-point values.",
        clause:     None,
        stage:      Stage::Asm,
    },
    CompilerTerm {
        id:         "assembly",
        name:       "Assembly language",
        definition: "A processor's instructions written as text, one per line, such as addl %eax, \
                     %ebx in the AT&T syntax GCC and bcc write. An assembler encodes each \
                     instruction as machine code.",
        clause:     None,
        stage:      Stage::Asm,
    },
    CompilerTerm {
        id:         "machine-code",
        name:       "Machine code",
        definition: "The bytes a processor decodes and runs. An x86-64 instruction takes from 1 \
                     to 15 bytes.",
        clause:     None,
        stage:      Stage::MachineCode,
    },
];

/// A piece of narration text: plain, or a defined term as written.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Term(&'a str, &'static Term),
}

/// `text` split at the first use of each term not in `seen`, which gains
/// the terms found, so a term is marked once per step. A phrase matches
/// whole words only, ignoring case.
pub fn pieces<'a>(text: &'a str, seen: &mut Vec<&'static str>) -> Vec<Piece<'a>> {
    let lower = text.to_ascii_lowercase();
    let mut found: Vec<(usize, usize, &'static Term)> = Vec::new();
    for term in &TERMS {
        if seen.contains(&term.id) {
            continue;
        }
        let first = term.phrases.iter().find_map(|phrase| {
            lower
                .match_indices(phrase)
                .map(|(start, _)| start)
                .find(|&start| {
                    let end = start + phrase.len();
                    let before = lower[..start].chars().next_back();
                    let after = lower[end..].chars().next();
                    let boundary = |c: Option<char>| {
                        !c.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
                    };
                    boundary(before)
                        && boundary(after)
                        && !found.iter().any(|&(s, e, _)| start < e && s < end)
                })
                .map(|start| (start, start + phrase.len()))
        });
        if let Some((start, end)) = first {
            found.push((start, end, term));
            seen.push(term.id);
        }
    }
    found.sort_by_key(|&(start, ..)| start);
    let mut pieces = Vec::new();
    let mut at = 0;
    for (start, end, term) in found {
        if start > at {
            pieces.push(Piece::Text(&text[at..start]));
        }
        pieces.push(Piece::Term(&text[start..end], term));
        at = end;
    }
    if at < text.len() {
        pieces.push(Piece::Text(&text[at..]));
    }
    pieces
}

/// The glossary section: every term with its definition and the paragraph
/// of C99 that defines it.
#[component]
pub fn Glossary(
    /// Open the compiler pipeline at a stage, from a compiler term's entry.
    on_stage: Callback<Stage>,
) -> impl IntoView {
    view! {
        <section class="glossary" id="glossary" aria-labelledby="glossary-title">
            <div class="section-heading">
                <h2 id="glossary-title"><span class="section-number">"Table 2"</span>"Glossary"</h2>
                <p>"The words of the C standard the explanations use. Each links to the paragraph that defines it."</p>
            </div>
            <dl class="glossary-list">
                {TERMS.iter().map(|term| view! {
                    <div class="glossary-entry" id=format!("term-{}", term.id)>
                        <dt>{term.name}</dt>
                        <dd>
                            {term.definition}" "
                            <a class="glossary-clause" href=term.link() target="_blank" rel="noopener">
                                {format!("C99 §{}", term.clause)}
                                <span class="sr-only">" (opens the standard in a new tab)"</span>
                            </a>
                        </dd>
                    </div>
                }).collect_view()}
            </dl>
            <h3 class="glossary-part" id="compiler-terms-title">"Compiler terms"</h3>
            <p class="glossary-part-note">
                "The words the compiler pipeline uses for what a compiler makes. A few are the \
                 standard's own and link to it; each names the stage that shows it."
            </p>
            <dl class="glossary-list" aria-labelledby="compiler-terms-title">
                {COMPILER_TERMS.iter().map(|term| {
                    let stage = term.stage;
                    let number = Stage::ALL.iter().position(|&each| each == stage).unwrap_or_default() + 1;
                    view! {
                        <div class="glossary-entry" id=format!("term-{}", term.id)>
                            <dt>{term.name}</dt>
                            <dd>
                                {term.definition}" "
                                {term.clause.map(|(clause, page)| view! {
                                    <a class="glossary-clause" href=format!("{N1256_URL}#page={page}") target="_blank" rel="noopener">
                                        {format!("C99 §{clause}")}
                                        <span class="sr-only">" (opens the standard in a new tab)"</span>
                                    </a>
                                    " "
                                })}
                                <a class="glossary-stage" href="#workbench" on:click=move |_| on_stage.run(stage)>
                                    {format!("Stage {number}: {}", stage.label())}
                                </a>
                            </dd>
                        </div>
                    }
                }).collect_view()}
            </dl>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Piece,
        TERMS,
        pieces,
    };

    fn marked(text: &str) -> Vec<(&str, &str)> {
        pieces(text, &mut Vec::new())
            .into_iter()
            .filter_map(|piece| match piece {
                | Piece::Term(written, term) => Some((written, term.id)),
                | Piece::Text(_) => None,
            })
            .collect()
    }

    #[test]
    fn marks_the_first_use_of_each_term_on_word_boundaries() {
        assert_eq!(
            marked("Lvalue conversion: an lvalue is read. The lvalue stays."),
            [
                ("Lvalue conversion", "lvalue-conversion"),
                ("lvalue", "lvalue")
            ]
        );
        assert_eq!(
            marked("Its objects end; the object ends."),
            [("objects", "object")]
        );
        // Not inside other words.
        assert_eq!(
            marked("subobjects and scoped and undefinedness"),
            [] as [(&str, &str); 0]
        );
        assert_eq!(
            marked("a pointer one past the end of `a`"),
            [("one past the end", "one-past-the-end")]
        );
    }

    #[test]
    fn keeps_the_text_whole() {
        let text = "The full expression ends here, which is a sequence point.";
        let joined: String = pieces(text, &mut Vec::new())
            .into_iter()
            .map(|piece| match piece {
                | Piece::Text(text) | Piece::Term(text, _) => text,
            })
            .collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn compiler_terms_share_no_anchor_and_cover_every_brief_link() {
        use super::COMPILER_TERMS;
        use crate::pipeline::{
            Stage,
            linked_terms,
        };
        for (index, term) in COMPILER_TERMS.iter().enumerate() {
            assert!(
                TERMS.iter().all(|other| other.id != term.id)
                    && COMPILER_TERMS[..index]
                        .iter()
                        .all(|other| other.id != term.id),
                "{}",
                term.id
            );
            assert!(
                term.clause
                    .is_none_or(|(_, page)| (15..=560).contains(&page)),
                "{}",
                term.id
            );
        }
        for stage in Stage::ALL {
            let brief = stage.brief();
            for text in [brief.input, brief.output, brief.why] {
                for (piece, id) in linked_terms(text) {
                    assert!(
                        !piece.contains(['[', ']', '(']),
                        "{stage:?}: a malformed link in {text}"
                    );
                    if let Some(id) = id {
                        assert!(
                            COMPILER_TERMS.iter().any(|term| term.id == id)
                                || TERMS.iter().any(|term| term.id == id),
                            "{stage:?} links the unknown term {id}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn terms_have_unique_anchors_and_pages() {
        for (index, term) in TERMS.iter().enumerate() {
            assert!(
                TERMS[..index].iter().all(|other| other.id != term.id),
                "{}",
                term.id
            );
            assert!((15..=560).contains(&term.page), "{}", term.id);
        }
    }
}
