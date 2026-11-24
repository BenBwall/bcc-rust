# Diagnostic golden corpus

TCC 0.9.27's warnings (2026-11-22): the `*-tcc-*` fixtures run with the
options `--tcc-mode` passes bcc (`.args`) and match Ubuntu's TCC 0.9.27
(`tcc -c`) byte for byte, exit status included.
`sema-tcc-storage-redefinition` pins `static storage ignored for
redefinition` and `storage mismatch for redefinition`, `extern` after
`static` and a block's `extern` included, and an `inline` one left silent;
`sema-tcc-werror` pins `-Werror` stopping at the first as an error, and
`sema-tcc-werror-phases` the first by place, a semantic warning before a
parser's. `sema-tcc-mode-attribute` pins `__mode__(X) not supported` with
its empty line, `mode` ignored, a known mode applied among the specifiers
only, where a later type specifier replaces it, and `__word__` as `int`
(checked by `_Static_assert`). `pp-tcc-integer-overflow`
pins `integer constant overflow` and the wrapped values and types.
`sema-tcc-empty-declarations`, `sema-tcc-empty-declaration-error` and
`sema-tcc-empty-typedef-error` pin `decl0`'s warning and `identifier
expected`, and silence for a `section` on a typedef, a member or a local.
`sema-tcc-return-flow` pins `function might return no value` after `exit`,
`abort` and `longjmp` declared without attributes, after a statement
expression's `return`, and for deferred `inline` functions after the unit,
at the line before the closing brace (`sema-tcc-return-flow.h:0`), in
TCC's order of generation. `parser-tcc-label-at-block-end` pins
`deprecated use of label at end of compound statement` at each `}`;
`sema-tcc-warning-order` pins one line's warnings in TCC's order; and
`pp-tcc-wording` pins `tccpp.c`'s words and the multi-character constants
of `#if` and wide constants, with their values.

`cl`'s numbered warnings and errors (2026-11-21):
`sema-cl-numbered-warnings-msvc` (the `cl` identity at level 4) pins C4100,
C4101 and C4189 at the end of the function, the parameters and outermost
block first, with neither a variable only assigned nor a `static` one; C4018
and C4389 for operands of one width and other signs, a promoted `short`
included and a constant left alone; C4706 with `cl`'s note, and none for an
assignment in parentheses; C4293 and no warning for `1 << 31`; and a
`#pragma warning(push, 3)` region that hides C4100 but not C4101.
`sema-cl-numbered-errors-msvc` pins C2084 and C2039 with `cl`'s notes,
C2100, C2197 at the first extra argument, C2198, C2063 at the name and C2106
at a constant. Both match `cl` 19.44 under Wine (`/W4 /diagnostics:column`,
`/diagnostics:column`) byte for byte. The code generator's C4715 and C4716,
which name the file by its full path, are pinned by
`tests/integration/personality_cli.rs` instead.
`sema-cl-pragma-suppress-msvc` pins `#pragma warning(suppress: N)` on the
pragma's line and the next (a blank line counts, a `__pragma` covers its own
line, a `suppress` on a line another reaches extends both) and `once`.
`sema-cl-seh-uses-msvc` pins names that only `__try`, `__except` and
`__finally` use as used, and C4018, C4706 and C4293 inside them.
`sema-cl-compound-shift-msvc` pins C4293 for `<<` and `>>` but not `<<=` and
`>>=`. These three match `cl` 19.44 (`/diagnostics:column`) byte for byte.
`sema-seh-jumps-msvc` pins C2705 for a `goto` or `case` label into a `__try`,
`__except` or `__finally` block (once per label, the innermost block, `'null'`
for a `case`), C2703 for a `__leave` outside every `__try` block, and C4532
at a label a `goto` leaves a `__finally` block for, at `break` and `continue`
(`'identifier'` when its loop is a do-while), and not at `return`, with no
C2705 for a jump to a label shallower in SEH blocks (a handler one level
inside its `__try` block), wherever it lands, and C2705 for one to a label as
deep in another innermost block, and no C4532 for a `goto` from a `__try`
block inside the `__finally` block;
`sema-cl-pragma-suppress-push-msvc` pins a `suppress`
whose next line is a `push` lasting until that `push`'s `pop`. Both match `cl`
19.44 (`/W4 /diagnostics:column`) byte for byte.
`cl`'s conversion and constant-condition warnings (2026-11-22):
`sema-cl-conversions-msvc` pins C4244 at its three levels (an integer made a
narrower type when its value is wider, a floating value made an integer or
a narrower floating type, an `int` made `float`), C4305 for constants that
do not fit, and C4245 for a negative one made unsigned, in each context
(`initializing`, `=`, `+=`, `function`, `return`), with types named as
written (`Byte`, `const int`, `U32`), none for values `cl` measures as
narrow enough (`ch + 1`, `i % 10`, `u >> 16`, `u & 0xff`) or for `|=`, and
an automatic object's initializers after the function's other warnings.
`sema-cl-constant-condition-msvc` pins C4127 and its exemptions: a literal
0 or 1 under parentheses, unary operators, a cast to its own type, a macro or
the operand a constant `?:` selects; none for an address.
`sema-cl-narrowing-wall-msvc` pins `/Wall`'s C4242 (`--cl-warning=4242:4`)
in place of C4244 for an uncomputed or 64-bit value, also where a pragma
disables C4244. `sema-cl-folded-conditional-msvc` pins a `?:` whose
condition `cl` folds (an integer, null pointer or floating constant, not a
comma): C4244 measured and reported at the selected operand in each
context, C4100 for a parameter named only in the other or behind `0 &&`,
but not under `sizeof`, the parameters last first, and C4127 at the
selected operand. `sema-cl-signed-constants-msvc` pins how wide C4244 takes
a constant: a 64-bit unsigned one of 2^63 or more read as negative (`~0ull`
is -1, so `flag ? ~0ull : 0` narrows silently), a negative one as its two's
complement (-32768 fits a `short`, -32769 does not), and a remainder by a
negative constant as its magnitude and a sign. The five match `cl` 19.44
(`/W4` or `/Wall`, `/diagnostics:column`) byte for byte. `sema-seh-jumps-clang` pins
Clang's error at each `goto`, last first at the end of its function, with a
note at each block entered, innermost first, the `case` and computed-goto
forms, `'__leave' statement not in __try block`, and `-Wjump-seh-finally` for
`goto`, `break`, `continue` and `return`, with the error for a jump from a
handler back into its own guarded block or into a later sibling's;
`sema-seh-reachability-clang` pins that Clang reports nothing about code that runs in an `__except` handler,
nothing from a filter's full-expression checks but its shift, `-Wreturn-type`
as if no handler ran, and, in a function with `__finally`, every such warning
and no `-Wreturn-type`; `sema-seh-filter-limits-clang` pins that a filter
draws none of the comparison checks Clang makes as it finishes a full
expression (`-Wtype-limits` and its kin) but keeps
`-Wtautological-pointer-compare`. All three match Clang 23.1.5 byte for byte.
`pragma-sign-compare-equality-gcc` and `-clang` pin that no pragma names the
group of `cl`'s C4389, as before it existed.

C23 directives before C23 (2026-11-15): `pp-c23-directives-gcc` (`-std=gnu17
-pedantic`) and `pp-c23-directives-strict-gcc` (`-std=c17 -pedantic`) pin GCC
14's reports of `#warning`, `#elifdef` and `#elifndef` and its invalid `#embed`;
`pp-c23-directives-clang` (`-std=gnu17`), `-clang-pedantic` (`-std=c17
-pedantic-errors -Wno-c23-extensions`) and `-clang-pedantic-errors` (`-std=c17
-pedantic-errors -Wno-pedantic`) pin Clang 23's default `-Wc23-extensions`
warnings, its `-Wpedantic` `#warning` and the options' severities. Each matches
its compiler byte for byte, including `#warning`'s message with `[-Wcpp]` or
`[-W#warnings]` and the snippet neither repeats for a second diagnostic at the
same place.

Separate problems at one place (2026-11-15): `pp-c23-directives-clang-werror`
(`-std=gnu17 -Werror`) pins an extension report that `-Werror` makes an error
printed beside the structural error in the same directive, an `#elifdef`
without `#if` and an `#embed` without a resource, as Clang 23 prints both
rather than folding them; only errors in the input's syntax fold. Clang's
`#elifdef without #if` matches byte for byte, and `pp-conditional-arms-clang`
pins the place-only snippet Clang draws for every unmatched conditional arm;
the `#embed` error keeps bcc's words and range (below).
`pp-c23-directives-gcc-werror` (`-std=gnu17 -pedantic -Werror`) pins GCC 14's
`[-Werror]` on its pedantic C23 directive reports, which `-pedantic-errors`
leaves off, and the `cc1: all warnings being treated as errors` line that ends
the diagnostics of a unit whose warnings `-Werror` made errors (`some` when a
`-Werror=` or a pragma did, as in `pragma-diagnostic-gcc`), and its report of an `#elifdef` only as an arm of an open
conditional, after an `after #else` error; GCC's words for the conditional
errors and its `the conditional began here` note remain bcc's (below).
`pp-c23-directives-msvc-identity` pins the base behaviour of the `cl` identity
with a pre-C23 `gnu` dialect: no report after a group it took, and one at the
name of an arm it takes.

Fix-it hints (2026-11-17): GCC and Clang print the text a hint suggests on
a line of its own under the source line it changes. `fixit-conversions-clang`
pins Clang's hints for a conversion it refuses: `&` or `*` before the value,
`&(` and `)` around a value that needs parentheses (a parenthesized one
included, as Clang asks about the expression under its implicit
conversions), several hints on one line, the value's own `&` or `*` removed
with `; remove &` or `; remove *`, which draws no text, and none for a null
pointer, a value that is no lvalue, or a bit-field or vector element, whose
address cannot be taken (a `register` object still gets its `&`, as in
Clang); it matches Clang 23 byte for byte.
`fixit-macros-clang` pins hints at a token a macro expansion produced: one an
argument wrote stays where the invocation spells it, one a macro's body wrote
is dropped with every other hint of its diagnostic, and a hint after a tab is
placed by display column. It matches Clang 23 but for the `expanded from
macro` notes Clang adds for a token an argument wrote (below).
`fixit-parseable-clang` (`-fdiagnostics-parseable-fixits
-fno-diagnostics-fixit-info`) and `fixit-parseable-lines-clang`
(`-fno-diagnostics-show-line-numbers`) pin Clang's `fix-it:` lines after the
snippet, with byte columns, a removal's empty text, and the fix-it line hidden
while the replaced range is still underlined; both match Clang 23 byte for
byte. `fixit-parseable-gcc` and `fixit-parseable-no-caret-gcc`
(`-fno-diagnostics-show-caret`) pin GCC's, written after each diagnostic
whether or not it quotes source, with byte columns and, after `#line`, the
presumed file name and line; GCC drops a hint at any token a macro expansion
produced. They match GCC 14 but for its `in definition of macro` notes,
which bcc does not write for a token an argument wrote. GCC's `-` row under a
replaced range its caret line did not draw, and its joining of hints whose
text would overlap, are pinned by unit tests in `src/diagnostics/tests.rs`,
since no diagnostic bcc reports in GCC's format suggests a replacement.

Context lines (2026-11-08): `context-includes-gcc` and `context-includes-clang`
pin the include stacks GCC 14 and Clang 23 print before a diagnostic in a
header: headers nested two deep through `"…"` and through `<…>` with `-I`, a
header read twice, and a function a header defines. GCC writes each
`#include` once in a compilation and only the part of a chain it has not
written; Clang writes the whole chain whenever the including place changes,
outermost first, and names a header found beside an includer in the working
directory `./name.h`. `context-repeat-readings-clang` pins the stacks of
headers read more than once with a diagnostic in a later reading only, or in
each: a diagnostic stands under the `#include` of the reading it was found
in, and the same diagnostic from two readings is reported twice, as Clang 23
reports it; `context-repeat-escape-clang` does so for an invalid escape that
`#if` converts in each of three readings. `context-functions-gcc` pins GCC's `In function 'f':`
and `At top level:` lines, which come when the function changes: a
diagnostic just past a definition's name, in its parameters or body, is in
the function, and one at the name (`-Wmain`, an unused function) at top level.
`context-functions-clang` shows that Clang names no function. Both outputs
match the compilers' line for line but for GCC's range for an unused
parameter. `context-includes-clang-cl` matches clang-cl 23 exactly: Clang's
include stacks, which keep `file:line`, and macro notes, with each
diagnostic at `cl`'s `file(line,col):` and no function lines. The other GCC-format snapshots gained these lines where GCC
prints them. `context-ranges-clang` pins the ranges Clang underlines with
`~` beside the caret: an initializer's and an assignment's value, both
operands of an invalid binary operator or of a pointer and integer
comparison, the count or divisor of a shift or division warning, and the
operands of a `-Wsign-compare` comparison (`sema-sign-compare-clang`). The
caret of such a diagnostic is then a point, not an underlined token
(`int *pointer_name = value;` -> `^              ~~~~~`), as it is for a
warning Clang gives no range at all: an unused parameter, a variable set
but not used, an unused local typedef, and a shadowing declaration with
its note (`sema-unused-variables-clang`, `sema-shadow-kinds-clang`). The
`-Wparentheses` family, the `switch` warnings and `-Wformat` have Clang's
ranges too (`sema-parentheses-clang`, `sema-switch-clang`,
`sema-format-ranges-clang`). A range whose end a macro's replacement list
wrote takes that end of the whole invocation, so `Min(a, b)` is underlined
from its name to its `)`, and a range over several lines runs to the ends
of its lines, as Clang draws them (`context-macro-ranges-clang`, which
matches Clang 23 but for its fix-it notes and an argument's range in an
`expanded from macro` note: Clang underlines there the parameter the
argument replaced, which bcc does not record). A `-Wformat` specification
in a concatenated string takes the place of each of its ends on its own: a
token a macro wrote is drawn as its invocation, a token the file wrote stays
in place, a specification split between two arguments, as `JOIN("%", "d")`,
or two uses of one is drawn as the invocation, and one split over lines is
drawn on each of them. An argument that is a `##` operand keeps its place
in the tokens pasting leaves as they are, on either side of the `##` and in
a chain, so `CAT("%" "d",)` is drawn where the invocation spells it, as is
the single string a placemarker leaves on either side, as `CAT("%d",)`
and `VR1(, "%d", 1)` (`__VA_OPT__(a) ## x`), while a token pasting forms
from two tokens is drawn as the invocation, even one within an argument,
as `u8"%d"` in `CAT(u8, "%d")`, or across an empty middle operand, as in
`CAT3(u8, , "%d")`. A `__VA_OPT__` replacement that
is a `##` operand keeps the place of each argument in it, so
`VL(, "%", "d", 1)` (`x ## __VA_OPT__(a b)`) is drawn as the invocation on
either side of the `##`, in a chain, with `__VA_ARGS__` or a paste inside
it, and when a call collected across it leaves its tokens. The rest of an
argument that holds the arguments of a macro named before it, as `"%" "d"`
in `CALL(NIL, () "%" "d")` (`f a`), keeps its place too
(`sema-format-concat-ranges-clang`, which matches
Clang 23 but for its fix-it lines). Only Clang's format places a warning at
the token a macro wrote; under the Clang identity the other formats keep it
at the string (`sema-format-macro-places-clang-bcc`,
`sema-format-macro-places-clang-gcc`). A string a macro invoked in another
macro's argument wrote, as `FMT` in `WRAP(FMT)`, is drawn as the outer
invocation, and a string from `#` whole; a specification split between the
arguments of a macro invoked in an argument, as `ID(JOIN("%", "d"))`, stays
in place, where Clang draws the outer invocation, as the inner macro's
arguments are replayed through one substitution of the outer one; so does
one split between two arguments that a list passes on as one argument of
another macro, which takes them as one argument of its own: directly, as
`OUTER("%", "d")` (`ID(a b)`); after a parameter that names the macro,
as `CALLJ(ID, "%", "d")` (`f (a b)`) and `OF(ID, "%", "d", 1)`
(`__VA_OPT__(f) (a b)`); and through a `##` operand, as
`ON(, "%", "d", 1)` (`ID(x ## __VA_OPT__(a b))`) and `OC(, ID, "%", "d",
1)` (`x ## __VA_OPT__(f (a b))`). A diagnostic whose caret names its
operand's range, as `-Wpointer-sign` on `J(u8"%", "d")` (`a b`) under
`-std=gnu2x`, keeps the caret and that range where the tokens are spelled,
where Clang moves them to the invocation as it does other ranges.
`context-ranges-clang`'s caret and fix-it lines match Clang 23's; its messages keep bcc's words for the operator errors. `context-macros-gcc` and
`context-macros-clang` pin the notes about a token a macro's body wrote:
GCC reports the diagnostic in the body and adds `in expansion of macro
'X'` at the invocation, Clang reports it at the invocation and adds
`expanded from macro 'X'` at the body, a continued definition included,
where Clang draws the ranges the diagnostic names that the expansion wrote,
on the lines next to its caret. A
macro a header defines (`context-headers/macro.h`) gets GCC's include stack
of the header under the function line of its invocation. A warning a
system header's macro wrote stays at the user's invocation under GCC's
format. bcc keeps only the outermost invocation of a token, so a
nested expansion gets GCC's note for the outermost macro and Clang's for
the innermost, not the levels between; a token from an argument gets
neither GCC's `in definition of macro` note nor Clang's notes.

Diagnostic pragmas: `pragma-diagnostic-{gcc,clang}` pin the warning of each phase
under `#pragma GCC diagnostic` and `#pragma clang diagnostic`, a macro's body and
invocation under different settings, `_Pragma` in an expansion, and a header read
twice; `pragma-diagnostic-system-*` system headers; `pragma-diagnostic-malformed-*` each compiler's complaints, in its
own words; `pragma-diagnostic-fatal-clang` Clang's `fatal`; `pragma-warning-clang-cl`
`#pragma warning`. Lines and severities were read from GCC 14.2 and Clang 23.1,
except the reported line of a warning in a macro's body, which GCC puts in the
body. `tests/integration/diagnostic_pragmas.rs` holds the option combinations,
lowering's warning, `cl` and the `-E` round trip.

Pragmas over `-Werror` (2026-11-21): `pragma-werror-gcc` (`-std=gnu17 -pedantic
-Werror`) and `pragma-werror-pedantic-gcc` (`-Werror=pedantic`) pin the
`[-Werror=pedantic]` GCC 14 keeps naming, with its `cc1:` line, after a pragma
naming another option, a `push` and `pop`, and a `push` and `pop` around one
naming `-Wpedantic`, and the warning a later pragma naming `-Wpedantic` makes.
`werror-extra-gcc` (`-Werror=extra -Wno-error=type-limits`) pins the errors
GCC's `-Werror=extra` makes of the options `-Wextra` holds, and
`werror-extra-clang` (`-Werror=extra -Wno-error=unused-parameter`) Clang 23's.
All four match their compiler byte for byte. `diagnostic_pragmas.rs` and
`personality_cli.rs` hold the orders of the options, both identities, and the
parser's reports, whose words remain bcc's.

Issues #49/#54 (2026-10-16): extension headlines now retain `[-Wpedantic]`,
and the predefined-name fixture retains
`[-Wpredefined-identifier-outside-function]`. Unknown STDC pragmas use the
`unknown-pragmas` warning group. Pointer assignment and root initializer labels
identify the operator and declarator; macro diagnostics identify the invocation.
The Clang comparison in `tests/integration/diagnostic_parity.rs` checks those positions,
C23 nullptr constraints, default warning groups and suppress/promote controls,
K&R calls, C89 predefined names, embedded directives, GNU subtraction and CLI
warning-option behavior. Semantic tests retain nullptr/nonnull facts and typedef
spellings; pipeline tests replay #49 with relocation, interpreter and native
execution checks. Snapshot updates are reviewed against those intended changes.

Issue #46 (2026-10-14): `sema-pointer-warnings` pins distinct object comparisons,
pointer signedness, immediate/nested qualifiers, pointer/integer comparisons
and ordered function-pointer comparisons with Clang group names. Ordering
distinct function pointer types emits both comparison groups independently.
`sema-pointer-errors` retains default errors for all four object conversion
contexts, incompatible function signatures, nested signedness and both
integer/pointer directions. `sema-pointer-werror` promotes accepted constraints;
`sema-pointer-demote` exercises explicit default-error demotion, including the
incompatible-pointer parent group. `sema-pointer-pedantic` pins pedantic-only
function/void conversion and comparison and pointer/zero ordering. Every fixture
preserves a following declaration. Semantic mode/target matrices, conversion
facts, unrelated type controls, CLI flag precedence and all five execution arms
are tested separately in `semantic_analysis/tests/pointers.rs` and
`tests/integration/pointer_diagnostics.rs`. The exact Clang oracle is in
`language-standards.md`; `-Wdiscarded-qualifiers` is not a Clang group.

PR #44 review fixes (2026-10-14): `sema-pointer-conditional` pins
`-Wpointer-type-mismatch` for conditional arms with incompatible targets
(signedness, function signatures, function/void and array-element
qualifiers) and `-Wconditional-type-mismatch` for a pointer/integer pair in
either order, beside the qualifier warning a merged `const int *` result
produces and a silent null arm. `sema-pointer-array-qualifiers` shows that
adding `const` through an array target is silent for parameters with
constant, variable and unknown bounds, for assignment and for comparison,
while dropping it warns and nested pointer levels keep Clang's exact-bound
rule. `sema-pointer-enum-sign` (Linux, unsigned `enum E`) and
`sema-pointer-enum-sign-msvc` (int-sized enums) pin enumeration pointer
signedness through the compatible type. `sema-pointer-atomics` gives atomic
builtin value operands the assignment groups; `sema-atomic-builtin` now shows
two of its operands as `-Wincompatible-pointer-types` and `-Wint-conversion`
errors, as Clang reports them. Every pointer-constraint note now also cites
§6.5.15p3. `sema-pointer-system-header` (`-isystem system-headers`, with
`system-headers/pointer-macros.h`) withholds every pointer group the header's
own functions commit, including the default errors, and reports the header's
macros expanded in the main file at the user's invocation. Each of these matches the bundled Clang's group and line on every
supported target.

Warnings that Clang does not report by default are off in bcc too, so their
fixtures enable them: `initial-missing-final-newline` (`-Wnewline-eof`),
`pp-undefined-identifier-in-preprocessor-expression` (`-Wundef`),
`pp-unknown-pragma-directive` (`-Wunknown-pragmas`), `sema3-static-unused`
(`-Wunused-function`) and both `pp-forced-*-promotion` fixtures (the bcc-only
`-Wconstant-promotion`). `pp-va-args-outside-variadic-macro` and both
`pp-line-directive-number-*` fixtures pass `-pedantic`, the only mode in which
Clang reports them.

Round-6 scalar builtin coverage: `sema-builtin-values.c` distinguishes constant
and value requirements for object-size mode, probability, alignment, and
prefetch read/write/locality from three incompatible-type controls.
`sema-builtin-designators.c` pins Clang's direct-call requirement for intrinsic
addresses and values through parentheses, choose-expr and generic selection.
Both retain a following ordinary declaration. The semantic tests check exact
diagnostic kinds and suppress duplicate value errors for incompatible types;
the shared selected-callee execution fixture also preserves user shadows.

GCC's non-local gotos (2026-10-27): `sema-builtin-setjmp.c` pins the
`__builtin_longjmp` value rule (an integer constant that is 1 after
conversion to `int`, as GCC and Clang require; GCC also accepts a converted
`1.0`) for a variable, 2, 0 and a floating constant, an integer buffer, and
wrong argument counts, and Clang's `-Wconstant-conversion` for
`4294967297LL`, which converts to 1. It runs under Clang's identity, whose
`void **` buffer refuses `int[5]` and `int (*)[5]`.

The jump buffer by identity (2026-11-23): `sema-builtin-jump-buffer-gcc`
and `-clang` pass the same buffers, `void *[5]`, `void **`, a null pointer,
`void *const[5]`, `const void **`, `char[64]`, `volatile char[64]`, `const
int *`, `struct S *`, `long[5]`, `int **`, a function, its address and an
integer, to `__builtin_longjmp` and `__builtin_setjmp`. GCC 14 types the
buffer `void *` and Clang 23.1.5 `void **`, and each identity follows its
compiler: every line and status matches GCC 14 (`-std=gnu17`) byte for byte,
and Clang 23.1.5 but for its spelling `'void *const[5]'`, which bcc spells
with a space, as for every array type.

GCC's discarded qualifiers (2026-11-23): `sema-discards-qualifiers-gcc`
pins GCC 14's words, `return`, `initialization`, `assignment` or `passing
argument N of 'f'` with `discards 'const' qualifier from pointer target
type [-Wdiscarded-qualifiers]`, naming the lost qualifiers in GCC's order
(`'const volatile'`, `'restrict'`), and its incompatible-pointer error where
a deeper level differs (`char **` from `const char **`), for an ordinary
function and `__builtin___clear_cache`, whose parameters are `void *` as in
GCC and Clang; and no `-Wnon-literal-null-conversion`, which only Clang has,
for `'\0'` or `1 - 1` as a pointer. Every warning and error line and the
status match GCC 14; GCC's `note: expected 'char *' but argument is of type
'const char *'` after an argument's diagnostic is not modelled, as for every
argument conversion.

GCC's discarded array qualifiers (2026-11-23):
`sema-discards-array-qualifiers-gcc` pins GCC 14's
`-Wdiscarded-array-qualifiers` for a qualifier lost from a pointed-to
array's elements, in `-Wdiscarded-qualifiers`' words at each site (return,
initialization, assignment, argument, to `void *` too, through a typedef, a
two-dimensional array and a lost `restrict`), with GCC's
`-Wno-discarded-qualifiers -Werror=discarded-qualifiers` in bcc's spelling
of that group, which leave it a warning; a pointed-to `const void` stays
`-Wdiscarded-qualifiers`, an error there.
`sema-discards-array-qualifiers-gcc-c23` pins the same warning as an error
under C23's `-pedantic-errors`. Every warning and error line and the status
match GCC 14 (`-std=c2x` for C23); GCC's `note: expected` after an
argument's diagnostic is not modelled.

GCC's pre-C23 pedantic array qualifiers (2026-11-24):
`sema-array-qualifiers-pedantic-gcc` pins GCC 14's `-pedantic-errors`
error `invalid use of pointers to arrays with different qualifiers in ISO C
before C23 [-Wpedantic]` under gnu17 wherever GCC compares such pointers'
targets: a return, an initialization and an assignment in either direction,
an argument, `==`, `<`, a subtraction and the arms of `?:`, each before the
`-Wdiscarded-array-qualifiers` warning at the same place, and none inside
`__extension__`. A qualified `void *` converted to a pointer to an array
whose elements carry its qualifiers (a return, an initialization and an
argument) is GCC's `-pedantic` `discards 'const volatile' qualifier`
error, naming all the `void`'s qualifiers, which a pragma ignoring
`-Wpedantic` does not silence. The value operands of `__atomic_store_n`,
`__atomic_exchange_n`, `__atomic_compare_exchange_n`,
`__sync_lock_test_and_set` and the `__sync` compare-and-swap builtins
get neither, as GCC converts them as by a cast (`sema-atomic-operands-gcc`
pins the other operand rules).
`sema-array-qualifiers-pedantic-warnings-gcc` pins the same reports as
warnings under plain `-pedantic`, which exits 0. Every warning and error
line and the status match GCC 14; carets over a cast or `&` operand span
it where GCC marks one column, and GCC's `note: expected` after an
argument is not modelled.

Type-generic bit builtins (2026-10-29): `sema-stdbit-gcc.c` and
`sema-stdbit-clang.c` pin each identity's operand rules for
`__builtin_popcountg` and its siblings: signed, `bool`, enumerated,
floating, pointer and structure operands, `ffsg` and `clrsbg` with unsigned
ones, a result for zero that is not an `int` after each compiler's
promotions, enumerated, floating and pointer rotation counts, wrong
argument counts, and operands and counts of an enumeration with a fixed
underlying type, which both compilers refuse where they refuse any
enumeration. Every error is on the line GCC 15 or Clang 23 reports;
`semantic_analysis/tests/bits.rs` checks the kinds.

Each top-level `.c` is run as `bcc-rust <file-name>` with this directory as the current directory. An optional sibling `.args` supplies whitespace-separated CLI flags before the filename. Its sibling `.stderr` stores the exact bytes, including the summary. `NO_COLOR=1` is set; `CLICOLOR_FORCE`, `CPATH`, and `C_INCLUDE_PATH` are removed. `RUST_BACKTRACE=0` keeps crash regressions from depending on an inherited backtrace setting. No paths, whitespace, source spellings, or panic output are normalized.

Run `cargo test --test integration diagnostics_golden`. Use `BLESS=1 cargo test --test integration diagnostics_golden` to rewrite every snapshot. The guard remains active while blessing and runs every C fixture again. Both tests accumulate failures so one mismatch does not hide later fixtures.

Snapshots record current behavior, including defects. The guard test rejects panics, internal representations, and raw NUL bytes in any output, so a crash or control-byte leak cannot be blessed into a passing snapshot. Fix the compiler instead of normalizing such output.

`clean.h` supports the include-directive test; `included-error.h` contains the included-file parser error; `include-next-relative.h` is found beside its includer, the primary source file, for the `#include_next` search-start warning; `pragma-system-header.h` and the `system-headers/` directory (`noisy.h`, `beside-noisy.h`) supply the user header with `#pragma GCC system_header` and the system headers of `pp-system-header-diagnostics.c`; `system-headers/pointer-macros.h` is the system header of `sema-pointer-system-header.c`, `system-headers/record-macros.h` of `sema-missing-fields-system-gcc.c`, and `system-headers/conversion-macros.h` of `sema-constant-conversion-system-macro-clang.c`. `context-headers/` holds the headers of `context-includes-{gcc,clang,clang-cl}.c`, `context-macros-{gcc,clang}.c`, `context-repeat-readings-clang.c` and `context-repeat-escape-clang.c`, and the headers read more than once by the `sema-unused-order-*repeat*-gcc.c` fixtures. The header files are included by C fixtures, not invoked independently.

## Coverage

### Review criteria

When changing diagnostics, inspect complete stderr as well as the structured
error. Preserve user spellings, escape control bytes, and label the offending
token or a zero-width EOF position. Related labels should identify prior
declarations, opening delimiters, or macro invocation sites when useful.
Recovery should preserve following valid input and avoid cascades caused only
by already diagnosed syntax. Missing operands in directives should point into
the directive expression, not the following `#endif`.

Prefer context-specific help over repeated headline text. Quote fixed language
tokens consistently, preserve user-provided `#error` messages, and distinguish
invalid shift counts from arithmetic result overflow. A standard citation must
support the exact claim; missing citations alone are a documentation-quality
issue, not evidence that the diagnostic is wrong. Use the repository's
[N1256 reference](../../../standards/c99-n1256.pdf): conditional replacement/arithmetic is
§6.10.1p4, shift-count constraints are §6.5.7p3, line-number syntax is
§6.10.4p3, pragma switches are §6.10.6p2, and repeated `inline` is §6.7.4p5.

### Remaining output issues

The following observations were checked against the tracked `.stderr` snapshots
on 3 October 2026 while consolidating the earlier diagnostic review; the
include-directive row was rechecked on 5 October 2026. They are
output improvements to revisit, not claims that the golden tests currently
fail. Rerun the linked fixtures before changing behavior or blessing snapshots.

| Evidence | Follow-up |
| --- | --- |
| [Missing declaration semicolon](parser-expected-declaration-continuation-after-declarator-missing-semicolon.stderr) | Prefer the missing declaration terminator as the primary error instead of diagnosing a function body after interpreting later declarations as parameters. |
| [Invalid assignment left operand](parser-expected-statement-expression-assignment.stderr) | The cannot-assign label points at `=`; retain and label the offending left expression. |
| [Pragma switch](pp-missing-on-off-switch-in-s-t-d-c-pragma.stderr) | Name `FP_CONTRACT` as the pragma requiring a switch, rather than saying the switch belongs after `MAYBE`. |
| [System header lookup](pp-header-not-found-system.stderr) | Distinguish an empty search-path list from an absolute header path. |
| [C23 literal encodings](lexpp-c23-constraints.stderr) | Since `u` and `U` literals are typed, a concatenation of conflicting prefixes also reports an incompatible pointer conversion; Clang drops the literal after the first error. Mark the diagnosed literal so that semantic analysis gives it no type. |
| [Left shift](pp-left-shift-overflow.stderr), [right shift](pp-right-shift-overflow.stderr) | Describe an invalid shift count separately from an overflowing result. |
| [Angle-header newline](tokenizer-angle-header-newline.stderr), [angle-header EOF](tokenizer-angle-header-eof.stderr), [quoted-header EOF](tokenizer-quoted-header-eof.stderr) | The missing closing `>` or `"` is diagnosed at the absent delimiter, but the lookup of the partial name still reports a missing header; review whether that lookup should run. Angle EOF also reports an unexpected end of file in the directive, which quoted EOF no longer does; review whether it is redundant beside the missing `>`. |
| [Unicode identifier recovery](parser-unicode-identifier.stderr) | Review the missing-type and continuation diagnostics for a single malformed declaration together. |
| [Macro keyword](parser-macro-expanded-keyword.stderr), [macro operand](parser-expected-statement-expression-macro.stderr) | Add useful macro invocation context alongside spelling or recovery locations. |
| [Conflicting storage classes](parser-storage-class-redefinition.stderr) | Label the earlier `static` as well as the new `extern`. |
| [GCC's conditional arms](pp-c23-directives-gcc-werror.stderr) | Checked 15 November 2026. GCC writes `#elifdef without #if` and `#elifdef after #else` without backticks, and follows the latter with a line-only `the conditional began here` error at the `#if`, after which it quotes the next diagnostic at the arm again. |
| [Clang's `#embed` without a resource](pp-c23-directives-clang-werror.stderr) | Checked 15 November 2026. Clang writes `expected "FILENAME" or <FILENAME>` at the end of the directive with a caret only; bcc names the resource and underlines `embed`. |
| [Clang's discarded qualifiers](sema-discards-qualifiers-clang.stderr) | Checked 16 November 2026 against Clang 23.1.5: every warning line, place and range matches. For an argument Clang follows the warning with `note: passing argument to parameter here` at the parameter's declaration, which bcc gives too ([below](sema-parameter-note-clang.stderr)). |
| [Clang's parameter notes](sema-parameter-note-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17`: identical, byte for byte. After a diagnostic of an argument's conversion (`DiagnoseAssignmentResult`: `-Wint-conversion`, incompatible and discarded-qualifier pointers, `-Wpointer-sign`, an argument that cannot convert), Clang notes the parameter of the latest declaration of the function the call names, as a place: its name, where Clang's parser places the name an unnamed one leaves out ([below](sema-parameter-note-abstract-clang.stderr)), or a typedef'd function's name; none for a call through a pointer, past a variadic function's parameters, for a definition with an identifier list, or after a value warning (`-Wimplicit-const-int-float-conversion`). bcc's own format draws the note as a secondary label. |
| [Clang's notes at unnamed parameters](sema-parameter-note-abstract-clang.stderr), [in C23](sema-parameter-note-abstract-c23-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17` and `-std=c23`: identical, byte for byte. Clang notes an unnamed parameter where its parser places the name the abstract declarator leaves out (`ParseDirectDeclarator`, `ParseParenDeclarator`): past the pointers with their qualifiers and attributes, at an array's `[` (`int [3]`, `int *[3]`, `int [static 3]`, `int [2][3]`), at the token after a function's `(` (`int (double)`, `int ()`, and in C23 `int ()` and `int (...)`), within parentheses where the declarator inside places it (`int (*)(double)`, `int (*const)(void)`, `int (*[2])(void)`, `int (*(*)(int))(char)`, `int (*)[3]`, a GNU attribute before the `*`, `int *(*)(void)`), and otherwise at the token after the declarator (`int`, `int *`, `int *const`, a GNU attribute after the `*`). Each qualifier written after a `*` counts as written, so a repeated `const`, `volatile`, `restrict` or `_Atomic`, on one pointer or several and with attributes between, still puts the note at the `[` (`int *const const [3]`), after Clang's `-Wduplicate-decl-specifier` warnings. |
| [Clang's functions without a prototype](sema-deprecated-non-prototype-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17`: identical, byte for byte. Clang reports a definition with an identifier list once its body is finished, with a declaration without a prototype before it; a declaration that one with parameters contradicts when it meets the second, noting the prototype unless the one without is a definition or the prototype declares a builtin; and a call with arguments at its `(` after the call's other reports, with `too many arguments in call` (no option) first when the definition is known. Declaration specifiers that name a typedef of a function type with a prototype give the function declared a prototype in Clang's eyes, even when the typedef is only the pointee of its return type (`Handler *get_handler(n)`). Not covered: a call whose `)` a macro's body wrote sorts at the macro's name, where Clang reports it after the calls in its arguments, and a call to a function declared without a prototype in a system header, which Clang leaves alone, has no fixture. |
| [Clang's declarations of builtins without a prototype](sema-deprecated-non-prototype-builtins-clang.stderr), [with `-fno-builtin-labs` in C17](sema-deprecated-non-prototype-no-builtin-clang.stderr), [freestanding](sema-deprecated-non-prototype-freestanding-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5: identical, byte for byte. Clang neither reports a declaration without a prototype that declares a builtin nor notes a prototype that does (`MergeFunctionDecl`): the first declaration of a builtin's name declares it when its type is compatible and it is not `static`, and a later one when the one it redeclares does, once Clang has compared them, so a declaration without a prototype that follows a builtin's prototype is reported without a note. A library function is a builtin when an implicit declaration of it would be one: not under `-ffreestanding`, `-fno-builtin` or `-fno-builtin-NAME`, and a GNU one such as `alloca` or `index` only in the GNU modes; a reserved name such as `__builtin_abs` is one under every option. A call with arguments to a declaration without a prototype is not reported when that declaration declares a builtin by these rules, so a GNU mode's `alloca()` is silent and a `static` `labs()` or `index()` is reported; a unit test (`calls_take_their_declarations_builtin_status`) pins those calls, since Clang's `-Wundefined-internal` accompanies the static ones. Not covered: a redeclaration without a prototype takes the prototype of the one it redeclares, so the next declaration reports it as one with parameters; a unit test pins that, since bcc quotes no source for that second report at the place it reported just before the note between them, where Clang quotes it again. Clang's `conflicting types` and `cannot redeclare builtin function` errors for an incompatible declaration of a reserved builtin, and its warning before reporting an incompatible `printf` or `fprintf` declaration, which bcc rejects as incompatible, have no fixture. |
| [Clang's pointer and integer casts](sema-pointer-casts-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17`: identical, byte for byte, in Clang's words (`cast to smaller integer type 'int' from 'void (*)(void)'`, `cast to 'int *' from smaller integer type 'int'`) for each of the six groups. bcc keeps no typedef names, so where Clang writes `'U' (aka 'unsigned int')` bcc writes `'unsigned int'`. |
| [Clang's unused objects in headers](sema-unused-header-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17 -Wall`: identical, byte for byte. Clang reports an unused internal object, constant or not, only in the main file (`ShouldWarnIfUnusedFileScopedDecl`), and an unused internal function in a header too. |
| [Clang's `char` subscripts](sema-char-subscripts-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17 -Wall`: identical, byte for byte. `-Wall` holds `-Wchar-subscripts`; Clang warns at the `[` of a subscript whose index, either operand, has plain `char` type (a typedef of it too), with the index underlined, unless an integer constant expression makes it not negative. GCC's form of the warning (`array subscript has type 'char'`) is not modeled, so GCC mode stays silent. |
| [Clang's uninitialized variables](sema-uninitialized-clang.stderr), [with `-Wconditional-uninitialized`](sema-uninitialized-conditional-clang.stderr), [a variable used 216 times](sema-uninitialized-many-uses-clang.stderr), [their fix-its](sema-uninitialized-fixits-clang.stderr) | Checked 17 November 2026, and on 18 November 2026 for the calls through `*`, `&` and casts, the discarded `?:` arms, the many uses and the fix-its, against Clang 23.1.5 with `-std=gnu17 -Wall`: identical, byte for byte, with and without `-fdiagnostics-parseable-fixits`. The notes carry Clang's edits (`SuggestInitializationFixit`, `CreateIfFixit` and `DiagUninitUse` in `AnalysisBasedWarnings.cpp`): ` = 0`, ` = 0.0` or ` = '\0'` after the declarator; the removal of an `if` or a `?:` up to the arm it keeps and of the other arm, or of the whole `if`; the `0` or `1` a loop's or `&&`'s and `||`'s condition becomes, or its removal. Differences left: a pointer or a `_Bool` gets no edit, since Clang writes ` = NULL` or ` = false` only where that macro is defined, which bcc does not keep past preprocessing, and ` = 0` otherwise; and a removal over several lines (an `if` whose arm is a block) is underlined with `~` on each line, but bcc quotes only the lines it underlines and every one of them, where Clang quotes each line of the range, blank parts too, up to 16 lines (`maybeAddRange` in `TextDiagnostic.cpp`). The warnings come from Clang's uninitialized-values analysis, which `uninitialized/cfg.rs` follows block for block: `-Wuninitialized` for a use no store reaches, `-Wsometimes-uninitialized` for each branch after which one is uninitialized (`'if'`, `'&&'`, `'\|\|'` and `'?:'` conditions, `'while'` and `'for'` loops entered or exited, `'do'` loops, switch cases and defaults, a use reached whenever the function is called or the declaration is reached), `-Wuninitialized-const-pointer` for `&x` passed as a pointer to `const` (not to a function with an empty body, which a call names through `*` and `&` but not through a cast, as it names an `analyzer_noreturn` or `returns_twice` function), the self-reference in `int x = x + 1`, which is on whenever the analysis runs, and `int x = x`, which leaves its uses only possibly uninitialized. A value discarded by `(void)` reaches a `?:` arm only through a conversion that changes nothing: `(void)(a ? 0L : x)` still reads `x`. A differential corpus of about 1,100 generated programs (`if`, loops, `switch` with fallthrough and case ranges, `goto` and computed `goto`, `&&`, `\|\|`, `?:` with and without its middle operand, statement expressions, `setjmp` and its kin, `noreturn` and `analyzer_noreturn` calls, `__builtin_unreachable`, `__builtin_assume`, `asm` operands, `_Generic`, `__builtin_choose_expr`, variable length arrays, records, enumerations, vectors, `const` pointer arguments, self-initialization) and every option combination of the four groups, `-Wno-` forms and pragmas matched Clang's warning and note lines exactly, and the graph matched Clang's `-analyzer-checker=debug.DumpCFG` block for block, but for the empty block Clang adds after a `?:` without a middle operand whose type converts and the static-initializer branch only the analyzer's graph has. In bash 5.2.37 all 11 warnings and their notes match; where the condition is a nested macro (`S_ISREG`), bcc writes only the innermost `expanded from macro` note and no macro notes under the `note:` lines, as for every warning, and its ranges in the macro differ. Silent where Clang warns, by design: a function holding a designated or overflowing initializer that reads a tracked variable (Clang's semantic form reorders or drops its elements), an object with `cleanup`, `asm goto`, a GNU nested function, `__label__`, SEH or Microsoft `asm`, or a C2y selection header or named loop; and the condition of a statement `assume` attribute, which bcc does not retain. |
| [GCC's uninitialized variables](sema-uninitialized-gcc.stderr) | Not modeled: GCC's `-Wuninitialized` and `-Wmaybe-uninitialized` come from its optimizer and depend on the optimization level, so GCC's rules give none of Clang's warnings. Checked 17 November 2026 against GCC 13.3: under `-fsyntax-only` it reports only the unused variable, as bcc does; compiling with `-O2 -Wall` it also reports `'x' is used uninitialized` in `always` and `itself`, but not in `sometimes`, which bcc leaves out. |
| [Clang's `-Wmissing-braces`](sema-missing-braces-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5 with `-std=gnu17 -Wall` (every C standard's `-Wall` holds the group): identical, byte for byte. Clang warns at the first element of each array, structure or union whose braces brace elision left out, inner subobjects first, underlines the elements and suggests `{` and `}` around them (`InitListChecker::CheckImplicitInitList` in `SemaInit.cpp`): nested structures and arrays, arrays of structures, an element left alone by a designator or the list's end, a designated member's elements, a union's first member, characters for an array, local objects, and a range over two lines. It warns of none in `{0}` alone (`{0, 0}` and `{(0), 1}` warn), for the only member of a structure, a string, a scalar in braces, braces given, a subobject a designator names, or an initializer a system header writes. A macro that writes the whole list moves the warning to its invocation without a hint; one that writes the subobject's elements whole, as `ONE` in `{ONE, 3}` and a system header's `SYS_PAIR`, takes the braces around the invocation; and an end inside a macro's elements, as `LAST` in `{1, LAST}`, leaves out its `}` alone. GCC 14's `-Wall` holds its own `-Wmissing-braces`, which warns once per braced list at the initializer's outer `{` with every hint of the whole initializer; it is not modeled, so GCC's identity stays silent. |
| [Clang's nonnull comparisons](sema-nonnull-compare-clang.stderr) | Checked 18 November 2026 against Clang 23.1.5 with `-std=gnu17`: identical, byte for byte. Clang's `DiagnoseAlwaysNonNullPointer` calls a parameter that a `nonnull` names, or that one written on the parameter makes nonnull, `nonnull` until an assignment (`p = 0`, not `(p) = 0`) or `&p` changes it, and not after `++` or `+=`; it warns of comparing one with a null pointer (`-Wtautological-pointer-compare`, on by default) and of converting one to `_Bool` (`-Wpointer-bool-conversion`), and of a call to a `returns_nonnull` function, printing the call as its `StmtPrinter` does. The note names the first attribute that applies, in the order Clang keeps them: a parameter's own first, then the function's, earlier declarations' before later ones (`Decl::addAttr`), as on lines 17-24. A parenthesized name, a macro's body and a macro's argument compared with a null pointer the body wrote are silent (lines 30 and 33-38). A parameter of a prototype is not the function's, even with the same name: only a `nonnull` written on it makes it nonnull, and only within a function's body (lines 58-62). A conversion to `_Bool` looks through parentheses and warns at the name or call inside; a comparison looks through them only to find a call, which it prints with them (lines 64-78). A call's function operand names its callee through parentheses, `*` and `&`, though not through a cast (lines 80-93). |
| [Clang's addresses compared with null](sema-null-compare-clang.stderr) | Checked 18 November 2026 against Clang 23.1.5 with `-std=gnu17`: every warning, note, place and range matches but for Clang's fix-it lines, `&` under the note that the address-of operator silences the warning and `()` after the name for the note that parentheses would call the function (Clang then also pads the caret line with spaces to the `()`). An array, a function and the operand of `&`, through `.` and `->`, compared with a null pointer (`comparison of array 'array' equal to a null pointer is always false`) or converted to `_Bool` (`address of function 'function' will always evaluate to 'true'`); `&` of a function, of an element or of a parenthesized name, a weak object and a string say nothing. The second note comes when calling the function with no arguments would give a pointer, an integer compared with `0`, or a `_Bool` converted. A conversion to `_Bool` looks through parentheses (lines 53-60). |
| [Clang's null arguments](sema-nonnull-arguments-clang.stderr), [GCC's](sema-nonnull-arguments-gcc.stderr) | Checked 17 November 2026 against Clang 23.1.5 and GCC 14.2 with `-std=gnu17`: identical, byte for byte. Clang reports an argument a `nonnull` without indices covers at the call's `)` and one an index names at the argument, with the argument's range, once the function is finished and only where control may reach the call (`DiagRuntimeBehavior`); GCC reports `argument N null where non-null expected` at the function's name as it builds the call, with a note at the function's latest declaration. Both take an argument whose value is null or zero, `(int *)0` and `(void *)(1 - 1)` too. |
| Not in a fixture: nonnull | Checked 17 November 2026. Clang's note at an attribute a macro wrote, as glibc's `__nonnull ((1))`, is followed by an `expanded from macro` note for each macro; bcc's notes give none (it notes only the innermost macro, and only for the warning itself). Within one full expression Clang reports a conversion to `_Bool` after the comparisons (`r() == 0 \|\| !r()`), where bcc sorts them by column. When the pointer is a macro's argument and the macro's body writes the comparison (`#define EQ(a, b) a == b`), Clang follows the warning with an `expanded from macro` note that bcc does not give. Clang's own words for the attribute's faults (`'nonnull' attribute only applies to pointer arguments`, `... applied to function with no pointer arguments`, `... parameter 1 is out of bounds`) and its `-Wgcc-compat` for an attribute after a definition's declarator remain bcc's words or absent. A type name in a printed call keeps its tokens without spaces (`(char*)x` for Clang's `(char *)x`). GCC's `-Wnonnull-compare` is left out: GCC 14 reports it from its SSA form after gimplification, so its place, range and order follow the IR (`(p) == 0 ? 3 : 4` underlines the whole conditional, and a function's reports come last line first), which semantic analysis cannot match; so is GCC's `-Waddress` comparison of an address with a null pointer. bcc has no nullability qualifiers (`_Nonnull`), so none of Clang's nullability checks. A `nonnull` on a block-scope declaration of a function stays with the function after the block ends, so bcc also warns (`-Wnonnull`) of a later call through the outer declaration (`{ void f(int *p) __attribute__((nonnull)); } f(0);`), where Clang does not: bcc keeps a function's attributes for the entity, not for each declaration. |
| [Clang's constant conversions of literals](sema-constant-conversion-literals-clang.stderr) | Checked 16 November 2026 against Clang 23.1.5: the warnings, their lines and their order match. Clang also underlines the whole converted expression and the token the conversion happens at (`~ ^~~` under `*p = 0xdf`, `~~~~~~~   ^~~` under `char decimal = 223`); bcc draws only the caret's range. For a literal pasted by `##`, Clang's caret is on the macro's name (`CAT`, column 23 on line 34) and bcc's on the pasted pieces (column 27). |
| [Clang's conversions in system macros](sema-constant-conversion-system-macro-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5, also as clang-cl: the warnings, their lines and their order match. Clang drops a conversion warning whose context location (the call, the assignment operator, the `return`, the declarator, the `{` or designator that begins the element's list, or the comma whose right operand converts) a system macro's body wrote, and a format's unused argument that begins in one. A list keeps the place it was made at when a later designator enters it again (`{ .pair.low = 1, HIGH }` warns), and a token `##` formed is in a system macro when a system header defines the macro whose `##` formed it, wherever its pieces are spelled. Clang also underlines the context location (`~` under `immediate(`, `=`, `return`, the declarator, the `{` or the comma); bcc draws only the caret's range. Clang notes `DECLARE_CHAR`'s expansion for the value its argument gives (line 42); bcc gives no note there. Clang reports the elements of one list last first; the fixture reports one element per list, since bcc reports them in source order. |
| [GCC's `-Wbool-operation`](sema-bool-operation-gcc.stderr) | Checked 17 November 2026 against GCC 14.2: every warning, note and fix-it line (the `!` GCC suggests inserting before each `~` its `did you mean to use logical not?` names) matches but for the order on line 13, where GCC reports the comma's left operand before the `~` and bcc's reporter sorts them by column. From line 29 GCC warns in every operand of a `_Generic` and `__builtin_choose_expr`, in the controlling operand, and in the operands of `_Alignof`, `typeof` and `__builtin_constant_p`, as bcc does. For a `~` a macro's body writes, GCC writes the note in the definition and notes the expansion again; bcc writes it at the invocation, so the fixture has none. |
| [Clang's `-Wbool-operation`](sema-bool-operation-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5: every warning, place, range and fix-it line (`!` in place of the `~`, dropped for a `~` a macro's body wrote) matches but for the range of the `expression result unused` warning on line 26, which Clang underlines over the operand (`^~`) where bcc draws only the caret, as in `sema-unused-value-clang`. Clang's arms of a conditional converted to `_Bool` would say `always evaluates to 'true'`; bcc says it only of an assignment, argument, initializer or return. From line 34, Clang warns only in the operand a `_Generic` or `__builtin_choose_expr` selects, never in a controlling operand, and underlines the selection when the `~` is the operand selected; a statement expression's statement warns wherever it is. A brace-enclosed list of `_Bool` (`{~b, ~b}`) is left out: Clang reports its elements last first, each `always evaluates to 'true'`, and bcc reports the first element without those words. |
| [Clang's discarded statement expressions](sema-unused-value-statement-expression-clang.stderr) | Checked 17 November 2026 against Clang 23.1.5: the warnings, their places and their notes match. From line 22, Clang says nothing when a macro's body or argument, a system macro's included, placed the warning of a statement expression's value, as for the comma's left operand on line 32, but warns when parentheses or `__extension__` surround the statement expression (lines 35 and 36). Clang also underlines the operands of `a + 1` and `a == 1` (`~ ^ ~`), which bcc's `-Wunused-value` draws nowhere, and the whole `VALUE(a)` invocation where bcc underlines the macro's name. |
| [GCC's ranges of discarded values](sema-unused-value-ranges-gcc.stderr) | Checked 17 November 2026 against GCC 14.2 (its `<stdbool.h>` standing in as `#define bool _Bool`): every line matches but for the order on line 41, where GCC reports the comma's left operand before the statement and bcc's reporter sorts the two by column. For an operand in a macro's argument GCC adds `in definition of macro`, which bcc never writes, so the fixture has none. |
| [Clang's checked integer arithmetic](sema-stdckdint-clang.stderr) | Checked 16 November 2026 against Clang 23.1.5: the errors, their places and their words match. Clang underlines an argument's whole macro invocation (`^~~~~~~~~~~~~~~~~~~` under `ckd_add(&c, 127, 1)`) and the `(R)` it notes in its header (`^~~`), where bcc underlines the macro's name and points at the `(`. Clang's note names its own header (`stdckdint.h:37`); bcc's names `<built-in>/stdckdint.h:18`. For `ADD` around `ckd_add`, Clang also notes `ADD`'s expansion; bcc notes only the innermost macro. |
| [GCC's checked integer arithmetic](sema-stdckdint-gcc.stderr) | Checked 16 November 2026 against GCC 14.2 with its own `<stdckdint.h>`: identical, byte for byte. GCC reports an error a system header's macro wrote where the user invoked the macro, as it does a warning. |
| [GCC's folded bounds under `__extension__`](sema-extension-pedantic-gcc.stderr) | Checked 20 November 2026 against GCC 14. bcc reports a folded array bound such as `(int)(1.0 + 1)` with its `-pedantic` folded-constant warning whether or not `__extension__` marks the declaration, where GCC makes the array variable length and judges it by its storage and uses: silent for an automatic object, a parameter or a block-scope type name, and an error at file scope, with static storage, in a compound literal, under an initializer, after a jump into its scope, or through a typedef or structure a static object takes, none of which `__extension__` silences. Model that variable length array, then silence the report in a marked declaration where GCC accepts the array. |

The prior panic and raw-NUL findings remain protected by the guard test and
their existing fixtures. Keep those regressions even though the old review's
captured failing output is no longer the current snapshot.

### Dispatch inventory

There are **410 top-level C inputs and 410 stderr snapshots**, plus eight supporting headers and 159 mode/policy `.args` sidecars. One hundred twenty-seven of the inputs cover semantic analysis (twenty-nine for declarations, twenty-nine for expressions and initializers, forty-six for statements and functions, three for 128-bit integers, four for resource intrinsics, two for scalar builtin constraints, six for atomics and generic selection, four for binary128 and type selections, four for vector constraints) and are listed in their own tables below. The [machine inventory](coverage.tsv) records **1 initial-processing**, **5 tokenizer**, **143 preprocessor** and **66 parser** variants. These include nineteen folded preprocessor variants and four folded parser variants. The other 124 preprocessor and 62 parser variants have separately rendered messages in that inventory; the parser total exceeds the requested minimum of 40. The per-variant tables below also describe untested and unreachable paths. Shared extension-origin diagnostics are tracked separately below.

The mapping below comes from checking the emitter/dispatch paths and their CLI output. It is not private-enum instrumentation. Variants sharing wording are distinguished by their source trigger; folded variants do not claim an independently rendered golden message. Supplementary EOF, literal, macro, tab, Unicode, include, and operand-position `:` cases may target the same variant more than once.

### InitialProcessorError

| Variant | Status | Fixture or reason |
| --- | --- | --- |
| `MissingFinalNewline` | rendered | [initial-missing-final-newline.c](initial-missing-final-newline.c) |

### PreprocessorTokenizerErrorType

| Variant | Status | Fixture or reason |
| --- | --- | --- |
| `UnknownToken` | rendered | [tokenizer-unknown-token-backtick.c](tokenizer-unknown-token-backtick.c), [tokenizer-unknown-token-nul.c](tokenizer-unknown-token-nul.c), [tokenizer-unknown-token.c](tokenizer-unknown-token.c) |
| `UnterminatedCharacter` | rendered | [tokenizer-unterminated-character.c](tokenizer-unterminated-character.c) |
| `UnterminatedString` | rendered | [tokenizer-unterminated-string.c](tokenizer-unterminated-string.c) |
| `NewlineInCharacter` | rendered | [tokenizer-newline-in-character.c](tokenizer-newline-in-character.c) |
| `NewlineInString` | rendered | [tokenizer-newline-in-string.c](tokenizer-newline-in-string.c), [tokenizer-newline-in-include-string.c](tokenizer-newline-in-include-string.c), [tokenizer-quoted-header-eof.c](tokenizer-quoted-header-eof.c) |

The lexer no longer forms header names: an `#include` operand is ordinary preprocessing tokens, so an unterminated `"…"` operand draws the lexer's string-literal error, and the `#include` handler adds a missing closing `"` or `>` error at the delimiter position. For a written quoted name whose final `"` follows a backslash, the default `ExtensionPolicy::Allow` (and Warn) reads the backslash as a path character, closes the name at that quote, and withdraws the lexer's error for that one token; Deny keeps the lexer and missing-quote errors.

### PreprocessorErrorType

| Variant | Status | Fixture or reason |
| --- | --- | --- |
| `LanguageConstraint` | rendered | [C23 literal encodings](lexpp-c23-constraints.c), [C2y delimited escapes](lexpp-c2y-escapes.c); structured mode tests also cover malformed queries and embed parameters |
| `EmbeddedResourceNotFound` | rendered | [pp-embedded-resource-not-found.c](pp-embedded-resource-not-found.c), [lexpp-c23-constraints.c](lexpp-c23-constraints.c) |
| `EmbeddedResourceUnreadable` | environment-dependent; not covered | Like `HeaderFileInaccessible`, requires a discovered resource whose open, size query, or read then fails; the OS error wording is host-dependent. |
| `EmbeddedResourceTooLarge` | unreachable on 64-bit hosts | A resource length is a `u64`, which always converts to the 64-bit `usize` that bcc requires. |
| `VaOptUnavailable` | rendered | [pp-va-opt-unavailable.c](pp-va-opt-unavailable.c) |
| `MissingOpeningParenthesisAfterVaOpt` | rendered | [pp-missing-opening-parenthesis-after-va-opt.c](pp-missing-opening-parenthesis-after-va-opt.c) |
| `NestedVaOpt` | rendered | [pp-nested-va-opt.c](pp-nested-va-opt.c) |
| `UnterminatedVaOpt` | rendered | [pp-unterminated-va-opt.c](pp-unterminated-va-opt.c) |
| `HashHashAtVaOptBoundary` | rendered | [pp-hash-hash-at-va-opt-boundary.c](pp-hash-hash-at-va-opt-boundary.c), [lexpp-c23-constraints.c](lexpp-c23-constraints.c) |
| `WarningDirective` | rendered | [GNU89 warning message and extension policy](lexpp-gnu89-extensions.c), GCC's and Clang's words in [pp-c23-directives-gcc.c](pp-c23-directives-gcc.c) and [pp-c23-directives-clang.c](pp-c23-directives-clang.c); structured tests cover native C23 warnings and earlier strict rejection |
| `DirectiveExtension` | rendered | [pp-c23-directives-gcc.c](pp-c23-directives-gcc.c), [pp-c23-directives-gcc-werror.c](pp-c23-directives-gcc-werror.c), [pp-c23-directives-strict-gcc.c](pp-c23-directives-strict-gcc.c), [pp-c23-directives-clang-pedantic.c](pp-c23-directives-clang-pedantic.c), [pp-c23-directives-msvc-identity.c](pp-c23-directives-msvc-identity.c); bcc's words through the language-mode tests |
| `C23DirectiveExtension` | rendered | [pp-c23-directives-clang.c](pp-c23-directives-clang.c), [pp-c23-directives-clang-pedantic-errors.c](pp-c23-directives-clang-pedantic-errors.c), [pp-c23-directives-clang-werror.c](pp-c23-directives-clang-werror.c) |
| `InvalidHexadecimalFloatLiteral` | rendered | [pp-invalid-hexadecimal-float-literal.c](pp-invalid-hexadecimal-float-literal.c) |
| `InvalidDecimalFloatLiteral` | rendered | [pp-invalid-decimal-float-literal.c](pp-invalid-decimal-float-literal.c) |
| `FloatConstantOutOfRange` | rendered | [pp-float-constant-out-of-range-underflow.c](pp-float-constant-out-of-range-underflow.c), [pp-float-constant-out-of-range.c](pp-float-constant-out-of-range.c) |
| `InvalidHexadecimalIntegerLiteral` | rendered | [pp-invalid-hexadecimal-integer-literal.c](pp-invalid-hexadecimal-integer-literal.c) |
| `InvalidBinaryIntegerLiteral` | rendered | [pp-invalid-binary-integer-literal.c](pp-invalid-binary-integer-literal.c) |
| `InvalidOctalIntegerLiteral` | rendered | [pp-invalid-octal-integer-literal.c](pp-invalid-octal-integer-literal.c) |
| `InvalidDecimalIntegerLiteral` | rendered | [pp-invalid-decimal-integer-literal.c](pp-invalid-decimal-integer-literal.c) |
| `IntegerLiteralOverflow` | rendered | [pp-integer-literal-overflow.c](pp-integer-literal-overflow.c) |
| `TccIntegerOverflow` | rendered | [pp-tcc-integer-overflow.c](pp-tcc-integer-overflow.c) |
| `TccMulticharacterConstant` | rendered | [pp-tcc-wording.c](pp-tcc-wording.c) |
| `ForcedSignedToUnsignedConversion` | rendered | [pp-forced-signed-to-unsigned-conversion.c](pp-forced-signed-to-unsigned-conversion.c) |
| `ForcedUnsignedPromotion` | rendered | [pp-forced-unsigned-promotion.c](pp-forced-unsigned-promotion.c) |
| `ForcedSignedPromotion` | rendered | [pp-forced-signed-promotion.c](pp-forced-signed-promotion.c) |
| `HashMustBeFirstCharacterOnLine` | rendered | [pp-hash-must-be-first-character-on-line.c](pp-hash-must-be-first-character-on-line.c) |
| `HashMustBeFollowedByIdentifier` | rendered | [pp-hash-must-be-followed-by-identifier.c](pp-hash-must-be-followed-by-identifier.c) |
| `UnknownDirective` | rendered | [pp-unknown-directive.c](pp-unknown-directive.c) |
| `EmptyParenthesesInPreprocessorExpression` | rendered | [pp-empty-parentheses-in-preprocessor-expression.c](pp-empty-parentheses-in-preprocessor-expression.c) |
| `UnaryPlusWithoutOperand` | rendered | [pp-unary-plus-without-operand.c](pp-unary-plus-without-operand.c) |
| `UnaryMinusWithoutOperand` | rendered | [pp-unary-minus-without-operand.c](pp-unary-minus-without-operand.c) |
| `BitwiseNotWithoutOperand` | rendered | [pp-bitwise-not-without-operand.c](pp-bitwise-not-without-operand.c) |
| `LogicalNotWithoutOperand` | rendered | [pp-logical-not-without-operand.c](pp-logical-not-without-operand.c) |
| `BinaryPlusWithoutRhs` | reached; folded | [pp-binary-plus-without-rhs.c](pp-binary-plus-without-rhs.c) |
| `BinaryMinusWithoutRhs` | reached; folded | [pp-binary-minus-without-rhs.c](pp-binary-minus-without-rhs.c) |
| `MultiplyWithoutRhs` | reached; folded | [pp-multiply-without-rhs.c](pp-multiply-without-rhs.c) |
| `DivideWithoutRhs` | reached; folded | [pp-divide-without-rhs.c](pp-divide-without-rhs.c) |
| `ModuloWithoutRhs` | reached; folded | [pp-modulo-without-rhs.c](pp-modulo-without-rhs.c) |
| `LessThanWithoutRhs` | reached; folded | [pp-less-than-without-rhs.c](pp-less-than-without-rhs.c) |
| `LessThanEqualsWithoutRhs` | reached; folded | [pp-less-than-equals-without-rhs.c](pp-less-than-equals-without-rhs.c) |
| `GreaterThanWithoutRhs` | reached; folded | [pp-greater-than-without-rhs.c](pp-greater-than-without-rhs.c) |
| `GreaterThanEqualsWithoutRhs` | reached; folded | [pp-greater-than-equals-without-rhs.c](pp-greater-than-equals-without-rhs.c) |
| `EqualsWithoutRhs` | reached; folded | [pp-equals-without-rhs.c](pp-equals-without-rhs.c) |
| `NotEqualsWithoutRhs` | reached; folded | [pp-not-equals-without-rhs.c](pp-not-equals-without-rhs.c) |
| `LeftShiftWithoutRhs` | reached; folded | [pp-left-shift-without-rhs.c](pp-left-shift-without-rhs.c) |
| `RightShiftWithoutRhs` | reached; folded | [pp-right-shift-without-rhs.c](pp-right-shift-without-rhs.c) |
| `BitwiseAndWithoutRhs` | reached; folded | [pp-bitwise-and-without-rhs.c](pp-bitwise-and-without-rhs.c) |
| `BitwiseXorWithoutRhs` | reached; folded | [pp-bitwise-xor-without-rhs.c](pp-bitwise-xor-without-rhs.c) |
| `BitwiseOrWithoutRhs` | reached; folded | [pp-bitwise-or-without-rhs.c](pp-bitwise-or-without-rhs.c) |
| `LogicalAndWithoutRhs` | reached; folded | [pp-logical-and-without-rhs.c](pp-logical-and-without-rhs.c) |
| `LogicalOrWithoutRhs` | reached; folded | [pp-logical-or-without-rhs.c](pp-logical-or-without-rhs.c) |
| `TernaryOperatorWithoutMhs` | rendered | [pp-ternary-operator-without-mhs.c](pp-ternary-operator-without-mhs.c) |
| `TernaryOperatorWithoutRhs` | rendered | [pp-ternary-operator-without-rhs.c](pp-ternary-operator-without-rhs.c) |
| `ColonWithoutMatchingQuestionMark` | rendered | [pp-colon-without-matching-question-mark.c](pp-colon-without-matching-question-mark.c) |
| `CommaOperatorInPreprocessorExpression` | reachable with policy flags | The CLI default allows extensions; `-pedantic`/`-Wpedantic` select Warn and `-pedantic-errors` selects Deny. Unit tests cover the specialized emitter. |
| `BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression` | rendered | [pp-binary-operator-instead-of-unary-expression-in-preprocessor-expression.c](pp-binary-operator-instead-of-unary-expression-in-preprocessor-expression.c) |
| `DivideByZero` | rendered | [pp-divide-by-zero.c](pp-divide-by-zero.c) |
| `ModuloByZero` | rendered | [pp-modulo-by-zero.c](pp-modulo-by-zero.c) |
| `UnaryMinusOverflow` | rendered | [pp-unary-minus-overflow.c](pp-unary-minus-overflow.c) |
| `BinaryPlusOverflow` | rendered | [pp-binary-plus-overflow.c](pp-binary-plus-overflow.c) |
| `BinaryMinusOverflow` | rendered | [pp-binary-minus-overflow.c](pp-binary-minus-overflow.c) |
| `MultiplyOverflow` | rendered | [pp-multiply-overflow.c](pp-multiply-overflow.c) |
| `DivideOverflow` | rendered | [pp-divide-overflow.c](pp-divide-overflow.c) |
| `ModuloOverflow` | rendered | [pp-modulo-overflow.c](pp-modulo-overflow.c) |
| `LeftShiftOverflow` | rendered | [pp-left-shift-overflow.c](pp-left-shift-overflow.c) |
| `RightShiftOverflow` | rendered | [pp-right-shift-overflow.c](pp-right-shift-overflow.c) |
| `UnterminatedOpeningParenthesisInPreprocessorExpression` | rendered | [pp-unterminated-opening-parenthesis-in-preprocessor-expression.c](pp-unterminated-opening-parenthesis-in-preprocessor-expression.c) |
| `TildeInsteadOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c](pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c) |
| `ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c](pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c) |
| `FunctionCallOperatorNotSupportedInPreprocessorExpression` | rendered | [pp-function-call-operator-not-supported-in-preprocessor-expression.c](pp-function-call-operator-not-supported-in-preprocessor-expression.c) |
| `DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c](pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c) |
| `AddressOfOperatorNotSupportedInPreprocessorExpression` | rendered | [pp-address-of-operator-not-supported-in-preprocessor-expression.c](pp-address-of-operator-not-supported-in-preprocessor-expression.c) |
| `DereferenceOperatorNotSupportedInPreprocessorExpression` | rendered | [pp-dereference-operator-not-supported-in-preprocessor-expression.c](pp-dereference-operator-not-supported-in-preprocessor-expression.c) |
| `ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-colon-in-operand-position.c](pp-colon-in-operand-position.c), [pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c](pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c) |
| `NumberInsteadOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-number-instead-of-binary-operator-in-preprocessor-expression.c](pp-number-instead-of-binary-operator-in-preprocessor-expression.c) |
| `IdentifierInsteadOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-identifier-instead-of-binary-operator-in-preprocessor-expression.c](pp-identifier-instead-of-binary-operator-in-preprocessor-expression.c) |
| `CharacterInsteadOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-character-instead-of-binary-operator-in-preprocessor-expression.c](pp-character-instead-of-binary-operator-in-preprocessor-expression.c) |
| `UnexpectedTokenInPreprocessorExpression` | rendered | [pp-unexpected-token-in-preprocessor-expression.c](pp-unexpected-token-in-preprocessor-expression.c) |
| `UnexpectedTokenAtPhase7` | believed unreachable | The emitter only handles Placeholder and Whitespace. Whitespace is filtered, and empty argument placeholders are consumed by macro expansion before phase 7. No C-only CLI reproducer was found. |
| `FloatInsteadOfIntegerInPreprocessorExpression` | rendered | [pp-float-instead-of-integer-in-preprocessor-expression.c](pp-float-instead-of-integer-in-preprocessor-expression.c) |
| `ExpectedBinaryOperatorInPreprocessorExpression` | rendered | [pp-expected-binary-operator-in-preprocessor-expression.c](pp-expected-binary-operator-in-preprocessor-expression.c) |
| `MissingOpeningParenthesisOrIdentifierInDefinedDirective` | rendered | [pp-missing-opening-parenthesis-or-identifier-in-defined-directive.c](pp-missing-opening-parenthesis-or-identifier-in-defined-directive.c) |
| `MissingIdentifierInDefinedDirective` | rendered | [pp-missing-identifier-in-defined-directive.c](pp-missing-identifier-in-defined-directive.c) |
| `MissingClosingParenthesisInDefinedDirective` | rendered | [pp-missing-closing-parenthesis-in-defined-directive.c](pp-missing-closing-parenthesis-in-defined-directive.c) |
| `DefinedFromObjectLikeMacroExpansion` | rendered | [pp-defined-from-object-like-macro-expansion.c](pp-defined-from-object-like-macro-expansion.c) is the warning, which every policy keeps a warning, as in Clang; [pp-system-header-diagnostics.c](pp-system-header-diagnostics.c) reports it where the main file expands a system macro and withholds it inside the system header. |
| `DefinedFromFunctionLikeMacroExpansion` | rendered | [pp-defined-from-function-like-macro-expansion.c](pp-defined-from-function-like-macro-expansion.c) uses `-pedantic` for MinGW-w64's `##`-formed operand; the default policy allows it. [pp-system-header-diagnostics.c](pp-system-header-diagnostics.c) shows the `-pedantic-errors` error in the main file and withholds it inside the system header. |
| `NoConditionInIfDirective` | rendered | [pp-no-condition-in-if-directive.c](pp-no-condition-in-if-directive.c) |
| `NoConditionInElifDirective` | rendered | [pp-no-condition-in-elif-directive.c](pp-no-condition-in-elif-directive.c) |
| `MoreIfDirectivesThanEndifDirectives` | rendered | [pp-more-if-directives-than-endif-directives.c](pp-more-if-directives-than-endif-directives.c) |
| `MoreEndifDirectivesThanIfDirectives` | rendered | [pp-more-endif-directives-than-if-directives.c](pp-more-endif-directives-than-if-directives.c), Clang's in [pp-conditional-arms-clang.c](pp-conditional-arms-clang.c) |
| `ElifDirectiveWithoutIfDirective` | rendered | [pp-elif-directive-without-if-directive.c](pp-elif-directive-without-if-directive.c), Clang's in [pp-conditional-arms-clang.c](pp-conditional-arms-clang.c) |
| `ElseDirectiveWithoutIfDirective` | rendered | [pp-else-directive-without-if-directive.c](pp-else-directive-without-if-directive.c), Clang's in [pp-conditional-arms-clang.c](pp-conditional-arms-clang.c) |
| `ConditionalArmAfterElse` | rendered | Clang's in [pp-conditional-arms-clang.c](pp-conditional-arms-clang.c), GCC's identity in [pp-c23-directives-gcc-werror.c](pp-c23-directives-gcc-werror.c) |
| `ExpectedIdentifierInIfdefDirective` | rendered | [pp-expected-identifier-in-ifdef-directive.c](pp-expected-identifier-in-ifdef-directive.c) |
| `ExpectedIdentifierInIfndefDirective` | rendered | [pp-expected-identifier-in-ifndef-directive.c](pp-expected-identifier-in-ifndef-directive.c) |
| `ExpectedIdentifierInDefineDirective` | rendered | [pp-expected-identifier-in-define-directive.c](pp-expected-identifier-in-define-directive.c) |
| `RedefinitionOfBuiltInMacro` | rendered | [protected ISO macro](pp-redefinition-of-built-in-macro.c), [overridable GNU builtin](lexpp-gnu-builtin-overrides.c) |
| `UndefinitionOfBuiltInMacro` | rendered | [protected ISO macro](pp-undefinition-of-built-in-macro.c), [overridable GNU builtin](lexpp-gnu-builtin-overrides.c) |
| `UndefinedIdentifierInPreprocessorExpression` | rendered | [pp-undefined-identifier-in-preprocessor-expression.c](pp-undefined-identifier-in-preprocessor-expression.c) |
| `ExpectedIncludeStringOrAngleBracketString` | rendered | [pp-expected-include-string-or-angle-bracket-string.c](pp-expected-include-string-or-angle-bracket-string.c) |
| `InvalidCharacterInHeaderName` | rendered | [pp-invalid-character-in-header-name.c](pp-invalid-character-in-header-name.c) |
| `UnterminatedHeaderName` | rendered | [pp-unterminated-header-name.c](pp-unterminated-header-name.c), [tokenizer-angle-header-eof.c](tokenizer-angle-header-eof.c), [tokenizer-angle-header-newline.c](tokenizer-angle-header-newline.c), [tokenizer-newline-in-include-string.c](tokenizer-newline-in-include-string.c), [tokenizer-quoted-header-eof.c](tokenizer-quoted-header-eof.c). Labelled zero-width where the delimiter is missing. Quoted EOF keeps the lexer's string error and the missing-`"` error without an unexpected-EOF error; angle EOF keeps the unexpected-EOF error and the parser's separate empty-translation-unit error. Despite its name, [pp-unterminated-header-name-quoted.c](pp-unterminated-header-name-quoted.c) (`"dir\"`) is now a closed name under the default policy; see `BackslashInQuotedHeaderName`. |
| `BackslashInQuotedHeaderName` | unavailable in default CLI | A backslash in a `"…"` header name is an extension that the default ExtensionPolicy::Allow accepts silently, as [pp-header-not-found-backslash.c](pp-header-not-found-backslash.c) shows. Allow and Warn also treat `#include "dir\"` as the name `dir\` and withdraw only that token's unterminated-string error, as [pp-unterminated-header-name-quoted.c](pp-unterminated-header-name-quoted.c) shows; Deny keeps that error, reports the missing `"`, and skips the lookup. Warn and Deny are covered by unit tests in `preprocessing/tests/header_name_regressions.rs`. |
| `UnexpectedEndOfInput` | rendered | [pp-unexpected-end-of-input.c](pp-unexpected-end-of-input.c), [tokenizer-angle-header-eof.c](tokenizer-angle-header-eof.c) |
| `WrongNumberOfArgumentsInFunctionLikeMacroInvocation` | rendered | [pp-wrong-number-of-arguments-in-function-like-macro-invocation.c](pp-wrong-number-of-arguments-in-function-like-macro-invocation.c) |
| `HeaderNotFound` | rendered | [pp-header-not-found-system.c](pp-header-not-found-system.c), [pp-header-not-found.c](pp-header-not-found.c), [pp-header-not-found-backslash.c](pp-header-not-found-backslash.c), [pp-unterminated-header-name-quoted.c](pp-unterminated-header-name-quoted.c), [tokenizer-angle-header-eof.c](tokenizer-angle-header-eof.c), [tokenizer-angle-header-newline.c](tokenizer-angle-header-newline.c) |
| `HeaderFileInaccessible` | environment-dependent; not covered | Requires a discovered header whose subsequent read fails, such as a permissions/sharing violation or filesystem race. A normal checked-in C/header pair cannot establish that condition portably; the OS error wording is also host-dependent. |
| `HashHashUsedOutsideOfMacro` | rendered | [pp-hash-hash-used-outside-of-macro.c](pp-hash-hash-used-outside-of-macro.c) |
| `CannotUseHashHashAfterFunctionLikeMacroCall` | rendered | [pp-cannot-use-hash-hash-after-function-like-macro-call.c](pp-cannot-use-hash-hash-after-function-like-macro-call.c). A `##` at either end of a replacement list is now rejected with its definition, so the remaining trigger is a `##` that pasting `#` and `#` creates (C99 §6.10.3.3p4); the message's "invocation" wording does not fit it. |
| `UnknownEscapeSequence` | rendered | [pp-unknown-escape-sequence.c](pp-unknown-escape-sequence.c) |
| `UnterminatedEscapeSequence` | reached; folded | [pp-unterminated-escape-sequence.c](pp-unterminated-escape-sequence.c) |
| `InvalidHexEscapeSequence` | rendered | [pp-invalid-hex-escape-sequence.c](pp-invalid-hex-escape-sequence.c) |
| `HexEscapeSequenceTooLarge` | rendered | [pp-hex-escape-sequence-too-large.c](pp-hex-escape-sequence-too-large.c) |
| `InvalidOctalEscapeSequence` | unreachable by range analysis | The decoder reads at most three octal digits: 0..511. Every resulting number is a valid Unicode scalar, so char::try_from cannot fail. |
| `OctalEscapeSequenceTooLarge` | unreachable by range analysis | At most three octal digits are accumulated in u16. A maximum of 511 cannot overflow u16. |
| `InvalidSmallUnicodeEscapeSequence` | rendered | [pp-invalid-small-unicode-escape-sequence.c](pp-invalid-small-unicode-escape-sequence.c) |
| `SmallUnicodeEscapeSequenceTooShort` | rendered | [pp-small-unicode-escape-sequence-too-short.c](pp-small-unicode-escape-sequence-too-short.c) |
| `InvalidLargeUnicodeEscapeSequence` | rendered | [pp-invalid-large-unicode-escape-sequence.c](pp-invalid-large-unicode-escape-sequence.c) |
| `LargeUnicodeEscapeSequenceTooSmall` | rendered | [pp-large-unicode-escape-sequence-too-small.c](pp-large-unicode-escape-sequence-too-small.c) |
| `MultiCharacterLiteralsUnsupported` | rendered | [pp-multi-character-literals-unsupported-empty.c](pp-multi-character-literals-unsupported-empty.c), [pp-multi-character-literals-unsupported.c](pp-multi-character-literals-unsupported.c) |
| `RedefinitionOfFunctionLikeMacroAsObjectLikeMacro` | rendered | [pp-redefinition-of-function-like-macro-as-object-like-macro.c](pp-redefinition-of-function-like-macro-as-object-like-macro.c). A warning, an error under `-pedantic-errors`, as in GCC and Clang; the changed form is not also reported as a different definition. |
| `RedefinitionOfObjectLikeMacroAsFunctionLikeMacro` | rendered | [pp-redefinition-of-object-like-macro-as-function-like-macro.c](pp-redefinition-of-object-like-macro-as-function-like-macro.c). A warning, an error under `-pedantic-errors`. |
| `ExpectedIdentifierInMacroDefinition` | rendered | [pp-expected-identifier-in-macro-definition.c](pp-expected-identifier-in-macro-definition.c) |
| `VariadicMacroMustBeLastParameter` | rendered | [pp-variadic-macro-must-be-last-parameter.c](pp-variadic-macro-must-be-last-parameter.c) |
| `DuplicateMacroParameter` | rendered | [pp-duplicate-macro-parameter.c](pp-duplicate-macro-parameter.c) |
| `MissingWhitespaceAfterMacroName` | rendered | [pp-missing-whitespace-after-macro-name.c](pp-missing-whitespace-after-macro-name.c) |
| `VaArgsOutsideVariadicMacro` | rendered | [pp-va-args-outside-variadic-macro.c](pp-va-args-outside-variadic-macro.c) |
| `VaOptOutsideVariadicMacro` | rendered | [pp-va-opt-outside-variadic-macro.c](pp-va-opt-outside-variadic-macro.c) |
| `ExpectedCommaOrClosingParenthesisInMacroDefinition` | rendered | [pp-expected-comma-or-closing-parenthesis-in-macro-definition.c](pp-expected-comma-or-closing-parenthesis-in-macro-definition.c) |
| `MacroRedefinedWithDifferentDefinition` | rendered | [pp-macro-redefined-with-different-definition.c](pp-macro-redefined-with-different-definition.c) is the default warning, beside accepted redefinitions that differ only in whitespace amount or comments; [pp-system-header-diagnostics.c](pp-system-header-diagnostics.c) shows the `-pedantic-errors` error in a user file and the same redefinition withheld in a system header. |
| `ExpectedIdentifierInUndefDirective` | rendered | [pp-expected-identifier-in-undef-directive.c](pp-expected-identifier-in-undef-directive.c) |
| `ExpectedNewlineAfterUndefDirective` | rendered | [pp-expected-newline-after-undef-directive.c](pp-expected-newline-after-undef-directive.c) |
| `HashOperatorMustBeFollowedByAMacroArgument` | rendered | [pp-hash-operator-must-be-followed-by-a-macro-argument.c](pp-hash-operator-must-be-followed-by-a-macro-argument.c) |
| `IdentifierNotMacroArgumentAfterHashOperator` | rendered | [pp-identifier-not-macro-argument-after-hash-operator.c](pp-identifier-not-macro-argument-after-hash-operator.c) |
| `MissingRightHandSideOfHashHashOperator` | rendered | [pp-missing-right-hand-side-of-hash-hash-operator.c](pp-missing-right-hand-side-of-hash-hash-operator.c), a first definition and its identical redefinition |
| `MissingLeftHandSideOfHashHashOperator` | rendered | [pp-missing-left-hand-side-of-hash-hash-operator.c](pp-missing-left-hand-side-of-hash-hash-operator.c), a first definition and its identical redefinition |
| `TokenMergingError` | rendered | [pp-token-merging-error.c](pp-token-merging-error.c) |
| `MissingNumberInLineDirective` | rendered | [pp-missing-number-in-line-directive.c](pp-missing-number-in-line-directive.c) |
| `MissingNewlineAfterLineDirective` | rendered | [pp-missing-newline-after-line-directive.c](pp-missing-newline-after-line-directive.c) |
| `LineDirectiveIsNotASimpleDigitSequence` | rendered | [pp-line-directive-is-not-a-simple-digit-sequence.c](pp-line-directive-is-not-a-simple-digit-sequence.c), [pp-line-directive-suffix-panic.c](pp-line-directive-suffix-panic.c) |
| `LineDirectiveNumberTooLarge` | rendered | [pp-line-directive-number-too-large.c](pp-line-directive-number-too-large.c) |
| `LineDirectiveNumberZero` | rendered | [pp-line-directive-number-zero.c](pp-line-directive-number-zero.c) |
| `WideStringInLineDirective` | rendered | [pp-wide-string-in-line-directive.c](pp-wide-string-in-line-directive.c) |
| `EncodedStringInLineDirective` | rendered | [pp-encoded-string-in-line-directive.c](pp-encoded-string-in-line-directive.c) |
| `MissingOpeningParenthesisInPragmaOperator` | rendered | [pp-missing-opening-parenthesis-in-pragma-operator.c](pp-missing-opening-parenthesis-in-pragma-operator.c) |
| `MissingClosingParenthesisInPragmaOperator` | rendered | [pp-missing-closing-parenthesis-in-pragma-operator.c](pp-missing-closing-parenthesis-in-pragma-operator.c) |
| `MissingStringLiteralInPragmaOperator` | rendered | [pp-missing-string-literal-in-pragma-operator.c](pp-missing-string-literal-in-pragma-operator.c) |
| `UnknownPragmaDirective` | rendered | [pp-unknown-pragma-directive.c](pp-unknown-pragma-directive.c) |
| `UnknownPragmaSTDCArgument` | rendered | [pp-unknown-pragma-s-t-d-c-argument.c](pp-unknown-pragma-s-t-d-c-argument.c) |
| `ExtraTokensAfterPragmaOnce` | rendered | [pp-extra-tokens-after-pragma-once.c](pp-extra-tokens-after-pragma-once.c) |
| `ExtraTokensAfterPragmaOperator` | rendered | [pp-extra-tokens-after-pragma-operator.c](pp-extra-tokens-after-pragma-operator.c) |
| `ExtraTokensAfterIncludeDirective` | rendered | [pp-extra-tokens-after-include-directive.c](pp-extra-tokens-after-include-directive.c). A written angle name followed immediately by `>` or `=` (which ordinary lexing reads as `>>` or `>=`) warns at the first character after the closing `>`. While the quoted-backslash extension is accepted, non-whitespace text after the first closing quote also warns at its first character; whitespace alone does not. Both cases are covered by `preprocessing/tests/header_name_regressions.rs`. |
| `ExtraTokensAfterIfdefDirective` | rendered | [pp-extra-tokens-after-ifdef-directive.c](pp-extra-tokens-after-ifdef-directive.c) |
| `ExtraTokensAfterIfndefDirective` | rendered | [pp-extra-tokens-after-ifndef-directive.c](pp-extra-tokens-after-ifndef-directive.c) |
| `STDCPragmaDirectiveWithoutArgument` | rendered | [pp-s-t-d-c-pragma-directive-without-argument.c](pp-s-t-d-c-pragma-directive-without-argument.c) |
| `STDCPragmaDirectiveWithoutOnOffSwitch` | rendered | [pp-s-t-d-c-pragma-directive-without-on-off-switch.c](pp-s-t-d-c-pragma-directive-without-on-off-switch.c) |
| `MissingOnOffSwitchInSTDCPragma` | rendered | [pp-missing-on-off-switch-in-s-t-d-c-pragma.c](pp-missing-on-off-switch-in-s-t-d-c-pragma.c) |
| `PragmaOnceInNonHeader` | rendered | [pp-pragma-once-in-non-header.c](pp-pragma-once-in-non-header.c) |
| `IncludeNextInPrimarySource` | rendered | [pp-include-next-in-primary-source.c](pp-include-next-in-primary-source.c) covers `#include_next` and `__has_include_next`; both then search as `#include` would, as GCC and Clang do. |
| `IncludeNextWithoutSearchEntry` | rendered | [pp-include-next-without-search-entry.c](pp-include-next-without-search-entry.c), through `include-next-relative.h`, which is found beside the primary source file and so has no search entry, as in Clang's `-Winclude-next-absolute-path`. |
| `ErrorDirective` | rendered | [pp-error-directive.c](pp-error-directive.c) |
| `PragmaMessage` | rendered | [pp-pragma-message.c](pp-pragma-message.c) covers `#pragma GCC warning` and `#pragma GCC error`, with a macro-replaced, a parenthesized and concatenated, and a malformed operand. |
| `SystemHeaderPragmaInMainFile` | rendered | [pp-system-header-pragma-in-main-file.c](pp-system-header-pragma-in-main-file.c) |

### ParserErrorType

| Variant | Status | Fixture or reason |
| --- | --- | --- |
| `EmptyTranslationUnit` | rendered | [parser-empty-translation-unit.c](parser-empty-translation-unit.c), [tokenizer-angle-header-eof.c](tokenizer-angle-header-eof.c) |
| `ResourceLimitExceeded` | reachable; not covered | The CLI uses the default memory/frame budgets and does not expose smaller budgets. Reaching them needs a deliberately large input, contrary to the small-fixture scope. Existing unit tests lower the budgets; no tiny C-only default-CLI fixture is claimed. |
| `ParserFrameConsumedAtEndOfInput` | internal invariant; not covered | Reports a parser-machine bug rather than a malformed C production. No reproducer was found, and it should not be manufactured by changing parser state. |
| `ExpectedFunctionBody` | rendered | [parser-expected-function-body.c](parser-expected-function-body.c) |
| `DeclarationListAfterParameterTypeList` | rendered | [parser-declaration-list-after-parameter-type-list.c](parser-declaration-list-after-parameter-type-list.c) |
| `ExpectedOpeningCurlyBraceInCompoundStatement` | believed unreachable from normal dispatch | The function/statement owners check for { before pushing this frame; a missing function brace is handled by ExpectedFunctionBody. The fallback is defensive frame-entry validation. |
| `ExpectedClosingCurlyBraceInCompoundStatement` | rendered | [parser-expected-closing-curly-brace-in-compound-statement.c](parser-expected-closing-curly-brace-in-compound-statement.c) |
| `ExpectedStatement` | rendered | [parser-expected-statement.c](parser-expected-statement.c) |
| `ExpectedGotoLabel` | rendered | [parser-expected-goto-label.c](parser-expected-goto-label.c) |
| `ExpectedStatementExpression` | rendered | [parser-expected-statement-expression-assignment.c](parser-expected-statement-expression-assignment.c), [parser-expected-statement-expression-macro.c](parser-expected-statement-expression-macro.c), [parser-expected-statement-expression-operand.c](parser-expected-statement-expression-operand.c), [parser-expected-statement-expression-operator.c](parser-expected-statement-expression-operator.c), [parser-expected-statement-expression.c](parser-expected-statement-expression.c) |
| `ExpectedMemberIdentifier` | rendered | [parser-expected-member-identifier.c](parser-expected-member-identifier.c) |
| `ExpectedClosingSquareBracketInSubscript` | rendered | [parser-expected-closing-square-bracket-in-subscript.c](parser-expected-closing-square-bracket-in-subscript.c) |
| `ExpectedClosingSquareBracketInArrayDesignator` | rendered | [parser-expected-closing-square-bracket-in-array-designator.c](parser-expected-closing-square-bracket-in-array-designator.c) |
| `ExpectedClosingCurlyBraceInInitializerList` | rendered | [parser-expected-closing-curly-brace-in-initializer-list.c](parser-expected-closing-curly-brace-in-initializer-list.c) |
| `ExpectedEqualsAfterInitializerDesignation` | rendered | [parser-expected-equals-after-initializer-designation.c](parser-expected-equals-after-initializer-designation.c) |
| `ExpectedOpeningParenthesisInStatement` | rendered | [parser-expected-opening-parenthesis-in-statement.c](parser-expected-opening-parenthesis-in-statement.c) |
| `ExpectedClosingParenthesisInStatement` | rendered | [parser-expected-closing-parenthesis-in-statement.c](parser-expected-closing-parenthesis-in-statement.c) |
| `ExpectedSemicolonInStatement` | rendered | [parser-expected-semicolon-in-statement.c](parser-expected-semicolon-in-statement.c) |
| `ExpectedColonInLabel` | rendered | [parser-expected-colon-in-label.c](parser-expected-colon-in-label.c) |
| `DuplicateDefaultLabel` | rendered | [parser-duplicate-default-label.c](parser-duplicate-default-label.c) |
| `ExpectedWhileAfterDoBody` | rendered | [parser-expected-while-after-do-body.c](parser-expected-while-after-do-body.c) |
| `ExpectedDeclaratorInTypedef` | rendered | [parser-expected-declarator-in-typedef.c](parser-expected-declarator-in-typedef.c) |
| `TypedefDeclaresNoName` | rendered | [parser-typedef-declares-no-name.c](parser-typedef-declares-no-name.c) |
| `TccLabelAtBlockEnd` | rendered | [parser-tcc-label-at-block-end.c](parser-tcc-label-at-block-end.c) |
| `ExpectedDeclaratorInDeclaration` | reached; folded | [parser-expected-declarator-in-declaration-include.c](parser-expected-declarator-in-declaration-include.c), [parser-expected-declarator-in-declaration.c](parser-expected-declarator-in-declaration.c) |
| `ExpectedDeclarationContinuationAfterDeclarator` | rendered | [parser-expected-declaration-continuation-after-declarator-long-double.c](parser-expected-declaration-continuation-after-declarator-long-double.c), [parser-expected-declaration-continuation-after-declarator-missing-semicolon.c](parser-expected-declaration-continuation-after-declarator-missing-semicolon.c), [parser-expected-declaration-continuation-after-declarator-tab.c](parser-expected-declaration-continuation-after-declarator-tab.c), [parser-expected-declaration-continuation-after-declarator.c](parser-expected-declaration-continuation-after-declarator.c) |
| `UnexpectedEndBeforeDeclarationSpecifier` | believed unreachable from normal dispatch | Owners only push declaration-specifier parsing after recognizing a declaration starter, or after a token has already been consumed. EOF without a starter is handled by the owner; EOF after consumed specifiers uses UnexpectedEndBeforeTypeSpecifier. |
| `UnexpectedEndBeforeTypeSpecifier` | rendered | [parser-unexpected-end-before-type-specifier.c](parser-unexpected-end-before-type-specifier.c) |
| `DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis` | rendered | [parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c](parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c), [parser-macro-expanded-keyword.c](parser-macro-expanded-keyword.c), [parser-thread-keyword-as-name.c](parser-thread-keyword-as-name.c) (the note names `__thread` as written) |
| `ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator` | reached; folded | [parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.c](parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.c), [parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.c](parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.c) |
| `ExpectedClosingParenthesisAfterParenthesizedDeclarator` | rendered | [parser-expected-closing-parenthesis-after-parenthesized-declarator.c](parser-expected-closing-parenthesis-after-parenthesized-declarator.c) |
| `ExpectedClosingSquareBracketInArrayDirectDeclarator` | rendered | [parser-expected-closing-square-bracket-in-array-direct-declarator.c](parser-expected-closing-square-bracket-in-array-direct-declarator.c) |
| `UnexpectedEndOfFunctionDeclaratorParameterList` | rendered | [parser-unexpected-end-of-function-declarator-parameter-list.c](parser-unexpected-end-of-function-declarator-parameter-list.c) |
| `ExpectedIdentifierInKAndRFunctionDeclaratorParameterList` | rendered | [parser-expected-identifier-in-k-and-r-function-declarator-parameter-list.c](parser-expected-identifier-in-k-and-r-function-declarator-parameter-list.c) |
| `ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList` | rendered | [parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.c](parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.c) |
| `ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList` | rendered | [parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.c](parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.c) |
| `ExpectedParameterDeclarationAfterCommaInFunctionDeclarator` | rendered | [parser-expected-parameter-declaration-after-comma-in-function-declarator.c](parser-expected-parameter-declaration-after-comma-in-function-declarator.c) |
| `ExpectedCommaOrClosingParenthesisInFunctionCall` | reached; folded | [parser-expected-comma-or-closing-parenthesis-in-function-call.c](parser-expected-comma-or-closing-parenthesis-in-function-call.c) |
| `ExpectedStructOrUnionKeyword` | believed unreachable from normal dispatch | The specifier owner selects this frame only for a verified struct or union token. The frame-entry check is defensive. |
| `StructOrUnionSpecifierWithoutNameAndBody` | rendered | [parser-struct-or-union-specifier-without-name-and-body.c](parser-struct-or-union-specifier-without-name-and-body.c) |
| `ExpectedClosingCurlyBraceInStructDeclarationList` | rendered | [parser-expected-closing-curly-brace-in-struct-declaration-list.c](parser-expected-closing-curly-brace-in-struct-declaration-list.c) |
| `ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList` | rendered | [parser-expected-semicolon-before-closing-curly-brace-in-struct-declarator-list.c](parser-expected-semicolon-before-closing-curly-brace-in-struct-declarator-list.c) |
| `ExpectedCommaOrSemicolonInStructDeclaratorList` | rendered | [parser-expected-comma-or-semicolon-in-struct-declarator-list.c](parser-expected-comma-or-semicolon-in-struct-declarator-list.c) |
| `ExpectedEnumKeyword` | believed unreachable from normal dispatch | The specifier owner selects this frame only for a verified enum token. The frame-entry check is defensive. |
| `EnumSpecifierWithoutNameAndBody` | rendered | [parser-enum-specifier-without-name-and-body.c](parser-enum-specifier-without-name-and-body.c) |
| `ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList` | rendered | [parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.c](parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.c) |
| `ExpectedEnumeratorBeforeClosingCurlyBrace` | rendered | [parser-expected-enumerator-before-closing-curly-brace.c](parser-expected-enumerator-before-closing-curly-brace.c) |
| `ExpectedCommaOrClosingCurlyInEnumeratorList` | rendered | [parser-expected-comma-or-closing-curly-in-enumerator-list.c](parser-expected-comma-or-closing-curly-in-enumerator-list.c) |
| `StorageClassRedefinition` | rendered | [parser-storage-class-redefinition.c](parser-storage-class-redefinition.c) |
| `ThreadStorageConflict` | rendered | [parser-thread-storage-conflict.c](parser-thread-storage-conflict.c): `register`, `typedef`, `constexpr` and the other spelling beside a thread storage-class specifier (C11 §6.7.1p2) |
| `ThreadLocalSpecifiedTwice` | rendered | [parser-thread-local-specified-twice.c](parser-thread-local-specified-twice.c): `__thread` twice, and `_Thread_local` with its C23 alias |
| `DeclarationSpecifierNotAllowedHere` | rendered | [parser-declaration-specifier-not-allowed-here.c](parser-declaration-specifier-not-allowed-here.c) |
| `ConstSpecifiedTwice` | rendered | [parser-const-specified-twice.c](parser-const-specified-twice.c) |
| `VolatileSpecifiedTwice` | rendered | [parser-volatile-specified-twice.c](parser-volatile-specified-twice.c) |
| `RestrictSpecifiedTwice` | rendered | [parser-restrict-specified-twice.c](parser-restrict-specified-twice.c) |
| `InlineSpecifiedTwice` | rendered | [parser-inline-specified-twice.c](parser-inline-specified-twice.c) |
| `StaticSpecifiedTwice` | rendered | [parser-static-specified-twice.c](parser-static-specified-twice.c) |
| `TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator` | rendered | [parser-type-qualifiers-both-before-and-after-static-in-array-direct-declarator.c](parser-type-qualifiers-both-before-and-after-static-in-array-direct-declarator.c) |
| `ConflictingTypeSpecifiers` | rendered | [parser-conflicting-type-specifiers-typedef.c](parser-conflicting-type-specifiers-typedef.c), [parser-conflicting-type-specifiers.c](parser-conflicting-type-specifiers.c) |
| `TypeSpecifierSpecifiedTwice` | rendered | [parser-type-specifier-specified-twice.c](parser-type-specifier-specified-twice.c) |
| `UnsupportedImaginaryTypeSpecifier` | rendered | [parser-unsupported-imaginary-type-specifier.c](parser-unsupported-imaginary-type-specifier.c) |
| `LongSpecifiedThrice` | rendered | [parser-long-specified-thrice.c](parser-long-specified-thrice.c) |
| `LongLongDoubleSpecified` | rendered | [parser-long-long-double-specified.c](parser-long-long-double-specified.c) |
| `EmptyDeclarationSpecifiers` | rendered | [parser-empty-declaration-specifiers.c](parser-empty-declaration-specifiers.c) |
| `NoTypeSpecifiersInDeclarationSpecifiers` | rendered | [parser-no-type-specifiers-in-declaration-specifiers.c](parser-no-type-specifiers-in-declaration-specifiers.c), [parser-unicode-identifier.c](parser-unicode-identifier.c) |
| `BothStaticAndPointerInArrayDirectDeclarator` | rendered | [parser-both-static-and-pointer-in-array-direct-declarator.c](parser-both-static-and-pointer-in-array-direct-declarator.c) |
| `ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator` | rendered | [parser-expected-assignment-expression-after-static-in-array-direct-declarator.c](parser-expected-assignment-expression-after-static-in-array-direct-declarator.c) |
| `ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator` | unreachable with current lookahead | is_array_pointer_marker recognizes * only when the following token is ]. Therefore ArrayExpectClose begins with ] and cannot see an unexpected token. |
| `UnexpectedEndOfArrayDeclaratorAfterPointer` | unreachable with current lookahead | Entering ArrayExpectClose requires a buffered following ], so EOF cannot immediately follow the recognized marker. |
| `PointerSpecifiedTwice` | unreachable with current lookahead | A second * prevents the first * from being recognized as an array pointer marker. The input instead enters ordinary bound-expression parsing. |
| `TypeQualifiersWithoutDeclarator` | reached; folded | [parser-type-qualifiers-without-declarator.c](parser-type-qualifiers-without-declarator.c) |
| `TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator` | rendered | [parser-type-qualifiers-before-pointer-in-array-abstract-direct-declarator.c](parser-type-qualifiers-before-pointer-in-array-abstract-direct-declarator.c) |
| `KAndRFunctionDeclaratorMixedWithModernDeclarator` | rendered | [parser-k-and-r-function-declarator-mixed-with-modern-declarator.c](parser-k-and-r-function-declarator-mixed-with-modern-declarator.c) |
| `ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList` | rendered | [parser-expected-closing-parenthesis-after-ellipsis-in-function-declarator-parameter-list.c](parser-expected-closing-parenthesis-after-ellipsis-in-function-declarator-parameter-list.c) |
| `UnexpectedEndOfVariadicFunctionDeclaratorParameterList` | rendered | [parser-unexpected-end-of-variadic-function-declarator-parameter-list.c](parser-unexpected-end-of-variadic-function-declarator-parameter-list.c) |
| `MemberDeclaresNothing` | rendered | [parser-empty-struct-declarator.c](parser-empty-struct-declarator.c) |

## Folded diagnostic evidence

- `parser::ExpectedDeclaratorInDeclaration`: reported after the more specific direct-declarator failure at the same token.
- `parser::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator`: reported after the nested direct-declarator failure at the same token.
- `parser::ExpectedCommaOrClosingParenthesisInFunctionCall`: reported after the expression failure at the same token.
- `parser::TypeQualifiersWithoutDeclarator`: reported after the direct-declarator failure at the same token.
- `pp::UnterminatedEscapeSequence`: reported on the same literal as the tokenizer EOF error.
- `pp::BinaryPlusWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::BinaryMinusWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::MultiplyWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::DivideWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::ModuloWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::LessThanWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::LessThanEqualsWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::GreaterThanWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::GreaterThanEqualsWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::EqualsWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::NotEqualsWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::LeftShiftWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::RightShiftWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::BitwiseAndWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::BitwiseXorWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::BitwiseOrWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::LogicalAndWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.
- `pp::LogicalOrWithoutRhs`: the end-of-condition RHS diagnostic and reducer diagnostic are folded together.

## Tested source snapshot

The scratch copy uses committed build configuration from `0165497c79a1fd577ec85df4327a8f7a0e7f4f60`, with `use cmake as _;` appended to the build script, and current uncommitted compiler sources. Source hashes pin the reviewed snapshot:

- `src/diagnostics.rs`: `e029e74d45063f6f8ea90c7c94eda4bbc632e7bf7052d130bf76e28120624c4d`
- `src/lib.rs`: `46e44180593237429184ca9e667abf59734236673bafa4bf45155b4b76bc59bc`
- `src/translation_phases/initial_processing.rs`: `9dc233b0a8f57374cb1ba0d925b097a497a95b9c7a70cf5e524e2d949b505783`
- `src/translation_phases/preprocessor_tokenizer.rs`: `75938a65383fe3013753df2b540971c1616cbc02039e63e55fc4bc032c4b9a0f`
- `src/translation_phases/preprocessing.rs`: `297ba822934e344fa1bead583aca1fc5fc5ccc2ac04ac453ee6a5c932754fb0c`
- `src/translation_phases/parsing.rs`: `b28b06587d7363d6ce812b0b1348fe7f708a6c721d00651faa70933ce5dbba5f`

## Language-standard foundation

The CLI now defaults to GNU17; configured library tests remain strict C99.
`TranslationError::Extension` is shared by all phases and renders standard/GNU/
MSVC origin with policy-selected severity. Keyword classification tests cover
Allow/Warn/Deny and original alternate spellings. CLI goldens under
`language/` cover policy severity and macro-expansion provenance. Newly recognized
unsupported keywords also have parser recovery coverage; recognition is separate
from implementing their grammar. See the root language-standards.md matrix.

## ISO phase-7 parser modes

The ISO parser recognizes later-standard syntax while shared extension diagnostics
select Allow/Warn/Deny. Grammar-only C99 extensions have exact warning/error CLI
goldens in [language/iso-c89-warning.stderr](language/iso-c89-warning.stderr) and
[language/iso-c89-error.stderr](language/iso-c89-error.stderr); native revision CLI
samples produce no diagnostics. Reserved-keyword origin diagnostics remain the
foundation's responsibility and are not duplicated by the parser.

| Diagnostic surface | Status | Evidence |
| --- | --- | --- |
| `ExpectedIsoSyntax` attribute delimiter/component | rendered | [parser-iso-attribute.c](parser-iso-attribute.c), malformed-name and balanced-argument unit tests |
| `ExpectedIsoSyntax` unavailable case range | rendered | [strict C99](parser-case-range-c99.c), [strict C17](parser-case-range-c17.c); unit tests cover every policy, suppression, retained range syntax and following input |
| Case-range C2y origin / native silence | rendered | [GNU17 warning](parser-case-range-gnu17.c), [native C2y](language/iso-c2y.c); GNU parser Warn/Deny goldens use C2y origin for ranges |
| `ExpectedIsoSyntax` static assertion message | rendered | [parser-iso-static-assert.c](parser-iso-static-assert.c) |
| Generic association expression | rendered | [parser-iso-generic.c](parser-iso-generic.c) |
| Grammar-only ISO origin/severity | rendered | C89 warning/error goldens above; unit matrix exercises all policies through C2y |
| Missing type and Unicode label | rendered | Existing missing-type fixtures now use struct members: ordinary missing declaration types are ImplicitInt extensions under the default Allow policy |
| Atomic/BitInt operands, fixed enum types, selection headers, attribute names and EOF | parser-tested | `parsing::tests::standards`: following declarations survive, every prefix terminates with restored scopes, and deep nesting avoids native recursion |

The ISO diagnostics describe the required production component instead of treating
an attribute delimiter or static-assert message as an expression. No diagnostic
is based on evaluating an assertion, inferring a type, applying attributes or
resolving a named control target. The new mode suite also pins AST provenance
through macros and compaction. Zero-global-allocation tests exercise the new ISO
frame paths and the rendered diagnostic corpus.

## GNU phase-7 parser modes

Empty structures/unions, statement expressions and nested functions now retain
complete GNU syntax. Strict modes report shared policy diagnostics rather than
repairing supported grammar. The removed empty-aggregate syntax-error fixture is
covered by `EmptyStructs` warning/error cases in `language/gnu-parser.c`.

| Diagnostic surface | Status | Evidence |
| --- | --- | --- |
| Assembly required operand/delimiter | rendered | [parser-gnu-asm.c](parser-gnu-asm.c) |
| Builtin type operand | rendered | [parser-gnu-builtin.c](parser-gnu-builtin.c) |
| GNU attribute parentheses | rendered | [parser-gnu-attribute.c](parser-gnu-attribute.c) |
| GNU origins and policy severity | rendered | [language/gnu-parser-warning.stderr](language/gnu-parser-warning.stderr), [language/gnu-parser-error.stderr](language/gnu-parser-error.stderr) |
| Extra `;` at file scope and in a member list | rendered | [parser-gnu-extra-semicolon.c](parser-gnu-extra-semicolon.c), including the `;` an empty Windows SDK macro leaves behind; `parsing::tests::gnu` covers every mode and policy |
| Extension suppression and macro occurrences | parser-tested | `parsing::tests::gnu` checks scoped suppression and later diagnostics |
| Recovery, EOF, nesting and provenance | parser-tested | every prefix, malformed children followed by a declaration, explicit AST/inspection checks and deep frame tests |

Assembly target rules, builtin semantics, layout and attribute application remain
analysis responsibilities. All three canonical diagnostic fixtures require a
production component and preserve following valid input.

## MSVC phase-7 parser modes

The eight parser-owned groups have independent configuration gates. Enabled
keywords report shared MSVC policy diagnostics; disabled spellings remain
identifiers. The parser preserves ASTs under Deny and reports malformed vendor
syntax with `ExpectedMsSyntax`, using the owning production's required component.

| Diagnostic surface | Evidence |
| --- | --- |
| MSVC origins and Warn/Deny severity | [language/msvc-parser-warning.stderr](language/msvc-parser-warning.stderr), [language/msvc-parser-error.stderr](language/msvc-parser-error.stderr) |
| Missing declspec opener, SEH handler, leave semicolon, asm delimiter | [language/msvc-recovery.stderr](language/msvc-recovery.stderr) |
| EOF, malformed filter/body, delimiter recovery and following input | `parsing::tests::msvc` feature, prefix and recovery suites |

CLI tests pass MSVC flags explicitly for these supplementary language goldens.
The top-level default-mode golden inventory above is unchanged. Enabled/disabled
token snapshots also pin provenance and identifier preservation. Arena allocation
checks exercise valid and recovered MSVC syntax.


## Lexical and preprocessing modes

The lexpp fixtures added on 8 October 2026 render standard, GNU and MSVC
behavior selected by their `.args` files:

| Fixture | Flags | Observed coverage |
| --- | --- | --- |
| [C89 extensions](lexpp-c89-extensions.c) | `-std=c89 -pedantic` | Variadic definitions, empty fixed and variadic arguments, long-long integer suffixes and hexadecimal floats report C99 origin. |
| [GNU89 extensions](lexpp-gnu89-extensions.c) | `-std=gnu89 -pedantic` | Named variadic macros, dollar identifiers, binary constants, line comments, `#warning`, `#ident`, and counter expansion retain spelling/provenance and policy severity. |
| [C23 constraints](lexpp-c23-constraints.c) | `-std=c23` | Incompatible string encodings, a UTF-8 character needing multiple code units, an invalid `__VA_OPT__` paste boundary, missing resource inclusion, a query outside conditional inclusion, and following declaration recovery. |
| [C2y escapes](lexpp-c2y-escapes.c) | `-std=c2y` | Empty, invalid-radix and excessive numeric delimited escapes with following declaration recovery. |
| [MSVC pragma](lexpp-ms-pragma.c) | `-std=c17 -fms-pragma -pedantic` | Independent MSVC operator policy diagnostic plus the existing structured pragma-switch diagnostic at the original payload. |

`TranslationError::Extension` is exercised by these rendered goldens and by the
seven-revision strict/GNU snapshot in
[`language_modes_lexpp.snap`](../lexing/language_modes_lexpp.snap). Structured
preprocessor tests cover Allow/Warn/Deny, C89/C95 lexical boundaries, all modern
literal gates and values, GNU suffix orders, native and extension directives,
query results, real include-next/resource paths (including punctuation and dollar
header characters, macro-generated names, and missing operand/quote recovery),
every standard optional-paste
example, independent MSVC comma elision, and malformed/truncated input. The
allocation harness reads the same `.args` before its measured compiler/reporting
intervals; it still requires every golden fixture to produce diagnostics and
requires zero global allocations while rendering them.

## Integrated language-mode CLI coverage

[`tests/integration/language_cli.rs`](../../integration/language_cli.rs) checks every standard alias
through macro expansion, parsing and syntax inspection. It covers C attribute
queries with macro-produced attribute specifiers, GNU keywords and imaginary
constants, embedded bytes as initializer elements, and MSVC macro pragmas with
empty variadic calls and parser syntax. Disabled feature flags and older literal
gates diagnose while preserving a following declaration. Malformed query operands,
missing resources, pragma payloads, imaginary suffixes and attributes also retain
following declarations. Existing lexpp/parser policy and recovery goldens remain
active. The [combined policy golden](language-integration-policy.stderr) pins GNU
imaginary, C attribute query/grammar and MSVC pragma/declspec warnings in one
translation unit. This integration adds no diagnostic kinds or severity exemptions.

### Final language-mode review

`language-extension-suppression` retains exactly one unsuppressed macro keyword
warning while filtering declaration/function/expression marker occurrences.
The fixture also exercises zero-global-allocation parsing and rendering.
C23 function grammar, repeated enum-underlying-type recovery, preprocessing query
boundaries, named GNU variadic arguments, builtin overrides and integer mode
selection have structured unit regressions; the CLI checks numeric opaque-token
spellings without internal NUL sentinels.

## Declaration semantic diagnostics

The default CLI appends declaration-semantic diagnostics after preprocessing and
syntax diagnostics. Syntax/token inspection remains a syntax-only path. Each
fixture below runs with `-std=c99`, adding `-pedantic-errors` where it shows an
extension policy diagnostic; each `.stderr` pins the source range, source
spelling, previous-declaration label where applicable, and C99 note. Positive
counterparts live in `semantic_analysis/tests.rs` and the semantic inspection
snapshot at `tests/fixtures/semantic/types.stderr`.

| Symbolic kind | Golden | Constraint |
| --- | --- | --- |
| `InvalidStorage` | [storage](sema-storage.c) | File-scope auto/register. |
| `InvalidFunctionStorage` | [function storage](sema-function-storage.c) | Block-scope function storage other than extern. |
| `ThreadLocalFunction`, `ThreadLocalBlockScope`, `ThreadLocalAfterNonThreadLocal`, `NonThreadLocalAfterThreadLocal` | [thread storage](sema-thread-local.c) | C11 §6.7.1p3-4 with `-std=c11`: a thread-local function, a block-scope thread-local object without `static`/`extern`, and declarations disagreeing on thread storage. The same fixture shows a thread-local parameter, a member, and a thread-local address in a static initializer through their existing kinds. |
| `LinkedBlockInitializer` | [linked initializer](sema-linked-initializer.c) | Block-scope declaration with linkage and an initializer; the label marks the initializer. |
| `InvalidRestrict` | [restrict](sema-restrict.c) | Restrict needs an object/incomplete-target pointer. |
| `QualifiedFunction` | [function qualifier](sema-function-qualifier.c) | Warning for undefined behavior from qualifying a function typedef; qualifiers are ignored. |
| `InvalidInline` | [inline](sema-inline.c) | Inline objects, typedef names and main. |
| `InlineParameter` | [inline-parameter](sema-inline-parameter.c) | Inline on named and unnamed parameters. |
| `InvalidDerivedType` | [derived](sema-derived.c) | Invalid array elements/function results. |
| `InvalidArrayBound` | [bound](sema-array-bound.c) | Negative constant and non-integer bounds. Zero bounds use shared extension policy. |
| `InvalidStarBound` | [star bound](sema-star-bound.c) | `[*]` outside function prototype scope; a definition's parameters use `DefinitionStarArray`. |
| `ObjectTooLarge` | [object size](sema-object-size.c) | Arrays and records beyond the PTRDIFF_MAX implementation limit. |
| `FileScopeVariableType` | [VLA](sema-file-vla.c) | Runtime file bound, linked/static VLA constraints and variably modified members. |
| `IncompatibleDeclaration` | [incompatible](sema-incompatible.c) | Incompatible redeclaration and previous source. |
| `DuplicateDeclaration` | [duplicate](sema-duplicate.c) | Repeated no-linkage declarations, including local then extern. |
| `DuplicateDeclaration` | [typedef redefinition](sema-typedef-redefinition.c) | A typedef repeated with a different type; repeating the same type is the C11 extension before C11, as in GCC and Clang. |
| `ConflictingLinkage` | [linkage](sema-linkage.c) | Internal/external linkage mix. Positive extern-after-static is in unit tests. |
| `TagKindMismatch` | [tag kind](sema-tag-kind.c) | Tag namespace kind conflicts. |
| `TagRedefinition` | [tag definition](sema-tag-redefinition.c) | Completing an already complete tag. |
| `Extension(ForwardEnumReference)` | [enum declaration](sema-enum-incomplete.c) | An enum tag without a prior completion under `-pedantic-errors`; accepted silently otherwise, as Clang does. |
| `InvalidConstant` | [constant](sema-constant.c) | Runtime enumerator operand, including unselected conditional/logical arms. |
| `ConstantOverflow` | [overflow](sema-overflow.c) | Exceptional ICE evaluation; no dependent enumerator cascade. |
| `EnumeratorRange` | [enum range](sema-enum-range.c) | Members that no 64-bit integer type holds; under `-pedantic-errors` the C23 extension diagnostic for values outside int precedes it. |
| `InvalidMember` | [member](sema-member.c) | Incomplete/function member and a flexible array that is not the last member. |
| `Extension(FlexibleArrayExtensions)` | [flexible extension](sema-flexible-extension.c) | GNU flexible arrays in unions or otherwise empty structures, nested flexible structures and arrays of them (§6.7.2.1p2, p16). |
| `FlexibleArrayBraces`, `FlexibleArrayDesignator` | [flexible designators](sema-flexible-designators.c) | A designated flexible array member initialized by a scalar, and a designator into its elements (which also lacks braces), each noting the member, as Clang reports them; a designated list stays valid. |
| `InitializerOverride`, `PartialInitializerOverride` | [initializer overrides](sema-initializer-overrides.c) | Clang's `-Winitializer-overrides`: a repeated member, a write into an element an expression initialized (at its designator), another union member (at the element) and a repeated flexible array member list, each noting the earlier initializer. |
| `TransparentUnionNotUnion`, `TransparentUnionNotDefinition`, `TransparentUnionPlacement` | [transparent_union applicability](sema-transparent-union-applicability.c) | `transparent_union` in a tag-only declaration's specifiers, on an object, on a union specifier that does not define the union, and on a structure. |
| `InvalidMachineEntry` | [GCC handlers](sema-interrupt-gcc.c), [GCC without general registers](sema-interrupt-gcc-sse.c), [Clang handlers](sema-interrupt-clang.c) | A handler's signature in each compiler's words, a direct call, `interrupt` with `naked`, and GCC's refusal of a handler or `no_caller_saved_registers` function compiled with SSE. Clang's carets stand at the offending part of the declaration and GCC's at its start; both of these stand at the attribute. |
| `ExcessiveRegsave` | [Clang handlers](sema-interrupt-clang.c) | Clang's `-Wexcessive-regsave`, from a handler and from a `no_caller_saved_registers` function. |
| `DuplicateMember` | [member name](sema-duplicate-member.c) | Duplicate names in a record member namespace. |
| `InvalidBitFieldWidth`, `InvalidBitFieldType`, `NamedZeroWidthBitField` | [bit-field](sema-bit-field.c) | Width, type and zero-width-name constraints, each with its own message. |
| `InvalidParameter` | [parameter](sema-parameter.c) | Parameter storage and non-outermost array static/qualifiers. |
| `IncompleteObject` | [object](sema-incomplete-object.c) | Automatic incomplete record and defined void object. |

`UnknownTypedef` is a defensive semantic binding check for a typedef-classified
syntax node without a semantic binding, reached only when parser and semantic
scopes disagree. Well-formed source reached it through a nested-declarator
function definition until Stage 3 bound the defining parameter list; no other
well-formed path is known, and recovery taint suppresses already-diagnosed
malformed declarations. It is covered with a
constructed semantic-resolution unit test rather than a misleading source golden.

Layout/type identities, old-style promoted parameter compatibility, conditional
integer conversions, floating-to-integer constant casts, function-prototype
visibility and deep non-recursive traversal have positive unit regressions.
Unmodeled extensions carry unknown/tainted types and suppress dependent errors.
The allocation harness additionally compiles generated C inputs through sema
and renders this same golden corpus with the default semantic CLI path.

## Expression and initializer semantic diagnostics

The `sema2-*` fixtures preserve a following declaration and render the structured
source range and standard note. They use C99 except the C11 assertion and the
GNU99 pedantic implicit-function fixture. Every new semantic kind is covered
below; implicit functions are reported under `-Wimplicit-function-declaration`
([C type-safety permerrors](#c-type-safety-permerrors)).

| Symbolic kind | Golden | Constraint |
| --- | --- | --- |
| `UndeclaredIdentifier` | [undeclared](sema2-undeclared.c) | Ordinary names need visible bindings; failed operands suppress cascades. |
| `InvalidAddressOperand` | [address](sema2-address.c) | Address operands, register objects and bit-fields. |
| `InvalidUnaryOperand` | [unary](sema2-unary.c) | Dereference, arithmetic unary operators and increment/decrement operand types. |
| `ExpectedModifiableLvalue` | [lvalue](sema2-lvalue.c) | Assignment and increment need modifiable lvalues. |
| `InvalidSubscript` | [subscript](sema2-subscript.c) | Object pointer plus integer subscripts. |
| `InvalidArithmeticOperands` | [arithmetic](sema2-arithmetic.c) | Multiplicative arithmetic operands. |
| `InvalidAdditiveOperands` | [additive](sema2-additive.c) | Arithmetic addition, complete-object pointer arithmetic and pointer difference. |
| `InvalidIntegerOperands` | [integer-operator](sema2-integer-operator.c) | Shift, remainder and bitwise integer operands. |
| `InvalidLogicalOperands` | [logical](sema2-logical.c) | Scalar logical operands. |
| `InvalidComparisonOperands` | [comparison](sema2-comparison.c) | Arithmetic/pointer comparison compatibility. |
| `InvalidConditionalOperands` | [conditional](sema2-conditional.c) | Scalar condition and compatible result alternatives. |
| `InvalidAssignment` | [assignment](sema2-assignment.c) | Assignment conversions, including nested pointer qualifiers. |
| `InvalidCast` | [cast](sema2-cast.c) | Scalar casts and pointer/floating exclusions. |
| `InvalidSizeof` | [sizeof](sema2-sizeof.c) | Function, incomplete and bit-field operands. |
| `IncompleteValue` | [incomplete-value](sema2-incomplete-value.c) | Value use of an incomplete structure, union or enumeration lvalue. |
| `InvalidMemberAccess` | [member-access](sema2-member-access.c) | Complete records and existing member names. |
| `InvalidCall` | [call](sema2-call.c) | Pointer to function designators. |
| `InvalidArgumentCount` | [argument-count](sema2-argument-count.c) | Prototype fixed and variadic arity. |
| `InvalidArgumentType` | [argument-type](sema2-argument-type.c) | Prototype assignment compatibility. |
| `InvalidCompoundLiteral` | [compound-literal](sema2-compound-literal.c) | Object type and no variably modified literal type. |
| `InvalidCondition` | [condition](sema2-condition.c) | Scalar selection/iteration conditions. |
| `InvalidSwitchExpression` | [switch](sema2-switch.c) | Integer switch operand. |
| `FailedAssertion` | [assertion](sema2-assertion.c) | Evaluated supported static assertion and its message (C11 mode). |
| `InvalidInitializer` | [initializer](sema2-initializer.c) | Scalar assignment conversion and valid object initialization. |
| `ExcessElements` | [excess](sema2-excess.c) | Scalar/aggregate/array current-object exhaustion; GCC and Clang warn. `ExcessInitializer` is the error TCC and `cl` give. |
| `InvalidDesignator` | [designator](sema2-designator.c) | Existing members and in-range nonnegative ICE array indices. |
| `NonConstantInitializer` | [static-initializer](sema2-static-initializer.c) | Static arithmetic/address constant eligibility. |
| `NonConstantInitializer`, `FoldedIntegerConstant`, `Extension(ConstantCompoundLiteral)` | [GCC](sema-static-compound-literal-gcc.c), [Clang](sema-static-compound-literal-clang.c) | Compound literals in static initializers and integer constants under each identity: GCC's whole-literal wrappers and their pedantic report, Clang's folded literal values, and the forms both reject. |
| `ConstantOverflow` | [static-overflow](sema2-static-overflow.c) | Shared exceptional constant evaluation in initializers. |
| `InvalidConstant`, `NonPositiveBitIntWidth`, `SignedBitIntWidth`, `BitIntWidthTooLarge` | [GCC](sema-bitint-width-gcc.c), [Clang](sema-bitint-width-clang.c) | `_BitInt` widths in declarations, typedefs, casts, `sizeof` and `_Generic`: GCC's value checks against its 65535-bit maximum, Clang's zero-extended 64-bit reading against 2^23 bits, and the compound literal Clang folds and GCC rejects. |
| `OverflowBuiltinOperand`, `OverflowBuiltinResult` | [Clang](sema-stdckdint-clang.c), [GCC](sema-stdckdint-gcc.c); kinds in `semantic_analysis/tests/scalar_builtins.rs` | The first argument `__builtin_add_overflow`, `__builtin_sub_overflow` or `__builtin_mul_overflow` refuses, in each compiler's words. Under Clang's rules a call that `ckd_add`, `ckd_sub` or `ckd_mul` wrote also refuses plain `char`, `bool`, bit-precise and enumerated types (C23 §7.20.1p3). A short result and direct builtin calls are not affected. |
| `OverflowBuiltinBitInt` | unit tests in `semantic_analysis/tests/bit_int_types.rs` | Clang's refusal of `__builtin_mul_overflow` with a signed `_BitInt` operand or result of more than 128 bits, under the Clang identity only; unsigned ones and the other overflow builtins are accepted. |
| `AtomicBuiltinBitInt`, `AtomicOperandSize`, `AtomicOperandBytes` | unit tests in `semantic_analysis/tests/bit_int_types.rs` | Clang's refusals of `_BitInt` objects in the `__atomic` value builtins and in `__sync` builtins of a width that is no power of two, under the Clang identity only, and of a `__sync` builtin on more than 16 bytes, which GCC refuses too. |
| `Extension(ArrayCompoundLiteral)`, `Extension(ConstantCompoundLiteral)`, `NonConstantArrayInitializer`, `ExcessString` | [GCC](sema-array-compound-literal-gcc.c), [Clang](sema-array-compound-literal-clang.c) | Arrays initialized by a compound literal of their type: GCC's static-only rule with element qualifiers dropped and its element and member forms, Clang's exact types in any storage without side effects, the size and type mismatches both reject, and too-long strings, which both store truncated with a warning. |
| `FloatOverflow`, `PedanticOverflow`, `LiteralConversion`, `NonConstantInitializer` | [GCC](sema-float-integer-gcc.c), [Clang](sema-float-integer-clang.c) | Floating constants converted out of an integer type's range: GCC's `-Woverflow` in static initializers only, with its bit-field width and pedantic report for casts and arithmetic too; Clang's `-Wliteral-conversion` for literals anywhere, inexact values included, and its rejection of an overflowing static bit-field. |
| `Extension(ImaginaryConstants)`, `InvalidDecimalFloatLiteral` | [GCC](sema-imaginary-interchange-gcc.c), [Clang](sema-imaginary-interchange-clang.c) | Imaginary interchange constants in both suffix orders: `f16` under both identities, GCC's `fN` and `fNx` forms, and the `bf16` form bcc cannot type. |
| `PointerConstraint(IntToPointerCast, PointerToIntCast, ...)` | [GCC](sema-pointer-int-cast-gcc.c), [Clang](sema-pointer-int-cast-clang.c), [options](sema-pointer-int-cast-options.c), [GCC system header](sema-pointer-int-cast-system-gcc.c), [Clang system header](sema-pointer-int-cast-system-clang.c) | Casts between a pointer and an integer of another width: GCC's two groups for any other width of a plain integer and a non-constant operand, Clang's narrower-only groups with their `void *` and enumeration children, the parent options that re-enable or promote a child, and GCC's silence for a cast spelled in a system header's macro, which Clang reports at the expansion. |
| `ImplicitFunctionDeclaration` | [implicit function](sema2-implicit-function.c) | Removed C89 function declaration, an error in GNU99 under the default Clang identity. |

Positive and negative rules are checked in
`semantic_analysis/tests/expressions.rs`: categories, promotions, null pointers,
qualifiers, all core operator families, calls, VLA sizeof, constant expressions,
compound literals, current-object traversal, designators, brace elision, strings,
inferred extents and statement expression sites. Diagnostic-free positive
inspection is pinned by `tests/fixtures/semantic/expressions.stderr`; the shared
`expression-sizeof-probe.c` is checked by sema and Linux-target Clang assertions.
Deep tests exercise 100,000 parentheses, unary operators and additions.
The allocation harness measures valid expression/initializer paths, erroneous
operands, and rendering of every golden here without global allocations.

## Statement and function semantic diagnostics

The 36 `sema3-*` fixtures cover all 30 new symbolic kinds below, plus GNU case
range overlaps, the K&R previous-declaration label, missing K&R declarations,
predefined-function-name redefinition, shared GNU statement-extension policy and
GNU void-expression returns. The parser continues to own duplicate-default
diagnostics; its existing golden remains authoritative. Modes and severity are
pinned by the `.args` sidecars.

| Symbolic kind | Golden |
| --- | --- |
| `InvalidFunctionDefinition` | [function-declarator](sema3-function-declarator.c) |
| `FunctionDefinitionStorage` | [function-storage](sema3-function-storage.c) |
| `NestedFunctionStorage` | [nested-function-storage](sema-nested-function-storage.c) |
| `IncompleteFunctionReturn` | [function-return](sema3-function-return.c) |
| `InvalidDefinitionParameterList` | [parameter-list](sema3-parameter-list.c) |
| `UnnamedDefinitionParameter` | [parameter-name](sema3-parameter-name.c), [parameter typedef warning](sema3-parameter-typedef-warn.c) |
| `IncompleteDefinitionParameter` | [parameter-complete](sema3-parameter-complete.c) |
| `DefinitionStarArray` | [parameter-star](sema3-parameter-star.c) |
| `Main` | [bcc's format](sema3-main.c); GCC's rules under `-Wall`: [return type](sema-main-gcc-return.c), [first](sema-main-gcc-first.c) and [third](sema-main-gcc-third.c) parameter, [count](sema-main-gcc-count.c), [static](sema-main-gcc-static.c), [variadic](sema-main-gcc-variadic.c), [object](sema-main-gcc-object.c), [`-Werror`](sema-main-gcc-werror.c); Clang's rules: [return type](sema-main-clang-return.c) and [its strict error](sema-main-clang-strict.c), [first](sema-main-clang-first.c) and [third](sema-main-clang-third.c) parameter, [one parameter](sema-main-clang-one.c), [too many](sema-main-clang-count.c), [static](sema-main-clang-static.c), [variadic](sema-main-clang-variadic.c), [object](sema-main-clang-object.c) |
| `IncompleteInternalTentative` | [internal-tentative](sema3-internal-tentative.c) |
| `DuplicateDefinition` | [definition](sema3-definition.c) |
| `TentativeArrayAssumedOne` | [array-completion](sema3-array-completion.c) |
| `IncompleteTentativeDefinition` | [incomplete-tentative](sema3-incomplete-tentative.c) |
| `UndefinedInternal` | [internal-undefined](sema3-internal-undefined.c), [cleanup use](sema3-internal-cleanup.c) |
| `UnusedStaticFunction` | [static-unused](sema3-static-unused.c) |
| `SignCompare` | [GCC](sema-sign-compare-gcc.c), [Clang](sema-sign-compare-clang.c) |
| `ValueWarning` | [GCC](sema-value-warnings-gcc.c), [Clang](sema-value-warnings-clang.c), [Clang int to float](sema-int-float-clang.c), through GNU `?:` and `__extension__` ([Clang](sema-int-float-gnu-clang.c)); constant conversions in code that does not run, in the operands `_Generic` and `__builtin_choose_expr` do not select (in file-scope and static initializers too), in conditional operands and from 128-bit values: [GCC](sema-constant-conversion-gcc.c), [Clang](sema-constant-conversion-clang.c); Clang's silence for a same-width conversion of a hexadecimal, octal or binary literal (pasted ones too) or of a character's element in its own braces, and its report of a same-width one in code that does not run but not in an unselected operand: [Clang](sema-constant-conversion-literals-clang.c), which the MSVC identity replaces with `cl`'s C4305, matching `cl` 19.44 (`/W2 /diagnostics:column`), whose column for a literal pasted by `##` is the macro's name ([MSVC](sema-constant-conversion-literals-msvc.c)); Clang's silence where a system macro wrote the conversion's context, through commas, revisited lists and pasted tokens: [Clang](sema-constant-conversion-system-macro-clang.c) |
| `TypeLimits` | [GCC](sema-type-limits-gcc.c), [Clang](sema-type-limits-clang.c) |
| `ShadowOrBody` | Shadowing: [GCC](sema-shadow-gcc.c), [Clang](sema-shadow-clang.c); by declaration kind: [GCC](sema-shadow-kinds-gcc.c), [Clang](sema-shadow-kinds-clang.c); of a system header's declarations: [GCC](sema-shadow-system-gcc.c), [Clang](sema-shadow-system-clang.c); empty bodies: [GCC](sema-empty-body-gcc.c), [Clang](sema-empty-body-clang.c); multiline headers and empty macros: [Clang](sema-empty-body-lines-clang.c) |
| `Parentheses` | [GCC](sema-parentheses-gcc.c), [Clang](sema-parentheses-clang.c) |
| `DeprecatedNonPrototype`, `CallArgumentCount` | [Clang](sema-deprecated-non-prototype-clang.c), declarations of builtins: [Clang](sema-deprecated-non-prototype-builtins-clang.c), [with `-fno-builtin-labs`](sema-deprecated-non-prototype-no-builtin-clang.c), [freestanding](sema-deprecated-non-prototype-freestanding-clang.c); bcc's format: [identifier list](sema3-knr-implicit.c) |
| `Nonnull` | Clang's nonnull parameters and `returns_nonnull` calls: [Clang](sema-nonnull-compare-clang.c); addresses that are never null: [Clang](sema-null-compare-clang.c); null arguments (`-Wnonnull`): [GCC](sema-nonnull-arguments-gcc.c), [Clang](sema-nonnull-arguments-clang.c) |
| `SwitchCases` | [GCC](sema-switch-gcc.c), [Clang](sema-switch-clang.c); constant and converted conditions: [GCC](sema-switch-constant-gcc.c), [Clang](sema-switch-constant-clang.c). Clang's carets are points, with the condition underlined for a missing enumerator or a constant condition; bcc orders the diagnostics by place, so line 20 of `sema-switch-clang` quotes no source for `-Wswitch-default`, which Clang reports after the `case` and quotes again |
| `ReturnFlow` | [GCC](sema-return-type-gcc.c), [Clang](sema-return-type-clang.c). GCC reports `no return statement` at the end of the function and its middle end's `control reaches end` and `'noreturn' function does return` after the unit, which the GCC snapshot's order follows |
| `MissingFieldInitializer` | [GCC](sema-missing-fields-gcc.c), [Clang](sema-missing-fields-clang.c), [GCC with a system macro](sema-missing-fields-system-gcc.c) |
| `Format`, `FormatAttribute` | [GCC](sema-format-gcc.c) under `-Wall -Wformat=2`, [Clang](sema-format-clang.c) by default: argument types, too few and too many arguments, an invalid conversion, a non-literal and an empty format, a null character, concatenation, a conditional, flags, `scanf`, and the attribute's own checks. Lines match GCC 14.2 and Clang 23.1 but for the order of GCC's two warnings on line 10. The review fixtures, [GCC](sema-format-review-gcc.c) and [Clang](sema-format-review-clang.c), hold operand numbers and amounts past every integer type, `u8` formats, `_Generic` and `__builtin_choose_expr` selections, constant offsets, formats on function pointers and members, constant-expression attribute operands, arrays cut short of their string and (Clang) fixed parameters, and match both compilers line for line. On Windows, [MinGW's `ms_printf`](sema-format-mingw.c) matches MinGW's GCC 13.3 but for the order of `too many arguments` on its line, and [the Microsoft C runtime's](sema-format-msvcrt.c) matches Clang 23.1 for `x86_64-pc-windows-msvc`. [Clang's ranges](sema-format-ranges-clang.c): the specification beside the caret, with the argument or a `*`'s argument, the position of `%0$` and `*0$`, the scan list, the format string for a null character or an unused argument, and a macro's string at its invocation; it matches Clang 23.1 but for Clang's fix-it lines. [Concatenated strings](sema-format-concat-ranges-clang.c): a macro's string at the start, middle and end of a concatenation, a macro expanding to another, a macro's string that concatenates two strings or holds another macro, a specification split between a macro's string and the file's, within one macro argument and between two, within `##` operands on the left, on the right and in a chain, and between two of them, a single string a placemarker leaves on either side, alone or in a `__VA_OPT__` replacement, the rest of an argument after the call it ends, within one argument of a `__VA_OPT__` replacement and between two arguments, two uses of one, the replacement's own string and an argument's, and what a call collected across it leaves, the replacement as a `##` operand on either side, in a chain and with `__VA_ARGS__` or a paste inside it, a token `##` formed within an argument, and specifications split over two and three strings and lines; it matches Clang 23.1 but for Clang's fix-it lines. Under the Clang identity, the [bcc](sema-format-macro-places-clang-bcc.c) and [GCC](sema-format-macro-places-clang-gcc.c) formats place a specification a macro's string holds at the string |
| `Unused` | GCC's words: [variables and parameters](sema-unused-variables-gcc.c), [file scope](sema-unused-file-scope-gcc.c), [labels](sema-unused-label-gcc.c), [values](sema-unused-value-gcc.c), [casts to the operand's own type](sema-unused-value-bool-cast-gcc.c), [ranges](sema-unused-value-ranges-gcc.c); Clang's: [variables and parameters](sema-unused-variables-clang.c), [file scope](sema-unused-file-scope-clang.c), [labels](sema-unused-label-clang.c), [values](sema-unused-value-clang.c), [statement expressions](sema-unused-value-statement-expression-clang.c); [`-Werror`](sema-unused-werror-clang.c). GCC's order: [scope ends](sema-unused-order-gcc.c), where it reports a block's variables and labels latest first after every warning inside the block, then at the end of a function the parameters only set, the local typedefs and the unused parameters; [errors](sema-unused-order-errors-gcc.c), after which GCC still reports variables but no local typedef, unlike [warnings `-Werror` promotes](sema-unused-order-werror-gcc.c); errors GCC has not read by a function's end: [later in the file](sema-unused-order-later-error-gcc.c), [in a header included later](sema-unused-order-header-error-gcc.c), [in a later reading of a header read before](sema-unused-order-repeat-header-gcc.c), as an invalid [`\x`](sema-unused-order-repeat-hex-gcc.c) or [`\u`](sema-unused-order-repeat-ucn-gcc.c) escape there, [in each of three readings](sema-unused-order-repeat-readings-gcc.c), [in the second reading of a header two deep](sema-unused-order-nested-repeat-gcc.c), [a `#warning` in each reading under `-Werror=cpp`](sema-unused-order-repeat-warning-gcc.c), [under `-Werror`](sema-unused-order-later-error-werror-gcc.c), and [the end of the input](sema-unused-order-unclosed-gcc.c), which counts in the function it leaves open; [the end of the unit](sema-unused-order-unit-gcc.c): undefined internal functions, then `control reaches end`, then the latest symbol first; [`-fsyntax-only`](sema-unused-order-syntax-only-gcc.c), which reports only the front end's warnings. Each matches GCC 14.2 line for line but for GCC's range over a parameter's type, and but for these: bcc prints the header's error before the earlier warning, since a change of file keeps the order diagnostics were reported in; under `-Werror` it prints the parser's error before the semantic errors and leaves out `[-Werror=cpp]`; and it words the unclosed function's error its own way, at the line after the input ends |
| `Unused` (local typedefs) | [GCC](sema-unused-typedefs-gcc.c), [Clang](sema-unused-typedefs-clang.c) |
| `BoolOperation` | [GCC](sema-bool-operation-gcc.c), [Clang](sema-bool-operation-clang.c) under `-Wall`: `++` and `--` of a `_Bool` (GCC), and `~` of a `_Bool`, a comparison, `!`, `&&` or `||` through parentheses, commas and casts as each compiler sees them |
| `InlineInternalReference` | [inline-reference](sema3-inline-reference.c) |
| `InlineStaticObject` | [inline-object](sema3-inline-object.c) |
| `VoidReturnMismatch` | [void-return](sema3-void-return.c) |
| `ReturnMismatch` | [missing-value](sema3-missing-value.c), [as a warning](sema3-missing-value-warning.c) (`-Wno-error=return-mismatch`) |
| `InvalidReturnConversion` | [return-conversion](sema3-return-conversion.c) |
| `InvalidForDeclaration` | [for-declaration](sema3-for-declaration.c) |
| `DuplicateLabel` | [label-duplicate](sema3-label-duplicate.c) |
| `UndefinedLabel` | [label-undefined](sema3-label-undefined.c) |
| `JumpIntoVariableScope` | [goto-vla](sema3-goto-vla.c) |
| `SwitchIntoVariableScope` | [switch-vla](sema3-switch-vla.c) |
| `CaseOutsideSwitch` | [case-outside](sema3-case-outside.c) |
| `EmptyCaseRange` | [case-empty](sema3-case-empty.c) |
| `BreakOutsideLoopOrSwitch` | [break](sema3-break.c) |
| `ContinueOutsideLoop` | [continue](sema3-continue.c) |
| `DuplicateCase` | [case-duplicate](sema3-case-duplicate.c) |

Positive and negative rules are exercised in `semantic_analysis/tests/statements.rs`,
including nested declarator identity, C89 K&R implicit int, C23 unnamed parameters,
forward and backward jumps, pointer/typedef VLA scopes, converted unsigned cases,
local labels and nested functions, return conversions, late extern declarations
and translation-unit completion. Deep tests use 10,000 nested if/while/block
statements and a 10,000-case switch. Allocation coverage includes valid and
erroneous bodies, synthesized definitions and deep statement traversal; rendering
uses the same complete golden corpus. Optional fallthrough and unused-label
warnings are not implemented and have no claimed coverage.

The unnamed-parameter default warning and typedef fallback under Warn are pinned
by [parameter-name](sema3-parameter-name.c) and
[parameter-typedef-warn](sema3-parameter-typedef-warn.c). Where the command line
alone classifies the parameter, as in the first, the parser reports it as the C23
extension Clang warns of by default, in order with the other C23 extensions; the
typedef fallback keeps semantic analysis's typed message and C99 note. C23 emits
no extension diagnostic, and Deny promotes the warning to an error (covered by
the semantic policy matrix tests).

Four more `sema3-*` fixtures extend statement coverage:

| Fixture | Coverage |
| --- | --- |
| [for-init C99](sema3-for-init-c99.c) | Type-only enum/struct for initializers report the C23 extension under Deny. |
| [for-init warning](sema3-for-init-warn.c) | GNU17 with Warn reports the same later-standard extension as warnings. |
| [local labels](sema3-local-labels.c) | `DuplicateLocalLabel` labels the previous declaration; `UndefinedLocalLabel` diagnoses an unused declaration in GNU17. |
| [local labels Deny](sema3-local-labels-deny.c) | Strict C99 reports GNU syntax policy and both local-label constraints. |

`semantic_analysis/tests/operand_regressions.rs` also checks C99/C17/C23 policy behavior,
register parameters, null-pointer alternatives, parenthesized strings, pointer
Boolean constants, directional compound assignment and runtime `offsetof`.
Operation counts at 128 and 512 elements protect VLA jump queries, indexed
member lookup and nested array initializer classification without timing limits.

### Freestanding resources

Four fixtures cover resource-header intrinsic diagnostics: `freestanding-va-list`
(invalid va-list operands), `freestanding-va-type` (void, incomplete and array
result types), `freestanding-va-start` (fixed-argument function) and
`freestanding-offsetof` (bit-field and invalid member paths). They exercise
`InvalidVaList`, `InvalidVaArgType`, `VaStartOutsideVariadic` and `InvalidOffsetof`.
Their `<built-in>/stddef.h` and `<built-in>/stdarg.h` lines moved when those
headers gained the `__need_*` partial-inclusion protocol; the diagnostics are
otherwise unchanged.
Missing-header fixtures and the corresponding lexing snapshots now list the final `<built-in>` directory. The system
missing-header fixture consequently reports an actual search list instead of
its former empty-list/absolute-path note. Other lexing snapshot changes are internal
file IDs: `<built-in>/predefined.h` (formerly `<built-in>/target.h`) is registered after the main source and before
any include or `#line` synthetic identity. Display paths and user text remain
unchanged.

The GNU parser policy fixtures intentionally lose the GNU-origin diagnostics for
`__builtin_va_arg` and `__builtin_offsetof`, which now support strict-mode
standard headers. Semantic analysis also identifies their previously opaque
undeclared `ap` operand. The case-range availability follow-up makes the strict
C17 range a parser error, so the existing recovered-function handling now
withholds that dependent semantic diagnostic. The GNU17 policy fixture reports
C2y origin for its range. Other extension diagnostics are unchanged.

### System headers

[pp-system-header-diagnostics.c](pp-system-header-diagnostics.c) runs with
`-std=c89 -pedantic-errors -isystem system-headers`. In `system-headers/noisy.h`
and the `beside-noisy.h` it includes, an undefined `#if` identifier, a
redefinition of the predefined `__INT64_C` and `long long` are withheld, while
`#error` is still reported. In `pragma-system-header.h` the warning before
`#pragma GCC system_header` remains and the ones after it are withheld. The main
file keeps its own warning and errors, including the redefinition of a system
header's macro. The `long long` that the system macro `NOISY_WIDE` supplies is
reported where the main file uses it, at that invocation, because the
expansion location decides, as in Clang 23; the snapshot gained that error. A `defined` that the system macros `NOISY_PROLOG` and `NOISY_DEFINED`
produce is withheld in `noisy.h` but reported where the main file expands
them, because that diagnostic is placed at the outermost invocation, as GCC and
Clang place it; the snapshot gained those two diagnostics and the `#error`
moved to line 16.

The three macro-redefinition snapshots changed from errors to warnings when
redefinitions took GCC's and Clang's severity.

### GNU 128-bit integer diagnostics

`sema-int128-overflow.c` covers signed addition, subtraction, multiplication,
negation, minimum / -1, minimum % -1, invalid full-width shift counts and zero
divisors. `sema-int128-constraints.c` covers 129-bit/zero-width named fields,
unsigned maximum array bounds, the 64-bit enum ABI ceiling and duplicate
high-half switch cases. `sema-int128-pedantic.c` pins keyword warnings,
`__extension__` suppression and builtin typedef spellings without warnings.
All use structured existing semantic kinds and C99 notes; following valid
input survives. Arithmetic/layout positives are in the shared target probe
and direct semantic unit tests, with allocation coverage for both paths.

The `semantic_analysis::tests::int128` unit regressions pair signed and unsigned
keyword casts with their builtin typedef aliases and `long long` equivalents
for false static assertions and nonconstant static assertions, enumerators and
bit-field widths. They check exact diagnostic messages and preservation of the
following declaration across all supported revisions, GNU settings and targets.
These unit tests add no golden inputs or snapshots to the inventory above.
The shared target probe builds the signed minimum using an immediate floating
cast followed by integer arithmetic, so strict modes do not need GNU folding.

### Command-line and MinGW compatibility regressions

- `cli-invalid-definition.c` / `.args` / `.stderr` inspect an invalid `-D1x` name at deterministic `<command line>` provenance, using the existing `ExpectedIdentifierInDefineDirective` diagnostic.
- `../targets/mingw-callconv.c` is a positive Clang/bcc target probe, not a diagnostic input, and does not contribute to the counts above.

### Atomic resources and type-generic operations

Eight fixtures cover atomic diagnostics and preserve following valid input:

| Fixture | Coverage |
| --- | --- |
| `sema-atomic-type` | `InvalidAtomicType`: arrays, functions, qualified/already-atomic specifier operands, incomplete/void operands and invalid qualifier applications. |
| `sema-atomic-builtin` | `InvalidAtomicOperand` and shared arity errors: non-atomic C11 pointers, const writes, incompatible expected/value operands, floating bitwise fetches and invalid GNU/sync pointees. |
| `sema-atomic-order` | `InvalidAtomicOrder` and `InvalidAtomicFailureOrder`; a qualified expected or generic operand is Clang's `-Wincompatible-pointer-types-discards-qualifiers` argument warning; valid runtime orders, stronger failure orders and unrestricted fences remain accepted. |
| `sema-atomic-operands-gcc` | GCC 14's operand checks under the GCC identity with `-std=gnu11 -pedantic`: a generic builtin's pointers of the first operand's size pass whatever their types, one of another size, an integer or a function pointer is `AtomicGenericOperand` (`size mismatch in argument N`, `must be a pointer type`, `must not be a pointer to a function`), a `const` output or a `volatile` later operand is a `-Wdiscarded-qualifiers` pedwarn, `__atomic_compare_exchange_n`'s expected value is an argument of `__atomic_compare_exchange_4`'s `void *` parameter, and value operands that discard qualifiers or mix pointers and integers convert silently, as by a cast; `__sync` arithmetic and bitwise builtins and `__atomic_clear` take a pointer to an array, and a function pointer passed to `__atomic_clear` or `__atomic_test_and_set` is GCC's `-Wpedantic` warning for its `volatile void *` parameter. Every line matches GCC 14 except the expected operand's column: GCC reports it at an unrelated earlier token (the statement or function start), bcc at the call. |
| `sema-atomic-operands-clang` | Clang 23's operand checks under the Clang identity with `-std=gnu11 -pedantic-errors`: qualified generic, expected and value operands are `-Wincompatible-pointer-types-discards-qualifiers` errors, a null expected or generic operand is `-Wnonnull` at the call, and `__atomic_test_and_set`/`__atomic_clear` take any object or function pointer. Matches Clang 23.1.5 line for line. |
| `sema-atomic-generic` | `InvalidGenericSelection`, no matching association, duplicate compatible associations, and a selected false assertion. |
| `sema-atomic-constant` | Atomic size assertion evaluation, non-ICE atomic casts/runtime lock-free sizes, nonconstant 16-byte queries, atomic bit-fields and incomplete pointer fetches. |
| `sema-atomic-deprecated` | `DeprecatedMacro` at direct and wrapped C17 resource/user macro uses, with expansion and pragma labels; identical redefinition, `#ifdef`/`#ifndef`, both `defined` forms, adjacent message literals, `_Pragma`, `#undef` clearing, and use-location suppression for a system-header replacement. |

Direct semantic tests additionally check C89/C99/C11/C17/C23 extension policy,
header deprecation suppression, retained result types, atomic/non-atomic pointer
identity and constant-evaluated `_Generic` results. The shared atomic probe runs
with both Clang and bcc for every target in C11, C17 and C23. It deliberately
disables header deprecation warnings; the separate golden pins that warning.
Preprocessor unit tests additionally pin Clang's C23 `#elifdef`/`#elifndef`
warning locations, skipped groups, argument prescan, cross-frame invocations,
changed definitions and malformed optional-message recovery. The corpus and
variant inventory counts are unchanged: this extends an existing fixture.
All new fixtures participate in the zero-global-allocation reporting harness.

### Binary128 and type-generic diagnostics

- `sema-float128-msvc.c` covers `UnsupportedFloat128` for real and complex
  types on the MSVC target, with following valid input preserved.
- `sema-float128-pedantic.c` covers keyword and literal extension warnings
  and `__extension__` suppression.
- `pp-float128-literals.c` covers binary128 overflow/underflow through the
  existing floating range warnings and rejects Clang-unsupported `f128`.
- `sema-type-generic.c` covers `InvalidChooseCondition`,
  `InvalidGenericSelection` (duplicate, missing and incomplete associations),
  invalid real-component operands and classification argument counts.
- `sema-type-generic-recovery.c` covers a failed choose operand without a
  dependent condition error, atomic variably modified jumps and associations,
  selected vector lane addresses, and nonconstant imaginary components.
  Static component addresses and atomic bool address initializers remain valid.

The shared four-target probe checks real/complex layout, arithmetic rank,
classification, compatibility, generic and choose selection, and component
lvalues against Clang. Arena allocation tests exercise exact decimal and
subnormal parsing, unfolded arithmetic constants and selection diagnostics.


### GNU vector constraints

The four `sema-vector-*` fixtures cover `InvalidVectorAttribute` (invalid byte
count and void element), `InvalidVectorOperand` (unsafe scalar splat, floating
remainder, logical negation and vector element addresses), `InvalidVectorBuiltin` (runtime shuffle index
and a non-vector conversion type), and `InvalidImmediate` (runtime and
out-of-range SSE2 shuffle control). Each keeps a following ordinary declaration.
Positive rules and known result types are checked through the translation-unit
seam and four-target Clang/bcc probes; the allocation harness compiles vector
operators and the complete supported intrinsic umbrella without heap allocation.

The `tests/fixtures/semantic/vectors/` regressions exercise vector casts,
conditional selection, whole-vector brace initialization, binary128 splats,
atomic value conversion, attribute argument identifiers, and declared-type
alignment boundaries. Invalid cases pin missing-operand cascade suppression,
complex element rejection, integer-only operators, shift lane counts,
parenthesized shuffle constraints, and Boolean min/max restrictions beside
valid controls. Two-operand shuffles and converted x86 immediates are also
covered. These are semantic unit-test fixtures; the four diagnostic vector
fixtures and `coverage.tsv` counts are unchanged.

### Type-only member declarations

`parser-member-declares-nothing.c` pins Clang-compatible warnings for tagged
struct definitions, forward tags and enum definitions without a member
declarator. They introduce types without anonymous-member promotion or storage.
C11 anonymous definitions and explicitly enabled MS anonymous members retain
their existing behavior. System-header warnings are suppressed normally.

`parser-member-declares-nothing-deny.c` pins the same constraint as an error
under `-pedantic-errors`. Scalar and typedef type-only declarations follow
the same rule.

The pedantic member fixture also includes `system-headers/mingw-members.h`,
which suppresses the promoted warning under `#pragma GCC system_header`.

### Implicit library declarations

`sema-implicit-library.c` renders `ImplicitLibraryFunction` warnings in GNU89
(errors from C99, under `-Wimplicit-function-declaration`) for
`memcpy`, `strcmp`, `printf`, `strlen`, `abort` and `exit`, including the
function name, complete target-specific type and C99 call/library note.
The semantic tests cover repeated calls, all extension policies, C89/GNU17,
LP64/LLP64, const inputs, variadic parameters, explicit incompatible declarations,
unknown-name `int ()` fallback and strict C99 rejection. The pipeline regressions
run headerless termination and memory/string calls before/after ABI lowering,
with/without bcc optimization, and through all four native arms. A live Clang
comparison checks LLVM scalar types and `noundef` at declarations and call sites.

### Statement-expression entry

`semantic-statement-expression-goto.c`, `semantic-statement-expression-switch.c`
and `semantic-statement-expression-prior-call.c` pin semantic diagnostics for
entering a GNU statement expression. Primary labels identify the goto or switch
label; secondary labels identify the expression. These prevent the previous
lowering/SSA internal errors from becoming user diagnostics. Semantic and
pipeline tests also preserve legal internal jumps and exits.

### Statement-expression header jumps

`semantic-statement-expression-header-jumps.c` renders the existing
`BreakOutsideLoopOrSwitch` and `ContinueOutsideLoop` constraints for statement
expressions in while/do conditions, for initializers/conditions/increments and
switch controlling expressions. Its GNU99 sidecar avoids extension-policy
noise; all twelve primary labels identify their jump and retain the C99 notes.
A following ordinary declaration survives. Semantic tests check each error
individually; the shared header-target execution probe preserves valid outer
loop/switch targets, including continue through a switch.

### Representation attributes and GNU constant forms

`sema-representation.c` pins invalid GNU alignment and scalar modes, weakening
ISO alignment, mismatched union-cast member types and reversed GNU ranges.
Its GNU11 sidecar keeps the diagnostics focused on operand constraints; a
following declaration survives every error.

### GNU casts to union type

`sema-union-cast-clang.c` and `sema-union-cast-gcc.c` render GNU's cast to a
union type under `-std=c99 -Wall -Wextra -pedantic` for each identity. Checked
20 November 2026 against Clang 23.1.5 and GCC 14 with the same options: both
warn on the same five lines and columns (the cast's `(`), of a union type
spelled out, through a typedef, qualified, and through a qualified typedef, and
of a structure operand matching a member; both report the same-type cast
`(union U)u` as a cast of a nonscalar to its own type, and both refuse a
`short` operand that matches no member without the extension warning. Casts
to a pointer to a union, to a pointer to such a pointer, and through a typedef
are silent in all three. The words differ: bcc's shared extension wording and
`-Wpedantic` tag stand for GCC's "ISO C forbids casts to union type" and
Clang's "cast to union type is a GNU extension [-Wgnu-union-cast]", and bcc's
`invalid operand or target type for cast` for their "cast to union type from
type ... not present in union". Under `__extension__`, on the cast, in a macro,
around a statement expression, before a block-scope or file-scope declaration,
before a function definition and on a same-type cast, all three are silent;
the declaration after a marked one and the file-scope `sizeof` operand after
the marked one warn again on the same lines and columns.

`sema-union-cast-static-gcc.c` (`-std=gnu11`) pins which casts to a union
type GCC 14 refuses in a static initializer ("initializer element is not
constant"), which takes the cast as constant only when the operand it parsed
was (`TREE_CONSTANT` in `build_c_cast`). It refuses a `?:` whose operands are
not all integer constant expressions, a comparison of floating or complex
operands (converted to `long` too), and a `&&` of a floating operand,
through a negation and in braces and a block-scope static object. It accepts
integer operands, `!1.0`, `1.0 + 1.0`, a conversion to another type that folds
the `?:`, and the same casts in an automatic object. Checked 21 November 2026
against GCC 14: the refusals stand on GCC's lines and columns, and main
refused none of them. The words and the underline remain bcc's: "static-storage
initializer requires a constant expression" across the cast, where GCC writes
its words at the cast's `(`.

`sema-extension-pedantic-gcc.c` checks GCC's `-pedantic` reports of
`__FUNCTION__` and `__PRETTY_FUNCTION__` under `__extension__`. Checked
20 November 2026 against GCC 14: an `__extension__` operand, a marked
declaration and a marked function definition silence them, and a use after the
marked declaration warns again on the same line and column. Its last rows
check folded constants: a marked declaration silences GCC 14's reports of a
folded enumerator, case label and bit-field width, but leaves bcc's report of
a folded array bound as it is unmarked. GCC makes such an array variable
length: it refuses the static and file-scope ones ("variably modified
'loud_bound' at file scope", "storage size of 'a' isn't constant"; the second
is an error without `-pedantic`), where bcc reports its `-pedantic`
folded-constant warning at the bound and GCC points at the declaration, and
accepts the automatic one in `loud_automatic` silently, which bcc reports as
it does unmarked (see Remaining output issues). The rows after it, checked
against GCC 14 on the same day, show a marked declaration silencing a
conditional between pointers to arrays whose elements differ in qualifiers and
a static initializer folded from an undefined shift, while the unmarked copies
and the initializer whose shift sits in an `__extension__` operand warn on
GCC's lines and columns, the conditional at its `:`. The last two rows initialize file-scope unions
with casts to a union type, which GCC accepts; only the unmarked one warns,
in bcc's wording for GCC's "ISO C forbids casts to union type".

### GCC's function and object pointer conversions

`sema-function-pointers-gcc.c` (`-std=gnu99 -pedantic`) pins GCC's
`-pedantic` reports of conversions between function and object pointers:
"ISO C forbids conversion of object pointer to function pointer type" and
the reverse at a cast's `(`, at file scope and in a function, silent for a
null pointer constant; "ISO C forbids initialization between function
pointer and 'void *'" at the initializer, and its `return` and `assignment`
forms; "ISO C forbids comparison of 'void *' with function pointer", silent
against a null pointer; and "ISO C forbids conditional expr between 'void *'
and function pointer" at the `:`, whose result is the `void *`. An
`__extension__` operand or a marked declaration silences each. Checked
21 November 2026 against GCC 14 (`LC_ALL=C`), which it matches byte for byte.
Argument passing ("ISO C forbids passing argument 1 of 'f' between function
pointer and 'void *'") is pinned by
`gcc_reports_function_and_object_pointer_conversions` in
`tests/integration/personality_cli.rs`, as GCC follows it with a note about
the parameter that bcc does not write. The option each report names under
`-Werror`, `-Werror=pedantic`, `-Wno-error=pedantic`, `-pedantic-errors` and
`#pragma GCC diagnostic error` or `warning "-Wpedantic"`, with the existing
`__FUNCTION__`, enumerator, case-label and static-assertion reports, is
pinned by `gcc_names_the_werror_that_made_a_pedantic_report_an_error`,
checked against GCC 14 on 21 November 2026. A call through an expression
other than a name keeps bcc's words, as GCC names the callee expression.
GCC underlines only part of some
initializers, a point for a conditional or a parenthesized one and the name
of a called function, where bcc underlines the whole expression, as it does
for every initializer it reports in GCC's format.

### GCC's negative left shifts

`sema-shift-negative-value-gcc.c` (`-std=gnu99 -Wextra`) pins GCC's
`-Wshift-negative-value`, "left shift of negative value", at the operator:
for a negative literal, an enumerator, a cast constant and a `long`, with a
constant count or a variable one, inside `__extension__`, and before the
`-Wshift-count-overflow` of the same shift. The second function is silent:
a comma operand, an operand a constant `?:` skips or `sizeof` holds, an
unsigned or nonnegative operand, a right shift. Checked 21 November 2026
against GCC 14 (`LC_ALL=C`), which it matches byte for byte. The options that
enable it (`-Wextra` or `-Werror=extra` from C99 on, its own option and
pragma in C90, never a pragma naming `-Wextra`) are pinned by
`gcc_warns_of_negative_left_shifts_under_wextra_from_c99` in
`tests/integration/personality_cli.rs`.

### Predefined function-name identifiers

`sema-predefined-outside-function.c` renders the default warning
`predefined identifier is only valid inside function` (Clang's
`-Wpredefined-identifier-outside-function`) for `__func__`, `__FUNCTION__` and
`__PRETTY_FUNCTION__` at file scope, in an initializer and in a `sizeof`
operand; each use warns and a following declaration survives. `sema-predefined-readonly.c`
renders the ordinary read-only errors for assigning to the const arrays.
Acceptance without a diagnostic in every mode, as in Clang 23.1.1 even under
`-std=c99 -pedantic`, has no golden because a silent compile has no stderr; the
semantic tests check each ISO and GNU revision and extension policy. The text of
`__PRETTY_FUNCTION__` is checked against the bundled Clang by the
`semantic/predefined-function-names.c` end-to-end program and by
`semantic_analysis/tests/predefined_identifiers.rs`.

## C type-safety permerrors

The `sema-permerrors-*` fixtures run one source through each identity and
format: GCC 14 in GNU17 ([gcc](sema-permerrors-gcc.c)), with `-fpermissive`
([permissive](sema-permerrors-permissive.c)) and in GNU89
([gnu89](sema-permerrors-gnu89.c)); Clang 23 in GNU17
([clang](sema-permerrors-clang.c)) and C23
([c23-clang](sema-permerrors-c23-clang.c)); and bcc's own words under the
default identity ([bcc](sema-permerrors-bcc.c)). They cover
`ImplicitFunctionDeclaration`, `ImplicitLibraryFunction`, `ImplicitInt`,
`ImplicitIntReturn`, `ImplicitIntParameter`, `ImplicitIntTypeName` (an unnamed
parameter), `ReturnMismatch`,
`VoidReturnMismatch`, `DeclarationMissingParameterType`,
`IdentifierListDeclaration`, `TypeSpecifierRequired` and the worded
`-Wint-conversion` and incompatible-pointer diagnostics, and GCC's
`-Wincompatible-pointer-types` for a conditional's mismatched pointers. Against GCC 14.2's
`cc1` and Clang 23.1.1 every line matches in severity and option, and in
wording but for GCC's extra `-Wbuiltin-declaration-mismatch` warning for
`printf` in GNU17, Clang's `-Wdeprecated-non-prototype` warning for the
old-style definition, Clang's wording of the conditional mismatch, and
Clang's C23 `use of undeclared identifier`, where bcc keeps its own words;
the unnamed parameter and GCC's conditional mismatch are reported a few
columns from the compilers' locations. Clang's `&` and `*` fix-it lines are
written; its `int ` fix-it lines for an implicit `int`, and the ranges it
underlines beside those carets, are not.
