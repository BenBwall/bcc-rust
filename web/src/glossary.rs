//! The words of the C standard the narration uses, each defined in plain
//! language with the paragraph that defines it. The narration marks the
//! first use of each term in a step's text, and the page lists them all in
//! its glossary.

use leptos::prelude::*;

use crate::snapshot::N1256_URL;

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
pub static TERMS: [Term; 47] = [
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
                     x86-64. Its arithmetic never overflows.",
        clause:     "6.2.5p6",
        page:       45,
    },
    Term {
        id:         "modulo-reduction",
        name:       "Reduction modulo 2^N",
        phrases:    &["reduced modulo", "reduces it modulo", "reduces the value modulo"],
        definition: "What an unsigned type does with a value it cannot hold: it adds or subtracts \
                     2^N, one more than its largest value, until the value fits, so -1 becomes \
                     the largest value. This is defined, unlike signed overflow.",
        clause:     "6.2.5p9",
        page:       46,
    },
    Term {
        id:         "size-t",
        name:       "size_t",
        phrases:    &["size_t"],
        definition: "The unsigned integer type of sizeof's result, defined by <stddef.h> and \
                     <stdio.h>: unsigned long on x86-64. printf prints it with %zu.",
        clause:     "7.17p2",
        page:       266,
    },
    Term {
        id:         "floating-type",
        name:       "Floating type",
        phrases:    &["real floating types", "floating types", "floating type"],
        definition: "A type that holds numbers with a fraction, approximately: float, double and \
                     long double, each holding every value of the one before. On x86-64 they are \
                     IEEE 754 binary32 and binary64 and the x87 80-bit extended format.",
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
        id:         "null-pointer",
        name:       "Null pointer",
        phrases:    &["null pointer"],
        definition: "A pointer that points to no object. Converting the integer constant 0 to a \
                     pointer type gives one.",
        clause:     "6.3.2.3p3",
        page:       59,
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
                     to a const object, gets a diagnostic and does not compile.",
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
pub fn Glossary() -> impl IntoView {
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
