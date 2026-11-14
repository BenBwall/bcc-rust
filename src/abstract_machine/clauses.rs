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
    /// C99: §6.2.4 paragraph 2, p. 32; PDF p. 44.
    LIFETIME = "6.2.4p2", "p. 32; PDF p. 44", "Storage durations of objects",
        "An object exists only during its lifetime; using it, or a pointer to it, after the \
         lifetime ends is undefined.";
    /// C99: §6.2.4 paragraph 3, p. 32; PDF p. 44.
    STATIC_STORAGE = "6.2.4p3", "p. 32; PDF p. 44", "Storage durations of objects",
        "An object declared outside every function, or with static, lives for the whole run of \
         the program and is initialized only once, before the program starts.";
    /// C99: §6.2.4 paragraph 5, p. 32; PDF p. 44.
    AUTOMATIC_LIFETIME = "6.2.4p5", "p. 32; PDF p. 44", "Storage durations of objects",
        "A local variable lives from entry into its block until the block finishes; each new \
         entry, including a recursive call, creates a fresh object.";
    /// C99: §6.2.5 paragraph 15, p. 35; PDF p. 47.
    CHAR_SIGNEDNESS = "6.2.5p15", "p. 35; PDF p. 47", "Types",
        "Whether plain char is signed or unsigned is up to the implementation; on x86-64 Linux \
         it is signed, holding -128 to 127.";
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
    /// C99: §6.3.1.1 paragraph 2, pp. 42-43; PDF pp. 54-55.
    INTEGER_PROMOTIONS = "6.3.1.1p2", "pp. 42-43; PDF pp. 54-55", "Boolean, characters, and integers",
        "Before arithmetic, values of types narrower than int (like char) are converted to int.";
    /// C99: §6.3.1.3 paragraph 3, p. 43; PDF p. 55.
    SIGNED_CONVERSION = "6.3.1.3p3", "p. 43; PDF p. 55", "Signed and unsigned integers",
        "Converting a value that does not fit into a signed type gives an implementation-defined \
         result; x86-64 compilers keep the low bits (wrap around).";
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    USUAL_ARITHMETIC = "6.3.1.8p1", "pp. 44-45; PDF pp. 56-57", "Usual arithmetic conversions",
        "Binary arithmetic first converts both operands to a common type; two ints need no \
         conversion.";
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
    /// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
    INTEGER_CONSTANT = "6.4.4.1p5", "pp. 55-56; PDF pp. 67-68", "Integer constants",
        "A decimal constant without a suffix has type int when its value fits in an int.";
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
    /// C99: §6.5.3.3 paragraph 2, p. 79; PDF p. 91.
    UNARY_PLUS = "6.5.3.3p2", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "Unary + yields the value of its operand after the integer promotions.";
    /// C99: §6.5.3.3 paragraph 3, p. 79; PDF p. 91.
    UNARY_MINUS = "6.5.3.3p3", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "Unary - negates its (promoted) operand.";
    /// C99: §6.5.3.3 paragraph 5, p. 79; PDF p. 91.
    LOGICAL_NOT = "6.5.3.3p5", "p. 79; PDF p. 91", "Unary arithmetic operators",
        "!E is 1 when E compares equal to 0, and 0 otherwise; the result has type int.";
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
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95.
    POINTER_ARITHMETIC = "6.5.6p8", "p. 83; PDF p. 95", "Additive operators",
        "Adding n to a pointer moves it n elements within its array. Pointing one past the last \
         element is allowed, but dereferencing that position or going further is undefined.";
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
    /// C99: §6.8 paragraph 4, p. 131; PDF p. 143.
    FULL_EXPRESSION = "6.8p4", "p. 131; PDF p. 143", "Statements and blocks",
        "The end of a full expression (an initializer, an expression statement, a condition, a \
         return value) is a sequence point: all its side effects are complete.";
    /// C99: §6.8.2 paragraph 2, p. 132; PDF p. 144.
    COMPOUND_STATEMENT = "6.8.2p2", "p. 132; PDF p. 144", "Compound statement",
        "A { ... } compound statement is a block, with its own scope for declarations.";
    /// C99: §6.8.3 paragraph 2, p. 132; PDF p. 144.
    EXPRESSION_STATEMENT = "6.8.3p2", "p. 132; PDF p. 144", "Expression and null statements",
        "An expression statement is evaluated only for its side effects; its value is discarded.";
    /// C99: §6.8.4.1 paragraph 2, pp. 133-134; PDF pp. 145-146.
    IF_STATEMENT = "6.8.4.1p2", "pp. 133-134; PDF pp. 145-146", "The if statement",
        "The first branch runs if the condition is nonzero; the else branch runs if it is zero.";
    /// C99: §6.8.5 paragraph 4, p. 135; PDF p. 147.
    ITERATION = "6.8.5p4", "p. 135; PDF p. 147", "Iteration statements",
        "A loop runs its body repeatedly until its controlling expression compares equal to 0.";
    /// C99: §6.8.5.3 paragraph 2, p. 136; PDF p. 148.
    OMITTED_CONDITION = "6.8.5.3p2", "p. 136; PDF p. 148", "The for statement",
        "A for statement's omitted controlling expression is replaced by a nonzero constant.";
    /// C99: §6.8.6.4 paragraph 3, p. 139; PDF p. 151.
    RETURN = "6.8.6.4p3", "p. 139; PDF p. 151", "The return statement",
        "return hands the expression's value back to the caller as the value of the call.";
    /// C99: §6.9.1 paragraph 9, p. 142; PDF p. 154.
    PARAMETERS = "6.9.1p9", "p. 142; PDF p. 154", "Function definitions",
        "Each parameter is a local object with automatic storage duration, private to this call.";
    /// C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
    TRANSLATION_PHASES = "5.1.1.2p1", "pp. 9-10; PDF pp. 21-22", "Translation phases",
        "A source file is translated in phases; in phase 4 the preprocessor executes directives \
         such as #include, long before the program runs.";
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
    /// C99: §7.1.1 paragraph 1, p. 164; PDF p. 176.
    STRING = "7.1.1p1", "p. 164; PDF p. 176", "Definitions of terms",
        "A string is a sequence of characters ending with the first null character; a pointer to \
         a string points to its first character.";
    /// C99: §7.1.2 paragraph 1, p. 165; PDF p. 177.
    LIBRARY_HEADERS = "7.1.2p1", "p. 165; PDF p. 177", "Standard headers",
        "Each library function is declared, with a prototype, in a header that #include makes \
         available.";
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
    /// C99: §7.19.6.1 paragraph 4, p. 275; PDF p. 287.
    CONVERSION_SPECIFICATION = "7.19.6.1p4", "p. 275; PDF p. 287", "The fprintf function",
        "A conversion specification starts with % and has flags, a minimum field width, a \
         precision, a length modifier and a conversion specifier, in that order; a shorter \
         result is padded with spaces to the width.";
    /// C99: §7.19.6.1 paragraph 6, pp. 275-276; PDF pp. 287-288.
    PRINTF_FLAGS = "7.19.6.1p6", "pp. 275-276; PDF pp. 287-288", "The fprintf function",
        "The - flag pads on the right instead of the left; the 0 flag pads a number with zeros, \
         and is undefined for %c and %s.";
    /// C99: §7.19.6.1 paragraph 8, pp. 277-280; PDF pp. 289-292.
    CONVERSIONS = "7.19.6.1p8", "pp. 277-280; PDF pp. 289-292", "The fprintf function",
        "%d and %i write an int in decimal, %u and %x an unsigned int in decimal and hexadecimal, \
         %c one character, %s a string up to its null, and %% a percent sign.";
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
