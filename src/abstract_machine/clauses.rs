//! The C99 rules the engine cites, each with its heading and a one-sentence
//! plain-language gloss for readers new to the standard.
//!
//! Every entry was checked against N1256 (`standards/c99-n1256.pdf`); the
//! `citation` field carries the printed and PDF page of the paragraph.

/// One cited paragraph of C99.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clause {
    /// Clause and paragraph, such as `6.5.16.1p2`.
    pub id:       &'static str,
    /// The clause heading in the standard.
    pub title:    &'static str,
    /// What the paragraph means, in plain language.
    pub gloss:    &'static str,
    /// The citation with pages, in the repository's compact form.
    pub citation: &'static str,
}

macro_rules! clauses {
    ($($(#[$attribute:meta])* $name:ident = $id:literal, $pages:literal, $title:literal, $gloss:literal;)*) => {
        $(
            $(#[$attribute])*
            pub const $name: Clause = Clause {
                id:       $id,
                title:    $title,
                gloss:    $gloss,
                citation: concat!("C99 §", $id, ", ", $pages),
            };
        )*
    };
}

clauses! {
    /// C99: §5.2.4.2.1 paragraph 1, pp. 21-23; PDF pp. 33-35.
    INTEGER_LIMITS = "5.2.4.2.1p1", "pp. 21-23; PDF pp. 33-35", "Sizes of integer types",
        "C sets only minimum ranges: an int must hold at least -32767 to 32767 and a long at \
         least -2147483647 to 2147483647; each implementation may give more.";
    /// C99: §5.1.2.3 paragraph 1, p. 13; PDF p. 25.
    ABSTRACT_MACHINE = "5.1.2.3p1", "p. 13; PDF p. 25", "Program execution",
        "The standard describes what an idealized abstract machine does; a real compiler may \
         produce any code that behaves the same way.";
    /// C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25.
    MAIN_RETURN = "5.1.2.2.3p1", "p. 13; PDF p. 25", "Program termination",
        "Returning from main ends the program with that status; reaching main's closing brace \
         returns 0.";
    /// C99: §5.1.2 paragraph 1, p. 11; PDF p. 23.
    STARTUP = "5.1.2p1", "p. 11; PDF p. 23", "Execution environments",
        "Every object with static storage duration gets its initial value before the program \
         starts, that is, before main is called.";
    /// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
    BLOCK_SCOPE = "6.2.1p4", "pp. 29-30; PDF pp. 41-42", "Scopes of identifiers",
        "A name declared inside a block can be used only from its declaration to the end of that \
         block.";
    /// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
    FILE_SCOPE = "6.2.1p4", "pp. 29-30; PDF pp. 41-42", "Scopes of identifiers",
        "A name declared outside every function has file scope: it can be used from its \
         declaration to the end of the file, in every function after it.";
    /// C99: §6.2.1 paragraph 7, p. 30; PDF p. 42.
    SCOPE_START = "6.2.1p7", "p. 30; PDF p. 42", "Scopes of identifiers",
        "A name's scope begins just after its declarator: before its declaration, the name \
         cannot be used, even where its object already exists.";
    /// C99: §6.2.4 paragraph 2, p. 32; PDF p. 44.
    LIFETIME = "6.2.4p2", "p. 32; PDF p. 44", "Storage durations of objects",
        "An object exists only during its lifetime: referring to it after the lifetime ends is \
         undefined, and every pointer to it becomes indeterminate.";
    /// C99: §J.2 paragraph 1, p. 492; PDF p. 504.
    DANGLING_USE = "J.2p1", "p. 492; PDF p. 504", "Undefined behavior",
        "Using the value of a pointer to an object whose lifetime has ended is undefined: even \
         storing, comparing or passing it, not only following it.";
    /// C99: §6.2.4 paragraph 3, p. 32; PDF p. 44.
    STATIC_STORAGE = "6.2.4p3", "p. 32; PDF p. 44", "Storage durations of objects",
        "An object declared outside every function, or with static, lives for the whole run of \
         the program and is initialized only once, before the program starts.";
    /// C99: §6.2.4 paragraph 5, p. 32; PDF p. 44.
    AUTOMATIC_LIFETIME = "6.2.4p5", "p. 32; PDF p. 44", "Storage durations of objects",
        "A local variable lives from entry into its block until the block finishes; each new \
         entry, including a recursive call, creates a fresh object.";
    /// C99: §6.2.5 paragraph 10, p. 34; PDF p. 46.
    FLOATING_TYPES = "6.2.5p10", "p. 34; PDF p. 46", "Types",
        "C has three real floating types, float, double and long double, each holding every \
         value of the one before it.";
    /// C99: §5.2.4.2.2 paragraph 8, pp. 24-25; PDF pp. 36-37.
    EVALUATION_FORMAT = "5.2.4.2.2p8", "pp. 24-25; PDF pp. 36-37",
        "Characteristics of floating types <float.h>",
        "FLT_EVAL_METHOD says in which format floating operations are computed; on x86-64 it is \
         0: each in its own type, so float arithmetic rounds to float.";
    /// C99: §F.1 paragraph 1, p. 444; PDF p. 456.
    ANNEX_F = "F.1p1", "p. 444; PDF p. 456", "Introduction (Annex F)",
        "An implementation that defines __STDC_IEC_559__, as bcc does, follows Annex F: its \
         floating types and operations are those of IEC 60559, better known as IEEE 754.";
    /// C99: §F.2 paragraph 1, p. 444; PDF p. 456.
    FLOATING_FORMATS = "F.2p1", "p. 444; PDF p. 456", "Types (Annex F)",
        "Under Annex F, float is IEEE 754 single precision and double is double precision; long \
         double is an extended format, on x86-64 the x87 80-bit one.";
    /// C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459.
    FLOATING_OPERATIONS = "F.3p1", "pp. 445-447; PDF pp. 457-459", "Operators and functions",
        "Under Annex F, +, -, * and / are IEEE 754's operations: each gives its exact result \
         rounded to the type, x / 0 is an infinity, 0 / 0 a NaN, and a NaN is unordered: <, <=, \
         >, >= and == with a NaN are false, and != is true.";
    /// C99: §F.7.3 paragraph 1, p. 449; PDF p. 461.
    ROUND_TO_NEAREST = "F.7.3p1", "p. 449; PDF p. 461", "Execution (Annex F)",
        "A program starts in IEEE 754's round-to-nearest mode: a result between two values of \
         its type becomes the nearer one, and an exact tie the one whose last bit is 0.";
    /// C99: §6.10.8 paragraph 2, p. 161; PDF p. 173.
    IEC_559_MACRO = "6.10.8p2", "p. 161; PDF p. 173", "Predefined macro names",
        "An implementation defines __STDC_IEC_559__ as 1 when it follows Annex F, IEEE 754 \
         floating-point arithmetic.";
    /// C99: §6.2.5 paragraph 15, p. 35; PDF p. 47.
    CHAR_SIGNEDNESS = "6.2.5p15", "p. 35; PDF p. 47", "Types",
        "Whether plain char is signed or unsigned is up to the implementation; on x86-64 Linux \
         it is signed, holding -128 to 127.";
    /// C99: §6.2.5 paragraph 6, p. 33; PDF p. 45.
    UNSIGNED_TYPES = "6.2.5p6", "p. 33; PDF p. 45", "Types",
        "Each signed integer type has an unsigned version of the same size, written with \
         unsigned, which holds only nonnegative values.";
    /// C99: §6.2.5 paragraph 9, p. 34; PDF p. 46.
    UNSIGNED_WRAP = "6.2.5p9", "p. 34; PDF p. 46", "Types",
        "Unsigned arithmetic never overflows: a result that does not fit is reduced modulo one \
         more than the type's largest value, 2^N.";
    /// C99: §6.2.5 paragraph 20, pp. 35-36; PDF pp. 47-48.
    STRUCTURE_TYPE = "6.2.5p20", "pp. 35-36; PDF pp. 47-48", "Types",
        "A structure is a sequence of member objects, each with its own name and type, allocated \
         one after another.";
    /// C99: §6.2.6.1 paragraph 5, pp. 37-38; PDF pp. 49-50.
    TRAP_REPRESENTATION = "6.2.6.1p5", "pp. 37-38; PDF pp. 49-50", "Representations of types",
        "Some bit patterns are not values of their type (trap representations); reading one as \
         that type is undefined, though a character type such as unsigned char may read any \
         byte.";
    /// C99: §6.2.6.1 paragraph 6, p. 38; PDF p. 50.
    STRUCTURE_VALUE = "6.2.6.1p6", "p. 38; PDF p. 50", "Representations of types",
        "A structure's value is never a trap representation, even if a member's is; its padding \
         bytes hold unspecified values.";
    /// C99: §6.3.1.1 paragraph 1, p. 42; PDF p. 54.
    INTEGER_RANK = "6.3.1.1p1", "p. 42; PDF p. 54", "Boolean, characters, and integers",
        "Integer conversion rank orders the integer types: char, short, int, long, long long, \
         each unsigned type ranking with its signed one.";
    /// C99: §6.3.1.1 paragraph 2, pp. 42-43; PDF pp. 54-55.
    INTEGER_PROMOTIONS = "6.3.1.1p2", "pp. 42-43; PDF pp. 54-55", "Boolean, characters, and integers",
        "Before arithmetic, values of types narrower than int (like char) are converted to int.";
    /// C99: §6.3.1.3 paragraph 1, p. 43; PDF p. 55.
    INTEGER_UNCHANGED = "6.3.1.3p1", "p. 43; PDF p. 55", "Signed and unsigned integers",
        "Converting an integer to another integer type keeps its value whenever the new type can \
         represent it.";
    /// C99: §6.3.1.3 paragraph 2, p. 43; PDF p. 55.
    UNSIGNED_CONVERSION = "6.3.1.3p2", "p. 43; PDF p. 55", "Signed and unsigned integers",
        "Converting a value that does not fit into an unsigned type adds or subtracts 2^N until \
         it fits: -1 becomes the type's largest value. This is always defined.";
    /// C99: §6.3.1.3 paragraph 3, p. 43; PDF p. 55.
    SIGNED_CONVERSION = "6.3.1.3p3", "p. 43; PDF p. 55", "Signed and unsigned integers",
        "Converting a value that does not fit into a signed type gives an implementation-defined \
         result; GCC documents that it keeps the low bits (wraps around), and Clang and bcc do the \
         same.";
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    FLOATING_TO_INTEGER = "6.3.1.4p1", "p. 43; PDF p. 55", "Real floating and integer",
        "Converting a floating value to an integer type discards its fraction, truncating toward \
         zero; if the integral part does not fit in the type, the behavior is undefined.";
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
    INTEGER_TO_FLOATING = "6.3.1.4p2", "pp. 43-44; PDF pp. 55-56", "Real floating and integer",
        "An integer converted to a floating type keeps its value if the type holds it exactly, \
         and otherwise becomes one of the two nearest values the type has.";
    /// C99: §F.4 paragraph 1, p. 447; PDF p. 459.
    FLOATING_TO_INTEGER_ANNEX = "F.4p1", "p. 447; PDF p. 459", "Floating to integer conversion",
        "Annex F says only that an out-of-range conversion to an integer raises the invalid \
         exception and gives an unspecified value; C's own rule, 6.3.1.4, leaves it undefined.";
    /// C99: §6.3.1.5 paragraph 1, p. 44; PDF p. 56.
    FLOATING_PROMOTION = "6.3.1.5p1", "p. 44; PDF p. 56", "Real floating types",
        "A float converted to double or long double, or a double to long double, keeps its value \
         exactly.";
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    FLOATING_DEMOTION = "6.3.1.5p2", "p. 44; PDF p. 56", "Real floating types",
        "A value converted to a narrower floating type keeps it when it fits exactly and \
         otherwise rounds to a neighbour; a value beyond the type's range is undefined, which \
         Annex F defines as an infinity.";
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    USUAL_ARITHMETIC = "6.3.1.8p1", "pp. 44-45; PDF pp. 56-57", "Usual arithmetic conversions",
        "Binary arithmetic first converts both operands to a common type. With the same \
         signedness it is the higher-ranked one. Otherwise it is the unsigned one when its rank \
         is at least as high; the signed one when it can represent every value of the other; \
         and else the unsigned type of the signed one's rank (long long and unsigned long give \
         unsigned long long). Two ints need no conversion.";
    /// C99: §6.3.2.1 paragraph 1, p. 46; PDF p. 58.
    LVALUE = "6.3.2.1p1", "p. 46; PDF p. 58", "Lvalues, arrays, and function designators",
        "An lvalue is an expression that designates an object: a location, not yet a value.";
    /// C99: §6.3.2.1 paragraph 2, p. 46; PDF p. 58.
    LVALUE_CONVERSION = "6.3.2.1p2", "p. 46; PDF p. 58", "Lvalues, arrays, and function designators",
        "Using an lvalue where a value is needed reads the value stored in the object; this read \
         is called lvalue conversion.";
    /// C99: §6.3.2.1 paragraph 3, p. 46; PDF p. 58.
    ARRAY_DECAY = "6.3.2.1p3", "p. 46; PDF p. 58", "Lvalues, arrays, and function designators",
        "An array used in an expression (other than with & or sizeof) becomes a pointer to its \
         first element.";
    /// C99: §6.3.2.3 paragraph 3, p. 47; PDF p. 59.
    NULL_POINTER = "6.3.2.3p3", "p. 47; PDF p. 59", "Pointers",
        "The integer constant 0 converted to a pointer type becomes a null pointer, which points \
         to no object at all.";
    /// C99: §6.4.4 paragraph 2, p. 54; PDF p. 66.
    CONSTANT_RANGE = "6.4.4p2", "p. 54; PDF p. 66", "Constants",
        "Every constant has a type, and its value must be in the range that type can represent.";
    /// C99: §6.4.4.2 paragraph 3, p. 58; PDF p. 70.
    FLOATING_CONSTANT = "6.4.4.2p3", "p. 58; PDF p. 70", "Floating constants",
        "A decimal floating constant becomes the nearest value its type can represent (or a \
         neighbour of it); a hexadecimal one is rounded correctly.";
    /// C99: §6.4.4.2 paragraph 4, p. 58; PDF p. 70.
    FLOATING_SUFFIX = "6.4.4.2p4", "p. 58; PDF p. 70", "Floating constants",
        "A floating constant without a suffix is a double; with f or F it is a float, and with l \
         or L a long double.";
    /// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
    INTEGER_CONSTANT = "6.4.4.1p5", "pp. 55-56; PDF pp. 67-68", "Integer constants",
        "A decimal constant's type is the first of int, long and long long that holds its value; \
         a suffix such as u or l starts the list elsewhere.";
    /// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
    PREFIXED_CONSTANT = "6.4.4.1p5", "pp. 55-56; PDF pp. 67-68", "Integer constants",
        "An octal or hexadecimal constant's type is the first of int, unsigned int, long, \
         unsigned long, long long and unsigned long long that holds its value.";
    /// C99: §6.4.4.1 paragraph 4, p. 55; PDF p. 67.
    INTEGER_BASE = "6.4.4.1p4", "p. 55; PDF p. 67", "Integer constants",
        "A constant is computed in base 10, or in base 8 when it starts with 0, or in base 16 \
         when it starts with 0x, whose digits a to f stand for 10 to 15.";
    /// C99: §6.5.3.3 paragraph 4, p. 79; PDF p. 91.
    COMPLEMENT = "6.5.3.3p4", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "~ flips every bit of its promoted operand; for an unsigned type, ~E is the type's largest \
         value minus E.";
    /// C99: §6.5.3.3 paragraph 1, p. 79; PDF p. 91.
    COMPLEMENT_CONSTRAINT = "6.5.3.3p1", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "The operand of ~ must have an integer type; that of + and - an arithmetic type, and that \
         of ! a scalar type.";
    /// C99: §6.5.10 paragraph 4, p. 87; PDF p. 99.
    BITWISE_AND = "6.5.10p4", "p. 87; PDF p. 99", "Bitwise AND operator",
        "After the usual arithmetic conversions, a bit of a & b is 1 only where the bits of both \
         operands are 1.";
    /// C99: §6.5.11 paragraph 4, p. 88; PDF p. 100.
    BITWISE_XOR = "6.5.11p4", "p. 88; PDF p. 100", "Bitwise exclusive OR operator",
        "After the usual arithmetic conversions, a bit of a ^ b is 1 where exactly one of the \
         operands' bits is 1.";
    /// C99: §6.5.12 paragraph 4, p. 88; PDF p. 100.
    BITWISE_OR = "6.5.12p4", "p. 88; PDF p. 100", "Bitwise inclusive OR operator",
        "After the usual arithmetic conversions, a bit of a | b is 1 where at least one of the \
         operands' bits is 1.";
    /// C99: §6.5.10 paragraph 2, p. 87; PDF p. 99.
    BITWISE_CONSTRAINT = "6.5.10p2", "p. 87; PDF p. 99", "Bitwise AND operator",
        "Each operand of &, and likewise of ^ and |, must have an integer type.";
    /// C99: §6.5.13 paragraph 4, p. 89; PDF p. 101.
    LOGICAL_AND = "6.5.13p4", "p. 89; PDF p. 101", "Logical AND operator",
        "&& evaluates its left operand first, and a sequence point follows; if that operand is 0, \
         the right one is never evaluated.";
    /// C99: §6.5.13 paragraph 3, p. 89; PDF p. 101.
    LOGICAL_AND_VALUE = "6.5.13p3", "p. 89; PDF p. 101", "Logical AND operator",
        "&& yields the int 1 when both operands compare unequal to 0, and 0 otherwise.";
    /// C99: §6.5.14 paragraph 4, p. 89; PDF p. 101.
    LOGICAL_OR = "6.5.14p4", "p. 89; PDF p. 101", "Logical OR operator",
        "|| evaluates its left operand first, and a sequence point follows; if that operand is \
         nonzero, the right one is never evaluated.";
    /// C99: §6.5.14 paragraph 3, p. 89; PDF p. 101.
    LOGICAL_OR_VALUE = "6.5.14p3", "p. 89; PDF p. 101", "Logical OR operator",
        "|| yields the int 1 when either operand compares unequal to 0, and 0 otherwise.";
    /// C99: §6.5.13 paragraph 2, p. 89; PDF p. 101.
    LOGICAL_CONSTRAINT = "6.5.13p2", "p. 89; PDF p. 101", "Logical AND operator",
        "Each operand of &&, and likewise of ||, must be a scalar: a number or a pointer.";
    /// C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
    CONDITIONAL = "6.5.15p4", "p. 90; PDF p. 102", "Conditional operator",
        "?: evaluates its first operand, then a sequence point follows, then only one of the \
         others: the second when the first is nonzero, the third when it is 0.";
    /// C99: §6.5.15 paragraph 2, p. 90; PDF p. 102.
    CONDITIONAL_FIRST = "6.5.15p2", "p. 90; PDF p. 102", "Conditional operator",
        "The first operand of ?: must be a scalar, a number or a pointer.";
    /// C99: §6.5.15 paragraph 5, p. 90; PDF p. 102.
    CONDITIONAL_TYPE = "6.5.15p5", "p. 90; PDF p. 102", "Conditional operator",
        "When the second and third operands are arithmetic, the result has the type the usual \
         arithmetic conversions would give the two.";
    /// C99: §6.5.15 paragraph 3, p. 90; PDF p. 102.
    CONDITIONAL_CONSTRAINT = "6.5.15p3", "p. 90; PDF p. 102", "Conditional operator",
        "The second and third operands must both be arithmetic, both void, the same structure or \
         union type, or pointers to compatible types; or one is a pointer and the other a null \
         pointer constant; or one is a pointer to an object or incomplete type and the other a \
         pointer to void.";
    /// C99: §6.5.17 paragraph 2, p. 94; PDF p. 106.
    COMMA = "6.5.17p2", "p. 94; PDF p. 106", "Comma operator",
        "The comma operator evaluates its left operand and discards the value; a sequence point \
         follows, and then the right operand gives the result.";
    /// C99: §C paragraph 1, p. 439; PDF p. 451.
    SEQUENCE_POINTS = "Cp1", "p. 439; PDF p. 451", "Sequence points (Annex C)",
        "The sequence points: each call, after its arguments; the end of the first operand of &&, \
         ||, ?: and the comma operator; the end of each full declarator and of each full \
         expression; and in the library, just before a function returns; after the actions of \
         each conversion specifier of a formatted input/output function, which are fprintf, \
         fscanf, printf, scanf, snprintf, sprintf, sscanf, vfprintf, vfscanf, vprintf, vscanf, \
         vsnprintf, vsprintf and vsscanf, and their wide versions fwprintf, fwscanf, swprintf, \
         swscanf, vfwprintf, vfwscanf, vswprintf, vswscanf, vwprintf, vwscanf, wprintf and \
         wscanf; and just before and just after each call qsort or bsearch makes to its \
         comparison function, and between that call and any movement of the objects passed to \
         it.";
    /// C99: §6.5.2.1 paragraph 1, p. 70; PDF p. 82.
    SUBSCRIPT_OPERANDS = "6.5.2.1p1", "p. 70; PDF p. 82", "Array subscripting",
        "One operand of [] must be a pointer to an object and the other an integer, in either \
         order.";
    /// C99: §6.5 paragraph 2, p. 67; PDF p. 79.
    UNSEQUENCED = "6.5p2", "p. 67; PDF p. 79", "Expressions",
        "Between two sequence points an object may be modified at most once, and then read only \
         to compute its new value.";
    /// C99: §6.5 paragraph 3, p. 67; PDF p. 79.
    EVALUATION_ORDER = "6.5p3", "p. 67; PDF p. 79", "Expressions",
        "Except for the function-call (), &&, ||, ?: and comma operators, C leaves the order in \
         which subexpressions are evaluated, and their side effects happen, unspecified.";
    /// C99: §6.5 paragraph 5, p. 67; PDF p. 79.
    EXCEPTIONAL_CONDITION = "6.5p5", "p. 67; PDF p. 79", "Expressions",
        "If a result is not mathematically defined or does not fit its type (such as signed int \
         overflow), the behavior is undefined.";
    /// C99: §6.5.1 paragraph 2, p. 69; PDF p. 81.
    IDENTIFIER = "6.5.1p2", "p. 69; PDF p. 81", "Primary expressions",
        "A variable's name is an lvalue designating that variable's object.";
    /// C99: §6.5.2.1 paragraph 2, p. 70; PDF p. 82.
    SUBSCRIPT = "6.5.2.1p2", "p. 70; PDF p. 82", "Array subscripting",
        "E1[E2] means *((E1)+(E2)): move E2 elements from the start, then designate that element.";
    /// C99: §6.5.2.3 paragraph 3, p. 73; PDF p. 85.
    MEMBER = "6.5.2.3p3", "p. 73; PDF p. 85", "Structure and union members",
        "s.m designates member m of the structure s; it is an lvalue when s is one.";
    /// C99: §6.5.2.3 paragraph 4, p. 73; PDF p. 85.
    MEMBER_POINTER = "6.5.2.3p4", "p. 73; PDF p. 85", "Structure and union members",
        "p->m designates member m of the structure p points to; it is always an lvalue.";
    /// C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83.
    CALL_ARGUMENTS = "6.5.2.2p4", "p. 71; PDF p. 83", "Function calls",
        "Arguments are evaluated and each parameter is assigned a copy of its argument's value: \
         C passes arguments by value.";
    /// C99: §6.5.2.2 paragraph 10, p. 72; PDF p. 84.
    CALL_SEQUENCE_POINT = "6.5.2.2p10", "p. 72; PDF p. 84", "Function calls",
        "The arguments are evaluated in an unspecified order, and there is a sequence point just \
         before the call: every argument is fully evaluated first.";
    /// C99: §6.5.2.4 paragraph 2, p. 75; PDF p. 87.
    POSTFIX_INCREMENT = "6.5.2.4p2", "p. 75; PDF p. 87", "Postfix increment and decrement operators",
        "x++ yields x's old value; the stored value grows by one before the next sequence point.";
    /// C99: §6.5.2.4 paragraph 3, p. 75; PDF p. 87.
    POSTFIX_DECREMENT = "6.5.2.4p3", "p. 75; PDF p. 87", "Postfix increment and decrement operators",
        "x-- yields x's old value, like x++, and the stored value shrinks by one before the next \
         sequence point.";
    /// C99: §6.5.3.1 paragraph 2, p. 78; PDF p. 90.
    PREFIX_INCREMENT = "6.5.3.1p2", "p. 78; PDF p. 90", "Prefix increment and decrement operators",
        "++x adds one to x and yields the new value; it is equivalent to x += 1.";
    /// C99: §6.5.3.1 paragraph 3, p. 78; PDF p. 90.
    PREFIX_DECREMENT = "6.5.3.1p3", "p. 78; PDF p. 90", "Prefix increment and decrement operators",
        "--x subtracts one from x and yields the new value, like ++x.";
    /// C99: §6.5.3.2 paragraph 3, pp. 78-79; PDF pp. 90-91.
    ADDRESS_OPERATOR = "6.5.3.2p3", "pp. 78-79; PDF pp. 90-91", "Address and indirection operators",
        "&x yields a pointer to x; &a[i] and &*p compute the address without reading anything.";
    /// C99: §6.5.3.2 paragraph 4, p. 79; PDF p. 91.
    INDIRECTION = "6.5.3.2p4", "p. 79; PDF p. 91", "Address and indirection operators",
        "*p designates the object p points to; using * on a null or invalid pointer is undefined.";
    /// C99: §6.3.2.1 paragraph 4, p. 46; PDF p. 58.
    FUNCTION_DESIGNATOR = "6.3.2.1p4", "p. 46; PDF p. 58", "Lvalues, arrays, and function designators",
        "A function's name used as an operand becomes a pointer to the function, except as the \
         operand of sizeof or &.";
    /// C99: §6.5.2.2 paragraph 1, p. 71; PDF p. 83.
    CALLED_FUNCTION = "6.5.2.2p1", "p. 71; PDF p. 83", "Function calls",
        "What comes before a call's parentheses must be a pointer to a function: a function's \
         name becomes one, and a pointer variable already is one.";
    /// C99: §6.5.2.2 paragraph 2, p. 71; PDF p. 83.
    CALL_ARGUMENT_COUNT = "6.5.2.2p2", "p. 71; PDF p. 83", "Function calls",
        "Calling through a type with a prototype passes as many arguments as it has parameters, \
         each assignable to its parameter.";
    /// C99: §6.5.2.2 paragraph 3, p. 71; PDF p. 83.
    FUNCTION_CALL = "6.5.2.2p3", "p. 71; PDF p. 83", "Function calls",
        "In a call, the expression before the parentheses denotes the function called, and the \
         expressions inside them are its arguments.";
    /// C99: §6.5.2.2 paragraph 9, p. 72; PDF p. 84.
    CALL_TYPE = "6.5.2.2p9", "p. 72; PDF p. 84", "Function calls",
        "Calling a function through a pointer to a function of an incompatible type is \
         undefined.";
    /// C99: §6.3.2.3 paragraph 8, p. 48; PDF p. 60.
    FUNCTION_POINTER_CONVERSION = "6.3.2.3p8", "p. 48; PDF p. 60", "Pointers",
        "A pointer to a function may be cast to a pointer to another function type and back \
         unchanged; calling through the wrong type is undefined.";
    /// C99: §6.5.9 paragraph 6, p. 87; PDF p. 99.
    POINTER_EQUALITY = "6.5.9p6", "p. 87; PDF p. 99", "Equality operators",
        "Two pointers compare equal exactly when both are null, both point to the same object \
         (or one to an object and the other to a subobject at its start) or the same function, \
         both point one past the last element of the same array, or one points one past the end \
         of an array and the other to the start of a different array that happens to follow it \
         immediately in memory.";
    /// C99: §6.5.9 paragraph 2, p. 86; PDF p. 98.
    EQUALITY_CONSTRAINT = "6.5.9p2", "p. 86; PDF p. 98", "Equality operators",
        "== and != compare two numbers, two pointers to compatible types, a pointer to an object \
         or incomplete type with a pointer to void, or a pointer with a null pointer constant.";
    /// C99: §6.5.6 paragraph 2, p. 82; PDF p. 94.
    FUNCTION_POINTER_ARITHMETIC = "6.5.6p2", "p. 82; PDF p. 94", "Additive operators",
        "Adding an integer to a pointer needs a pointer to an object type, the element of an \
         array; a pointer to a function points to no object.";
    /// C99: §6.5.3.4 paragraph 1, p. 80; PDF p. 92.
    SIZEOF_CONSTRAINT = "6.5.3.4p1", "p. 80; PDF p. 92", "The sizeof operator",
        "sizeof does not apply to a function or to an incomplete type.";
    /// C99: §6.2.5 paragraph 20, pp. 35-36; PDF pp. 47-48.
    FUNCTION_TYPE = "6.2.5p20", "pp. 35-36; PDF pp. 47-48", "Types",
        "A function type is given by its return type and its parameters' number and types; a \
         pointer to a function points to a function of one such type.";
    /// C99: §6.7.7 paragraph 3, p. 123; PDF p. 135.
    TYPEDEF_NAME = "6.7.7p3", "p. 123; PDF p. 135", "Type definitions",
        "A typedef name is a synonym for a type, not a new type: declarations may use it wherever \
         they would spell the type.";
    /// C99: §6.4.4.3 paragraph 2, p. 59; PDF p. 71.
    ENUMERATION_CONSTANT = "6.4.4.3p2", "p. 59; PDF p. 71", "Enumeration constants",
        "A name declared in an enumeration is a constant of type int, not an object: it has no \
         storage, and the compiler puts its value wherever it appears.";
    /// C99: §6.7.2.2 paragraph 2, p. 105; PDF p. 117.
    ENUMERATOR_CONSTRAINT = "6.7.2.2p2", "p. 105; PDF p. 117", "Enumeration specifiers",
        "The value written after an enumeration constant's = must be an integer constant \
         expression whose value an int can hold.";
    /// C99: §6.7.2.2 paragraph 3, p. 105; PDF p. 117.
    ENUMERATOR_VALUES = "6.7.2.2p3", "p. 105; PDF p. 117", "Enumeration specifiers",
        "Each enumeration constant is an int: the value after its =, or else the previous \
         constant's value plus one, starting from 0 for the first.";
    /// C99: §6.7.2.2 paragraph 4, p. 105; PDF p. 117.
    ENUMERATED_TYPE = "6.7.2.2p4", "p. 105; PDF p. 117", "Enumeration specifiers",
        "An enumeration's own type is compatible with an integer type the implementation \
         chooses, one that can hold all its constants' values.";
    /// C99: §6.7.2.3 paragraph 1, p. 106; PDF p. 118.
    TAG_DEFINED_ONCE = "6.7.2.3p1", "p. 106; PDF p. 118", "Tags",
        "A structure, union or enumeration type may have its contents defined only once.";
    /// C99: §6.7.2.3 paragraph 2, p. 106; PDF p. 118.
    TAG_KIND = "6.7.2.3p2", "p. 106; PDF p. 118", "Tags",
        "Declarations that use one tag for one type must all say struct, or all union, or all \
         enum.";
    /// C99: §6.7.2.3 paragraph 3, p. 106; PDF p. 118.
    ENUMERATION_COMPLETE = "6.7.2.3p3", "p. 106; PDF p. 118", "Tags",
        "`enum tag` without a list of constants may appear only once the enumeration has been \
         defined.";
    /// C99: §6.7 paragraph 3, p. 97; PDF p. 109.
    ONE_DECLARATION = "6.7p3", "p. 97; PDF p. 109", "Declarations",
        "A name with no linkage, such as an enumeration constant or a local variable, may be \
         declared only once in one scope.";
    /// C99: §6.2.3 paragraph 1, p. 31; PDF p. 43.
    NAME_SPACES = "6.2.3p1", "p. 31; PDF p. 43", "Name spaces of identifiers",
        "Tags, members and labels each have names of their own; every other name, objects, \
         functions, typedef names and enumeration constants alike, shares one name space.";
    /// C99: §6.7.5.2 paragraph 1, p. 116; PDF p. 128.
    ARRAY_SIZE = "6.7.5.2p1", "p. 116; PDF p. 128", "Array declarators",
        "An array's size has an integer type, and a constant size must be greater than zero.";
    /// C99: §6.8.4.2 paragraph 1, p. 134; PDF p. 146.
    SWITCH_CONSTRAINT = "6.8.4.2p1", "p. 134; PDF p. 146", "The switch statement",
        "The controlling expression of a switch must have an integer type.";
    /// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
    CASE_LABELS = "6.8.4.2p3", "p. 134; PDF p. 146", "The switch statement",
        "Each case label is an integer constant expression, no two of one switch may have the \
         same value, and a switch has at most one default label.";
    /// C99: §6.8.4.2 paragraph 4, p. 134; PDF p. 146.
    SWITCH_STATEMENT = "6.8.4.2p4", "p. 134; PDF p. 146", "The switch statement",
        "A switch jumps to, into or past its body, depending on its controlling expression's \
         value and its case and default labels.";
    /// C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
    SWITCH_JUMP = "6.8.4.2p5", "p. 134; PDF p. 146", "The switch statement",
        "The switch jumps to the case label whose value equals the promoted controlling \
         expression, else to default, else past the whole body.";
    /// C99: §6.8.1 paragraph 2, p. 131; PDF p. 143.
    LABEL_PLACE = "6.8.1p2", "p. 131; PDF p. 143", "Labeled statements",
        "A case or default label may appear only inside a switch statement.";
    /// C99: §6.8.1 paragraph 4, p. 132; PDF p. 144.
    LABELS_FLOW = "6.8.1p4", "p. 132; PDF p. 144", "Labeled statements",
        "A label does not alter the flow of control: execution that reaches it from the statement \
         before it continues across it.";
    /// C99: §6.8.6.3 paragraph 1, p. 138; PDF p. 150.
    BREAK_PLACE = "6.8.6.3p1", "p. 138; PDF p. 150", "The break statement",
        "A break may appear only inside a switch body or a loop body.";
    /// C99: §6.8.6.3 paragraph 2, p. 138; PDF p. 150.
    BREAK = "6.8.6.3p2", "p. 138; PDF p. 150", "The break statement",
        "A break ends the innermost switch or loop around it: control continues just after that \
         statement.";
    /// C99: §6.7.5.3 paragraph 1, p. 118; PDF p. 130.
    RETURN_TYPE = "6.7.5.3p1", "p. 118; PDF p. 130", "Function declarators",
        "A function may not return an array or a function; it may return a pointer to either.";
    /// C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
    LABEL_UNIQUE = "6.8.1p3", "p. 132; PDF p. 144", "Labeled statements",
        "No two labels of one function may have the same name.";
    /// C99: §6.2.1 paragraph 3, p. 29; PDF p. 41.
    LABEL_SCOPE = "6.2.1p3", "p. 29; PDF p. 41", "Scopes of identifiers",
        "A label's name can be used in a goto anywhere in its function, before or after the \
         label itself.";
    /// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
    NAME_HIDING = "6.2.1p4", "pp. 29-30; PDF pp. 41-42", "Scopes of identifiers",
        "A name declared in an inner block hides the same name declared outside it: inside the \
         inner scope the name means the inner one.";
    /// C99: §6.8.5.2 paragraph 1, p. 136; PDF p. 148.
    DO_STATEMENT = "6.8.5.2p1", "p. 136; PDF p. 148", "The do statement",
        "A do statement tests its controlling expression after each run of its body, so the body \
         runs at least once.";
    /// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
    GOTO_CONSTRAINT = "6.8.6.1p1", "p. 137; PDF p. 149", "The goto statement",
        "A goto must name a label in its own function, and may not jump into the scope of an \
         identifier with a variably modified type, such as a variable-length array.";
    /// C99: §6.8.6.1 paragraph 2, p. 137; PDF p. 149.
    GOTO = "6.8.6.1p2", "p. 137; PDF p. 149", "The goto statement",
        "A goto jumps unconditionally to the statement its label names, anywhere in the \
         function.";
    /// C99: §6.8.6.2 paragraph 1, p. 138; PDF p. 150.
    CONTINUE_PLACE = "6.8.6.2p1", "p. 138; PDF p. 150", "The continue statement",
        "A continue may appear only inside a loop body.";
    /// C99: §6.8.6.2 paragraph 2, p. 138; PDF p. 150.
    CONTINUE = "6.8.6.2p2", "p. 138; PDF p. 150", "The continue statement",
        "A continue jumps to the end of the innermost loop's body, so the loop goes on to its \
         next test (after a for statement's third clause).";
    /// C99: §4 paragraph 2, p. 7; PDF p. 19.
    UNDEFINED_BY_OMISSION = "4p2", "p. 7; PDF p. 19", "Conformance",
        "Behavior is undefined where the standard says so, and also wherever it gives no \
         definition at all.";
    /// C99: §6.6 paragraph 2, p. 95; PDF p. 107.
    CONSTANT_EXPRESSION = "6.6p2", "p. 95; PDF p. 107", "Constant expressions",
        "A constant expression can be evaluated while the program is translated, rather than \
         when it runs.";
    /// C99: §6.6 paragraph 3, p. 95; PDF p. 107.
    CONSTANT_OPERATORS = "6.6p3", "p. 95; PDF p. 107", "Constant expressions",
        "A constant expression has no assignment, ++, --, call or comma operator, outside an \
         operand that is not evaluated.";
    /// C99: §6.6 paragraph 4, p. 95; PDF p. 107.
    CONSTANT_IN_RANGE = "6.6p4", "p. 95; PDF p. 107", "Constant expressions",
        "A constant expression must evaluate to a value its type can represent.";
    /// C99: §6.6 paragraph 8, p. 96; PDF p. 108.
    ARITHMETIC_CONSTANT = "6.6p8", "p. 96; PDF p. 108", "Constant expressions",
        "An arithmetic constant expression is built from constants and sizeof with operators, \
         and casts only between arithmetic types; it reads no object.";
    /// C99: §6.5.3.3 paragraph 2, p. 79; PDF p. 91.
    UNARY_PLUS = "6.5.3.3p2", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "Unary + yields the value of its operand after the integer promotions.";
    /// C99: §6.5.3.3 paragraph 3, p. 79; PDF p. 91.
    UNARY_MINUS = "6.5.3.3p3", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "Unary - negates its (promoted) operand.";
    /// C99: §6.5.3.4 paragraph 2, p. 80; PDF p. 92.
    SIZEOF = "6.5.3.4p2", "p. 80; PDF p. 92", "The sizeof operator",
        "sizeof yields the size in bytes of its operand's type; the operand is not evaluated \
         unless its type is a variable length array.";
    /// C99: §6.5.3.4 paragraph 4, p. 80; PDF p. 92.
    SIZEOF_RESULT = "6.5.3.4p4", "p. 80; PDF p. 92", "The sizeof operator",
        "sizeof's result is an unsigned integer of type size_t, whose size is up to the \
         implementation.";
    /// C99: §6.5.3.3 paragraph 5, p. 79; PDF p. 91.
    LOGICAL_NOT = "6.5.3.3p5", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "!E is 1 when E compares equal to 0, and 0 otherwise; the result has type int.";
    /// C99: §6.5.5 paragraph 2, p. 82; PDF p. 94.
    MULTIPLICATIVE_CONSTRAINT = "6.5.5p2", "p. 82; PDF p. 94", "Multiplicative operators",
        "The operands of * and / must be arithmetic, and those of % integers.";
    /// C99: §6.5.5 paragraph 5, p. 82; PDF p. 94.
    DIVISION = "6.5.5p5", "p. 82; PDF p. 94", "Multiplicative operators",
        "/ gives the quotient (truncated toward zero) and % the remainder; dividing by zero is \
         undefined.";
    /// C99: §6.5.5 paragraph 6, p. 82; PDF p. 94.
    QUOTIENT = "6.5.5p6", "p. 82; PDF p. 94", "Multiplicative operators",
        "/ discards the fraction, and a % b is defined by (a/b)*b + a%b == a when a/b is \
         representable; C11 adds that otherwise both are undefined.";
    /// C99: §6.5.5 paragraph 4, p. 82; PDF p. 94.
    MULTIPLICATION = "6.5.5p4", "p. 82; PDF p. 94", "Multiplicative operators",
        "The result of * is the product of the operands.";
    /// C99: §6.5.6 paragraph 5, p. 83; PDF p. 95.
    ADDITION = "6.5.6p5", "p. 83; PDF p. 95", "Additive operators",
        "The result of + is the sum, and of - the difference, of the operands.";
    /// C99: §6.5.7 paragraph 2, p. 84; PDF p. 96.
    SHIFT_CONSTRAINT = "6.5.7p2", "p. 84; PDF p. 96", "Bitwise shift operators",
        "Both operands of << and >> must have integer types.";
    /// C99: §6.5.7 paragraph 3, p. 84; PDF p. 96.
    SHIFT_COUNT = "6.5.7p3", "p. 84; PDF p. 96", "Bitwise shift operators",
        "Each operand of a shift is promoted on its own, and the result has the left one's type; \
         a negative count, or one of at least its width, is undefined.";
    /// C99: §6.5.7 paragraph 4, p. 85; PDF p. 97.
    LEFT_SHIFT = "6.5.7p4", "p. 85; PDF p. 97", "Bitwise shift operators",
        "E1 << E2 is E1 × 2^E2: reduced modulo 2^N for an unsigned E1, and undefined for a \
         signed E1 that is negative or whose result does not fit.";
    /// C99: §6.5.7 paragraph 5, p. 85; PDF p. 97.
    RIGHT_SHIFT = "6.5.7p5", "p. 85; PDF p. 97", "Bitwise shift operators",
        "E1 >> E2 divides E1 by 2^E2, dropping the remainder; for a negative signed E1 the result \
         is implementation-defined.";
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95.
    POINTER_ARITHMETIC = "6.5.6p8", "p. 83; PDF p. 95", "Additive operators",
        "Adding n to a pointer moves it n elements within its array. Pointing one past the last \
         element is allowed, but dereferencing that position or going further is undefined.";
    /// C99: §6.5.6 paragraph 3, pp. 82-83; PDF pp. 94-95.
    SUBTRACTION_CONSTRAINT = "6.5.6p3", "pp. 82-83; PDF pp. 94-95", "Additive operators",
        "For -, both operands are numbers, or both are pointers to the same type, or the left is \
         a pointer and the right an integer.";
    /// C99: §6.5.6 paragraph 9, pp. 83-84; PDF pp. 95-96.
    POINTER_SUBTRACTION = "6.5.6p9", "pp. 83-84; PDF pp. 95-96", "Additive operators",
        "Subtracting two pointers into the same array gives how many elements apart they are, a \
         ptrdiff_t; for pointers into different arrays it is undefined.";
    /// C99: §6.5.8 paragraph 2, p. 85; PDF p. 97.
    RELATIONAL_CONSTRAINT = "6.5.8p2", "p. 85; PDF p. 97", "Relational operators",
        "<, >, <= and >= compare two numbers, or two pointers to the same type.";
    /// C99: §6.5.8 paragraph 5, pp. 85-86; PDF pp. 97-98.
    POINTER_ORDER = "6.5.8p5", "pp. 85-86; PDF pp. 97-98", "Relational operators",
        "Pointers into one array order like their subscripts, and pointers to one structure's \
         members like the members; ordering pointers to different objects is undefined.";
    /// C99: §6.5.8 paragraph 6, p. 86; PDF p. 98.
    RELATIONAL = "6.5.8p6", "p. 86; PDF p. 98", "Relational operators",
        "<, >, <= and >= yield the int 1 when the relation is true and 0 when it is false.";
    /// C99: §6.5.9 paragraph 3, p. 86; PDF p. 98.
    EQUALITY = "6.5.9p3", "p. 86; PDF p. 98", "Equality operators",
        "== and != yield the int 1 when the relation holds and 0 when it does not.";
    /// C99: §6.5.16.1 paragraph 2, p. 92; PDF p. 104.
    SIMPLE_ASSIGNMENT = "6.5.16.1p2", "p. 92; PDF p. 104", "Simple assignment",
        "x = v converts v to x's type and replaces the value stored in x.";
    /// C99: §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
    POINTER_ASSIGNMENT = "6.5.16.1p1", "p. 92; PDF p. 104", "Simple assignment",
        "A pointer may be assigned to one whose pointed-to type has all its qualifiers: a \
         pointer to const never converts to a pointer to non-const without a cast.";
    /// C99: §6.5.16 paragraph 3, p. 91; PDF p. 103.
    ASSIGNMENT_VALUE = "6.5.16p3", "p. 91; PDF p. 103", "Assignment operators",
        "An assignment expression has the value stored in its left operand, and that store is a \
         side effect C completes by the next sequence point.";
    /// C99: §6.5.16 paragraph 2, p. 91; PDF p. 103.
    ASSIGNMENT_CONSTRAINT = "6.5.16p2", "p. 91; PDF p. 103", "Assignment operators",
        "The left operand of =, += and the other assignment operators must be a modifiable \
         lvalue: an object that is not const, not an array.";
    /// C99: §6.5.2.4 paragraph 1, p. 75; PDF p. 87.
    INCREMENT_CONSTRAINT = "6.5.2.4p1", "p. 75; PDF p. 87", "Postfix increment and decrement operators",
        "The operand of ++ or -- must be a modifiable lvalue of arithmetic or pointer type: not \
         a const object.";
    /// C99: §6.5.16.2 paragraph 1, p. 93; PDF p. 105.
    COMPOUND_CONSTRAINT = "6.5.16.2p1", "p. 93; PDF p. 105", "Compound assignment",
        "+= and -= take an arithmetic left operand, or a pointer with an integer on the right; \
         the other compound assignments take arithmetic operands.";
    /// C99: §6.5.16.2 paragraph 3, p. 93; PDF p. 105.
    COMPOUND_ASSIGNMENT = "6.5.16.2p3", "p. 93; PDF p. 105", "Compound assignment",
        "E1 op= E2 is E1 = E1 op (E2), except that E1 is evaluated only once.";
    /// C99: §6.5.4 paragraph 2, p. 81; PDF p. 93.
    CAST_CONSTRAINT = "6.5.4p2", "p. 81; PDF p. 93", "Cast operators",
        "A cast names a scalar type and converts a scalar operand: a number or a pointer.";
    /// C99: §6.5.4 paragraph 4, p. 81; PDF p. 93.
    CAST = "6.5.4p4", "p. 81; PDF p. 93", "Cast operators",
        "(type) expression converts the expression's value to the named type; a cast to the type \
         the value already has changes nothing.";
    /// C99: §6.3.2.3 paragraph 2, p. 47; PDF p. 59.
    QUALIFIED_POINTER = "6.3.2.3p2", "p. 47; PDF p. 59", "Pointers",
        "A pointer to T converts to a pointer to const T and still points to the same object.";
    /// C99: §6.3.2.1 paragraph 1, p. 46; PDF p. 58.
    MODIFIABLE_LVALUE = "6.3.2.1p1", "p. 46; PDF p. 58", "Lvalues, arrays, and function designators",
        "A modifiable lvalue designates an object the program may store into: not an array and \
         not const-qualified.";
    /// C99: §6.7.3 paragraph 5, p. 109; PDF p. 121.
    CONST_WRITE = "6.7.3p5", "p. 109; PDF p. 121", "Type qualifiers",
        "Modifying an object defined const through an lvalue that is not const, such as a \
         pointer whose const a cast removed, is undefined.";
    /// C99: §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
    STRUCTURE_ASSIGNMENT = "6.5.16.1p1", "p. 92; PDF p. 104", "Simple assignment",
        "Assignment works on whole structures of the same type: unlike arrays, a structure can \
         be copied with =.";
    /// C99: §6.5.16 paragraph 4, p. 91; PDF p. 103.
    ASSIGNMENT_ORDER = "6.5.16p4", "p. 91; PDF p. 103", "Assignment operators",
        "Which operand of an assignment is evaluated first is unspecified.";
    /// C99: §6.7.2.1 paragraph 12, p. 103; PDF p. 115.
    MEMBER_ALIGNMENT = "6.7.2.1p12", "p. 103; PDF p. 115", "Structure and union specifiers",
        "Each member is aligned as its type needs, in an implementation-defined way; on x86-64 \
         an int starts at a multiple of 4 bytes and a pointer at a multiple of 8.";
    /// C99: §6.7.2.1 paragraph 13, p. 103; PDF p. 115.
    MEMBER_ORDER = "6.7.2.1p13", "p. 103; PDF p. 115", "Structure and union specifiers",
        "Members are laid out in the order they are declared, the first at the very start; \
         there may be unnamed padding between them.";
    /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112.
    TYPE_SPECIFIERS = "6.7.2p2", "pp. 99-100; PDF pp. 111-112", "Type specifiers",
        "An integer type is named by one of a fixed set of keyword combinations, such as \
         unsigned long int, the keywords in any order.";
    /// C99: §6.7.2.1 paragraph 15, p. 103; PDF p. 115.
    TRAILING_PADDING = "6.7.2.1p15", "p. 103; PDF p. 115", "Structure and union specifiers",
        "There may be unnamed padding at the end of a structure, so that every element of an \
         array of it stays aligned.";
    /// C99: §6.7.8 paragraph 4, p. 125; PDF p. 137.
    STATIC_INITIALIZER = "6.7.8p4", "p. 125; PDF p. 137", "Initialization",
        "A static object's initializer must be a constant expression: its value is known before \
         the program runs.";
    /// C99: §6.7.8 paragraph 7, p. 125; PDF p. 137.
    MEMBER_DESIGNATOR = "6.7.8p7", "p. 125; PDF p. 137", "Initialization",
        "In an initializer list, .m = names the member that the next value initializes.";
    /// C99: §6.7.8 paragraph 10, p. 126; PDF p. 138.
    INDETERMINATE = "6.7.8p10", "p. 126; PDF p. 138", "Initialization",
        "A local variable that is not initialized holds an indeterminate value, not zero.";
    /// C99: §6.7.8 paragraph 10, p. 126; PDF p. 138.
    STATIC_ZERO = "6.7.8p10", "p. 126; PDF p. 138", "Initialization",
        "A static object without an initializer starts at zero: every arithmetic member 0, every \
         pointer null.";
    /// C99: §6.7.8 paragraph 11, p. 126; PDF p. 138.
    SCALAR_INITIALIZER = "6.7.8p11", "p. 126; PDF p. 138", "Initialization",
        "A scalar's initial value is its initializer's value, converted as if by assignment.";
    /// C99: §6.7.8 paragraph 13, p. 126; PDF p. 138.
    STRUCTURE_INITIALIZER = "6.7.8p13", "p. 126; PDF p. 138", "Initialization",
        "A local structure is initialized by a brace-enclosed list or by a whole structure of \
         the same type, whose value it copies.";
    /// C99: §6.7.8 paragraph 17, pp. 126-127; PDF pp. 138-139.
    INITIALIZER_ORDER = "6.7.8p17", "pp. 126-127; PDF pp. 138-139", "Initialization",
        "Without designators, an initializer list fills an array's elements in increasing index \
         order.";
    /// C99: §6.7.8 paragraph 17, pp. 126-127; PDF pp. 138-139.
    MEMBER_INITIALIZER_ORDER = "6.7.8p17", "pp. 126-127; PDF pp. 138-139", "Initialization",
        "An initializer list fills a structure's members in declaration order; a designator \
         .m = jumps to member m, and the list continues from the member after it.";
    /// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
    IMPLICIT_INITIALIZATION = "6.7.8p19", "p. 127; PDF p. 139", "Initialization",
        "Every member or element an initializer list does not mention is initialized as a \
         static object would be: to zero.";
    /// C99: §6.7.8 paragraph 21, p. 127; PDF p. 139.
    PARTIAL_INITIALIZER = "6.7.8p21", "p. 127; PDF p. 139", "Initialization",
        "Array elements without an initializer of their own are initialized to zero.";
    /// C99: §6.7.8 paragraph 23, p. 128; PDF p. 140.
    LIST_SIDE_EFFECTS = "6.7.8p23", "p. 128; PDF p. 140", "Initialization",
        "C leaves unspecified the order in which the side effects of an initializer list's \
         expressions happen.";
    /// C99: §6.8 paragraph 4, p. 131; PDF p. 143.
    FULL_EXPRESSION = "6.8p4", "p. 131; PDF p. 143", "Statements and blocks",
        "The end of a full expression (an initializer, an expression statement, a condition, a \
         return value) is a sequence point: all its side effects are complete.";
    /// C99: §6.8 paragraph 3, p. 131; PDF p. 143.
    DECLARATION_REACHED = "6.8p3", "p. 131; PDF p. 143", "Statements and blocks",
        "A local's initializer is evaluated and stored each time execution reaches its \
         declaration, as if the declaration were a statement; a local without one gets an \
         indeterminate value there.";
    /// C99: §6.8.2 paragraph 2, p. 132; PDF p. 144.
    COMPOUND_STATEMENT = "6.8.2p2", "p. 132; PDF p. 144", "Compound statement",
        "A { ... } compound statement is a block, with its own scope for declarations.";
    /// C99: §6.8.3 paragraph 2, p. 132; PDF p. 144.
    EXPRESSION_STATEMENT = "6.8.3p2", "p. 132; PDF p. 144", "Expression and null statements",
        "An expression statement is evaluated only for its side effects; its value is discarded.";
    /// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145.
    SELECTION_BLOCK = "6.8.4p3", "p. 133; PDF p. 145", "Selection statements",
        "An if statement is a block, and each of its branches is a block inside it, with or \
         without braces.";
    /// C99: §6.8.4.1 paragraph 2, pp. 133-134; PDF pp. 145-146.
    IF_STATEMENT = "6.8.4.1p2", "pp. 133-134; PDF pp. 145-146", "The if statement",
        "The first branch runs if the condition is nonzero; the else branch runs if it is zero.";
    /// C99: §6.8.5 paragraph 4, p. 135; PDF p. 147.
    ITERATION = "6.8.5p4", "p. 135; PDF p. 147", "Iteration statements",
        "A loop runs its body repeatedly until its controlling expression compares equal to 0.";
    /// C99: §6.8.5 paragraph 5, p. 135; PDF p. 147.
    LOOP_BODY = "6.8.5p5", "p. 135; PDF p. 147", "Iteration statements",
        "A loop is a block, and its body is a block inside it, so the body's objects begin and \
         end with each pass.";
    /// C99: §6.8.5.1 paragraph 1, p. 136; PDF p. 148.
    WHILE_STATEMENT = "6.8.5.1p1", "p. 136; PDF p. 148", "The while statement",
        "A while statement evaluates its controlling expression before each run of its body, so \
         the body may not run at all.";
    /// C99: §6.8.5.3 paragraph 1, p. 136; PDF p. 148.
    FOR_STATEMENT = "6.8.5.3p1", "p. 136; PDF p. 148", "The for statement",
        "A for statement runs its first clause once, then evaluates its condition before each run \
         of its body and its third clause after each; a name declared in the first clause can be \
         used in the whole loop.";
    /// C99: §6.8.5.3 paragraph 2, p. 136; PDF p. 148.
    OMITTED_CONDITION = "6.8.5.3p2", "p. 136; PDF p. 148", "The for statement",
        "A for statement's omitted controlling expression is replaced by a nonzero constant.";
    /// C99: §6.8.6.4 paragraph 3, p. 139; PDF p. 151.
    RETURN = "6.8.6.4p3", "p. 139; PDF p. 151", "The return statement",
        "return hands the expression's value back to the caller as the value of the call.";
    /// C99: §6.9.1 paragraph 9, p. 142; PDF p. 154.
    PARAMETERS = "6.9.1p9", "p. 142; PDF p. 154", "Function definitions",
        "Each parameter is a local object with automatic storage duration, private to this call.";
    /// C99: §6.9.1 paragraph 12, p. 142; PDF p. 154.
    FUNCTION_END = "6.9.1p12", "p. 142; PDF p. 154", "Function definitions",
        "If the } that ends a function is reached and the caller uses the call's value, the \
         behavior is undefined.";
    /// C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
    TRANSLATION_PHASES = "5.1.1.2p1", "pp. 9-10; PDF pp. 21-22", "Translation phases",
        "A source file is translated in phases; in phase 4 the preprocessor executes directives \
         such as #include, long before the program runs.";
    /// C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
    LITERAL_CONCATENATION_PHASE = "5.1.1.2p1", "pp. 9-10; PDF pp. 21-22", "Translation phases",
        "In translation phase 6, after escape sequences are converted, adjacent string literals \
         are joined into one.";
    /// C99: §6.4.5 paragraph 4, p. 62; PDF p. 74.
    STRING_CONCATENATION = "6.4.5p4", "p. 62; PDF p. 74", "String literals",
        "Adjacent string literals, such as \"a\" \"b\", are concatenated into a single \
         literal, \"ab\", with one null at its end.";
    /// C99: §6.4.4.4 paragraph 10, p. 61; PDF p. 73.
    CHARACTER_CONSTANT = "6.4.4.4p10", "p. 61; PDF p. 73", "Character constants",
        "A character constant such as 'a' is an int whose value is the character's code: 'a' is \
         97.";
    /// C99: §6.4.5 paragraph 5, pp. 62-63; PDF pp. 74-75.
    STRING_LITERAL = "6.4.5p5", "pp. 62-63; PDF pp. 74-75", "String literals",
        "A string literal is an array of char with static storage duration, holding its \
         characters and a null character, '\\0', added at the end.";
    /// C99: §6.4.5 paragraph 6, p. 63; PDF p. 75.
    STRING_LITERAL_WRITE = "6.4.5p6", "p. 63; PDF p. 75", "String literals",
        "A program must not modify a string literal's array: doing so is undefined, and \
         identical literals may even share one array.";
    /// C99: §6.5.2.2 paragraph 6, pp. 71-72; PDF pp. 83-84.
    DEFAULT_PROMOTIONS = "6.5.2.2p6", "pp. 71-72; PDF pp. 83-84", "Function calls",
        "The default argument promotions are the integer promotions (and float to double): a \
         char argument becomes an int.";
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    VARIADIC_ARGUMENTS = "6.5.2.2p7", "p. 72; PDF p. 84", "Function calls",
        "Arguments matching a prototype's parameters are converted to their types; the arguments \
         after them, matching its `...`, get the default argument promotions.";
    /// C99: §6.6 paragraph 9, p. 96; PDF p. 108.
    ADDRESS_CONSTANT = "6.6p9", "p. 96; PDF p. 108", "Constant expressions",
        "The address of an object with static storage duration, such as a string literal's \
         array, is a constant a static object may be initialized with.";
    /// C99: §6.7.8 paragraph 14, p. 126; PDF p. 138.
    STRING_INITIALIZER = "6.7.8p14", "p. 126; PDF p. 138", "Initialization",
        "A string literal can initialize a char array: its characters, and its null if there is \
         room, fill the elements in order.";
    /// C99: §6.10.2 paragraph 2, p. 149; PDF p. 161.
    SOURCE_INCLUSION = "6.10.2p2", "p. 149; PDF p. 161", "Source file inclusion",
        "#include <name> is replaced by the entire contents of the header that name \
         identifies.";
    /// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
    OBJECT_LIKE_MACRO = "6.10.3p9", "p. 152; PDF p. 164", "Macro replacement",
        "#define NAME replacement makes each later NAME stand for the replacement: the \
         preprocessor substitutes it before the program is compiled.";
    /// C99: §6.10.3 paragraph 2, p. 151; PDF p. 163.
    MACRO_REDEFINITION = "6.10.3p2", "p. 151; PDF p. 163", "Macro replacement",
        "A macro may be defined again only with an identical replacement list.";
    /// C99: §7.1.1 paragraph 1, p. 164; PDF p. 176.
    STRING = "7.1.1p1", "p. 164; PDF p. 176", "Definitions of terms",
        "A string is a sequence of characters ending with the first null character; a pointer to \
         a string points to its first character.";
    /// C99: §7.1.2 paragraph 1, p. 165; PDF p. 177.
    LIBRARY_HEADERS = "7.1.2p1", "p. 165; PDF p. 177", "Standard headers",
        "Each library function is declared, with a prototype, in a header that #include makes \
         available.";
    /// C99: §7.17 paragraph 2, p. 254; PDF p. 266.
    SIZE_T = "7.17p2", "p. 254; PDF p. 266", "Common definitions <stddef.h>",
        "size_t is the unsigned integer type of sizeof's result; <stdio.h> defines it too.";
    /// C99: §7.1.2 paragraph 4, p. 165; PDF p. 177.
    HEADER_ORDER = "7.1.2p4", "p. 165; PDF p. 177", "Standard headers",
        "A standard header may be included more than once with no further effect, and must be \
         included before the first use of what it declares.";
    /// C99: §7.1.4 paragraph 1, pp. 166-167; PDF pp. 178-179.
    LIBRARY_ARGUMENTS = "7.1.4p1", "pp. 166-167; PDF pp. 178-179", "Use of library functions",
        "Passing a library function an invalid argument, such as a null pointer where it needs \
         a string, is undefined.";
    /// C99: §7.1.4 paragraph 3, p. 167; PDF p. 179.
    LIBRARY_SEQUENCE_POINT = "7.1.4p3", "p. 167; PDF p. 179", "Use of library functions",
        "There is a sequence point just before a library function returns.";
    /// C99: §7.15.1.1 paragraph 2, pp. 249-250; PDF pp. 261-262.
    VARIADIC_TYPES = "7.15.1.1p2", "pp. 249-250; PDF pp. 261-262", "The va_arg macro",
        "A function reading its `...` arguments must read each with its promoted type; an int \
         may stand for an unsigned int only if its value fits in both.";
    /// C99: §7.19.6.1 paragraph 2, p. 274; PDF p. 286.
    FPRINTF = "7.19.6.1p2", "p. 274; PDF p. 286", "The fprintf function",
        "printf's format string says how to convert each later argument to text; too few \
         arguments is undefined, and extra ones are evaluated and ignored.";
    /// C99: §7.19.6.1 paragraph 7, pp. 276-277; PDF pp. 288-289.
    LENGTH_MODIFIERS = "7.19.6.1p7", "pp. 276-277; PDF pp. 288-289", "The fprintf function",
        "A length modifier says which integer type a conversion takes: hh a char, h a short, l a \
         long, ll a long long, z a size_t.";
    /// C99: §7.19.6.1 paragraph 4, p. 275; PDF p. 287.
    CONVERSION_SPECIFICATION = "7.19.6.1p4", "p. 275; PDF p. 287", "The fprintf function",
        "A conversion specification starts with % and has flags, a minimum field width, a \
         precision, a length modifier and a conversion specifier, in that order; a shorter \
         result is padded with spaces to the width.";
    /// C99: §7.19.6.1 paragraph 4, p. 275; PDF p. 287.
    PRECISION = "7.19.6.1p4", "p. 275; PDF p. 287", "The fprintf function",
        "A precision, a period and a number, is the minimum number of digits for %d, %i, %u and \
         %x, and the maximum number of characters for %s; with any other conversion it is \
         undefined.";
    /// C99: §7.19.6.1 paragraph 6, pp. 275-276; PDF pp. 287-288.
    PRINTF_FLAGS = "7.19.6.1p6", "pp. 275-276; PDF pp. 287-288", "The fprintf function",
        "The - flag pads on the right instead of the left; the 0 flag pads a number with zeros, \
         and is undefined for %c and %s.";
    /// C99: §7.19.6.1 paragraph 8, pp. 277-280; PDF pp. 289-292.
    CONVERSIONS = "7.19.6.1p8", "pp. 277-280; PDF pp. 289-292", "The fprintf function",
        "%d and %i write an int in decimal, %u and %x an unsigned int in decimal and hexadecimal, \
         %c one character, %s a string up to its null, and %% a percent sign.";
    /// C99: §7.19.6.1 paragraph 8, pp. 277-280; PDF pp. 289-292.
    FLOATING_CONVERSIONS = "7.19.6.1p8", "pp. 277-280; PDF pp. 289-292", "The fprintf function",
        "%f writes a double as [-]ddd.ddd, %e as d.ddde±dd, %g in whichever of the two suits it \
         with trailing zeros removed, and %a in hexadecimal; the precision (6 by default) counts \
         digits, and the value is rounded.";
    /// C99: §7.19.6.1 paragraph 7, pp. 276-277; PDF pp. 288-289.
    LONG_DOUBLE_MODIFIER = "7.19.6.1p7", "pp. 276-277; PDF pp. 288-289", "The fprintf function",
        "The L modifier makes %f, %e, %g and %a take a long double; l before them has no effect, \
         and any other modifier there is undefined.";
    /// C99: §7.19.6.1 paragraph 9, p. 280; PDF p. 292.
    PRINTF_UNDEFINED = "7.19.6.1p9", "p. 280; PDF p. 292", "The fprintf function",
        "An invalid conversion specification, or an argument of the wrong type for its \
         conversion, is undefined.";
    /// C99: §7.19.6.3 paragraph 2, p. 290; PDF p. 302.
    PRINTF = "7.19.6.3p2", "p. 290; PDF p. 302", "The printf function",
        "printf is fprintf writing to standard output.";
    /// C99: §7.19.6.3 paragraph 3, p. 290; PDF p. 302.
    PRINTF_RETURN = "7.19.6.3p3", "p. 290; PDF p. 302", "The printf function",
        "printf returns the number of characters it wrote.";
    /// C99: §7.19.7.3 paragraph 2, p. 297; PDF p. 309.
    FPUTC = "7.19.7.3p2", "p. 297; PDF p. 309", "The fputc function",
        "The character to write is converted to unsigned char first, so -56 writes the byte \
         200.";
    /// C99: §7.19.7.3 paragraph 3, p. 297; PDF p. 309.
    FPUTC_RETURN = "7.19.7.3p3", "p. 297; PDF p. 309", "The fputc function",
        "fputc, and so putchar, returns the character it wrote, as an unsigned char converted to \
         int.";
    /// C99: §7.19.7.9 paragraph 2, p. 299; PDF p. 311.
    PUTCHAR = "7.19.7.9p2", "p. 299; PDF p. 311", "The putchar function",
        "putchar(c) writes one character to standard output, like fputc(c, stdout).";
    /// C99: §7.19.7.10 paragraph 2, p. 299; PDF p. 311.
    PUTS = "7.19.7.10p2", "p. 299; PDF p. 311", "The puts function",
        "puts writes a string to standard output, without its null, and then a new-line.";
    /// C99: §7.19.7.10 paragraph 3, p. 299; PDF p. 311.
    PUTS_RETURN = "7.19.7.10p3", "p. 299; PDF p. 311", "The puts function",
        "puts returns a nonnegative value when it succeeds; which one is up to the library.";
}
