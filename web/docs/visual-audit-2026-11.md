# Site audit: a visual pass over the new lessons (2026-11)

The second half of 2026-10 and the start of 2026-11 added four lessons and
several new kinds of step, each built and checked on its own: printf and
strings, `#define`/`const`/casts/`+=`, every integer type with a byte-wide
tape, floating point with its bit fields and 55-digit exact values, the
compiler stages' output before their explanation, and the C library in the
in-browser compiler. Table 1 grew from 10 lessons to 14. This audit looks at
them together, as a newcomer would, at
`agent/machine-floating` (#366) merged with `agent/site-output-first` (#353)
and `agent/explorer-stdio` (#354).

## Method

The site was built with `web/build.sh`, served from `web/dist`, and driven
with Playwright in Chromium at 1440x900 and 390x844, each in the light and
dark themes, with reduced motion so the tape does not glide between shots.
For each configuration the script opened:

- the landing page, Table 1 and the glossary;
- every lesson at its first step, at the first step of each kind it has (the
  note's category, plus whether the step draws floating bits, ran a
  preprocessing directive or stopped), and at its last step;
- the compiler pipeline at all eleven stages for "Objects, loops and calls"
  and "Floating point: close, not exact";
- the WebAssembly view of the floating lesson, before and after six
  instructions.

That made 715 screenshots (one browser run per lesson, to stay under the
machine's memory limit), with the phone views taken twice: with the
workbench at the top of the window and with the note at the top. On each state the script also measured: whether
the page scrolls sideways; every visible element that clips its text; the
WCAG contrast of every visible text node against its composited background;
and where the note's lead sentence, the tape and the cells sit relative to
the fold. I looked at each screenshot, at full size for 1440x900 light and
the phone views of the new lessons, and as contact sheets for the rest, and
read every lead sentence of the four new lessons from the recorded run.

After the fixes the same script ran again over every lesson at 1440x900
light and 390x844 dark. Each lesson still steps to its end with a title and
a lead sentence at every step, no page logged an error, and the step counts
(Step presses from the first note to the last) are the same as before, in
Table 1's new order:

| Lesson | Steps |
| --- | ---: |
| Objects, loops and calls | 197 |
| Hello, world: printf and strings | 86 |
| Names for values: #define, const and casts | 97 |
| Recursion and frames | 78 |
| Integer promotions | 34 |
| Unsigned numbers wrap; signed numbers don't | 183 |
| Signed overflow | 37 |
| Floating point: close, not exact | 230 |
| Pointers and arrays | 140 |
| Structs and static storage | 134 |
| Reading past the end | 20 |
| Uninitialized reads | 6 |
| From source to a running program | 23 |
| Before the compiler: the preprocessor | 0 (stops at `#include <limits.h>`) |

The screenshots of each fix are in
[`screenshots/visual-pass/`](screenshots/visual-pass/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`.

## Findings

Most visible first. "Fixed" items have a before and after screenshot.

### 1. The lessons that print push the note below the fold

Every string literal is an object with static storage duration, so the tape
gave each its own lane at the top of the deck. The floating lesson has six,
the unsigned lesson three, the printf and constants lessons two or one. At
1440x900 the floating lesson's note started at y = 842 and its lead sentence
ended at y = 905 to 942 from step 20 on: the line that ran was on screen,
but what it did was not, and the new bit fields were entirely below the
fold. The literals' lanes say little (a literal never changes), while
`main`'s objects, the ones the lesson is about, were pushed under them.

**Fixed.** A static group that holds nothing but string literals' arrays
starts folded to one row, labelled with how many literals it holds; its
label opens it, as any folded group's does. A program with its own static
objects (the structs lesson) keeps the group open. Reads of a literal still
draw their slant, from the folded row.
[`tape-floating-1440-light`](screenshots/visual-pass/tape-floating-1440-light-before.png)
([after](screenshots/visual-pass/tape-floating-1440-light-after.png)),
[`tape-unsigned-1440-dark`](screenshots/visual-pass/tape-unsigned-1440-dark-before.png)
([after](screenshots/visual-pass/tape-unsigned-1440-dark-after.png)),
[`tape-hello-390-dark`](screenshots/visual-pass/tape-hello-390-dark-before.png)
([after](screenshots/visual-pass/tape-hello-390-dark-after.png)).

### 2. Figure 2 draws string literals over their operator

The schematic placed each logic level a fixed 132 px apart, enough for a
variable's name but not for a string literal: `"Hello, world!"` ran into
`puts()`, and `"total" " is %.3d\n"` covered `printf()`, its value text
running off the figure's edge. The full expression's own value was also
written twice, above its wire and at its end, a few pixels apart, so
`3.0 3.0` and `14orld!"` overlapped.

**Fixed.** Each level is now as wide as its widest node plus the value on
its wire (at least the old 132 px, so short expressions look as before);
an operand's label and a wire's value longer than 16 characters end in an
ellipsis, as the tape's literals do, so the figure still fits its column;
and the full expression's value is written once, at the end of its wire.
[`schematic-puts-1440-light`](screenshots/visual-pass/schematic-puts-1440-light-before.png)
([after](screenshots/visual-pass/schematic-puts-1440-light-after.png)),
[`schematic-literal-1440-light`](screenshots/visual-pass/schematic-literal-1440-light-before.png)
([after](screenshots/visual-pass/schematic-literal-1440-light-after.png)),
[`schematic-puts-390-light`](screenshots/visual-pass/schematic-puts-390-light-before.png)
([after](screenshots/visual-pass/schematic-puts-390-light-after.png)).

### 3. Table 1 no longer reads as a progression

Fourteen lessons in one list, in the order they were written: the
whole-compiler lesson sat fourth, between `#define` and pointers; the three
undefined-behavior lessons were split from the unsigned lesson they follow
on from by the floating lesson; and nothing said which lessons belong
together. At 390 px the list is 2,245 px tall.

**Fixed.** Table 1 is in four parts, each captioned like a figure with what
its lessons share: *Objects and control* (objects and calls, printf,
`#define`/`const`, recursion), *Types and conversions* (promotions, unsigned,
signed overflow, floating point), *Memory and undefined behavior* (pointers,
structs, reading past the end, uninitialized reads) and *Inside the
compiler* (the whole path, the preprocessor). The lessons are numbered
straight through, and the page still opens at the first lesson.
[`table1-1440-light`](screenshots/visual-pass/table1-1440-light-before.png)
([after](screenshots/visual-pass/table1-1440-light-after.png)),
[`table1-390-dark`](screenshots/visual-pass/table1-390-dark-before.png)
([after](screenshots/visual-pass/table1-390-dark-after.png)).

### 4. String literals and exact values read as prose

The note's lead marks names and source as code, but string literals and the
55-digit exact values were plain text in the 25 px italic serif:
`"total" " is %.3d\n"` read as a quotation with stray quotes, and a
comparison's two exact values filled six lines. A literal's creation also
listed every one of its characters (42 for the unsigned lesson's format).

**Fixed** in the engine's narration (`src/abstract_machine/narration*`):
string literals, their arrays and their first elements are code; the first
60 digits of an exact value are code, so they set in the smaller monospace
face and wrap as one number; and a literal's creation leads with its length
and its null character, leaving the characters to the explanation.
[`joined-literal-1440-light`](screenshots/visual-pass/joined-literal-1440-light-before.png)
([after](screenshots/visual-pass/joined-literal-1440-light-after.png)),
[`compare-1440-light`](screenshots/visual-pass/compare-1440-light-before.png)
([after](screenshots/visual-pass/compare-1440-light-after.png)),
[`literal-created-1440-light`](screenshots/visual-pass/literal-created-1440-light-before.png)
([after](screenshots/visual-pass/literal-created-1440-light-after.png)).

### 5. The floating bit fields look borrowed

The bit fields were drawn in a style of their own: hairline grey boxes, a
sentence-case caption in body text, and labels that collided on a phone
("signexponent 1024", since the one-digit sign box was narrower than its
label). Next to them the value-flow chips have 1.5 px ink borders on the
cell colour, and the note's other headings are small capitals.

**Fixed.** The fields are chips like the value flow's, with the sign marked
in vermilion and the exponent in blue by an inset rule, a small-capitals
label over each, and a caption like "Why it matters"; no field shrinks
below its label.
[`bits-390-dark`](screenshots/visual-pass/bits-390-dark-before.png)
([after](screenshots/visual-pass/bits-390-dark-after.png)),
[`bits-1440-dark`](screenshots/visual-pass/bits-1440-dark-before.png)
([after](screenshots/visual-pass/bits-1440-dark-after.png)).

### 6. The preprocessor button runs off a phone screen

"See the real preprocessor: open the Preprocessor stage", on every
`#include` and `#define` step, kept to one line, so at 390 px it ran past the
card and widened the workbench to 459 px.

**Fixed.** Its label wraps.
[`joined-literal-390-dark`](screenshots/visual-pass/joined-literal-390-dark-before.png)
([after](screenshots/visual-pass/joined-literal-390-dark-after.png)).

### 7. Text below WCAG AA contrast

Three real failures among the measured text (the others were the hatched
cells' transparent text and visually hidden labels):

- the program's exit status, vermilion on the inverted "Program finished"
  plate: 2.6:1 in the dark theme;
- a floating cell's bytes on a cell lit by a store: 4.1:1 (light);
- the "Undefined behavior" label of a stop: 4.4:1 (light).

**Fixed.** The status sits on a vermilion chip in dark ink, a lit cell's
bytes take the lit cell's ink, and the light theme's danger red is a little
darker (#a3221b).
[`return-status-390-dark`](screenshots/visual-pass/return-status-390-dark-before.png)
([after](screenshots/visual-pass/return-status-390-dark-after.png)),
[`lit-cell-bytes-1440-light`](screenshots/visual-pass/lit-cell-bytes-1440-light-before.png)
([after](screenshots/visual-pass/lit-cell-bytes-1440-light-after.png)),
[`fault-kind-1440-light`](screenshots/visual-pass/fault-kind-1440-light-before.png)
([after](screenshots/visual-pass/fault-kind-1440-light-after.png)).

### 8. Lead sentences that do not stand on their own

Reading only the lead sentences of the four new lessons:

- **A macro's use** led with "`LIMIT` is a macro the preprocessor replaced
  with `4` before the program ran: …", and for the joined literal went on
  with a capitalized "Translation phase 6 joined …" mid-sentence. **Fixed:**
  "Before the program ran, the preprocessor replaced the macro `LABEL` with
  `"total"`, and translation phase 6 joined …".
  [`macro-use`](screenshots/visual-pass/macro-use-1440-light-before.png)
  ([after](screenshots/visual-pass/macro-use-1440-light-after.png)).
- **A const object** was created as "an int object" while its type read
  `const int`. **Fixed:** "a const int object".
  [`const-declaration`](screenshots/visual-pass/const-declaration-1440-light-before.png)
  ([after](screenshots/visual-pass/const-declaration-1440-light-after.png)).
- **printf's result** was "The statement's value, 13, is not used", with
  nothing to say where 13 came from. **Fixed:** "The value `printf`
  returned, 13, is not used: the statement called it only for what it does."
  [`discard`](screenshots/visual-pass/discard-1440-light-before.png)
  ([after](screenshots/visual-pass/discard-1440-light-after.png)).
- **`7 / 2 = 3`**, the step the floating lesson turns on, said only that the
  result "fits in an int, so it is well defined". **Fixed:** an int division
  that drops a remainder says so, and that the quotient truncates toward
  zero (C99 6.5.5p6).
  [`division`](screenshots/visual-pass/division-1440-light-before.png)
  ([after](screenshots/visual-pass/division-1440-light-after.png)).
- **A rounded sum** read "The exact sum 0.3000… is no double". **Fixed:** "No
  double holds the exact sum `0.3000…`, so it rounds to the nearest one".
  [`sum`](screenshots/visual-pass/sum-1440-light-before.png)
  ([after](screenshots/visual-pass/sum-1440-light-after.png)).

### 9. Terms the new lessons use but the glossary lacks

Counting the narration of every step, five terms of the standard appeared
in the new lessons without a definition: *string literal* (55 uses),
*header* (16), *null character* (30), *conversion specification* (11) and
*compound assignment* (48).

**Fixed.** The glossary defines all five, each with its paragraph of N1256
(6.4.5p5, 7.1.2p1, 5.2.1p2, 7.19.6.1p4, 6.5.16.2p3), so the narration marks
them like the others.
[`glossary-1440-light`](screenshots/visual-pass/glossary-1440-light-after.png).

### 10. Figure captions leave a word alone

At 1440 px "Figure 2 — The full expression in main(), wired up" broke before
its last word, on every step. **Fixed:** the figure, table and tape captions
balance their lines (`text-wrap: balance`), visible in every workbench shot
above.

## Left for later

- **A run with many named objects still reaches the fold.** The last steps
  of the structs and unsigned lessons have eight and nine lanes of the
  programs' own objects; their leads end at y = 904 at 1440x900. Folding a function's lanes when they have not changed
  for a while, or letting the deck scroll inside a fixed height, would fix
  it, but changes how the tape reads, and wants its own design pass.
- **`size_t` is narrated as `unsigned long`.** The machine resolves `size_t`
  to its x86-64 type when it parses, so `size_t size = sizeof values;`
  creates "an unsigned long object", and the tape and Table 1 of the
  datasheet say `unsigned long` too. The `sizeof` step and the glossary
  explain the link; keeping the spelling would need the parser to carry the
  typedef name to each object.
- **Citations repeat their heading.** A puts step cites 7.19.7.10p2 and
  7.19.7.10p3, both titled "The puts function"; folding paragraphs of one
  clause into one line (`§7.19.7.10p2, p3`) would shorten every library
  call's note.
- **The phone pipeline keeps the output below the fold.** At 390x844 the
  source pane, the stage strip and the stage's lead fill the screen, so the
  output starts below it at every stage. The output-first change (#353) put
  the output before the explanation; on a phone the source could shrink too.
- **The Logical lines stage breaks a source path mid-word** at 390 px
  (`…/spli` / `cing.rs`).
