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
    /// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
    BLOCK_SCOPE = "6.2.1p4", "pp. 29-30; PDF pp. 41-42", "Scopes of identifiers",
        "A name declared inside a block can be used only from its declaration to the end of that \
         block.";
    /// C99: §6.2.4 paragraph 2, p. 32; PDF p. 44.
    LIFETIME = "6.2.4p2", "p. 32; PDF p. 44", "Storage durations of objects",
        "An object exists only during its lifetime; using it, or a pointer to it, after the \
         lifetime ends is undefined.";
    /// C99: §6.2.4 paragraph 5, p. 32; PDF p. 44.
    AUTOMATIC_LIFETIME = "6.2.4p5", "p. 32; PDF p. 44", "Storage durations of objects",
        "A local variable lives from entry into its block until the block finishes; each new \
         entry, including a recursive call, creates a fresh object.";
    /// C99: §6.2.5 paragraph 15, p. 35; PDF p. 47.
    CHAR_SIGNEDNESS = "6.2.5p15", "p. 35; PDF p. 47", "Types",
        "Whether plain char is signed or unsigned is up to the implementation; on x86-64 Linux \
         it is signed, holding -128 to 127.";
    /// C99: §6.2.6.1 paragraph 5, pp. 37-38; PDF pp. 49-50.
    TRAP_REPRESENTATION = "6.2.6.1p5", "pp. 37-38; PDF pp. 49-50", "Representations of types",
        "Some bit patterns are not values of their type (trap representations); reading one as \
         that type is undefined, though a character type such as unsigned char may read any \
         byte.";
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
    /// C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83.
    CALL_ARGUMENTS = "6.5.2.2p4", "p. 71; PDF p. 83", "Function calls",
        "Arguments are evaluated and each parameter is assigned a copy of its argument's value: \
         C passes arguments by value.";
    /// C99: §6.5.2.2 paragraph 10, p. 72; PDF p. 84.
    CALL_SEQUENCE_POINT = "6.5.2.2p10", "p. 72; PDF p. 84", "Function calls",
        "There is a sequence point just before the call: every argument is fully evaluated first.";
    /// C99: §6.5.2.4 paragraph 2, p. 75; PDF p. 87.
    POSTFIX_INCREMENT = "6.5.2.4p2", "p. 75; PDF p. 87", "Postfix increment and decrement operators",
        "x++ yields x's old value; the stored value grows by one before the next sequence point.";
    /// C99: §6.5.3.1 paragraph 2, p. 78; PDF p. 90.
    PREFIX_INCREMENT = "6.5.3.1p2", "p. 78; PDF p. 90", "Prefix increment and decrement operators",
        "++x adds one to x and yields the new value; it is equivalent to x += 1.";
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
    /// C99: §6.7.8 paragraph 10, p. 126; PDF p. 138.
    INDETERMINATE = "6.7.8p10", "p. 126; PDF p. 138", "Initialization",
        "A local variable that is not initialized holds an indeterminate value, not zero.";
    /// C99: §6.7.8 paragraph 11, p. 126; PDF p. 138.
    SCALAR_INITIALIZER = "6.7.8p11", "p. 126; PDF p. 138", "Initialization",
        "A scalar's initial value is its initializer's value, converted as if by assignment.";
    /// C99: §6.7.8 paragraph 17, pp. 126-127; PDF pp. 138-139.
    INITIALIZER_ORDER = "6.7.8p17", "pp. 126-127; PDF pp. 138-139", "Initialization",
        "Without designators, an initializer list fills an array's elements in increasing index \
         order.";
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
    /// C99: §6.8.6.4 paragraph 3, p. 139; PDF p. 151.
    RETURN = "6.8.6.4p3", "p. 139; PDF p. 151", "The return statement",
        "return hands the expression's value back to the caller as the value of the call.";
    /// C99: §6.9.1 paragraph 9, p. 142; PDF p. 154.
    PARAMETERS = "6.9.1p9", "p. 142; PDF p. 154", "Function definitions",
        "Each parameter is a local object with automatic storage duration, private to this call.";
}
