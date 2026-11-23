# Site audit: the claims the lessons make about real compilers (2026-11)

The abstract machine follows the C standard, but its notes also say what
real implementations do: that a conversion wraps, how many bytes a type
takes, where padding goes, what a program prints, and what an optimizer may
do with undefined behavior. The compiler stages say what each phase of bcc
does. This audit checks each such claim against GCC and Clang, and reads
every lesson in Table 1's order as a programmer new to C would.

The C standard (N1256) stays the authority for what C requires; the
compilers are the evidence for what implementations do.

## Method

- **The narration.** A throwaway test stepped each of the 13 lessons the
  machine runs to its end and wrote every step's title, lead sentence,
  details, first "Why it matters" note and output (626 KB). Every sentence
  that states what an implementation, a compiler or x86-64 does was taken
  out of it, along with the stage explanations in `web/src/pipeline.rs` and
  the whole-path lesson's notes in `web/src/demos.rs`.
- **The compilers.** GCC 14's `cc1` (Ubuntu 14.2 package) driven by the host
  GCC 13.3 driver (`gcc -B<gcc-14 libexec>`, with GCC 13's freestanding
  headers), Clang 23.1.5 (the repository's pinned LLVM), and bcc from this
  branch (dev profile, LLVM back end), each at `-O0` and `-O2` with
  `-std=c17 -Wall -Wextra`, on x86-64 Linux with glibc 2.43. Other targets'
  type facts come from `clang --target=… -dM -E`.
- **The programs.** Every lesson in `web/demos/`, and a minimal probe for
  each claim the lessons cannot show (a shift by 32, `(int)3e9`, a division
  by zero, a store into a literal). The probes are in
  [`claims/`](claims/), with [`claims/run.sh`](claims/run.sh), which
  compiles and runs them all; [`claims/results.txt`](claims/results.txt) is
  its output for the three compilers. Warnings were also checked with no
  options, `-Wall`, `-Wall -Wextra` and `-Wconversion`.

## Results

64 claims checked: 37 hold as written, 12 were wrong, 14 over-promised and 1
was too vague to check. All 27 are fixed on this branch. The first pass
found 53 of them (its count said 54); the review of the branch found the
rank picture's mixed-rank rule, a NaN wording that still over-promised, and
the `sizeof` and "constraint" claims below, and a second reading for the
same kinds of claim (evaluation, refusal to compile, a compiler's output)
found the rest of the [second-pass table](#second-pass-after-review). bcc agreed with
GCC and Clang wherever those two agree.

### What the lessons print and return

Every lesson that finishes printed byte for byte what the machine prints,
and exited with the status it reports, under all three compilers at both
levels: Objects, loops and calls 15; Recursion 24; Integer promotions 255;
Pointers and arrays 17; Structs and static storage 31; From source to a
running program 16; Hello, Names for values, Unsigned and Floating point 0,
with their text identical (`total is 056`, `0.30000000000000004`,
`0.99999999999999989`, `-1 < 1u is 0`, …). The preprocessor lesson, which
the machine does not run, exits with 11. Each printf's return value (18 for
`Ada has 3 letters\n`, …) and puts's 14 for `Hello, world!` are what glibc
returned.

### Claims checked

"Holds" means the claim was right as written; the last column says what
changed otherwise. Step numbers are the note numbers on the page.

| Claim | Where | GCC 14 | Clang 23 | bcc | Verdict |
| --- | --- | --- | --- | --- | --- |
| `(char)300` wraps to 44: "bcc and GCC wrap" | Names for values, step 84 | 44 | 44 | 44 | Holds; now names Clang too, and says GCC documents it ("reduced modulo 2^N", GCC manual, *Integers implementation*) |
| "Where plain char is unsigned, as on ARM Linux, the cast would give 44 instead" | same | | | | **Wrong**: 300 is 44 either way. Now: "becomes 44 too, and there the result is defined" |
| `char narrow = 200` holds -56 | Integer promotions, step 25 | -56 | -56 | -56 | Holds |
| Plain char is unsigned on ARM Linux, so it would hold 200 | same, note | | `__CHAR_UNSIGNED__` on aarch64, armv7 | | Holds |
| "Every x86-64 compiler keeps the low bits" of a signed conversion | note (store into a non-char signed type) | 4000000000u → -294967296 | same | same | **Over-promised** ("every"): now GCC documents it, Clang and bcc do the same |
| `>>` of a negative value copies the sign bit (bcc, GCC, Clang) | shift step | -8 >> 1 = -4 | -4 | -4 | Holds; the lead now names Clang as the details did |
| int 4, long 8, pointer 8, size_t unsigned long, int range | declarations | as claimed | as claimed | as claimed | Holds |
| On 64-bit Windows a long is 4 bytes | note, Unsigned step 6 | | 4 (MSVC and MinGW targets) | | Holds |
| `struct point` 8 bytes aligned to 4; `struct entry` tag at 0, 3 bytes padding, count at 4 | Structs, steps 2, 76 | sizeof/offsetof agree | agree | agree | Holds (the text ended in "4.."; fixed) |
| An int is aligned to 4, a pointer to 8 | Padding note | 4, 8 | 4, 8 | 4, 8 | Holds |
| Padding holds unspecified bytes, so memcmp of structures is unsafe | Padding note | members equal, memcmp differs | same | same | Holds |
| `sizeof values` is 12 | Unsigned, step 146 | 12 | 12 | 12 | Holds |
| `'A'` is 65 and has type int | Hello note | 65, sizeof 4 | same | same | Holds |
| `-1 < 1u` is 0 | Unsigned, step 153 | 0 | 0 | 0 | Holds |
| "compilers warn about it (-Wsign-compare)" | note | only with -Wextra | only with -Wextra | only with -Wextra | **Over-promised**: now "with -Wextra" |
| `1u << 31` is 2147483648; `<< 1` again is 0 | Unsigned, steps 160, 167 | as claimed | as claimed | as claimed | Holds |
| Shifting an int by 32: "x86-64 quietly shifts by 0" | Shift note | 1 at run time; 0 when -O2 sees the count | 1; -2001422456 at -O2 | 1; garbage at -O2 | **Over-promised**: now the instruction uses the low 5 bits at run time, but a compiler that knows the count may produce anything |
| `0.1` is exactly 0.1000000000000000055511151231257827…; nearest rounding "like GCC and Clang" | Floating, step 46 | `%.55f` agrees | agrees | agrees | Holds |
| `0.1 + 0.2` is 0.30000000000000004, ten 0.1s 0.99999999999999989, `== 0.3` is 0 | Floating | as claimed | as claimed | as claimed | Holds |
| `(int)2.99` is 2 and `(int)-2.99` is -2 | Truncation note | 2, -2 | 2, -2 | 2, -2 | Holds |
| `(int)3e9`: "x86-64 gives -2147483648" | Truncation note | -2147483648 at run time; 2147483647 when -O2 folds it | -2147483648; garbage when -O2 folds | same as Clang | **Over-promised**: now the run-time instruction's result, and GCC -O2's different one |
| float/double/long double "about 7, 16, 19" digits | FloatingSizes note | FLT_DIG 6, DBL_DIG 15, LDBL_DIG 18 | same | same | **Wrong** against the declaration steps, which said 6, 15 and 18: both now say "keeps 6, 15, 18 significant decimal digits" |
| C promises 6 digits for float, 10 for double | same | | | | Holds (C99 §5.2.4.2.2p9) |
| long double is the x87 80-bit format in 16 bytes | same | 16 bytes, 64-bit significand | same | same | Holds |
| "On 64-bit Windows [long double] is just a double" | same | | 8 bytes for MSVC; 16 bytes, 80-bit for MinGW | | **Over-promised**: now "with Microsoft's compiler" |
| On ARM64 Linux long double is a 128-bit format | same | | 113-bit significand | | Holds |
| `-7 / 2` is -3 and `-7 % 2` is -1 | Division note | -3, -1 | same | same | Holds |
| Integer division by zero "kills the program with SIGFPE" on x86-64 | Annex F note | SIGFPE at -O0; exits 0 at -O2 | same | same | **Over-promised**: now traps without optimization, but an optimizer may compute something else (GCC -O2 turns 1 / b into a comparison) |
| printf writes 0.0 / 0.0 as nan | Annex F note | `-nan` at run time (`c19_nan`) and folded (`c17_fp`) | `-nan` at run time, `nan` folded at -O0 and -O2 | as GCC | **Wrong**: the sign depends on who computes the NaN. The first fix said x86-64's NaN has its sign bit set, which Clang's folded NaN contradicts; the note now says the division instruction sets it, so a run-time division prints -nan, and a NaN folded from constants may have either sign |
| printf with a mismatched conversion: "Compilers warn about it (-Wformat)" | Format note, Hello step 83 | with -Wall (Ubuntu's GCC by default) | by default | by default | **Over-promised**: now Clang by default, GCC with -Wall |
| A mismatched `%d` prints garbage | same | an address's low bits, different each run | same | same | Holds |
| A store into a string literal "on most systems crashes" | Hello note, step 10 | SIGSEGV at -O0 and -O2 | SIGSEGV at -O0; exits 104 at -O2 (store dropped) | SIGSEGV at -O0; exits 72 at -O2 | **Over-promised**: now crashes without optimization, while an optimizer may drop the store |
| A store through a cast-away const "may or may not survive" | Const note, Names for values step 6 | prints the new 4 at -O0, the old 3 at -O2 | 3 at both | 4 at both | Vague: now says the program can print the old value at one level and the new at another |
| A cast stops the warning: `(char)300` quietly becomes 44 | Cast note | `char c = 300;` warns by default (-Woverflow), the cast does not | warns (-Wconstant-conversion), the cast does not | as Clang | Holds; the note now names both warnings |
| -Wconversion warns about a signed conversion that changes the value | SignedConversion note | -Wconversion, -Wsign-conversion | -Wconversion | | Holds |
| An uninitialized read "returns whatever was left in that register or stack slot" | Uninitialized reads, step 6 | 73, 89, 105, 201, 249 at -O0 (each run differs); 1 at -O2 | 185, 201 at -O0; 44 at -O2 | 161 at -O0; 1 at -O2; no warning | **Over-promised**: -O2 results are the optimizer's choice, not leftovers. Now says so, and that GCC's and Clang's -Wall warn |
| Reading one past the end "could crash, print garbage, or appear to work" | Reading past the end, step 20 | exits 0, warns -Warray-bounds at -O2 | 0 at -O0, 44 at -O2 | 96, 128, 192 at -O0; 124 at -O2 | Holds (it predicts no one outcome) |
| Signed overflow: "the same program can behave differently at -O0 and -O2" | Signed overflow, step 37 | exits 1 at -O0 (wrapped), 255 at -O2, warns -Waggressive-loop-optimizations | 1, 255 | 1, 255 | Holds, as this lesson itself shows |
| Compilers "may delete a check like `x + 1 > x`" | Undefined note | folds it to 1 even at -O0 | 0 at -O0, 1 at -O2 | 0, 1 | Holds |
| Only the low 8 bits of the exit status reach the shell | ExitStatus note | `return 271` gives 15 | 15 | 15 | Holds |
| Static objects start at zero "because the value is already in the program file" | StaticStorage note, Structs step 1 | a zeroed 400 KB array: 400032 bytes of .bss, a 15856-byte file | | | **Wrong** for zeros: they are not in the file; the loader supplies them. Fixed |
| String literals live in static, read-only storage | Hello | `.section .rodata` | | | Holds |
| A debugger usually cannot show a macro | Macro note | no `.debug_macro` with -g, present with -g3 | none with -g or -g3 | | Holds ("usually") |
| puts returns 14 for `Hello, world!`, as glibc does | Hello, step 6 | 14 | 14 | 14 | Holds |
| printf returns the number of characters written | every printf step | 18 for the hello line | 18 | 18 | Holds |
| `-O2` defines `__OPTIMIZE__`, "as GCC and Clang do" | pipeline, front-end stages | yes | yes | yes | Holds |
| bcc's optimizer "repeats them until nothing changes" | pipeline, bcc IR at -O2 | | | at most 8 rounds (`MAX_ROUNDS`) | **Over-promised**: now "until a round changes nothing, at most eight rounds" |
| `nsw` "tells LLVM the product never overflows" | whole-path lesson, LLVM IR | | | | **Over-promised**: `nsw` makes an overflow poison; now "lets LLVM assume" |
| char arithmetic shows trunc and sext in LLVM IR | pipeline, LLVM IR | | | promotion.c has both | Holds only for programs with char arithmetic: now said so |
| Variables whose address is never taken live in values; arrays get a stack_slot | pipeline, bcc IR | | | an `&x` local gets a slot, a plain one does not | Holds; the explanation now adds address-taken variables |
| Arguments arrive in %rdi, …; the result leaves in %eax | pipeline, assembly | | | path.c | Holds for an int result; now says "an int result" |
| A call is e8 then four zero bytes for the linker | whole-path lesson, machine code | | | path.c | Holds (checked by the site's tests) |
| x86-64 instructions are 1 to 15 bytes | pipeline, machine code | | | | Holds |

Two claims about undefined behavior predicted one outcome where the
compilers disagree, and now say what was seen instead: the shift by 32 and
`(int)3e9`. The others keep to what C says and what may happen.

### Second pass, after review

The probes are `c26` to `c28` in [`claims/`](claims/), and
[`claims/results.txt`](claims/results.txt) was made again with all of them,
with the same GCC and Clang and a dev build of bcc whose compiler is this
branch's. It differs from the first run only in the new probes and in the
values the undefined probes print, which change from run to run (the folded
shift and conversion, the uninitialized and out-of-bounds reads). C99 is
N1256; every clause was checked against `standards/c99-n1256.html`.

| Claim | Where | GCC 14 | Clang 23 | bcc | Verdict |
| --- | --- | --- | --- | --- | --- |
| The rank picture's rule for `long long a; unsigned long b; a + b`: "The two have the same rank, and then the signed operand converts to the unsigned type" | rank picture, any mixed-rank conversion | | | | **Wrong**: the picture compared one operand's type before and after, not the two operands. `long long` outranks `unsigned long` (§6.3.1.1p1) but cannot hold all its values, so both become `unsigned long long` (§6.3.1.8p1). Each conversion step now carries the other operand's type, the picture marks it, and the rule names the branch the two select; tests cover each integer branch from both operands' sides, run through the engine with each operand on the left and on the right |
| A float passed to printf's `...` was captioned "The usual arithmetic conversions" | rank picture, Floating point | | | | **Wrong**: it is a default argument promotion (§6.5.2.2p6); the caption and rule now say so |
| "sizeof is worked out while the program is compiled: its operand is never evaluated" | Sizeof note | `sizeof(int[n++])` is 8 and n becomes 3 (`c26`) | same | same | **Wrong** for a variable length array, whose operand §6.5.3.4p2 evaluates. Now says so, and that the machine runs only arrays of constant length |
| sizeof's clause summary: "the operand is not evaluated" | clause list, every sizeof step | | | | **Wrong** the same way: now "unless its type is a variable length array" |
| A constraint violation "gets a diagnostic and does not compile" | glossary, "Constraint" | refuses `int *p = 1;` (`-Wint-conversion` is an error by default from GCC 14; with `-std=c89` only a warning) and `x++` on a const `x` (`c27`); warns about `int a[1] = {1, 2};` and compiles it (`c28`) | same, with `-Wint-conversion` an error in every mode | same as Clang | **Wrong**: §5.1.1.3p1 requires a diagnostic, and its footnote 8 lets an implementation translate the program anyway. Now says that, and which violations the three refuse and which they only warn about |
| A `#define` with a different replacement "would be an error the compiler must report" | `#define` step | warns "N redefined", uses the new replacement: exits 2 (`c28`) | warns (`-Wmacro-redefined`), exits 2 | warns, exits 2 | **Wrong**: §6.10.3p2 is a constraint, so it needs a diagnostic, not an error. Now: the compiler must report it, this machine stops, and the three compilers warn and use the new replacement |
| An assignment to a const object "does not compile" | Const note, and a const object's declaration step (two wordings) | refuses (`c27`) | refuses | refuses | **Over-promised**: true of these compilers, but C requires only a diagnostic. Now: it breaks a constraint, the compiler must report it, and GCC, Clang and bcc refuse to compile it |
| Signed conversion's clause summary: "x86-64 compilers keep the low bits" | clause list, every signed conversion | | | | **Over-promised** ("x86-64 compilers"), as the note was in the first pass: now GCC documents it, and Clang and bcc do the same |
| An invalid operation's detail: "x86-64 makes it the default NaN, whose sign bit is set" | Floating point, any invalid operation (`inf - inf`, `0.0 / 0.0`) | `-nan 1` at -O0 and -O2 | `-nan 1` at -O0, `nan 1` at -O2 (folded) | `-nan 1` at -O0, `nan 1` at -O2 | **Over-promised** (found in the third review): true of the instruction at run time only. Now says a compiler that works it out from constants may give either sign |
| Annex F's clause summary: "comparisons with a NaN are false" | clause list, every floating comparison | `invalid != invalid` is 1 | 1 | 1 | **Wrong** (third review): `!=` with a NaN is true. Now: `<`, `<=`, `>`, `>=` and `==` are false, `!=` is true |
| The usual arithmetic conversions' clause summary: "the higher-ranked one, or the unsigned one when its rank is at least as high" | clause list, every conversion step | `long long` + `unsigned long` is `unsigned long long` | same | same | **Wrong** (third review), the clause-list twin of the rank picture's rule: now gives all of §6.3.1.8p1's integer cases |

## Teaching each step in order

Reading the 1,265 steps in Table 1's order:

- **Terms used before they are explained.** Structures are "aligned to 4"
  from their first declaration, and "prototype" appears in the hello
  lesson's first step, but neither was in the glossary. Both are now
  glossary terms (C99 §3.2 and §6.2.1p2), so their first use shows the
  definition.
- **What without why.** Several notes said what C does but not why it does
  it: why arithmetic happens in int (the machine's natural size), why the
  order of evaluation is open (so the compiler can pick the fastest), why
  arrays decay (a call never copies the elements, so pass the length), why
  unsigned arithmetic wraps (binary hardware does, and hashes rely on it),
  why signed overflow and the other errors are undefined (no check on every
  addition or access, and loops simplify), and why implementation-defined
  behavior exists (machines differ). Each note now says it in a clause.
- **Notes that did not reach real programs.** The implementation-defined
  note now ends with what to do about it (unsigned char for bytes), the
  uninitialized-read and format notes name the warnings that catch them,
  and the undefined-behavior note, shown first at the out-of-bounds read,
  now names a bounds check, not only an overflow test.
- **The pipeline stages.** Each stage's explanation now opens with what goes
  in, what comes out and what the stage decides (for example: "In: the
  tokens. Out: a syntax tree … Decided here: how the tokens group, which
  operator takes which operands"). Every program, not only the whole-path
  lesson, now gets an "In this program" line with one example read from
  its own output (`pipeline/example.rs`): the statement phase 3 cut into
  tokens (and its keyword still an identifier), the macros phase 4
  replaced, the keyword and typed constant phase 7 made, the first operator
  node and its operands, a function's type, and the IR, assembly and
  machine-code counts. The whole-path lesson keeps its hand-written notes
  at -O0.

The step counts did not change: no fix needed another step.

## Visuals

- **The usual arithmetic conversions.** A promotion or a usual arithmetic
  conversion step now draws the arithmetic types by rank (`panels/ranks.rs`),
  integers in a signed and an unsigned row, floating types after them, with
  the operand's type outlined, the other operand's type dotted and the type
  it became filled, and the branch of the rule the two operands select in
  one line ("int and unsigned int have the same rank, so the signed operand
  converts to the unsigned type, unsigned int, even when its value is
  negative"; for `long long` against `unsigned long`, "long long has the
  higher rank, but it cannot hold every unsigned long value, so both
  operands convert to unsigned long long").
- **What the optimizer removed.** The optimizer note under each back-end
  stage now draws its comparison as paired bars, -O0 in ink and -O2 in
  vermilion, for instructions, basic blocks, loads and stores, stack slots
  and bytes of code.
- **Padding** was already drawn (hatched cells and the datasheet's offsets),
  so it needed no new picture.

Of the visual audit's three items left for later, the third was quick and is
done: the tape, the cells and Table 1 spell a char array as a string,
`"Ada\0"`, with every null character written out as the string cell boxes
it, and a long one is cut before its closing quote. The other two (folding
wide arrays of other types, and a char array inside a structure) need a new
kind of cell and remain open.

## Verification

The site was built with `web/build.sh` and driven with Playwright in
Chromium, with reduced motion. At 1440x900 every lesson was stepped from
its first note to its last: each step had a title and a lead sentence, and
no page logged an error. The step counts are the same as before the
changes (the November visual audit's table, and the native run of the same
engine on `main`):

| Lesson | Steps before | Steps after |
| --- | ---: | ---: |
| Objects, loops and calls | 197 | 197 |
| Hello, world: printf and strings | 86 | 86 |
| Names for values: #define, const and casts | 97 | 97 |
| Recursion and frames | 78 | 78 |
| Integer promotions | 34 | 34 |
| Unsigned numbers wrap; signed numbers don't | 183 | 183 |
| Signed overflow | 37 | 37 |
| Floating point: close, not exact | 230 | 230 |
| Pointers and arrays | 140 | 140 |
| Structs and static storage | 134 | 134 |
| Reading past the end | 20 | 20 |
| Uninitialized reads | 6 | 6 |
| From source to a running program | 23 | 23 |
| Before the compiler: the preprocessor | 0 (stops at `#include <limits.h>`) | 0 |

Each changed step was then shot at 1440x900 and 390x844 in the light and
dark themes ([`screenshots/claims/`](screenshots/claims/)), with no page
error. The first phone shots of the rank picture broke "double" inside the
word; its column labels now keep whole words.

After the second pass the site was built and stepped through again: every
lesson reached the same count as in the table above, every step had a
title and a lead, and no page logged an error. The rank picture's steps
were shot again, with two programs typed into the editor for the cases no
lesson has, `long long` against `unsigned long` (from each operand's side)
and a float passed to printf. On a phone the "other, to" mark first broke
inside "other"; the cells now break only between words.

| Step | 1440x900 | 390x844 |
| --- | --- | --- |
| Hello, step 10: a store into a literal | [light](screenshots/claims/literal-note-1440-light.png), [dark](screenshots/claims/literal-note-1440-dark.png) | [light](screenshots/claims/literal-note-390-light.png), [dark](screenshots/claims/literal-note-390-dark.png) |
| Hello, step 13: `name` on the tape as `"Ada\0"` | [light](screenshots/claims/string-tape-1440-light.png), [dark](screenshots/claims/string-tape-1440-dark.png) | [light](screenshots/claims/string-tape-390-light.png), [dark](screenshots/claims/string-tape-390-dark.png) |
| Hello, step 26: a char promoted, with the rank picture | [light](screenshots/claims/promotion-ranks-1440-light.png), [dark](screenshots/claims/promotion-ranks-1440-dark.png) | [light](screenshots/claims/promotion-ranks-390-light.png), [dark](screenshots/claims/promotion-ranks-390-dark.png) |
| Hello, step 83: which compilers warn about a format | [light](screenshots/claims/format-note-1440-light.png), [dark](screenshots/claims/format-note-1440-dark.png) | [light](screenshots/claims/format-note-390-light.png), [dark](screenshots/claims/format-note-390-dark.png) |
| Names for values, step 6: a cast-away const | [light](screenshots/claims/const-note-1440-light.png), [dark](screenshots/claims/const-note-1440-dark.png) | [light](screenshots/claims/const-note-390-light.png), [dark](screenshots/claims/const-note-390-dark.png) |
| Names for values, step 84: `(char)300`, details open | [light](screenshots/claims/cast-details-1440-light.png), [dark](screenshots/claims/cast-details-1440-dark.png) | [light](screenshots/claims/cast-details-390-light.png), [dark](screenshots/claims/cast-details-390-dark.png) |
| Integer promotions, step 25: why it is implementation-defined | [light](screenshots/claims/implementation-defined-note-1440-light.png), [dark](screenshots/claims/implementation-defined-note-1440-dark.png) | [light](screenshots/claims/implementation-defined-note-390-light.png), [dark](screenshots/claims/implementation-defined-note-390-dark.png) |
| Unsigned, step 14: int meets unsigned int | [light](screenshots/claims/conversion-ranks-1440-light.png), [dark](screenshots/claims/conversion-ranks-1440-dark.png) | [light](screenshots/claims/conversion-ranks-390-light.png), [dark](screenshots/claims/conversion-ranks-390-dark.png) |
| Unsigned, step 153: -1 converts to unsigned int | [light](screenshots/claims/minus-one-ranks-1440-light.png), [dark](screenshots/claims/minus-one-ranks-1440-dark.png) | [light](screenshots/claims/minus-one-ranks-390-light.png), [dark](screenshots/claims/minus-one-ranks-390-dark.png) |
| Unsigned, step 160: a shift by the width | [light](screenshots/claims/shift-note-1440-light.png), [dark](screenshots/claims/shift-note-1440-dark.png) | [light](screenshots/claims/shift-note-390-light.png), [dark](screenshots/claims/shift-note-390-dark.png) |
| Floating point, step 17: digits each type keeps | [light](screenshots/claims/floating-sizes-note-1440-light.png), [dark](screenshots/claims/floating-sizes-note-1440-dark.png) | [light](screenshots/claims/floating-sizes-note-390-light.png), [dark](screenshots/claims/floating-sizes-note-390-dark.png) |
| Floating point, step 36: an int meets a double | [light](screenshots/claims/int-to-double-ranks-1440-light.png), [dark](screenshots/claims/int-to-double-ranks-1440-dark.png) | [light](screenshots/claims/int-to-double-ranks-390-light.png), [dark](screenshots/claims/int-to-double-ranks-390-dark.png) |
| Floating point, step 220: `(int)3e9` | [light](screenshots/claims/truncation-note-1440-light.png), [dark](screenshots/claims/truncation-note-1440-dark.png) | [light](screenshots/claims/truncation-note-390-light.png), [dark](screenshots/claims/truncation-note-390-dark.png) |
| Edited program, step 15: `long long` meets `unsigned long`, and both convert | [light](screenshots/claims/mixed-ranks-1440-light.png), [dark](screenshots/claims/mixed-ranks-1440-dark.png) | [light](screenshots/claims/mixed-ranks-390-light.png), [dark](screenshots/claims/mixed-ranks-390-dark.png) |
| The same program, step 16: the `unsigned long` operand's side | [light](screenshots/claims/mixed-ranks-right-1440-light.png), [dark](screenshots/claims/mixed-ranks-right-1440-dark.png) | [light](screenshots/claims/mixed-ranks-right-390-light.png), [dark](screenshots/claims/mixed-ranks-right-390-dark.png) |
| Edited program, step 11: a float promoted for printf's `...` | [light](screenshots/claims/float-promotion-ranks-1440-light.png), [dark](screenshots/claims/float-promotion-ranks-1440-dark.png) | [light](screenshots/claims/float-promotion-ranks-390-light.png), [dark](screenshots/claims/float-promotion-ranks-390-dark.png) |
| Structs, step 1: zeroed statics and the program file | [light](screenshots/claims/static-note-1440-light.png), [dark](screenshots/claims/static-note-1440-dark.png) | [light](screenshots/claims/static-note-390-light.png), [dark](screenshots/claims/static-note-390-dark.png) |
| Structs, step 76: the layout, details open | [light](screenshots/claims/padding-details-1440-light.png), [dark](screenshots/claims/padding-details-1440-dark.png) | [light](screenshots/claims/padding-details-390-light.png), [dark](screenshots/claims/padding-details-390-dark.png) |
| Signed overflow, step 37: why C leaves it undefined | [light](screenshots/claims/undefined-note-1440-light.png), [dark](screenshots/claims/undefined-note-1440-dark.png) | [light](screenshots/claims/undefined-note-390-light.png), [dark](screenshots/claims/undefined-note-390-dark.png) |
| Uninitialized reads, step 6 | [light](screenshots/claims/indeterminate-note-1440-light.png), [dark](screenshots/claims/indeterminate-note-1440-dark.png) | [light](screenshots/claims/indeterminate-note-390-light.png), [dark](screenshots/claims/indeterminate-note-390-dark.png) |
| Objects, Tokens stage: in, out, decided, and the example | [light](screenshots/claims/pipeline-tokens-1440-light.png), [dark](screenshots/claims/pipeline-tokens-1440-dark.png) | [light](screenshots/claims/pipeline-tokens-390-light.png), [dark](screenshots/claims/pipeline-tokens-390-dark.png) |
| Objects, Syntax tree stage | [light](screenshots/claims/pipeline-syntax-tree-1440-light.png), [dark](screenshots/claims/pipeline-syntax-tree-1440-dark.png) | [light](screenshots/claims/pipeline-syntax-tree-390-light.png), [dark](screenshots/claims/pipeline-syntax-tree-390-dark.png) |
| Objects, bcc IR at -O2: the bars | [light](screenshots/claims/pipeline-ir-O2-1440-light.png), [dark](screenshots/claims/pipeline-ir-O2-1440-dark.png) | [light](screenshots/claims/pipeline-ir-O2-390-light.png), [dark](screenshots/claims/pipeline-ir-O2-390-dark.png) |
| From source, assembly at -O2: the bars | [light](screenshots/claims/pipeline-asm-O2-1440-light.png), [dark](screenshots/claims/pipeline-asm-O2-1440-dark.png) | [light](screenshots/claims/pipeline-asm-O2-390-light.png), [dark](screenshots/claims/pipeline-asm-O2-390-dark.png) |

## Left open

- bcc warns as Clang does about formats, signed/unsigned comparisons and
  constant conversions, but has no -Wuninitialized; the notes name GCC's and
  Clang's warnings.

## Parts 11 to 19 (2026-11-23)

Parts 11 to 19 of the [visual audit](visual-audit-2026-11.md) taught the
machine bitwise operators, `&&`, `||`, `?:`, the comma operator, pointer
idioms, pointers to functions, enumerations, `switch`, `goto`, `do` and
`continue`, and added five lessons ("Short-circuit evaluation", "Bits and
masks", "Walking a string with a pointer", "Callbacks" and "Loops three
ways"). This pass checks every claim they added, in the same way as the
passes above. Part 20 of the visual audit records the work.

### Method

- **The narration.** A throwaway test stepped the 19 lessons and the 95
  programs of the visual audit's corpus (`corpus/`) to their ends and wrote
  each step's title, lead, explanation, "why it matters" note, clauses and
  next step (5.4 MB). The 17 notes Parts 11 to 19 added, the 74 clause
  glosses they added and every sentence that names an implementation
  (x86-64, GCC, Clang, a compiler, a crash, a warning) or one of the new
  constructs were read against the sources below.
- **The static text.** Every string Parts 11 to 19 changed in the landing,
  "How it works" and its "Limits of this model", the glossary (13 new
  terms), the lesson cards, the stage and WebAssembly panels and the
  WebAssembly narration. Each claim in "Limits of this model" about what
  the machine runs or refuses was tried with a program of its own (36 in
  all).
- **The sources.** C99 is N1256, read with the `search-standard` skill:
  every clause's text and its printed and PDF pages. The compilers are GCC
  14's `cc1` (14.2) driven by the host GCC 13 driver, Clang 23.1.5 and a
  dev build of bcc from this branch, at `-O0` and `-O2` with `-std=c99
  -Wall -Wextra`, on x86-64 Linux with glibc 2.43; other targets' type
  facts come from `clang --target=… -dM -E`. The probes are `c29` to `c37`
  in [`claims/`](claims/), and
  [`claims/results-part20.txt`](claims/results-part20.txt) is the output of
  `run.sh`'s loop over them in C99.

### Results

All five new lessons print what the machine prints and exit 0 under GCC,
Clang and bcc at `-O0` and `-O2`, without a warning under `-std=c99
-pedantic -Wall -Wextra`: `not found after 3 tries`, `4 4 4`, `mode 4: read
1, write 0`, `walk has 4 chars` and `14 25`.

Of the 52 claims in the table, 36 hold as written (four of them now also
name Clang or the warning options), 12 over-promised and 4 were wrong; all
16 are fixed. Three of the fixes come from the review of Part 19 (#441),
and two (the size of `size_t` and a refusal's wording) are older text found
on the way. The 74 new clause glosses were each checked
against their paragraph: 71 hold, and the three that left out cases are in
the table.

| Claim | Where | Source | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Clang built the module "at `-O0` and without a C library" | WebAssembly view, Clang's module | `web/wasm/generate.sh`: a lesson that includes `<stdio.h>` is a WASI command linked with wasi-libc, whose `_start` and `__original_main` are in the module | **Wrong** for the 9 lessons that print | Says so for those (wasi-libc, whose `$_start` calls `main`); the 10 others keep "without a C library, so the module starts in `main`", and the bcc sentence drops its "also" for them |
| A `do` loop's pass "ends in a conditional branch back to the top" | WebAssembly view, every "Enter loop" step | Clang's module: `br_if 0`; bcc's: a `br 1` inside an `if` (review of #441) | **Over-promised**: literal only for Clang's module | "a branch back to the top that is taken only while the condition holds: a `br_if`, or a `br` inside an `if`" |
| The while and for steps cite §6.8.5p4 only | every loop test, jump back, `continue` and for statement's block | §6.8.5.1p1 (the while test before each pass) and §6.8.5.3p1 (for's three clauses), p. 136, PDF p. 148 | **Wrong** (missing): note 176's "goes to `k++`" rests on §6.8.5.3p1 | Each now cites its own paragraph beside §6.8.5p4 (a `do`'s `continue` §6.8.5.2p1); a test pins them |
| "continue skips the rest of one pass and goes on to the next test" | note on loops | §6.8.6.2p2, §6.8.5.3p1 | **Over-promised**: a for runs its step first | "(in a for, after its step)" |
| `*p` through a null or dangling pointer is undefined | note on `*` | §6.5.3.2p4, §6.2.4p2 | Holds | |
| Subtracting pointers counts elements, gives a signed ptrdiff_t, and is defined within one array | note on pointer difference | §6.5.6p9 | Holds | |
| `a[i]` is `*(a + i)` for an array or a pointer | note on subscripts | §6.5.2.1p2 | Holds | |
| `return` copies the value out; the address of an automatic local would dangle | note on a frame's end | §6.2.4p2, §6.8.6.4p3 | Holds | |
| A hexadecimal digit is four bits, `010` is 8, `0755` packs three 3-bit groups | note on bases | §6.4.4.1p3-4 | Holds | |
| `x & 1` is 1 exactly when x is odd; `2 & 1` is 0 and `2 && 1` is 1; `&` evaluates both operands | note on bitwise operators | §6.5.10p4, §6.5.13p3; GCC, Clang, bcc | Holds (for a negative x, on two's complement machines, which x86-64 and WebAssembly are) | |
| `&&` and `\|\|` go left to right with a sequence point, unlike `+` | note on `&&` and `\|\|` | §6.5.13p4, §6.5.14p4, §6.5p3 | Holds | |
| `c ? 1 : 2.5` is a double | note on `?:` | §6.5.15p5 | Holds | |
| Arguments have no order and no sequence point between them; in `int a = 1, b = a + 1;` b is 2 | note on the comma operator | §6.5.2.2p10, §6.8p3 | Holds | |
| A call through a null pointer, or a pointer cast to the wrong type, "and a compiled program usually crashes there" | note on pointers to functions | `c30`: the null call crashes at `-O0` under all three and at `-O2` under GCC, but exits 44 under Clang and 0 under bcc at `-O2`; `c31`: the wrong type prints 21 (GCC `-O2`: a different garbage double each run), never crashes | **Over-promised** | The first usually crashes without optimization; the second often runs on, with caller and callee disagreeing about the arguments and result |
| A call through a null pointer: "a compiled program usually crashes here" | the stop at such a call | `c30` | **Over-promised** | Crashes without optimization, but an optimizer may assume the call never happens |
| "In compiled code the pointer holds the address where twice's machine code starts" | every call through a pointer (callbacks, notes 43, 71, 126, 155) | `c36`: an address on x86-64; the WebAssembly view's `call_indirect` steps: a table index | **Over-promised**, and contradicted by the WebAssembly view | "In x86-64 code … ; in WebAssembly it holds an index into a table of functions" |
| A static initializer "reading a variable … is refused there" | note on constant expressions | `c32`: `static const int a = 1; int b = a;` compiles silently under GCC and Clang, even with `-pedantic`, and bcc refuses it; `c32b`, `c33`: a plain variable or a call is refused by all three; §6.6p10 lets an implementation accept other constant expressions | **Over-promised** | Compilers refuse those, though GCC and Clang accept reading a const variable whose value they know, as C lets them |
| "GCC accepts a const one as an extension, which §6.6p10 allows" | the machine's refusal of such an initializer | `c32` | Holds, but Clang does too | Names Clang |
| A call's operand order "can print different things under different compilers" | the stop at `g + f()` or two printing calls | `c37`: GCC prints 11 and `ba`, Clang and bcc 1 and `ab` | Holds | |
| Enumeration constants are int; an enumeration's object may hold any value of its type | note and steps on enumerations | §6.4.4.3p2, §6.7.2.2p4 | Holds | |
| "GCC chooses int when a constant is negative and unsigned int otherwise" | every enumeration's declaration | `c29`: GCC, Clang and bcc all agree; GCC manual, *Structures, Unions, Enumerations, and Bit-Fields* | Holds | Names Clang too |
| "The type  itself is compatible with unsigned int", with a blank name | an enumeration with neither a tag nor a typedef name (corpus `e03`) | | **Wrong** | "The enumeration's type itself …"; a test pins it |
| Forgetting a `break` is a bug "compilers can warn about" | note on `switch` | `c34`: GCC warns with `-Wextra`, Clang only with `-Wimplicit-fallthrough`, bcc not at all | Holds | Names the two options |
| An inner name hides an outer one; "GCC's -Wshadow" warns | note on hidden names | `c35`: GCC, Clang and bcc warn with `-Wshadow`, none with `-Wall -Wextra` | Holds | Names Clang's too |
| A goto past an initializer leaves the object without its initial value | note on goto, and the jump's steps | §6.8p3, §6.8.6.1p2 | Holds | |
| A typedef makes no new type and no object | note and steps on typedefs | §6.7.7p3 | Holds | |
| ptrdiff_t "is long on x86-64" | pointer difference (strings, note 145) | `__PTRDIFF_TYPE__`: long for x86_64-linux-gnu, long long for x86_64-pc-windows-msvc | **Over-promised** | "on x86-64 Linux" |
| size_t "is unsigned long on x86-64" | sizeof steps (unsigned, note 146), glossary | `__SIZE_TYPE__`: unsigned long long for x86_64-pc-windows-msvc | **Over-promised** (older text) | "on x86-64 Linux" |
| `~2` is -3 because "x86-64 stores [negative values] in two's complement" | `~` steps | §6.5.3.3p4, §6.2.6.2p2 | Holds | |
| `0x7FFFFFFFFFFFFFFF` is a long; decimal 4294967295 "would be a wider signed type" | constant steps | §6.4.4.1p5 | Holds | |
| Pointers into one array compare like their subscripts; one past the end counts as the length | pointer comparisons | §6.5.8p5 | Holds | |
| A refusal: "nested assignments/increments and unsupported syntax are rejected" | the machine's refusal of an expression it cannot parse, such as `_Bool` | Parts 13 and 14 run nested assignments and increments | **Wrong** (older text) | "expected an expression; this one uses syntax outside this subset" |
| `?:`'s operands "must both be arithmetic, the same structure type, pointers to compatible types, or a pointer and a null pointer constant" | clause gloss, §6.5.15p3 | §6.5.15p3 also allows two void operands and a pointer with a pointer to void | **Over-promised** | Lists all six cases |
| `==` compares "two numbers, two pointers to compatible types, or a pointer with the null pointer constant" | clause gloss, §6.5.9p2 | §6.5.9p2 also allows a pointer to an object with a pointer to void | **Over-promised** | Adds that case |
| Two pointers "compare equal exactly when both are null, or … the same object or the same function (or one past the same array)" | clause gloss, §6.5.9p6 | §6.5.9p6 also makes one past an array equal to an object that follows it | **Over-promised** ("exactly") | Adds that case |
| "C99 also allows two other representations, which no current machine uses" | glossary, two's complement | §6.2.6.2p2 (C99); C23 (N3220) §6.2.6.2p2 requires two's complement | **Over-promised**: not checkable | Names the two, and says C23 requires two's complement |
| Two's complement is how "every machine bcc targets" stores a negative integer | glossary | `src/target.rs`: x86-64 and wasm32 | Holds | |
| Glossary terms' clauses and pages (13 new) | glossary | `search-standard` pages | Holds | |
| Phase 4 executes directives (§5.1.1.2p1, p. 10); a directive ends at its new-line (§6.10p2) | the stop before a directive | N1256 | Holds | |
| Landing: the standard describes an abstract machine, not the CPU | landing | §5.1.2.3p1 | Holds | |
| "Limits of this model": what the machine runs (octal and hexadecimal, enumerations as constants, the operators, pointers to pointers and functions, typedefs at either scope, static initializers from constants, literals or function names, returned pointers, nested assignments, the statements) | How it works §3 | 36 programs through the engine | Holds | |
| "Limits of this model": what it refuses (a declaration directly in a switch's block, union, `_Bool`, void and static functions, a pointer to a library function, a static pointer to an object, arrays of structures or arrays, other library functions, a typedef of a const type, an update in an initializer list, function-like and expression macros) | How it works §3 | the same programs: each is refused before the first step, with a note | Holds | |
| `i = i++` is refused before the first step; `a[a[0]] = 1` stops; two printing calls and `g + f()` stop | How it works §3 | the same programs | Holds | |
| The link "C99 §6.5p3" opens PDF page 79 | How it works §3 | `search-standard` | Holds | |
| The five new lessons' cards (what each shows) | Table 1 | the lessons' runs | Holds | |
| `call_indirect` pops a table index, checks the function's type and traps if it differs | WebAssembly view | WebAssembly 2.0 core specification, `call_indirect`; the stepper's `indirect` traps on an undefined element or a type mismatch | Holds | |
| "A C function pointer compiles to such an index" | WebAssembly view | Clang's and bcc's modules of the callbacks lesson | Holds | |
| A call through a pointer "runs the function it points to, exactly as `twice(…)` would" | every call through a pointer | §6.5.2.2p1, p3 | Holds | |
| A function's name becomes a pointer to it, except after `&` or `sizeof` | designator steps | §6.3.2.1p4 | Holds | |
| A continue in a for "goes to the for statement's third clause, … and then to the condition" | loops, note 176 | §6.8.6.2p2, §6.8.5.3p1 | Holds (now cited) | |
| String literals get a null character in phase 7; adjacent literals join in phase 6 | literal steps | §5.1.1.2p1, §6.4.5p5 | Holds | |
| A switch compares the promoted value with each label's constant converted to that type, then runs on past later labels | switch steps | §6.8.4.2p5, §6.8.1p4 | Holds | |

### The lessons, step by step

The 18 lessons the machine runs were stepped to their ends before and
after the fixes (the preprocessor lesson stops before its first step, as
before). Every lesson keeps its step count. The text changed only on these
notes:

| Lesson | Notes | What changed |
| --- | --- | --- |
| Loops three ways | 47, 80, 110, 176 | the note on loops: a for's `continue` runs its step first |
| Callbacks | 15, 98 | the note on pointers to functions: when a bad call crashes |
| Callbacks | 43, 71, 126, 155 | a call through `f`: an address in x86-64 code, an index in WebAssembly |
| Walking a string with a pointer | 145 | ptrdiff_t is long on x86-64 Linux |
| Unsigned numbers wrap | 146 | size_t is unsigned long on x86-64 Linux |

The clauses changed on every loop's tests, jumps back and `continue`s, in
the 11 lessons with a while or a for: Objects, loops and calls (12 steps),
Hello (11), Names for values (15), Short-circuit evaluation (11), Loops
three ways (25), Unsigned (23), Signed overflow (6), Floating point (33),
Pointers and arrays (14), Walking a string (28) and Callbacks (18).

### Left open

- bcc has no `-Wimplicit-fallthrough`: it warns about neither a missing
  `break` nor the option.
- `x & 1` for a negative x assumes two's complement; the note does not say
  so, since every machine the site describes uses it.
