#!/usr/bin/env python3
"""Generate focused parser differential cases without a compiler or downloads."""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import random
import textwrap


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(".scratch/parser-study/grammar"))
    parser.add_argument("--seed", type=int, default=20261021)
    parser.add_argument("--fuzz-count", type=int, choices=range(33), default=16,
                        metavar="0..32", help="bounded generated valid expressions (default: 16)")
    args = parser.parse_args()
    cases = focused_cases() + fuzz_cases(args.seed, args.fuzz_count)
    names = [case["name"] for case in cases]
    if len(names) != len(set(names)):
        raise ValueError("duplicate corpus case name")
    args.output.mkdir(parents=True, exist_ok=True)
    entries = []
    for case in cases:
        source = case["source"].encode("utf-8")
        (args.output / case["name"]).write_bytes(source)
        entries.append({key: value for key, value in case.items() if key != "source"} | {
            "bytes": len(source), "sha256": hashlib.sha256(source).hexdigest(),
        })
    counts = dict(sorted(Counter(case["classification"] for case in cases).items()))
    manifest = {
        "schema_version": 1,
        "generator": "scripts/parser_study_cases.py",
        "seed": args.seed,
        "fuzz_count": args.fuzz_count,
        "configuration": "CompilerConfiguration::default(): C99, GNU off, extensions Allow",
        "classification_scope": "Intended parser syntax only; no semantic-validity assertion",
        "counts": counts,
        "cases": entries,
    }
    (args.output / "manifest.json").write_bytes(
        (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    )
    print(f"Prepared {len(cases)} cases in {args.output}: "
          + ", ".join(f"{key}={value}" for key, value in counts.items()))


def focused_cases():
    cases = []

    def add(name, source, coverage, classification="valid", note=None):
        case = {"name": name + ".c", "classification": classification,
                "coverage": coverage, "origin": "focused",
                "source": textwrap.dedent(source).strip() + "\n"}
        if note:
            case["note"] = note
        cases.append(case)

    def expr(name, expression, coverage, classification="valid"):
        add(name, expression_source(expression), coverage, classification)

    # Each unparenthesized chain exposes the reducer's tree shape.
    expressions = [
        ("primary-constants", "1 + 2u + 3L + 4LL + 'a' + 0x1p2 + 1.5f", ["primary", "constants"]),
        ("primary-strings", '("ab" "cd")[a] + (L"wide")[0]', ["primary", "strings", "subscript"]),
        ("primary-grouping", "(a) + ((b + c))", ["primary", "parentheses"]),
        ("postfix-chain", "factory()(a)[b].next->x++", ["postfix", "call", "member", "subscript"]),
        ("postfix-arguments", "call(call(), a + b, (a, b), a ? b : c)", ["call", "argument-boundaries"]),
        ("postfix-increments", "a++ + b--", ["postfix", "increment", "decrement"]),
        ("unary-operators", "++a + --b + &a - *p + +a - -b + ~c + !a", ["unary", "all-operators"]),
        ("unary-nesting", "!~-(int)*p + *&a + ++*p", ["unary", "cast", "prefix-order"]),
        ("multiplicative", "a * b / c % 3 * a", ["multiplicative", "left-associativity"]),
        ("additive", "a - b + c - a", ["additive", "left-associativity"]),
        ("shift", "a << b >> c << 1", ["shift", "left-associativity"]),
        ("relational", "a < b <= c > a >= b", ["relational", "left-associativity"]),
        ("equality", "a == b != c == a", ["equality", "left-associativity"]),
        ("bitwise-and", "a & b & c", ["bitwise-and", "left-associativity"]),
        ("bitwise-xor", "a ^ b ^ c", ["bitwise-xor", "left-associativity"]),
        ("bitwise-or", "a | b | c", ["bitwise-or", "left-associativity"]),
        ("logical-and", "a && b && c", ["logical-and", "left-associativity"]),
        ("logical-or", "a || b || c", ["logical-or", "left-associativity"]),
        ("precedence-ladder", "a || b && c | a ^ b & c == a < b << c + a * b", ["precedence", "all-binary-levels"]),
        ("precedence-parenthesized", "(a || b) && (c | a) ^ ((b & c) == a) < ((b << c) + a) * b", ["precedence", "parentheses"]),
        ("conditional-right", "a ? b : c ? a : b", ["conditional", "right-associativity"]),
        ("conditional-middle", "a ? b = c, a += b : c", ["conditional", "middle-assignment", "middle-comma"]),
        ("conditional-middle-nested", "a ? b ? c : a, b : c", ["conditional", "middle-nesting", "colon-ownership"]),
        ("conditional-assignment-right", "a = b ? c : a ? b : c", ["assignment", "conditional", "right-operand"]),
        ("conditional-grouped-right", "a ? b : (c = a, b)", ["conditional", "grouped-right-assignment"]),
        ("assignment-chain", "a = b = c = 1", ["assignment", "right-associativity"]),
        ("assignment-compound", "a += b -= c *= a /= b %= c <<= a >>= b &= c ^= a |= b", ["assignment", "all-operators", "right-associativity"]),
        ("assignment-grouped-lhs", "(a) = (*p) = b", ["assignment", "unary-left-operand"]),
        ("comma-chain", "a = b, b += c, c ? a : b, c", ["comma", "left-associativity", "assignment"]),
        ("cast-chain", "(int)(unsigned long)(T)a + (const int *)p[0]", ["cast", "typedef", "qualifiers"]),
        ("cast-function-pointer", "((int (*)(int))p)(a)", ["cast", "abstract-declarator", "function-pointer", "call"]),
        ("cast-array-pointer", "((int (*)[3])p)[a][b]", ["cast", "abstract-declarator", "array-pointer"]),
        ("sizeof-forms", "sizeof a + sizeof(a + b) + sizeof(T) + sizeof(int [3]) + sizeof(int (*)(T))", ["sizeof", "type-name", "abstract-declarator"]),
        ("sizeof-binding", "sizeof *p++ + sizeof a + b * sizeof(T)", ["sizeof", "precedence", "postfix"]),
        ("sizeof-compound-literal", "sizeof (T){1} + sizeof (int[2]){1, 2}", ["sizeof", "compound-literal", "ambiguity"]),
        ("compound-literal-member", "(struct S){.x = a, .y = b}.x", ["compound-literal", "postfix", "designation"]),
        ("compound-literal-array", "(int[]){a, b, c,}[1]", ["compound-literal", "array", "trailing-comma"]),
        ("compound-literal-nested", "((struct S){.x = (T){1}, .y = 2}).y", ["compound-literal", "typedef", "nested-initializer"]),
    ]
    for name, expression, coverage in expressions:
        expr("expr-" + name, expression, coverage)

    declarations = [
        ("decl-function-pointers", "typedef int (*F)(int); F table[2]; int (*f(int x, int (*cb)(int)))(int) { return cb; }", ["declarator", "function-pointer", "parameter"]),
        ("decl-abstract-types", "int f(void) { return sizeof(int (*[2])(int)) + sizeof(int (*const)[3]); }", ["type-name", "abstract-declarator"]),
        ("decl-array-parameters", "int f(int a[static const 3], int b[restrict 4]) { return a[0] + b[1]; } int g(int a[*]);", ["declarator", "array-parameter", "prototype-scope"]),
        ("decl-aggregate", "struct S { unsigned x:3; unsigned :0; int a[2]; }; union U { int x; double d; }; enum E { A, B = 1 ? 2 : 3, C, };", ["struct", "union", "enum", "constant-expression"]),
        ("init-designator-chain", "struct S { int a[3]; }; struct S s[2] = { [1].a[2] = 7, [0] = { .a = {1, 2, 3} }, };", ["initializer", "designation", "chained-designators"]),
        ("init-conditional-designator", "int a[4] = { [1 ? 2 : 3] = 5, [(0, 1)] = 7 };", ["initializer", "constant-expression", "conditional", "comma"]),
        ("init-nested-braces", "int a[2][2] = { {1, 2}, [1] = { [0] = 3, 4 }, }; int b = {1};", ["initializer", "nested", "scalar-braces"]),
        ("typedef-cast-group", "typedef int T; int f(int x) { T y = (T)x; { int T = 2; y += (T) * x + sizeof(T); } return (T)y; }", ["typedef", "cast-group-ambiguity", "block-scope"]),
        ("typedef-own-initializer", "typedef int T; int f(void) { int T = sizeof(T), x = (T) + 1; return x; } T after;", ["typedef", "declaration-point", "initializer", "scope-restore"]),
        ("typedef-parameter", "typedef int T; int f(int T) { return (T) + sizeof(T); } T after;", ["typedef", "parameter-scope", "scope-restore"]),
        ("typedef-prototype", "typedef int T; int f(int T, int a[sizeof(T)]); T after;", ["typedef", "prototype-scope", "declaration-point"]),
        ("typedef-for-scope", "typedef int T; int f(void) { for (int T = 0; T < 2; ++T) T += sizeof(T); T after = 1; return (T)after; }", ["typedef", "for-declaration", "implicit-scope", "scope-restore"]),
        ("typedef-nested-restore", "typedef int T; int f(void) { { int T; { typedef long T; T x = 1; } T = 2; } T x = 3; return x; }", ["typedef", "nested-scopes", "scope-restore"]),
        ("typedef-member-label", "typedef int T; struct S { int T; }; int f(struct S *p) { goto T; T: return p->T; }", ["typedef", "member-namespace", "label-namespace"]),
        ("typedef-declarator-list", "typedef int T, (*F)(T); F f; T x, *p, a[sizeof(T)];", ["typedef", "declaration-point", "declarator-list"]),
        ("typedef-function-return-scope", "typedef int T; int (*f(int x))(int T) { T value; return 0; } T after;", ["typedef", "nested-prototype-scope", "function-definition"]),
        ("decl-old-style", "int f(a, b) int a; char *b; { return a + b[0]; }", ["function-definition", "old-style-parameters"]),
    ]
    for name, source, coverage in declarations:
        add(name, source, coverage)

    statements = [
        ("stmt-null-mixed", "int f(int a) { ; a++; int b = a; { ; b--; } return b; }", ["statement", "null", "mixed-block-items", "compound"]),
        ("stmt-dangling-else", "int f(int a, int b) { if (a) if (b) a++; else b++; return a; }", ["if", "dangling-else"]),
        ("stmt-braced-else", "int f(int a, int b) { if (a) { if (b) a++; } else b++; return a; }", ["if", "else", "compound"]),
        ("stmt-else-if", "int f(int a) { if (a) a++; else if (a + 1) a--; else ; return a; }", ["if", "else-if", "null"]),
        ("stmt-while", "int f(int a) { while (a--) { if (a) continue; break; } return a; }", ["while", "break", "continue"]),
        ("stmt-do-while", "int f(int a) { do if (a) a--; else break; while (a); return a; }", ["do-while", "if", "semicolon-ownership"]),
        ("stmt-for-empty", "void f(void) { for (;;) { break; } for (; 0;) ; }", ["for", "absent-slots", "null"]),
        ("stmt-for-expression", "int f(int a, int b) { for (a = 0, b = 2; a < b; a++, b--) ; return a; }", ["for", "expression-slots", "comma"]),
        ("stmt-for-declaration", "int f(void) { for (int i = 0, j = 3; i < j; ++i) { int j = i; if (j) continue; } return 0; }", ["for", "declaration", "block-scope"]),
        ("stmt-labels-goto", "void f(int a) { goto done; first: second: a++; if (a) goto first; done: return; }", ["label", "goto", "return-empty"]),
        ("stmt-switch", "int f(int a) { switch (a) { case 0: case 1: a++; break; default: a--; } return a; }", ["switch", "case", "default", "break"]),
        ("stmt-switch-nested", "int f(int a) { switch (a) { default: switch (a + 1) { default: break; } break; } return a; }", ["switch", "switch-scope", "default"]),
        ("stmt-case-conditional", "int f(int a) { switch (a) { case 1 ? 2 : 3: return a ? 1 : 0; default: return 0; } }", ["case", "conditional", "colon-ownership"]),
        ("stmt-switch-in-loop", "int f(int a) { for (; a; --a) switch (a) { case 1: continue; default: break; } return a; }", ["switch", "loop-scope", "continue", "break"]),
        ("stmt-selection-typedef", "typedef int T; int f(int a) { if (sizeof(enum { T = 1 })) a = T; else a = T; T after; return a; }", ["if", "implicit-scope", "typedef", "scope-restore"]),
        ("stmt-iteration-typedef", "typedef int T; int f(void) { while (sizeof(enum { T = 1 })) { int x = T; break; } T after; return 0; }", ["while", "implicit-scope", "typedef", "scope-restore"]),
    ]
    for name, source, coverage in statements:
        add(name, source, coverage)

    extensions = [
        ("ext-alignof-types", "_Alignof(T) + _Alignof(int (*)[3])", ["alignof", "type-name", "c11"]),
        ("ext-alignof-expressions", "__alignof__(*p++) + __alignof(a + b)", ["alignof", "unary-operand", "gnu"]),
        ("ext-omitted-conditional", "a ?: b ?: c", ["conditional", "omitted-middle", "gnu"]),
        ("ext-omitted-conditional-assignment", "a = b ?: c ? a : b", ["conditional", "omitted-middle", "assignment"]),
        ("ext-statement-expression", "({ int x = a; x += b; x; }) + c", ["statement-expression", "compound", "gnu"]),
        ("ext-statement-expression-nested", "({ typedef int U; U x = ({ int y = a; y; }); x; })", ["statement-expression", "typedef", "nested"]),
        ("ext-extension-marker", "__extension__ ({ int x = a; x; }) ?: b", ["extension-marker", "statement-expression", "conditional"]),
        ("ext-builtin-va-arg", "__builtin_va_arg(p, int (*)(int))", ["builtin", "va-arg", "type-operand"]),
        ("ext-builtin-offsetof", "__builtin_offsetof(struct S, items[1].x)", ["builtin", "offsetof", "member-designator"]),
        ("ext-builtin-types", "__builtin_types_compatible_p(T, int) + __builtin_types_compatible_p(int (*)(int), int (*)(int))", ["builtin", "types-compatible", "abstract-declarator"]),
        ("ext-builtin-choose", "__builtin_choose_expr(1, (a, b), __builtin_choose_expr(0, b, c))", ["builtin", "choose-expr", "argument-boundaries"]),
        ("ext-builtin-call", "__builtin_expect(a, 1) + __builtin_constant_p(b)", ["builtin", "ordinary-call"]),
        ("ext-complex-unary", "__real__ a + __imag__ b", ["unary", "complex", "gnu"]),
        ("ext-generic", "_Generic(a, int: b, default: c)", ["generic-selection", "colon-ownership", "c11"]),
    ]
    for name, expression, coverage in extensions:
        expr(name, expression, coverage, "extension")
    for name, source, coverage in [
        ("ext-range-designator", "int a[4] = { [1 ... 3] = 2 };", ["initializer", "range-designator", "gnu"]),
        ("ext-obsolete-designators", "struct S { int x; }; struct S s = { x: 1 }; int a[3] = { [1] 2 };", ["initializer", "gnu-field-designator", "omitted-equals"]),
        ("ext-case-range", "int f(int x) { switch (x) { case 1 ... 3: return 1; default: return 0; } }", ["switch", "case-range", "gnu"]),
        ("ext-local-label-computed-goto", "int f(void) { __label__ L; void *p = &&L; goto *p; L: return 0; }", ["local-label", "label-address", "computed-goto"]),
        ("ext-nested-function", "typedef int T; int f(void) { int nested(int T) { return T; } T x = 1; return nested(x); }", ["nested-function", "typedef", "function-scope"]),
        ("ext-asm", 'int f(int x) { __asm__ __volatile__("" : [out] "=r"(x) : "r"(x) : "memory"); __asm__ goto("" : : : : L); L: return x; }', ["asm", "expression-operands", "asm-goto"]),
        ("ext-typeof", "int x; __typeof__(x) y; __typeof__(int (*)(int)) p;", ["typeof", "expression-or-type", "abstract-declarator"]),
    ]:
        add(name, source, coverage, "extension")

    # Broken productions, never merely undeclared names/type/lvalue constraints.
    malformed = [
        ("missing-operand", "a + ;", ["expression", "semicolon"]),
        ("stray-operand-group", "a = (a b); a = 1;", ["expression", "parenthesis", "recovery"]),
        ("missing-group-close", "a = (b + c; a = 1;", ["expression", "missing-closer"]),
        ("mismatched-close", "a = (b + c]; a = 1;", ["expression", "mismatched-closer"]),
        ("missing-call-comma", "call(a b, c); a = 1;", ["call", "separator"]),
        ("empty-call-argument", "call(a,,c); a = 1;", ["call", "missing-argument"]),
        ("missing-call-close", "call(a, b; a = 1;", ["call", "missing-closer"]),
        ("empty-subscript", "p[]; a = 1;", ["subscript", "missing-expression"]),
        ("missing-subscript-close", "p[a; a = 1;", ["subscript", "missing-closer"]),
        ("missing-member", "p->; a = 1;", ["member", "missing-identifier"]),
        ("conditional-colon", "a ? b; a = 1;", ["conditional", "missing-colon"]),
        ("conditional-right", "a ? b : ; a = 1;", ["conditional", "missing-right"]),
        ("conditional-middle-comma", "a ? b, : c; a = 1;", ["conditional", "middle-comma", "missing-operand"]),
        ("assignment-right", "a = ; a = 1;", ["assignment", "missing-right"]),
        ("assignment-nonunary-left", "a + b = c; a = 1;", ["assignment", "unary-left-grammar"]),
        ("sizeof-type-close", "sizeof(int; a = 1;", ["sizeof", "type-name", "missing-closer"]),
        ("cast-operand", "(int); a = 1;", ["cast", "missing-operand"]),
        ("initializer-empty-value", "int x = ; a = 1;", ["initializer", "missing-value"]),
        ("designation-equals", "int x[2] = { [0] = }; a = 1;", ["initializer", "designation", "missing-value"]),
        ("designation-bracket", "int x[2] = { [0 = 1 }; a = 1;", ["initializer", "designation", "missing-closer"]),
        ("initializer-close", "int x[2] = { 1, 2; a = 1;", ["initializer", "semicolon", "missing-closer"]),
        ("compound-literal-close", "(int[]){1, 2; a = 1;", ["compound-literal", "initializer", "missing-closer"]),
        ("statement-expression-close", "({ a++; a; }; a = 1;", ["statement-expression", "missing-closer", "semicolon"]),
        ("if-empty-condition", "if () a++; else a--;", ["if", "missing-condition", "else"]),
        ("if-stray-operand", "if (a b) { a++; } else a--;", ["if", "header-boundary", "else"]),
        ("if-missing-body", "if (a) else a--;", ["if", "missing-body", "else"]),
        ("while-header-brace", "while (a b { a--; } a = 1;", ["while", "header-boundary", "body-brace"]),
        ("do-missing-while", "do a--; a = 1;", ["do-while", "missing-while", "next-statement"]),
        ("for-extra-clause", "for (a; b; c; a) a--;", ["for", "extra-separator"]),
        ("for-declaration-init", "for (int x = ; x; ++x) { a++; }", ["for", "declaration", "header-boundary"]),
        ("switch-condition", "switch (a b) { default: a--; }", ["switch", "header-boundary"]),
        ("case-colon", "switch (a) { case 1 a++; default: break; }", ["case", "missing-colon"]),
        ("goto-target", "goto ; a = 1;", ["goto", "missing-identifier"]),
        ("stray-else", "else a++; a = 1;", ["else", "recovery"]),
        ("builtin-argument", "__builtin_choose_expr(1,,3); a = 1;", ["builtin", "missing-operand"]),
        ("builtin-type", "__builtin_va_arg(p,); a = 1;", ["builtin", "missing-type"]),
        ("alignof-type", "_Alignof(); a = 1;", ["alignof", "missing-type"]),
        ("typedef-restoration", "{ int T; a = (T + ); } T restored;", ["typedef", "recovery", "scope-restore"]),
    ]
    for name, body, coverage in malformed:
        add("bad-" + name, "typedef int T;\nint f(int a, int b, int c, int *p) {\n  "
            + body + "\n  return a;\n}\nint after;", coverage, "malformed")
    for name, source, coverage in [
        ("bad-eof-expression", "int f(void) { return (1 +", ["eof", "expression", "scope-unwind"]),
        ("bad-eof-statement", "typedef int T; int f(void) { for (int T = 0; T; ++T)", ["eof", "for", "typedef", "scope-unwind"]),
        ("bad-eof-initializer", "int a[2] = { [1] =", ["eof", "initializer", "designation"]),
        ("bad-eof-declarator", "int (*f(int a", ["eof", "declarator", "parameter"]),
    ]:
        add(name, source, coverage, "malformed")
    return cases


def expression_source(expression):
    # Deliberately parser-only: the postfix stress chain need not type check.
    return ("typedef int T;\nstruct S { int x, y; struct S *next; };\n"
            "int f(int a, int b, int c, int *p) {\n  return " + expression + ";\n}\n")


def fuzz_cases(seed, count):
    """Bounded grammar generation; every assignment target is an identifier.

    Parenthesized children keep comma/assignment out of restricted operand
    positions. Types, lvalues, evaluation order, and constantness are not checked.
    """
    rng = random.Random(seed)

    def expression(depth):
        if depth == 0:
            return rng.choice(["a", "b", "c", "1", "2", "sizeof(T)", "p[0]"])
        kind = rng.randrange(8)
        child = lambda: expression(depth - 1)
        if kind == 0:
            return rng.choice(["+", "-", "!", "~"]) + "(" + child() + ")"
        if kind == 1:
            operator = rng.choice(["*", "/", "%", "+", "-", "<<", ">>", "<", "<=",
                                   ">", ">=", "==", "!=", "&", "^", "|", "&&", "||"])
            return f"({child()}) {operator} ({child()})"
        if kind == 2:
            return f"({child()}) ? ({child()}), ({child()}) : ({child()})"
        if kind == 3:
            return f"a {rng.choice(['=', '+=', '-=', '*=', '^='])} ({child()})"
        if kind == 4:
            return f"({child()}), ({child()})"
        if kind == 5:
            return f"(T)({child()})"
        if kind == 6:
            return f"call(({child()}), ({child()}))"
        return f"(int[]){{({child()}), ({child()})}}[0]"

    return [{"name": f"fuzz-expression-{index:03d}.c", "classification": "valid",
             "coverage": ["seeded-grammar", "expression", "nested-boundaries"],
             "origin": "seeded-grammar", "depth": 2 + index % 3,
             "source": expression_source(expression(2 + index % 3))}
            for index in range(count)]


if __name__ == "__main__":
    main()
