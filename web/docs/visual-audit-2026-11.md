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

That made 766 screenshots (one browser run per lesson, to stay under the
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

Part 2 below fixes all five.

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

## Part 2: what the first pass left (2026-11)

The five items above were fixed on `agent/site-polish-2`, stacked on this
pass (#371). The method was the same, with the script extended: at every
step of every lesson at 1440x900 it scrolls the workbench to the top of the
window and records where the note's lead sentence ends, whether each cell
the step lit passes a hit test at its centre (inside the window, not
scrolled out of the cells' row, not under a sticky bar), and whether the
tape segments the step used sit inside the deck's scrolled window. On the
fixed build it ran at all four configurations: every lesson, the landing
page and glossary, the pipeline at all eleven stages for two lessons, and
the WebAssembly view, 756 screenshots in all. On this pass's build it ran
every lesson at 1440x900 light and the phone pipeline, for the before
column below.

| Measure, 1440x900, all 1,279 notes of the 14 lessons | Before | After |
| --- | ---: | ---: |
| Notes whose lead ends below the fold (y > 900) | 98 | 0 |
| Lowest lead bottom | 1054 | 894 |
| Notes with a segment they used outside the deck's window | 5 | 0 |
| Notes with no lit cell on screen | 0 | 0 |
| Tallest deck window | 493 px | 360 px |

The 98 were in "Structs and static storage" (55), "Recursion and frames"
(21), "Unsigned numbers wrap" (21) and "Floating point" (1). After the fix
the light and dark runs measured the same. Every lesson still steps to its
end with a title and a lead sentence at every step in all four
configurations, no page logged an error, and the step counts are those in
the table above (197, 86, 97, 78, 34, 183, 37, 230, 140, 134, 20, 6, 23 and
0).

The screenshots are in
[`screenshots/visual-pass-2/`](screenshots/visual-pass-2/), named as in
part 1.

### 1. The note under a tape of many lanes

**Fixed.** The deck grew to 493 px before it scrolled, so eight or nine
lanes pushed the note's lead past the fold. It now grows only to what the
window leaves for its key, the cells, the note's heading on two lines and
four lines of its lead (`100svh` less the sticky bars and 400 px), then
scrolls, and each step scrolls it to the rows it used: all of them when
they fit, else those it stored into, in the same scroll that moves the tape
sideways. When the "More rows below" key appears it takes 24 px from the
window, so the deck follows the step's rows again then. Below 821 px, where
the note comes before the deck, the deck keeps its former height. Two leads
were long enough to reach the fold on their own: a printf with five
conversions (six lines) and the decay of a 41-character literal, spelled
out twice. A printf with more than three conversions now leads with which
argument each took, and a literal longer than 24 characters decays to "its
first element".
[`tape-structs-1440-light`](screenshots/visual-pass-2/tape-structs-1440-light-before.png)
([after](screenshots/visual-pass-2/tape-structs-1440-light-after.png)),
[`tape-structs-static-1440-light`](screenshots/visual-pass-2/tape-structs-static-1440-light-before.png)
([after](screenshots/visual-pass-2/tape-structs-static-1440-light-after.png)),
[`tape-structs-call-1440-light`](screenshots/visual-pass-2/tape-structs-call-1440-light-before.png)
([after](screenshots/visual-pass-2/tape-structs-call-1440-light-after.png)),
[`tape-unsigned-1440-dark`](screenshots/visual-pass-2/tape-unsigned-1440-dark-before.png)
([after](screenshots/visual-pass-2/tape-unsigned-1440-dark-after.png)),
[`size-printf-1440-light`](screenshots/visual-pass-2/size-printf-1440-light-before.png)
([after](screenshots/visual-pass-2/size-printf-1440-light-after.png)).

### 2. `size_t` keeps its name

**Fixed.** A typedef name is a synonym for its type (6.7.7p3), so the
object's type is still `unsigned long`, but an object declared `size_t`
(automatic, static, through a pointer or in an array) now spells its type
as the program wrote it: its declaration leads with "a `size_t` object",
and the tape label, the cell and Table 1 of the datasheet say `size_t`. The
declaration cites 7.17p2, and its explanation says the header defines the
name as `unsigned long` here; printf's `%zu` names the `size_t` it takes.
[`size-declaration-1440-light`](screenshots/visual-pass-2/size-declaration-1440-light-before.png)
([after](screenshots/visual-pass-2/size-declaration-1440-light-after.png)),
[`size-datasheet-1440-light`](screenshots/visual-pass-2/size-datasheet-1440-light-before.png)
([after](screenshots/visual-pass-2/size-datasheet-1440-light-after.png)),
[`size-tape-1440-dark`](screenshots/visual-pass-2/size-tape-1440-dark-before.png)
([after](screenshots/visual-pass-2/size-tape-1440-dark-after.png)),
[`size-cells-390-dark`](screenshots/visual-pass-2/size-cells-390-dark-before.png)
([after](screenshots/visual-pass-2/size-cells-390-dark-after.png)).

### 3. Each cited clause once

**Fixed.** Paragraphs of one clause share one line, `C99 §7.19.7.10p2, p3`
and the heading, each paragraph still its own link and named in full to a
screen reader. A printf step's citations went from seven lines to four.
[`citations-puts-1440-light`](screenshots/visual-pass-2/citations-puts-1440-light-before.png)
([after](screenshots/visual-pass-2/citations-puts-1440-light-after.png)),
[`citations-puts-390-dark`](screenshots/visual-pass-2/citations-puts-390-dark-before.png)
([after](screenshots/visual-pass-2/citations-puts-390-dark-after.png)).

### 4. A phone's pipeline leads with the output

**Fixed.** At 390x844 the stage's output began at y = 1016 to 1081, below
the fold, at every stage. On a phone the output (or why bcc stopped, then
the missing stage's note) now comes right after the stage keys that name
the stage, at y = 516 at all eleven stages of both lessons, and the
heading, the switches and the hand-off follow it.
[`pipeline-lines-390-light`](screenshots/visual-pass-2/pipeline-lines-390-light-before.png)
([after](screenshots/visual-pass-2/pipeline-lines-390-light-after.png)),
[`pipeline-ir-390-light`](screenshots/visual-pass-2/pipeline-ir-390-light-before.png)
([after](screenshots/visual-pass-2/pipeline-ir-390-light-after.png)),
[`pipeline-asm-390-dark`](screenshots/visual-pass-2/pipeline-asm-390-dark-before.png)
([after](screenshots/visual-pass-2/pipeline-asm-390-dark-after.png)).

### 5. Source paths wrap at their slashes

**Fixed.** Each `/` in the stage's source path is followed by a word-break
opportunity, so `src/translation_phases/preprocessor_tokenizer/splicing.rs`
wraps between directories.
[`path-lines-390-light`](screenshots/visual-pass-2/path-lines-390-light-before.png)
([after](screenshots/visual-pass-2/path-lines-390-light-after.png)).

### Left for later

Part 3 below fixes both.

- **A wide object's cells run past the cells' row.** When a step creates or
  reads a string literal or an array of more bytes than the row shows, the
  row starts at its first lit byte and the
  rest are off to the right: 2 of 42 bytes on screen at the third literal
  of the unsigned lesson. Some lit cells are on screen at every step, so
  the rule holds, but a string could read better as one wide cell.
- **The phone's "Earlier steps" strip clips long titles** with an ellipsis
  ("#include <stdio.h>: declare printf, puts…"), as designed; the full
  title is in the note above it.

## Part 3: wide objects under the head (2026-11)

The two items Part 2 left were fixed on `agent/site-wide-objects`. The
script is Part 2's, with one measure added: at every step it takes each
byte the step created, read, wrote or designated (a lit scalar cell, or one
lit byte of a string cell) and counts it on screen when all of it lies
inside the cells' scrolled row and the window. At 1440x900 it measures with
the workbench at the top of the window, as Part 2 did. At 390x844, where the
note comes before the deck and the cells follow it, it measures with the
cells scrolled to the middle of the window, so there it asks whether the
row shows those bytes, not whether they share the first screen with the
note. It ran every lesson at all four configurations on `main`'s build
(before) and on this branch's (after): 1,279 notes, 1,350 bytes used, in
each.

| Measure, all 1,279 notes of the 14 lessons | 1440 before | 1440 after | 390 before | 390 after |
| --- | ---: | ---: | ---: | ---: |
| Bytes a step used that are off screen (of 1,350) | 442 | 0 | 500 | 0 |
| Notes with a byte they used off screen | 24 | 0 | 25 | 0 |
| Worst note: bytes on screen | 2 of 42 | 42 of 42 | 1 of 42 | 42 of 42 |
| Widest cells' row, scrolled | 25,934 px | 2,240 px | 25,934 px | a row per call |
| Cells' height, median and tallest | 64, 87 px | 64, 87 px | 64, 87 px | 142, 411 px |
| Notes whose lead ends below the fold (y > 900) | 0 | 0 | – | – |
| Lowest lead bottom | 894 | 894 | – | – |

The light and dark runs measured the same in every row. The 24 notes were
the creation and every printf or puts of the string literals in the hello,
`#define`, unsigned and floating lessons (4, 2, 6 and 12 notes); on a phone
one more, `values` of the unsigned lesson being created, whose third cell
was off the right edge. Every lesson still steps to its end with a title
and a lead sentence at every step in all four configurations, no page
logged an error, and the step counts are those of Part 1 (197, 86, 97, 78,
34, 183, 37, 230, 140, 134, 20, 6, 23 and 0).

The screenshots are in
[`screenshots/visual-pass-3/`](screenshots/visual-pass-3/), named as in
part 1.

### 1. A string is one cell of its characters

**Fixed.** Each of a literal's bytes was a cell of its own, 44 px or more
wide and named by the whole literal (`"%zu bytes, sum %u, -1 < 1u is %d,
%u, %u\n"[2]`), so each was as wide as the spelling: the unsigned lesson's
three literals, 83 bytes, made its row 21,634 px wide, and the strip
scrolled to the first byte the step used. Of
the three ways to fix it, the one wide cell reads best: a string is text,
and the program wrote it as text. A string literal's array, or a char array
of more than eight bytes, is now one cell with the scalar cells' grammar
and height: its type and size above; its characters, a column per byte,
over an index ruler (a tick per byte, its index every eighth byte); and its
name below. The null character is boxed, a byte of its own, and so is
counted in the ruler and in `char[42] 42B`. The cell is lit as a scalar
cell is when the step uses the whole string (created, or read by printf);
a step that uses some of its bytes marks only those, in place (the decay
of a literal to a pointer outlines its first byte). The row breaks between
words before the cell outgrows a phone, so the 42-byte format is one line
at 1440 px and two at 390 px. A key in the cell's corner opens it into its
byte cells, named by index (`[2]`), with a key cell in front that folds
them back; the choice holds from step to step. A char array of up to eight
bytes keeps its byte cells: the hello lesson's `name` (`"Ada"`) is read one
byte at a time by `length()`, and its four cells show each value with its
character.

Wrapping every object's bytes onto rows (the brief's second way) would
have made the 42-byte format six rows of byte cells on a phone, and a row
that scrolls to the lit bytes (the third) still shows a string through a
slot. The strip also now scrolls to every cell the step used, not only the
first: in the middle when they fit, else the first of them at the left.
[`literal-unsigned-1440-light`](screenshots/visual-pass-3/literal-unsigned-1440-light-before.png)
([after](screenshots/visual-pass-3/literal-unsigned-1440-light-after.png)),
[`literal-floating-1440-light`](screenshots/visual-pass-3/literal-floating-1440-light-before.png)
([after](screenshots/visual-pass-3/literal-floating-1440-light-after.png)),
[`printf-unsigned-1440-dark`](screenshots/visual-pass-3/printf-unsigned-1440-dark-before.png)
([after](screenshots/visual-pass-3/printf-unsigned-1440-dark-after.png)),
[`decay-hello-1440-light`](screenshots/visual-pass-3/decay-hello-1440-light-before.png)
([after](screenshots/visual-pass-3/decay-hello-1440-light-after.png)),
[`open-unsigned-1440-light`](screenshots/visual-pass-3/open-unsigned-1440-light-before.png)
([after, opened](screenshots/visual-pass-3/open-unsigned-1440-light-after.png)),
[`literal-unsigned-390-dark`](screenshots/visual-pass-3/literal-unsigned-390-dark-before.png)
([after](screenshots/visual-pass-3/literal-unsigned-390-dark-after.png)).

### 2. A phone gives each call a row

**Fixed.** With strings as one cell, one kind of step still lost bytes on
a phone: one that uses objects of two calls. printf in the hello lesson
reads `name` in `main()` and its format in static storage, which do not
fit together in a strip 366 px wide. Below 600 px each call's cells are
now a row of their own under its name, as wide as its cells up to the
screen, and each row scrolls on its own to the cells the step used, so
both show. A first version wrapped every cell onto as many rows as it
needed; it showed every byte too, but the cells grew to a median of 253 px
and up to 673 px (the floating lesson's six literals and eight objects), so
the cells now grow by a row per active call instead: a median of 142 px,
the floating lesson's tallest 204 px, and the tallest of all 411 px, where
the recursion lesson has five calls active and reads as the stack it is,
one row per frame.
[`printf-hello-390-light`](screenshots/visual-pass-3/printf-hello-390-light-before.png)
([after](screenshots/visual-pass-3/printf-hello-390-light-after.png)),
[`printf-hello-1440-dark`](screenshots/visual-pass-3/printf-hello-1440-dark-before.png)
([after](screenshots/visual-pass-3/printf-hello-1440-dark-after.png)),
[`printf-unsigned-390-dark`](screenshots/visual-pass-3/printf-unsigned-390-dark-before.png)
([after](screenshots/visual-pass-3/printf-unsigned-390-dark-after.png)).

### 3. Earlier steps keep their titles

**Fixed.** The folded "Earlier steps" strip and its list cut a long title
with an ellipsis ("#include <stdio.h>: declare printf, puts…"). A title now
wraps: beside the heading when it fits, else on its own lines under it.
[`recent-hello-390-light`](screenshots/visual-pass-3/recent-hello-390-light-before.png)
([after](screenshots/visual-pass-3/recent-hello-390-light-after.png)),
[`recent-unsigned-390-dark`](screenshots/visual-pass-3/recent-unsigned-390-dark-before.png)
([after](screenshots/visual-pass-3/recent-unsigned-390-dark-after.png)).

### 4. Escapes and the string keys

**Fixed after review.** A string cell's words broke after 16 bytes. That
bounds a word of letters (about 125 px) but not a word of escapes: 16
`\xff` made a word 448 px wide, more than a phone's 318 px row, so the
bytes at the end of each line were clipped. The reviewer's 33-byte
literal lost 10 of them at 390 px. A word now ends before it is 16
columns wide. An escape counts a column for each character of its
spelling, and the boxed null character counts three. A word also wraps
within itself if a row is narrower still. A pointer into such a literal
showed the literal's whole spelling, a cell 1,085 px wide. It now shows
the start of the spelling, as the tape's lanes do
(`→"\xff\xff\x…"[0]`).

The `BYTES` and `TEXT` keys redraw the strip, and that left the focus on
the page's body. Now the focus moves to the key that replaces the one
pressed, and a status region announces that the string opened into its
byte cells or folded back.

A probe ran eight programs to their end at eleven sizes from 320 to
1440 px. The programs were built from escapes and from runs with no
spaces: 32 `\xff` as a literal and as an array, 32 `\377`, 20 `\n\t`,
mixed escapes, 24 `\0`, 120 letters with no space, and the reviewer's
mixed string. Before the fix, 29 of the 402 bytes the steps used were
off screen at 390 px, and 43 at 320 px. After it, none were off screen at
any size. The tallest string cell was 150 px at 390 px, and 240 px at
320 px (the 120 letters). Enter, Space and a click each left the focus on
the replacement key, at 390 and 1440 px in both themes. The table above
was measured again after these fixes and did not change.

### Left for later

- **Other wide arrays still scroll.** No lesson has an array of another type
  wider than the window, but a program in the workbench can (`int a[100]`):
  a step that creates it lights more cells than the row shows. The same
  folding (one cell of its values, opened on demand) would fit it. Done
  later: an array of more than eight elements of a type other than char is
  one cell of its values, a column per element over the string cell's index
  ruler, wrapping between words of values, with a `CELLS` key that opens it
  into its element cells and a `VALUES` key that folds them back. For a
  program with `int squares[100] = {0, 1, 4, 9};`, the step that creates it
  had 12 of its 100 elements on screen at 1440x900 and 4 at 390x844, in a
  row 8,566 px wide; it is now one cell 521 px wide (366 px on a phone)
  and two rows of values high, which scroll within the cell to the
  elements each step uses, so each later step's element is on screen as
  before. Lessons are unchanged: their arrays have three or four elements.
  A review found that the fold first grew as tall as its values: for
  `int a[500]`, the write to `a[499]` was marked 1,292 px down a 686 px
  cell at 1440x900, with its note pushed below it, where the unfolded row
  had been 88 px high and followed the element. Two rows (a cell 90 px
  high) keep the old row's footprint, and the first, middle and last
  elements of 500 and of 512 (the engine's largest array) are each marked
  inside the cell's two rows at both sizes, in both themes, with and
  without reduced motion.
  [`follow-1440-light`](screenshots/wide-arrays/follow-1440-light-before.png)
  ([after](screenshots/wide-arrays/follow-1440-light-after.png)),
  [`follow-390-dark`](screenshots/wide-arrays/follow-390-dark-before.png)
  ([after](screenshots/wide-arrays/follow-390-dark-after.png), `long long a[100]`).
  [`create-1440-light`](screenshots/wide-arrays/create-1440-light-before.png)
  ([after](screenshots/wide-arrays/create-1440-light-after.png)),
  [`initialize-1440-light`](screenshots/wide-arrays/initialize-1440-light-before.png)
  ([after](screenshots/wide-arrays/initialize-1440-light-after.png),
  [opened](screenshots/wide-arrays/open-1440-light-after.png)),
  [`create-390-dark`](screenshots/wide-arrays/create-390-dark-before.png)
  ([after](screenshots/wide-arrays/create-390-dark-after.png)),
  [`initialize-390-dark`](screenshots/wide-arrays/initialize-390-dark-before.png)
  ([after](screenshots/wide-arrays/initialize-390-dark-after.png)).
- **A char array inside a structure** keeps its byte cells.
- **The tape and Table 1** still show a literal's value as `{'%','z',…}`,
  cut with an ellipsis; they could spell it as the string cell does. Done
  later: they spell it `"%zu…"` ([`claims-audit.md`](claims-audit.md#visuals)).

## Part 4: the compiler stages as teaching (2026-11)

The first three parts and the earlier audits checked the stage views for
layout, contrast and correctness, but none asked whether they teach: the
eleven stages Figure 0 promises, from the logical lines to machine code,
and the WebAssembly text beside them. This part reads them as a programmer
new to compilers would, on `main` at `da963840b` (before) and on
`agent/site-pipeline-views` (after).

### Method

The site was built with `web/build.sh`, served from `web/dist`, and driven
with Playwright in Chromium with reduced motion, one browser run per lesson
and configuration. "Objects, loops and calls" ran at 1440x900 and 390x844 in
both themes, "From source to a running program" (the whole-path lesson) at
1440x900 light and 390x844 dark, and "Floating point: close, not exact" at
1440x900 light. At every stage the script took a screenshot with the stage
card at the top of the window and recorded:

- the stage's header (its kicker, command, hand-off sentence and
  explanation) and where each part sits;
- the colour, weight and style of every token on screen in the output
  editor, with its WCAG contrast on the editor's background, and samples
  of the text each colour carried;
- every source line hovered in turn, with the number of output lines it
  marked, and every output line on screen hovered, with the number of
  source lines it marked;
- the folding controls in the output editor;
- for the five stages after semantic analysis, the first line on screen
  after scrolling the output 400 px, after switching to `-O2`, after
  switching back, and after visiting another stage and returning;
- sideways scroll of the page, clipped text, text running off the window,
  and the contrast of every text node in the pipeline card.

The WebAssembly view's listing (the floating lesson) was measured the same
way for its tokens. A second script took the before and after pairs below,
named `<item>-<width>-<theme>-before.png` and `…-after.png` in
[`screenshots/pipeline-views/`](screenshots/pipeline-views/).

### Findings

#### What each stage is, takes, gives and is for

Every stage led with one sentence on what it hands to the next (the
hand-off) and a button to that stage. What goes in and what comes out
opened the stage's explanation ("In: the typed tree. Out: bcc's
intermediate representation…"), but the explanation sat under the output,
the C99 rules and the "In this program" note: 884 to 934 px below the
stage's heading at 1440x900, always below the fold. No stage said why a
compiler has it. The words the stages use (IR, SSA, basic block, ABI,
register) were in no glossary: the glossary held only the C standard's
terms.

**Fixed.** Beside the hand-off each stage now lists *In*, *Out* and *Why*:
"In: the typed syntax tree. Out: bcc IR: simple operations on values, in
basic blocks. Why: C has many ways to write one computation. A few simple
operations, with every jump explicit, give the optimizer one form to
improve and every back end one form to translate." The explanation starts
at what the stage decides. The glossary has a second part, *Compiler
terms*: logical source line, preprocessing token, translation unit and
token, each with its C99 paragraph, and syntax tree, intermediate
representation, static single assignment, basic block, optimizer, ABI,
register, assembly language and machine code, each with a link that opens
the stage showing it. The In, Out and Why lines link their terms there. The
narration does not mark these words, so no lesson's note changed. The
brief sits in a column of its own where the view beside the source is
900 px wide or more, and under the hand-off on a phone; at 1440x900 the
output moved down by 0 to 61 px (it starts 384 to 457 px under the stage's heading, from 384 to
431).
[`brief-ir-1440-light`](screenshots/pipeline-views/brief-ir-1440-light-before.png)
([after](screenshots/pipeline-views/brief-ir-1440-light-after.png)),
[`brief-asm-1440-dark`](screenshots/pipeline-views/brief-asm-1440-dark-before.png)
([after](screenshots/pipeline-views/brief-asm-1440-dark-after.png)),
[`brief-ir-390-dark`](screenshots/pipeline-views/brief-ir-390-dark-before.png)
([after](screenshots/pipeline-views/brief-ir-390-dark-after.png)),
[`glossary-terms-1440-light`](screenshots/pipeline-views/glossary-terms-1440-light-after.png).

#### Token colours

Every editor is the device's dark screen in both themes (`#0b0b0b` in the
light theme, `#050505` in the dark), so a token's contrast is the same in
both. Every colour passed WCAG AA: the lowest, the muted grey of comments,
directives and offsets, is 6.9:1. The colours did not separate the kinds a
newcomer needs to tell apart, though. Measured as CIEDE2000 differences,
where about 10 is a difference seen at a glance:

| Kinds side by side | Where | Before | After |
| --- | --- | ---: | ---: |
| A type and a value (`i32`, `v3`; `double`, `%v3`) | bcc IR, LLVM IR | 7.5 | 25.7 |
| A register and a label (`%rbx`, `.LBB1_2:`) | assembly, machine code | 7.0 | 42.9 |
| A label and a number | everywhere | 36.1 | 15.8 |
| A value and a string | IR | 39.1 | 20.0 |

Beyond the palette, the grammars gave one thing two colours or two things
one:

- a jump's or call's target (`jge .LBB1_4`, `callq bump`) was the near-white
  of a plain name, while the label it names was pink;
- in LLVM IR a block named by a branch (`label %block2`) or by a phi was a
  value, while its definition (`block2:`) was a label;
- an instruction's modifiers (`nsw`, `align`, `inbounds`, `noundef`,
  `external`) were the instructions' orange, set apart only by weight, and a
  comparison's condition (`slt`) was pink like the block names around it;
- an assembler directive's operands (`.cfi_offset %rbp, -16`) were as
  bright as an instruction's, and `objects.c`'s assembly has 32 directive
  lines among its 36 instructions;
- the machine code's bytes, the stage's whole output, were the same muted
  grey as the offsets beside them;
- in the floating lesson a float constant's bits (`0x401C000000000000`)
  split into a `0` and a name, LLVM's `sitofp`, `fptosi` and `oeq` were
  plain text, and `3.000000e-01` broke at its point.

**Fixed.** Values are teal and labels khaki; a jump's or call's target, and
an LLVM block named as a target, takes its label's colour; modifiers and
conditions recede (grey, and plain orange); a directive is muted whole; the
machine code's bytes are as plain as text, with only the offsets muted; and
the floating IR's constants, casts and conditions are coloured as the
integer IR's are. The WebAssembly text (`wat`) shares the palette: its
locals, the values of that listing, are teal too. Comments stay italic grey,
registers pink, immediates and other numbers ochre, types blue and
instructions bold orange, all 6.9:1 or more.
[`colors-asm-1440-light`](screenshots/pipeline-views/colors-asm-1440-light-before.png)
([after](screenshots/pipeline-views/colors-asm-1440-light-after.png)),
[`colors-ir-1440-light`](screenshots/pipeline-views/colors-ir-1440-light-before.png)
([after](screenshots/pipeline-views/colors-ir-1440-light-after.png)),
[`colors-llvm-1440-light`](screenshots/pipeline-views/colors-llvm-1440-light-before.png)
([after](screenshots/pipeline-views/colors-llvm-1440-light-after.png)),
[`colors-machine-1440-light`](screenshots/pipeline-views/colors-machine-1440-light-before.png)
([after](screenshots/pipeline-views/colors-machine-1440-light-after.png)),
[`wat-1440-light`](screenshots/pipeline-views/wat-1440-light-before.png)
([after](screenshots/pipeline-views/wat-1440-light-after.png)).

#### `-O0` against `-O2`

The two levels could be seen only one at a time. The optimizer note under
each stage lists what changed and draws bars of the counts, but to see what
`-O2` did to a line a reader switched levels and lost the `-O0` text.

**Fixed.** At the five stages the optimizer changes, *Compare -O0 with -O2*
sets the other level's output beside the selected one, `-O0` on the left,
each under its caption (one above the other on a phone). Pointing at a
source line marks what it became at both levels: at `-O2` the call to
`bump` on line 13 is the hatched, inlined body of `bump` in `main`, beside
the `call @bump` of `-O0`. Pointing at a line of either output marks its
source, and the reading names the level ("Line 9 of the -O2 bcc IR came
from source line 14."). A diff was the other choice, but it pairs lines by their
text, not by the source line they came from: a line diff of `objects.c`'s
LLVM IR marks 20 of the 36 lines of its `-O2` functions, most for a value
renumbered, while the links pair the two levels by source line.
[`compare-ir-1440-light`](screenshots/pipeline-views/compare-ir-1440-light-before.png)
([after](screenshots/pipeline-views/compare-ir-1440-light-after.png)),
[`compare-asm-1440-light`](screenshots/pipeline-views/compare-asm-1440-light-before.png)
([after](screenshots/pipeline-views/compare-asm-1440-light-after.png)),
[`compare-ir-390-dark`](screenshots/pipeline-views/compare-ir-output-390-dark-after.png)
(after only: the phone sets the levels one above the other).

A review found two gaps in the comparison, both fixed. A level whose
compilation stopped before the stage (`-O0` refusing `__rdtsc` in the
assembly, which `-O2` folds away) left its pane empty; it now shows that
level's stop banner and why the stage did not run, and when the selected
level is the one that failed, the other level's output stays beside the
explanation.
[`failed-level-1440-light`](screenshots/pipeline-views/failed-level-1440-light-before.png)
([after](screenshots/pipeline-views/failed-level-1440-light-after.png);
[phone](screenshots/pipeline-views/failed-level-390-dark-after.png),
[`-O0` selected](screenshots/pipeline-views/failed-selected-level-1440-dark-after.png)).
And the cursor in the other level's editor changed the reading on the page
but was never announced; it now is, after the cursor rests, as in the
selected level's editor, and with both levels on screen every sentence names
its level ("Line 7 of the -O2 bcc IR came from source line 3.").
[`keyboard-beside-1440-light`](screenshots/pipeline-views/keyboard-beside-1440-light-before.png)
([after](screenshots/pipeline-views/keyboard-beside-1440-light-after.png)).

#### Text below WCAG AA

In the dark theme, "In this program:", the label of the note that reads
the stage's output, was vermilion on the note's plate of light ink: 2.6:1,
at every stage of both lessons. No other text in the pipeline card fell
below AA in any configuration.

**Fixed.** The label takes the light theme's darker accent on that plate
(5.0:1), and keeps vermilion on the light theme's dark plate (6.1:1). All
four configurations now measure no failure.
[`brief-asm-1440-dark`](screenshots/pipeline-views/brief-asm-1440-dark-before.png)
([after](screenshots/pipeline-views/brief-asm-1440-dark-after.png)).

#### What held

- **Source and output link both ways at every stage.** Hovering each source
  line of `objects.c` marked output lines at all eleven stages: every line
  in the logical lines, the preprocessing tokens and the preprocessed text,
  and every line with code from the tokens to the machine code (13 to 16 of
  its 19 lines; a declaration whose value lives in a register, such as
  `int total = 0;`, has no line of its own in the LLVM IR or the assembly,
  and the reading says that nothing came from it). Hovering the output lines on screen marked their source lines,
  except the lines that are no single line's code (a function's directives,
  its prologue), which a dotted margin marks and the legend explains. The
  cursor's links share the hover's code path and were not measured again.
- **Long outputs fold.** The IR, ABI-lowered IR, LLVM IR, assembly and
  machine code fold by function and block; the token and type listings are
  flat lists and have no folds.
- **Phones.** No stage scrolled the page sideways or clipped text at 390 px
  in either theme, before or after; the output still comes first.
- **The WebAssembly text** measured 6.9:1 or more for every token class.

### Left for later

- **Scroll position follows pixels, not code.** Switching levels or stages
  keeps the output's scroll offset in pixels: after scrolling the assembly to
  line 45, `-O2` showed line 33 of a shorter listing, and the machine code
  opened in the middle of `main`. Keeping the same function (or the line
  under the pointer) in view would need the editor to report its first
  visible line to Rust.
- **The linked lines can be below the fold.** At 1440x900, with the stage
  card at the top of the window, the output shows its first 9 or 10 lines
  above the fold; hovering source line 13 marks assembly lines 55 and 56,
  inside the editor's view but under the window's edge. The source pane
  stays beside the output when the page scrolls down to it, so a reader can
  see both, but the page does not move there by itself.
- **The IR opens with two lines a newcomer cannot read**, the target triple
  and the data layout string (`e-m:e-p270:32:32-…`); folding them into one
  line, as the preprocessed text folds headers, would put the first function
  first.
- **Function names are the colour of plain text** in the C source and the IR
  (`@main`), as the theme intends for the source; the IR could set them
  apart.

## Part 5: the stage views' follow-ups (2026-11)

Part 4 left four items: the output's scroll position followed pixels, not
code; linked lines could sit under the window's edge; the IR opened with two
lines a newcomer cannot read; and function names had no colour of their own.
This part fixes them on `agent/site-stage-followups` and then reads the stage
views once more as a newcomer meets them.

### Method

The site was built with `web/build.sh` from `main` at `99f9b655d` (before)
and from this branch (after), and served from `web/dist`. On this machine the
build ran Trunk without `--release` (the release profile's link-time
optimization is off limits here) and with the dev profile at `opt-level=s`,
which keeps the Wasm small enough for one browser within the memory limit;
the sources, the editor bundle and the page are those CI builds. A
Playwright script drove "Objects, loops and calls" in Chromium at 1440x900
and 390x844, in both themes, with reduced motion, and once more at 1440x900
light with motion allowed. Each run recorded:

- **the place:** the output of bcc IR, the assembly, the machine code and
  LLVM IR scrolled into `main`, then the first line in view after switching
  to `-O2`, back to `-O0`, to another stage and back, with the source line
  the reading names for it (read by pointing at it, which makes it the
  reader's place); and both levels side by side;
- **the linked lines:** with the stage card at the top of the window and the
  output at its start, the pointer resting on a source line, the page's
  scroll sampled every 100 ms for 1.4 s and where the linked lines end up;
  then the pointer swept across nine lines without resting, and the wheel
  turned just before resting, with how far the page moved; and a line chosen
  with the keyboard (1440) or a tap (390);
- **the IR's opening lines:** the first lines of bcc IR, the ABI-lowered IR
  and LLVM IR, the reading and the screen-reader announcement for line 1,
  and **Show target** pressed with the keyboard;
- **function names:** the colour and WCAG contrast of every function name
  on screen in the five listings;
- sideways scroll of the page at all eleven stages.

The screenshots are in
[`screenshots/stage-followups/`](screenshots/stage-followups/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`.

### Findings

#### 1. The output keeps the reader's place in the code

Monaco kept the output's scroll offset in pixels when the text changed under
it. With bcc IR scrolled to line 28, `v9 = iconst.i64 8` (the second
element's offset in `int values[3] = {2, 4, 6};`, source line 9), the
assembly then opened at its line 28, `.type main,@function`, a directive
before `main`'s first instruction. With the levels side by side, the other
level's pane opened at its first line, `.text`, whatever the selected one
showed.

**Fixed.** The output editor reports its first line in view to Rust, which
keeps the reader's place as a source line and its row on screen
(`pipeline::Place`): the source line of the first line in view, or of the
line under the pointer, or, for a label or directive, of the nearest line
below it that has one. When the text changes (another level, another stage,
an edit), the editor opens with that source line's first output line at the
same row, or, where the new text has none (a declaration whose value lives
in a register, a line `-O2` folded away), with the nearest line of the same
function that has code. A scroll the editor makes itself to reach the place
leaves the place as it was, so a round trip through `-O2` comes back to the
same code. Now bcc IR at line 28 (source line 9) opens `-O2` at line 16, `v1
= iconst.i32 2`, the same statement, and the assembly at line 44, `movl $2,
0(%rsp)`, its first store; the other level's pane, beside the selected one,
opens at the same statement too, as far down as its shorter text allows
(at `-O2` the loop with `bump` inlined, beside `-O0`'s `total + next`).
[`place-ir-to-asm-1440-light`](screenshots/stage-followups/place-ir-to-asm-1440-light-before.png)
([after](screenshots/stage-followups/place-ir-to-asm-1440-light-after.png)),
[`place-compare-asm-1440-light`](screenshots/stage-followups/place-compare-asm-1440-light-before.png)
([after](screenshots/stage-followups/place-compare-asm-1440-light-after.png)).

#### 2. Linked lines come into view, gently

With the stage card at the top of a 1440x900 window and the output at its
start, pointing at source line 9 marked three lines of the machine code at
y 969 to 1035 and five of the thirteen lines of bcc IR it became at 932 to
1042: all under the window's edge, where they stayed. On a phone, a tap on
source line 3 marked assembly lines 15 and 16 at y 820 to 864, under the
844 px window.

**Fixed.** Pointing at a source line (or moving the cursor to it) still
brings the lines it became into the output editor's view, now the whole run
of them when it fits, and then, once the pointer or the cursor has rested
for 400 ms, scrolls the page as little as shows them clear of the sticky
header and above the window's edge. It leaves the page alone when the
reader turned the wheel, touched or dragged the page, or pressed a scroll
key in the last 1.5 s; when no part of the output is on screen; and when
the scroll would slide another line under a hovering pointer: on a wide
screen the source pane is stuck beside the output, so the pointer stays on
its line, and on a phone the page scrolls for a tap or the keyboard only as
far as keeps the chosen line in view. A run the reader scrolled away from
with the wheel is not brought back when the pointer leaves and the cursor's
link returns. With reduced motion both scrolls jump; otherwise they glide.
Now the machine code's three lines move to 826 to 892 (the page scrolled
143 px, about 400 ms after the pointer came to rest), bcc IR's thirteen
lines to 606 to 892 (52 px), and the phone's to 792 to 836 (28 px), with the
tapped line still on screen. Sweeping the pointer across nine lines without
resting, and resting just after a turn of the wheel, moved the page by
0 px in every configuration.
[`linked-machine-code-1440-light`](screenshots/stage-followups/linked-machine-code-1440-light-before.png)
([after](screenshots/stage-followups/linked-machine-code-1440-light-after.png)),
[`selected-line-390-light`](screenshots/stage-followups/selected-line-390-light-before.png)
([after](screenshots/stage-followups/selected-line-390-light-after.png)).

#### 3. The IRs open with their first function

bcc IR opened with `target triple = "x86_64-unknown-linux-gnu"` and `target
datalayout = "e-m:e-p270:32:32-…"`, LLVM IR with `; ModuleID = 'bcc'`,
`source_filename = "bcc"` and the same two lines, and the reading said only
"Line 1 of the bcc IR has no source line."

**Fixed.** These lines fold into one comment that says what they are,
`; x86_64-unknown-linux-gnu: the target and its data layout, folded (Show
target to read them)`, as the preprocessed text folds each header
(`pipeline/target.rs`); the first function is on line 3. The line is
marked as a fold, and its hover reads the triple's parts (processor
x86_64, vendor unknown, operating system linux, environment gnu) and the
data layout's commonest entries (`e`: little-endian; `i64:64`: a 64-bit
integer aligned to 64 bits; `S128`: the stack aligned to 128 bits). The
reading, and for the cursor the announcement, say "Line 1 of the bcc IR
stands for the lines that name the machine the code is for
(x86_64-unknown-linux-gnu), and how that machine stores data (its data
layout). No source line made them; Show target puts them back." **Show
target**, beside **Compare -O0 with -O2**, is a toggle button
(`aria-pressed`), and pressed with the keyboard it puts the lines back. The
links below the fold move up with the text. The glossary's compiler terms
gain *Target triple* and *Data layout*.
[`target-bcc-ir-1440-light`](screenshots/stage-followups/target-bcc-ir-1440-light-before.png)
([after](screenshots/stage-followups/target-bcc-ir-1440-light-after.png),
[hover](screenshots/stage-followups/target-ir-hover-1440-light-after.png)),
[`target-llvm-ir-1440-dark`](screenshots/stage-followups/target-llvm-ir-1440-dark-before.png)
([after](screenshots/stage-followups/target-llvm-ir-1440-dark-after.png)),
[`target-bcc-ir-390-dark`](screenshots/stage-followups/target-bcc-ir-390-dark-before.png)
([after](screenshots/stage-followups/target-bcc-ir-390-dark-after.png)).

#### 4. Function names have their own colour

A function's name was the near-white of plain text in the IRs (`@main`,
17.5:1 and 18.1:1 in the two themes) and the khaki of a jump's label in the
assembly and the machine code (`main:`, `<main>:`, 15.4:1 and 15.9:1), so a
function's start looked like any block's.

**Fixed.** Where a listing defines, declares or calls a function (`function
@main(`, `define i32 @main(`, `call @bump(`, `main:`, `callq bump`,
`<main>:`), its name is orchid, `#ff88e2`: 9.2:1 on the light theme's
screen and 9.6:1 on the dark one's, and at least 17 in CIEDE2000 from every
other token colour (the nearest is the slots' lavender). The first orchid,
`#ee78d6` (7.8:1 and 8.1:1, measured on every name on screen in the five
listings in all four configurations), fell to 3.9:1 and 4.2:1 where a
selection and a linked line's fill overlap under a name (`rgb(93,57,55)` and
`rgb(90,53,51)`, measured by review); `#ff88e2` keeps 4.7:1 and 5.0:1
there. A global's name keeps the plain colour (`@calls`,
`@moved.total` in "Structs and static storage", whose `@moved` and
`@llvm.memcpy` calls are orchid), and a local label (`.LBB1_2:`) keeps its
khaki. The C source keeps its plain names.
[`names-assembly-1440-dark`](screenshots/stage-followups/names-assembly-1440-dark-before.png)
([after](screenshots/stage-followups/names-assembly-1440-dark-after.png)),
[`names-bcc-ir-1440-light`](screenshots/stage-followups/names-bcc-ir-1440-light-before.png)
([after](screenshots/stage-followups/names-bcc-ir-1440-light-after.png)),
[`names-machine-code-1440-light`](screenshots/stage-followups/names-machine-code-1440-light-before.png)
([after](screenshots/stage-followups/names-machine-code-1440-light-after.png)).

#### 5. A newcomer's pass

Read again as a newcomer meets them, the stage views showed one more thing
worth fixing, and the new scrolling one more thing to settle:

- **A hover on the first line was cut off.** Monaco opened the folded line's
  explanation above it, under the editor's top edge, so only its last line
  showed. Hovers now open below their line where they fit.
- **The editors' own smooth scrolling was always on.** It now follows the
  reader's reduced-motion preference, also when that changes, as does the
  caret's animation, so the editor's reveal jumps when the page's scroll
  does.

No stage scrolled the page sideways in any configuration, before or after,
and no run logged an error.

### Left for later

- On a phone, where the output editor is shorter, the folded line's hover
  has no room below it and still opens above, cut off; a touch screen does
  not hover, and the reading under the output says the same.
- LLVM IR still ends with its attribute groups and module flags
  (`attributes #0 = { "frame-pointer"="all" }`, `!llvm.module.flags`), which
  a newcomer cannot read either; the same fold could take them.

## Part 6: the abstract machine's steps (2026-11)

Parts 4 and 5 read the compiler's stages as a newcomer meets them. This part
reads the abstract machine's stepper the same way: every lesson, every step,
as a programmer who knows some C but not how the standard specifies it,
asking at each step what just happened, why C requires it, what to look at,
and why one line of C takes several steps. It fixes what it found on
`agent/site-machine-steps`.

### Method

The site was built with `web/build.sh` from `main` at `a3d8da99a` (before)
and from this branch (after), and served from `web/dist`, as in Part 5:
Trunk without `--release`, the dev profile at `opt-level=s`. Playwright
scripts drove Chromium:

- **the walk:** at 1440x900 in the light theme with reduced motion, every
  step of the twelve machine lessons from the first note to the last (1,254
  notes), recording each note's title, kind, sequence point, lead, value
  flow, clauses and "Why it matters", the source the step lit, and whether
  the page scrolled sideways; screenshots at chosen steps of "Objects, loops
  and calls";
- **the steps of a line:** "Objects, loops and calls" at 1440x900 and
  390x844 in both themes with reduced motion, and at 1440x900 dark with
  motion allowed: fourteen presses of Over from the start; the note at steps
  5, 15, 31, 37, 45 and 59, and at step 35 after stepping back from 59; a
  store into a cell, with the animations running on it; the WCAG contrast of
  every text node in the new panel against its composited background; and a
  keyboard focus on its first entry;
- **after the review** (item 4 below): the review's own probes, a program
  of two `f();` calls stepped and stepped over, and step 37 of the first
  lesson; and Over pressed from the first note of every lesson to its end,
  recording the panel's heading at each stop. The walk and the steps of a
  line ran again on the fixed build.

The screenshots are in
[`screenshots/machine-steps/`](screenshots/machine-steps/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`.

### Findings

#### 1. Why one line takes several steps

The notes are good one by one: every step has a title, a kind, a plain lead,
its values as chips and its clauses, and the walk found no step without a
lead and no page that scrolled sideways. What a newcomer cannot see is how
they add up. `int total = 0;` takes four notes (create `total`, designate
it, evaluate `0`, store), `int values[3] = {2, 4, 6};` takes ten, and
`int next = bump(cursor);` takes six of main's own and 21 inside `bump`. Nothing on the page said which line or expression a step
belonged to, how far along it was, or when it was done; the "Earlier steps"
list shows the last four titles, whatever they belonged to. The engine also
reported what the next step stores ("Next: store into `total`") on every
snapshot, and the page never showed it.

**Fixed.** The note now shows the steps of the expression the current step
belongs to, in the order the machine took them (`panels/evaluation.rs`):
"Evaluating `next = bump(cursor)` · step 5 of 5 · in main()", then one
entry per step with its number and title, wired left to right. The step
shown is vermilion, the page's colour for "now"; steps after it (when the
reader stepped back) are dashed; each entry jumps to its step. A call the
expression made folds into one double-ruled entry, "bump() runs: 21 steps",
so the six steps of `next = bump(cursor)` read as one line with a call in
it; the steps inside `bump` show `bump`'s own expressions. A dashed vermilion
rule after a step marks a sequence point, and one leading a folded call marks
the sequence point after its arguments are evaluated and before the call
(C99 §6.5.2.2p10), however many steps the call folds. A key under the steps says
what one is (every side effect before it is complete, C99 §5.1.2.3p2, linked
to N1256) and that the tape's clock pulse stands taller there, which it
already did without saying so. While the run has not recorded the rest, the
last entry is what the engine says comes next, "next: store into total", or
"more to come"; the engine's next operation also tells when the expression
is done, so its last step reads "step 3 of 3" before the run goes past it.

The grouping reads only what the engine reports: a step belongs to the
expression whose tree it shows (its root's source range) in the same call,
and the steps of a call belong to it while its frame waits on a call inside
that range. A call's first step shows the called function, which has no
expression yet: it belongs to the expression that holds the call its caller
waits on, which the engine reports by its source range, so the two calls of
`f(); f();` are two expressions however alike they are spelled. A
declaration creating its object, a block's entry and exit, and a jump
evaluate no expression, and show no panel, in a called function as well.
Its unit tests run the first lesson and small programs on the real engine
(`panels/evaluation/tests.rs`): a call without arguments, two calls spelled
alike, calls nested through frames, a call a `return` statement makes, the
sequence point before a call, and where Over stops. Of the walk's 1,254
notes, 1,027 show the panel; at the 192 steps where the next note belongs
to another expression, none says "so far".

**Over** (O), beside Step, runs to the last step of the current expression,
through any call it makes, as the WebAssembly view's Over runs a call to its
return; at that last step, or at a step that evaluates no expression, it
runs to the last step of the next one. From the start of the first lesson,
fourteen presses visit steps 1, 2, 5, 6, 9, 12, 15, 16, 21, 22, 23, 26, 31
and 32: `total = 0`, each element of `values`, `cursor = &values[0]`, `i =
0`, the loop's condition, with the declarations' own steps between them.
It shows "Pause" on Run while it runs, and stops after Run's 1,000-step
budget.

On a phone the steps run down a rail at full width, a sequence point a
dashed rule across it.

[`expression-1440-light`](screenshots/machine-steps/expression-1440-light-before.png)
([after](screenshots/machine-steps/expression-1440-light-after.png)),
[`back-1440-light`](screenshots/machine-steps/back-1440-light-before.png)
([after](screenshots/machine-steps/back-1440-light-after.png)),
[`expression-1440-dark`](screenshots/machine-steps/expression-1440-dark-before.png)
([after](screenshots/machine-steps/expression-1440-dark-after.png)),
[`expression-390-dark`](screenshots/machine-steps/expression-390-dark-before.png)
([after](screenshots/machine-steps/expression-390-dark-after.png),
[light](screenshots/machine-steps/back-390-light-after.png)).

#### 2. A call's first step was "not part of an expression"

At step 37, "Call bump (frame #2)", Figure 2 said "This step is not part of
an expression": the engine shows the called function's state, which has no
expression yet. It is part of one, `next = bump(cursor)`, which waits for
the call.

**Fixed.** At a call's first step Figure 2 now says "The call
`bump(cursor)` is part of an expression in main(), which waits for it to
return. bump()'s own expressions are wired up here as they run.", from the
frame the engine reports as waiting on that call.
[`call-1440-light`](screenshots/machine-steps/call-1440-light-before.png)
([after](screenshots/machine-steps/call-1440-light-after.png)).

#### 3. A step's effect did not move

Each step marks what it did, still: the cell it stored into is vermilion,
the one it read underlined in blue, the one it designated outlined, and the
value flow draws an arrow from operands to result. Stepping quickly, the
marks jump from cell to cell, and a store looks the same as a cell that was
vermilion a step ago.

**Fixed, with motion allowed.** A stored value now lands in its cell (the
value drops in as the vermilion comes up, 380 to 520 ms), a read cell
flashes a ring of the data-flow blue that settles into its underline
(700 ms), a designated cell's outline draws in, and the value-flow wire
draws from the operands to the result, which arrives after it. The current
step's entry among the expression's steps arrives the same way. With
reduced motion (`prefers-reduced-motion: reduce`) the marks are the same
and still: the motion run found `cell-lit` and `cell-stamp` running on the
stored cell, the reduced-motion runs none.
[`cells-1440-dark-motion`](screenshots/machine-steps/cells-1440-dark-motion-after.png).

#### 4. What the review of the panel found

A review of this branch found two gaps in the panel itself, both fixed here.

- **The sequence point before a call was dropped.** The engine marks a
  call's first step as a sequence point, but the folded entry had no field
  for it: at step 37 the panel showed the argument read, then
  `bump() runs`, with no rule between them, and the entry's label did not
  say so. The entry now leads with the dashed rule (across the rail on a
  phone) and its label opens "A sequence point, then the call to bump".
  [`call-point-1440-light`](screenshots/machine-steps/call-point-1440-light-before.png)
  ([after](screenshots/machine-steps/call-point-1440-light-after.png),
  [phone](screenshots/machine-steps/call-point-390-light-after.png)).
- **A call without arguments lost its expression.** The panel took a call's
  expression from the step before it, and `f();` has none of its own: in
  `int main(void) { f(); f(); return 0; }` the first call showed no panel
  and Over stopped inside `f` at its block's entry, and the second call was
  grouped with the first call's steps, since the caller's frame was still
  on the stack. The caller's frame now carries the source range of the call
  it waits on (the engine already reported it, as the text Figure 2 shows),
  and the call's expression is the one that holds it. Over from the first
  call stops at step 6, "Discard the result", and the second call's steps
  begin at its own entry, step 7.
  [`empty-call-1440-light`](screenshots/machine-steps/empty-call-1440-light-before.png)
  ([after](screenshots/machine-steps/empty-call-1440-light-after.png)),
  [`empty-second-call-1440-light`](screenshots/machine-steps/empty-second-call-1440-light-before.png)
  ([after](screenshots/machine-steps/empty-second-call-1440-light-after.png)).

#### Checks

- **Contrast:** the panel's lowest is 5.25:1 (a step's number on the
  vermilion entry, `#3a1406` on `#ff5a1f`) in both themes and both widths;
  plain text on the panel is at least 5.36:1 in the light theme and 5.47:1
  in the dark one. The current call's entry, whose tint the measured step
  does not show, was computed: its title 12.3:1 and 11.9:1, its number
  8.3:1 and 8.4:1 (the plainer grey of the other numbers would have been
  4.6:1 in the light theme).
- **Keyboard and screen readers:** each entry is a button with a visible
  focus ring (the page's 2 px vermilion outline) and a label that says the
  step, its title, a sequence point after it and whether it is the step
  shown (`aria-current`); a call's says its steps, after the sequence
  point that leads it. The panel is a section
  named by its heading. O presses Over, and Over announces the step it
  stops at as Step does.
- **No sideways scroll** at any step of any lesson at 1440, or at the
  checked steps at 390, in either theme.
- **The notes are unchanged:** the walk's step counts and every title and
  lead are the same before and after in all twelve lessons, and no page
  logged an error. Each of the eleven call entries the walk passes shows
  the panel with the sequence point leading its call.
- **Over stops where it should:** pressed from the first note of each of
  the twelve lessons to its end (335 presses), it stops each time at a
  step that evaluates no expression or at the last step of a complete
  expression, never inside a call, and reaches every lesson's last state.

### Left for later

- **The note is below the fold at 1440x900** with the workbench at the top
  of the window, as the 2026-10-30 audit found: at step 40 of the first
  lesson, with seven lanes on the tape, the note's title is at y = 783, and
  the new panel sits below it. A phone
  already puts the note first.
- **A block's entry does not say which objects begin.** C99 §6.2.4p5 starts
  every automatic object of a block at the block's `{`; the engine creates
  each at its declaration, and its note says C has it exist from the start
  of the block. Marking the block's objects at its `{` (and their end at its
  `}`, which "Leave a block" already names) needs the engine to report the
  block's objects at entry. Done in Part 7.
- **An initializer list is split by element.** The tree the engine reports
  is one element's, so `int values[3] = {2, 4, 6};` shows three expressions
  ("Evaluating 2", "4", "6"), with the sequence point after the last.
- **What comes next** is known only for a store, an update and a library
  call (the engine's pending effect); for a read or an operator the last
  entry says "more to come". The engine's next operation could name it.
- **The tape keeps its clock unexplained** except by this panel's key; its
  caption could say what the taller pulses are.

## Part 7: lifetimes at a block's braces (2026-11)

Part 6 left one thing about objects untold: when they begin. C99 §6.2.4p5
gives an automatic object (one without a variable-length array type, which
the machine leaves out) a lifetime "from entry into the block with which it
is associated until execution of that block ends in any way". Its initial
value is indeterminate, and its initializer runs "each time the declaration
is reached" (§6.2.4p5, §6.8p3). Its name, though, is in scope only "just
after the completion of its declarator" (§6.2.1p7). So the object exists
before its name can be used. The machine created each object at its
declaration, and its note said so ("The machine draws `total` here; C has
it exist from the start of its block"). This part makes the machine do what
C says, and the page show it.

### Method

The site was built with `web/build.sh` from `main` at `8275dac74` (before)
and from this branch (after), as in Part 6: Trunk without `--release`, the
dev profile at `opt-level=s`. Playwright scripts drove Chromium:

- **the walk:** every step of the twelve machine lessons from the first note
  to the last (1,254 notes), recording each note's title, lead, value flow,
  clauses and "Why it matters", and whether the page scrolled sideways. The
  after build was walked at 1440x900 and 390x844, in the light and dark
  themes, with and without reduced motion: eight runs. The before build was
  walked at 1440x900 light and 390x844 dark, both with reduced motion. Each
  run took screenshots at every lesson's first block entry, first
  declaration and first end of a lifetime. The two phone runs with motion
  allowed were stopped by the memory watchdog after eight lessons and after
  none, so they ran again in four groups of lessons, each in a fresh browser;
- **workbench probes:** eight programs loaded into the workbench and stepped
  to their end, with screenshots at every block entry, declaration, end of
  a lifetime and return, before and after, at 1440x900 light and 390x844
  dark; the dangling-pointer probe also at 1440x900 dark, with motion
  allowed. The programs are below;
- **contrast:** the WCAG contrast of every new text node against its
  composited background, at both widths in both themes.

The screenshots are in [`screenshots/lifetimes/`](screenshots/lifetimes/),
named `<item>-<width>-<theme>-before.png` and `…-after.png`.

### Findings

#### 1. A block's objects begin at its brace

**Fixed.** On entry to a block, the engine now creates every automatic
object the block declares, each indeterminate, and the step names them:
"Begin the lifetime of `total`, `values` and `cursor`", leading "Entering
the block creates `total`, `values` and `cursor`: each exists from here,
indeterminate until something is stored." Its explanation adds that the
names cannot be used yet, and that entering a nested block or calling a
function suspends the block without ending it. It cites §6.2.4p5, §6.2.1p7,
§6.7.8p10 and §6.8.2p2. A block that declares nothing still says "Enter a
block". The value flow lists the objects as chips, "`total` `values`
`cursor` begin, holding ?". Their cells are stamped vermilion, as a store's
are, and their tape lanes start at the brace, cross-hatched. Table 1 dates
them from this step.

The engine change is small (`parser.rs` lists each block's objects on its
`Enter` instruction, and `machine.rs` creates them there), and the step
counts are the same: the step that created an object now reaches its
declaration instead.

[`entry-1440-light`](screenshots/lifetimes/entry-1440-light-before.png)
([after](screenshots/lifetimes/entry-1440-light-after.png),
[dark](screenshots/lifetimes/entry-1440-dark-after.png)),
[`entry-390-dark`](screenshots/lifetimes/entry-390-dark-before.png)
([after](screenshots/lifetimes/entry-390-dark-after.png)).

#### 2. Reaching a declaration is a step of its own

**Fixed.** The step that used to read "Create `total` (int)" now reads
"Declare `total` (int)". Its lead is "`total`, an int object that has
existed since its block began, is declared here: its name can be used from
now on, and its initializer runs next" (or "with no initializer it stays
indeterminate"). It cites §6.8p3 and §6.2.1p7 first. The initializer's
steps follow it as before, so the object's birth, its declaration and its
first value are three distinct moments. The value flow reads "`total` in
scope, holds ?". The cell is outlined in solid ink, where a designated
cell's outline is dashed. A notch over the object's tape lane marks the
step.

Until its declaration, an object's cell names it in italic, underlined with
dots, and its row in Table 1 says "not declared yet". The engine reports
this as each object's new `declared` field: the page's `Object` carries it,
and the JSON trace, now schema version 8, encodes it.

The name lookup changed with it. An object that a block's entry created
does not hide an outer object of the same name until its own declaration,
so the shadowing probe reads the outer `x` into `y` before the inner `x` is
declared.
[`declare-1440-light`](screenshots/lifetimes/declare-1440-light-before.png)
([after](screenshots/lifetimes/declare-1440-light-after.png)),
[`shadow-1440-light`](screenshots/lifetimes/shadow-1440-light-before.png)
([after](screenshots/lifetimes/shadow-1440-light-after.png)).

#### 3. Each pass through a loop body begins new objects

**Fixed.** A loop body is a block of its own (§6.8.5p5), so each pass
enters it and creates its objects anew. Entering a loop body now leads
"Each pass through the loop body creates a new `t`: it exists from here,
indeterminate until something is stored". Its explanation adds that the
objects of the last pass ended at the closing brace, and it cites §6.8.5p5.
On the tape each pass's `t` is a new segment on the same lane, after the
end tick of the one before.
[`loop-1440-light`](screenshots/lifetimes/loop-1440-light-before.png)
([after](screenshots/lifetimes/loop-1440-light-after.png)).

#### 4. Lifetimes end at `}` and at `return`, declared or not

Leaving a block already named the objects that end ("End the lifetime of
`t`"), and a return named its frame's objects. Leaving a block now also
shows them as chips ("`t` ends"). A return can end an object whose
declaration was never reached. In the early-return probe, `b` begins at
`pick`'s brace, and `return a + k;` runs before `int b = 2;`. Its tape lane
is hatched from the brace to the return, with no notch, and the return's
explanation adds "`b` existed from its block's opening brace, though its
declaration was never reached." Before, `b` never appeared at all.
[`early-1440-light`](screenshots/lifetimes/early-1440-light-before.png)
([after](screenshots/lifetimes/early-1440-light-after.png)),
[`early-390-dark`](screenshots/lifetimes/early-390-dark-before.png)
([after](screenshots/lifetimes/early-390-dark-after.png)).

#### 5. A dangling pointer looks dangling

The engine already ended the pointer's target and stopped the read through
it ("Use of a dangling pointer"). The page showed the pointer's value as
"→ ended" in plain type, and Table 1 gave nothing more. When its target
ends, a pointer's value becomes indeterminate (§6.2.4p2). Now the
pointer's tape lane is cross-hatched from that step, as an indeterminate
value is, its cell's value is italic over the same hatch, and its row in
Table 1 reads "dangling", with the reason in its tooltip. The cell's
label for screen readers says "dangling: the object it pointed to has
ended, so its value is indeterminate".
[`dangling-1440-light`](screenshots/lifetimes/dangling-1440-light-before.png)
([after](screenshots/lifetimes/dangling-1440-light-after.png),
[dark](screenshots/lifetimes/dangling-1440-dark-after.png)),
[`dangling-390-dark`](screenshots/lifetimes/dangling-390-dark-before.png)
([after](screenshots/lifetimes/dangling-390-dark-after.png)).

#### Intended changes to the text

Of the walk's 1,254 notes, these changed, and nothing else in any title or
lead:

- the 29 entries of blocks that declare objects: the title "Enter a block"
  became "Begin the lifetime of …", with a new lead, clauses and value
  flow. Three of these are loop bodies (`next` in "Objects, loops and
  calls"), whose lead begins "Each pass through the loop body";
- the 56 declarations: "Create `x` (T)" became "Declare `x` (T)", with a
  new lead, clauses and value flow;
- the value flow of seven block exits, which now name what ends;
- two "Why it matters" notes, at the step where each lesson first shows
  them (twelve times each). Scope: "a name
  declared inside it can be used from its declaration up to the closing
  brace … created when execution enters the block". Indeterminate: "The
  object exists from entry into its block, before its declaration is
  reached", replacing "this machine draws it when the declaration is
  reached";
- the explanations behind the disclosure, for the entries and declarations
  above and for a return that ends an undeclared object;
- the category hint for lifetime steps, now "a block begins or ends,
  creating or ending its objects, or a declaration is reached", and the
  empty cells' line, now "Entering a block puts the objects it declares on
  the tape".

#### Workbench probes

| Probe | What it shows |
| --- | --- |
| nested blocks, `a`, then `b`, then `c` | each block's object begins at its own brace; `c` and `b` end at their braces, innermost first |
| `for` with `int t` in its body | `i` begins with the `for`; a new `t` on each of three passes, each ended at the body's `}` |
| `goto` out of a block | rejected: `goto` is outside the machine's subset |
| early `return` inside an `if` | `b` begins and ends without its declaration being reached |
| an inner `x` shadowing an outer one | `int y = x;` reads the outer `x` (1); the inner `x` is `x #2` on the tape |
| `int b = a; int a = 1;` | rejected as outside the subset: `a` is not declared where it is used |
| `int x = x + 1;` | `x` is in scope in its own initializer, still indeterminate: the run stops at the read |
| `p = &x;` in an inner block, `*p` after it | `p` turns hatched and "dangling" when `x` ends; the read through it stops |

#### Checks

- **Step counts and text:** every lesson has the same number of notes
  before and after, in all twelve lessons, and every title and lead is the
  same in all eight configurations. The only differences from the before
  build are those listed above. No page logged an error.
- **No sideways scroll** at any step of any lesson in any of the eight
  configurations, or at any step of the probes.
- **Contrast:** the lowest new text is 4.99:1, "not declared yet" in the
  accent ink of a row the step used (`#b33a0b` on `#ecebe6`, the colour
  Table 1 already uses for such rows). Elsewhere: 5.89:1 for that line in
  grey (light theme) and 6.22:1 (dark), 5.25:1 for an undeclared cell's
  name on the vermilion stamp, at least 5.47:1 for a dangling cell's type
  and name, and 11.7:1 or more for its value, at both widths. The hatch
  was first across the whole dangling cell, where its lines left the small
  grey type at about 3.2:1 in the dark theme. It now sits behind the value
  alone, which keeps at least 6.9:1 against a hatch line (computed from
  the hatch's colour). The notch on the
  tape is the tape's ink on its black screen.
- **Reduced motion:** the declaration's outline draws in only with motion
  allowed, in the same `prefers-reduced-motion: no-preference` block as a
  designated cell's. The notch and the hatch do not move.
- **Tests:** the engine's tests cover a block's entry creating its objects
  and not a nested block's, a block of six objects whose lead names two and
  "the rest", the name lookup before an inner declaration, a
  self-initializing read, a new `t` on each pass, and a return ending an
  undeclared object. The site's tests cover the chips and accesses of an
  entry and a declaration, the tape's notch and its dangling segment, and
  the screen-reader summary of an entry.

### Left for later

- **`goto` is outside the subset,** so a jump out of a block, or into one
  past a declaration (where §6.2.4p5's "each time the declaration is
  reached" matters most), cannot be shown yet.
- **A name used before its declaration** is reported as outside the subset,
  not as the error it is in C: the name is not in scope there.
- **A shadowing object's title** says "Begin the lifetime of `y` and `x`"
  for the inner `x`, which the tape and Table 1 call `x #2`; the engine does
  not know the page's lane numbers.

## Part 8: the note beside the tape (2026-11)

Parts 6 and 7 left the step's note below the fold on a laptop, and four
smaller things: a shadowing object's title, Table 1's "Lives in" heading,
"what comes next", and the tape's clock. This part gives the note a column
of its own on wide screens, which is the main change, and fixes three of
the four, on `agent/site-layout-fold`.

### Method

The site was built with `web/build.sh` as in Part 7: Trunk without
`--release`, the dev profile at `opt-level=s`. The before build is #412's
last, from `9704bb59e`, whose site and engine sources are those of `main`
at `169b81fc8`. The after build is this branch at `97430e88c`. Playwright
scripts drove Chromium:

- **the fold walk:** every step of the twelve machine lessons (1,254
  notes), as a reader who brings the workbench to the top of the window
  once and then only presses Step. The script clicks the key where it is,
  as a pointer does: Playwright's own `click()` scrolls the page first, which
  moved it 424 px on the first try. At each note the walk records where the
  current line, the note's title and lead, its expression panel (and that
  panel's current step) and the cells the step touched sit, and whether
  each is wholly in the window. In the window means below the sticky
  header and control strip and above the window's foot (on a phone, above
  the docked keys). A cell must also be uncovered at its centre and inside
  its row's scrolled window. The walk also records each note's title, lead
  and value flow, sideways scroll and page errors. It ran on both builds at
  1440x900, 1366x768 and 1280x800 in both themes with reduced motion, and
  at 1920x1080 light and 390x844 dark. With motion allowed it ran at
  1440x900 dark, 1366x768 light and 1280x800 dark, over "Objects, loops and
  calls" and "Structs and static storage" (333 notes), waiting 700 ms for
  each step's motion to settle;
- **probes:** Part 7's shadowing program at 1440x900 light; Table 1's
  headings over the first 60 steps of the first lesson at 1440x900 light;
  the hanger and the tape's playhead at step 45 of the first lesson at
  1100x800, where the deck is compact;
- **contrast:** the WCAG contrast of every text node in the note, Figure 1's
  caption and Table 1's headings, against its composited background, at
  step 45 of the first lesson and at the shadowing probe's block entry, at
  1440x900, 1366x768 and 390x844 in both themes.

The screenshots are in [`screenshots/fold/`](screenshots/fold/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`.

### Findings

#### 1. The note was below the fold

The machine view stacked the tape, the cells and then the note, with
Figure 2, the standard output and Table 1 beside the note. Part 2 sized the
tape so the note's heading and four lines of its lead stayed above the fold
at 1440x900, but Part 6 put the expression's steps under the lead, and they
never fit. Before this change, at 1440x900 the panel ran past the window's
foot at 907 of the 1,027 notes that show one. At 1366x768 the lead itself
did at 104 notes, and the panel at every one. A reader pressing Step saw
the line and the tape move but had to scroll to read what the step did, and
to scroll back to see the tape.

**Fixed.** From 1181 px wide, where the listing is already a sticky column,
the note takes a column of its own at the machine's right, 360 px wide. It
sticks under the control strip and is as tall as the window below it. The
tape, the cells, Figure 2, the standard output and Table 1 stack in the
column beside it. The line that ran (in the sticky listing), the note with
its expression's steps, and the cells the step touched now share the
window however many lanes the tape has. A note longer than the window
scrolls on its own, and each step starts it at its top. The page itself
never scrolls on a step, so nothing fights the reader's own scrolling: a
reader who scrolls down to Table 1 or the standard output keeps the note
beside them.

In the new column the expression's steps run down #411's rail, which
applies wherever the note is narrow: a container query on the note at
572 px, the widest a phone's note gets, replaces the phone's media query.
The lead is set at 22 px, as on a tablet. The machine column is narrower,
so the reels give the tape their width and the deck is compact (each name
rides on its lane, as on a phone). A vermilion rule under the cells leads
from the playhead to the note. Below 1440 px the mode keys take their
short names, so the control strip keeps to one row at 1366 px, where it
used to wrap to two and take 31 px from the window. Below 1181 px nothing
changes.

The other options were weaker. Auto-scrolling the note into view on Step,
as #409 scrolls a linked line, would have had to push the tape and most of
the cells under the control strip at 768 px. A docked note would cover the
cells it describes. Shrinking the tape to fit the note under it would leave
it less than 70 px at 768 px, not two lanes.

| Window, before → after | Notes | Something out of view | Title or lead | Expression panel | Its current step | No touched cell in view | Lowest lead bottom |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1440x900 light | 1254 | 907 → 3 | 2 → 0 | 907 → 3 of 1027 | 686 → 1 | 0 → 0 of 799 | 936 → 475 px |
| 1440x900 dark | 1254 | 907 → 3 | 2 → 0 | 907 → 3 of 1027 | 686 → 1 | 0 → 0 of 799 | 936 → 475 px |
| 1366x768 light | 1254 | 1049 → 12 | 104 → 0 | 1027 → 12 of 1027 | 979 → 1 | 0 → 0 of 799 | 877 → 475 px |
| 1366x768 dark | 1254 | 1049 → 12 | 104 → 0 | 1027 → 12 of 1027 | 979 → 1 | 0 → 0 of 799 | 877 → 475 px |
| 1280x800 light | 1254 | 1029 → 11 | 5 → 0 | 1027 → 10 of 1027 | 893 → 1 | 0 → 0 of 799 | 859 → 474 px |
| 1280x800 dark | 1254 | 1029 → 11 | 5 → 0 | 1027 → 10 of 1027 | 893 → 1 | 0 → 0 of 799 | 859 → 474 px |
| 1920x1080 light | 1254 | 445 → 1 | 0 → 0 | 445 → 1 of 1027 | 209 → 1 | 0 → 0 of 799 | 1007 → 475 px |
| 390x844 dark | 1254 | 1098 → 1098 | 0 → 0 | 777 → 777 of 1027 | 315 → 315 | 799 → 799 of 799 | 682 → 682 px |
| 1440x900 dark, motion | 333 | 270 → 0 | 0 → 0 | 270 → 0 of 291 | 200 → 0 | 0 → 0 of 253 | 857 → 368 px |
| 1366x768 light, motion | 333 | 291 → 0 | 0 → 0 | 291 → 0 of 291 | 273 → 0 | 0 → 0 of 253 | 737 → 369 px |
| 1280x800 dark, motion | 333 | 291 → 0 | 0 → 0 | 291 → 0 of 291 | 262 → 0 | 0 → 0 of 253 | 757 → 368 px |

The light and dark runs at each size measured the same. With motion
allowed, over its two lessons, nothing was out of view after the change. "Something out of view" counts the
notes where the current line, the note's title or lead, its expression
panel or every touched cell was not wholly in the window. The phone keeps
its layout: the note comes first, the tape and the cells after it, as Part
2 chose, and its numbers are the same before and after.

[`fold-1440-light`](screenshots/fold/fold-1440-light-before.png)
([after](screenshots/fold/fold-1440-light-after.png)),
[`fold-1366-dark`](screenshots/fold/fold-1366-dark-before.png)
([after](screenshots/fold/fold-1366-dark-after.png)),
[`fold-1280-light`](screenshots/fold/fold-1280-light-before.png)
([after](screenshots/fold/fold-1280-light-after.png)),
[`fold-1920-light`](screenshots/fold/fold-1920-light-before.png)
([after](screenshots/fold/fold-1920-light-after.png)),
[`fold-1440-dark-motion`](screenshots/fold/fold-1440-dark-motion-before.png)
([after](screenshots/fold/fold-1440-dark-motion-after.png)),
[`fold-390-dark`](screenshots/fold/fold-390-dark-before.png)
([after](screenshots/fold/fold-390-dark-after.png)).

#### 2. A long expression's steps outran the window

In the new column the steps of a long expression can still run past the
window's foot. A printf with five conversions leads with which argument
each took (five lines), and its expression has 12 steps. At 1366x768,
before the next fix, the panel ran past the window's foot at 105 notes.

**Fixed.** Beside the tape the steps scroll in a window of their own, as
tall as the space a few lines of lead leave (the window's height less the
control strip and 440 px, at least 150 px). Each step scrolls it just far
enough to show its entry and the one after it, the next step or what comes
next. The panel's current step is now out of view at one note in each
laptop size: that printf, at its last step, whose five-line lead and value
flow leave its steps too little of the column.
[`printf-1366-light`](screenshots/fold/printf-1366-light-before.png)
([after](screenshots/fold/printf-1366-light-after.png)).

#### 3. A stop wraps the control strip

When the run stops at undefined behavior, the status ("Stopped:
indeterminate read") wraps the control strip to two rows at 1440 px, and
the page shifts under it. In the three short lessons that stop, the machine
column ended at the note's foot, so the note had no room to stay under the
taller strip, and its title slipped 5 px beneath it.

**Fixed.** Where the note has its own column, the machine view is at least
a window tall. At those stops the note now sits under the strip.
[`stop-1440-light`](screenshots/fold/stop-1440-light-before.png)
([after](screenshots/fold/stop-1440-light-after.png)).

#### 4. A shadowing object's title said `x`

Part 7's shadowing probe named the inner `x` "`x`" in the note, while the
tape and Table 1 called it `x #2`.

**Fixed.** The note's title, lead and value flow now give each object the
step used the number the tape gives its lane, where the step uses that name
for one object only: "Begin the lifetime of `y` and `x #2`", "Declare `x #2`
(int)", "Read `x #2` → 10", "End the lifetime of `y` and `x #2`". The outer
`x` keeps its name, including the read of it before the inner one is
declared. The label keeps the object's colour. The note takes only the
tape's number, not the tape's shortening of a long string literal's name; a
first version took the whole lane label, and the fold walk caught 24 notes
of the printf lessons that read `"Hello, w…"` for `"Hello, world!"`. No lesson has a
shadowing object, so no lesson's text changes.
[`shadow-1440-light`](screenshots/fold/shadow-1440-light-before.png)
([after](screenshots/fold/shadow-1440-light-after.png)).

#### 5. Table 1's "Lives in" wrapped

At note 1 of the first lesson, three rows say "not declared yet", which
widened the object column until "Lives in" wrapped to two lines (39 px
tall).

**Fixed.** Table 1's headings keep to one line (23 px); a table too
narrow for them scrolls, as its rows already do.
[`sheet-1440-light`](screenshots/fold/sheet-1440-light-before.png)
([after](screenshots/fold/sheet-1440-light-after.png)).

#### 6. The tape's clock was explained only by the panel's key

**Fixed.** Figure 1's caption now reads "Every object of the run, recorded
on tape; the clock along its top pulses once a step, taller at a sequence
point." The expression panel's key still says it too.

#### 7. The hanger missed the playhead on a compact deck

The vermilion hanger drops from the playhead through the cells. It was
placed past the left reel and the lane names, but a compact deck lays the
names on its lanes. At 1100x800 the hanger was at x = 1135, 152 px right of
the playhead at 983 and outside the 1100 px window.

**Fixed.** The hanger's offset is the reel's width plus the names' width,
none on a compact deck. At 1100x800 both are at 983.
[`hanger-1100-light`](screenshots/fold/hanger-1100-light-before.png)
([after](screenshots/fold/hanger-1100-light-after.png)).

#### Intended changes to the text

None in the lessons. Every title, lead and value flow of the 1,254 notes is
the same before and after in every configuration of the fold walk. In the
workbench the shadowing probe's notes 6 and 12 to 23 name the inner `x` as
`x #2`. Figure 1's caption is longer by one clause.

#### Checks

- **No sideways scroll** at any note of any lesson in any configuration,
  and no page logged an error.
- **Contrast:** nothing new is below AA. The lowest text in the note's
  column is 4.99:1, the note's number in the accent ink in the light theme
  (`#b33a0b` on `#ecebe6`, the colour Table 1 and Part 7 already use), then
  5.25:1 for the current step's number on the vermilion entry. Figure 1's
  caption, Table 1's headings and the `x #2` labels are above the lowest six
  in every configuration: at least 5.36:1 in the light theme and 5.47:1 in
  the dark one.
- **Reduced motion:** nothing new moves. The note's column and the steps'
  window jump to their place on each step, with or without motion.
- **Tests:** the site's tests cover the label a shadowing object takes in
  the title and that the outer object keeps its name, that a label replaces
  only a whole code span and keeps the object's colour, and that a long
  string literal keeps its spelling. The site's four clippy runs pass. The
  engine is unchanged.

### Left for later

- **What comes next** is still named only for a store, an update and a
  library call. Naming it for a read or an operator needs the engine to
  describe each of its thirty or so operations before it runs one, which is
  not small.
- **Below 1181 px** (a small laptop at 1024x768, a tablet held sideways)
  the note still sits under the cells, as before.
- **The stop's callout** comes after "Why it matters" in the note, so at a
  stop in the column layout it is often below the window's foot until the
  reader scrolls the note.
- **One printf** with five conversions, at its last step, still has its
  current step below the foot at each laptop size.
- **The cells under the head** still name a shadowing object `x`; the
  tape, Table 1 and the note call it `x #2`.

## Part 9: what comes next (2026-11)

Part 8 left four things for later. This part takes three of them: "what
comes next" was named only for a store, an update and a library call; at
an undefined-behavior stop the explanation of the stop came after "Why it
matters"; and below 1181 px the note still sat under the cells. The work is
on `agent/site-whats-next`.

### Method

The engine was changed first and tested on its own: every test that runs a
program to its end, and a new one over the 1,242 steps of the twelve
lessons, checks that each step's description of the next one names the
kind of event the next step records (or a stop there). The site was built
with `web/build.sh` as in Parts 7 and 8: Trunk without `--release`, the dev
profile at `opt-level=s`. The before build is Part 8's last, from
`97430e88c`, whose site and engine sources are those of `main` at
`ec445bf3b`. The after builds are this branch at `4eeac8c57` (the walks
at 1440 and 1366 px, and the phone's with reduced motion), at `a0d2e9c9f`,
which only adds the step counter's change of finding 4 (the walk at 1024
px, the phone's with motion and the Over probe), and at `1043da16e`, which
only recolours Over's tag (contrast and screen-reader text). Playwright
scripts drove Chromium:

- **the lesson walk:** Part 8's fold walk, extended. It steps through every
  note of the twelve lessons (1,254 notes) as a reader who brings the
  workbench to the top of the window once and then only presses Step. At
  each note it also records the note's event kind, the "Next" line (its
  words, its kind, its clause link and where it sits), the expression
  panel's "next" entry, Over's tag or closing entry and the Over key's
  tooltip, and where a stop's callout sits. A check then compares each
  note's "Next" with the note that follows. The walk ran on the after build
  at 1440x900, 1366x768 and 390x844 in both themes, with reduced motion
  and with motion allowed (waiting 650 ms for each step's motion to settle),
  and at 1024x768 light; and on the before build at 1440x900 light,
  1366x768 dark, 390x844 light and 1024x768 light, for the text comparison;
- **Over:** a probe pressed Over at every note of "Objects, loops and calls"
  and checked that it stopped at the step the tag marked;
- **contrast and screen-reader text:** the WCAG contrast of every text node
  in the note and the control strip against its composited background, and
  the accessible text of the "Next" line, the panel's entries and the Over
  key, at notes 2 and 45 of the first lesson, at note 44 after stepping
  back (where the tag marks a recorded step), and at the "Signed overflow"
  stop, at 1440x900, 1366x768, 1024x768 and 390x844 in both themes.

The screenshots are in [`screenshots/next/`](screenshots/next/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`.

### Findings

#### 1. Only three kinds of step said what came next

The engine knew what it would write next, so a note could say "next: store
into total", but nothing about a read, an operator, a conversion, a call or
a block. A newcomer pressing Step met each of these unannounced.

**Fixed.** The engine now describes each of its operations before running
it (`src/abstract_machine/narration/upcoming.rs`). The description holds
the kind of event the operation will record, what it does in plain words,
and the paragraph of C99 that defines it. It names only what the machine
knows beforehand: the source the operation evaluates, its operands'
spellings and the objects their addresses designate, never the value it
will produce. Before `main` it names the next directive or static object.
Across the lessons the descriptions take 32 kinds of step:

| Kind | Notes | Example |
| --- | ---: | --- |
| designate | 306 | find the object `total` (C99 §6.5.1p2) |
| lvalue conversion | 186 | read `i` (§6.3.2.1p2) |
| constant | 126 | evaluate the constant `0`; evaluate `LIMIT`, which stands for `4` |
| initialize | 64 | initialize `total` (§6.7.8p11) |
| block entry | 61 | begin the lifetime of `total`, `values` and `cursor` (§6.2.4p5) |
| declaration | 56 | reach the declaration of `total` (§6.2.1p7) |
| comparison | 45 | compare `i < 3` (§6.5.8p6) |
| branch | 43 | test the `for` condition `i < 3` (§6.8.5p4) |
| arithmetic | 41 | add `before` and `1`; shift the bits of `1u` left by `31` |
| block exit | 35 | end the lifetime of `next` |
| assignment | 33 | store into `values[0]` (in main) (§6.5.16.1p2) |
| jump | 31 | jump back to the loop condition |
| `++`/`--` | 26 | add 1 to `count` (§6.5.2.4p2) |
| array decay | 21 | turn the array `values` into a pointer to its first element |
| return | 20 | return from `bump`; return from `main`, which ends the program |
| compound assignment | 18 | add `step` to `total` (§6.5.16.2p3) |
| indirection | 16 | find the object `value` points to (§6.5.3.2p4) |
| static storage | 15 | create the string literal `"Hello, world!"`, before `main` runs |
| library call, discard | 12 each | call `puts`, which writes to standard output |
| call | 11 | call `bump` (§6.5.2.2p4) |
| conversion | 11 | convert `3` from `int` to `unsigned int` (§6.3.1.8p1) |
| member | 11 | select the member `x` of `p` |
| pointer arithmetic | 10 | move a pointer: `cursor + 1`; compute the address `&values[0]` |
| promotion | 10 | promote `text[count]` from `char` to `int` (§6.3.1.1p2) |
| subscript | 7 | select `text[count]`, checking it is inside its array (§6.5.2.1p2) |
| directive | 6 | include the header `<stdio.h>`; define the macro `LIMIT` |
| cast | 3 | cast `price` from `double` to `int` (§6.5.4p4) |
| static local, address, `sizeof`, unary | 2, 2, 1, 1 | negate `1` |

Each clause is one the engine already cites for that rule; the new ones'
association was checked against N1256 (6.5.1p2, 6.5.2.2p4, 6.7.8p13,
6.7.8p17, 6.8.5p4, 6.2.1p7). The JSON trace carries the description as
schema version 9.

The note shows it as a dimmed, dashed "Next:" line after the lead and the
value flow, in the colour of the kind of step it will be, with the
paragraph as a link to the standard. Where the note's expression panel
already lists the next step (#411's "next: store into total", which now
takes the engine's words), the line is left to the panel, so each note
names the next step once.
[`next-1440-light`](screenshots/next/next-1440-light-before.png)
([after](screenshots/next/next-1440-light-after.png)).

#### 2. Over's target was not shown

Over runs to the last step of the expression the current step belongs to,
or from that last step to the end of the next expression. The panel's key
said so in words, but nothing showed which step that was.

**Fixed.** A tag with the Over key's arrow marks the step Over stops at,
when the run has recorded it (after stepping back). While it has not, a
closing entry says "Over runs to its last step", or, at the expression's
last step, "runs on to the next expression". At a run's last step, where
Over cannot run, there is no mark. The Over key's tooltip names the
expression it finishes.
[`panel-1366-dark`](screenshots/next/panel-1366-dark-before.png)
([after](screenshots/next/panel-1366-dark-after.png)),
[`over-tag-1440-light`](screenshots/next/over-tag-1440-light-after.png).

#### 3. A stop's explanation came last

At an undefined-behavior stop the callout that says what went wrong came
after the lead, the expression panel and "Why it matters". It was below
the window's foot at all three stops in every configuration the walk
measured on the before build, the phone's included.

**Fixed.** The callout now comes first, under the note's title. It is in
the window at all three stops in every configuration of the walk.
[`stop-1440-light`](screenshots/next/stop-1440-light-before.png)
([after](screenshots/next/stop-1440-light-after.png)).

#### 4. Below 1181 px the control strip took two rows

Between 821 and 1180 px the note sits under the tape and the cells, as Part
8 left it. At 1024x768 the control strip also wrapped to two rows, 135 px
tall, because the mode keys, the encoders, the transport keys and the
status need about 1,120 px in one row.

**Fixed, in part.** Between 821 and 1180 px Reset, Back and Run show their
icons alone, as on a phone, and the step counter drops its word "Step";
both stay for screen readers, and the keys' names in their tooltips. Step
and Over keep their words. At 1024x768 the strip then keeps to one row at
1,246 of the 1,254 notes, three-digit steps included. It still wraps at
8 of the 12 last notes, where the status is longer: "Finished" at a
three-digit step, and the three stops. The note sits 42 px higher. (A first version hid only the keys'
names; the walk showed the counter then took the status to a second row
from step 10, shifting the page under the strip.) At 1024x768 the notes
whose title or lead ran past the window's foot fall from 546 to 81, but
the "Next" line is still below the foot at 316 of its 410 notes and the
expression panel's current step at every note. Below 1024 px the strip
still takes two rows. Moving the note beside the tape at this width, as
from 1181 px, would leave the tape too narrow to read; that is still left
for later.
[`strip-1024-light`](screenshots/next/strip-1024-light-before.png)
([after](screenshots/next/strip-1024-light-after.png)).

#### What the walk measured

| Window, before → after | Notes | Title or lead out of view | "Next" shown, out of view | Panel's current step out of view | Stops with the callout in view |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1440x900 light | 1254 | 0 → 0 | 410, 0 | 1 → 1 of 1027 | 0 → 3 of 3 |
| 1440x900 dark | 1254 | 0 | 410, 0 | 1 of 1027 | 3 of 3 |
| 1366x768 light | 1254 | 0 | 410, 0 | 2 of 1027 | 3 of 3 |
| 1366x768 dark | 1254 | 0 → 0 | 410, 0 | 1 → 2 of 1027 | 0 → 3 of 3 |
| 1024x768 light | 1254 | 546 → 81 | 410, 316 | 1027 → 1027 of 1027 | 0 → 3 of 3 |
| 390x844 light | 1254 | 0 → 0 | 410, 2 | 315 → 352 of 1027 | 0 → 3 of 3 |
| 390x844 dark | 1254 | 0 | 410, 2 | 352 of 1027 | 3 of 3 |
| 1440x900 light, motion | 1254 | 0 | 410, 0 | 1 of 1027 | 3 of 3 |
| 1440x900 dark, motion | 1254 | 0 | 410, 0 | 1 of 1027 | 3 of 3 |
| 1366x768 light, motion | 1254 | 0 | 410, 0 | 2 of 1027 | 3 of 3 |
| 1366x768 dark, motion | 1254 | 0 | 410, 0 | 2 of 1027 | 3 of 3 |
| 390x844 light, motion | 1254 | 0 | 410, 2 | 352 of 1027 | 3 of 3 |
| 390x844 dark, motion | 1254 | 0 | 410, 2 | 352 of 1027 | 3 of 3 |

A single number is the after build alone; the before build was walked at
four of the configurations. At the other 832 notes that can step, the
expression panel's own "next" entry names the next step, so the "Next"
line is shown at 410. The one more note at 1366x768 whose panel's current
step is below the foot is the "Signed overflow" stop: its callout now leads
the note and pushes the panel down, as finding 3 intends. The other is
Part 8's printf.

On the phone the "Next" line sits between the lead and the panel, so the
panel's current step is below the window's foot at 37 more notes. A
variant that put the line after the panel (tried by injecting its CSS
into the after build) brought that back to 317, but left the line itself
below the foot at 194 of its 410 notes instead of 2. The line stays before
the panel: it is the newer and shorter thing to read, and a phone reader
scrolls to the panel anyway.
[`phone-390-light`](screenshots/next/phone-390-light-before.png)
([after](screenshots/next/phone-390-light-after.png)).

#### Every "next" names the step that follows

At every note of every configuration of the walk, each of the 1,242 notes
that can step names the next step exactly once (the "Next" line or the
panel's entry, whose words are the same), and the twelve last notes name
none. Each names the kind of event the following note records: 1,242 of
1,242 in every configuration, the three stops included, where the
operation named is the one that stopped. The engine's own test checks the
same over the lessons and every other program its tests run to the end;
it caught one wrong kind while the change was written (a `float` passed
through `...` becomes a `double` by a conversion between floating types,
not an integer promotion).

The description quotes the code it acts on, and where the next note's
title names the same thing it agrees: "read `total`" before "Read `total` →
0", "call `bump`" before "Call bump (frame #2)". Where the title gives
values instead ("compare `i < 3`" before "Compare 0 < 3 → 1"), the code
quoted is the source range the next step highlights.

The Over probe pressed Over at every note of "Objects, loops and calls"
and "Structs and static storage" that can step, 331 presses, stepping back
after each one. At the 240 where the tag marked a recorded step, Over
stopped at that step every time; at the other 91 it ran on past the
current note, as the closing entry said. The site's tests check the exact
stop at every note of every lesson that ends without a stop.

#### Intended changes to the text

None in the lessons' notes: every title, lead and value flow of the 1,254
notes is the same before and after at 1440x900 light, 1366x768 dark,
390x844 light and 1024x768 light. Each note gains the "Next" line or the
panel's entry, the panel gains Over's tag or closing entry and a new key
("Over (O) stops where its tag is"), and a stop's callout moves to the top
of its note.

#### Checks

- **No sideways scroll** at any note of any lesson in any configuration,
  and no page logged an error.
- **Contrast:** nothing new is below AA. The lowest text in the note and
  the control strip is still the note's number in the accent ink in the
  light theme, 4.99:1, and the current step's number on vermilion, 5.25:1.
  The "Next" line's words are 10.59:1 and its label and clause 5.89:1 in
  the light theme, and above the lowest six (5.47:1) in the dark one.
  Over's tag first measured 4.54:1 in the light theme, on the panel's
  grey; on the cell colour it is 5.45:1, and 6.04:1 in the dark theme.
- **Screen readers:** the "Next" line reads "Next step: find the object
  total C99 §6.5.1p2 (opens the standard in a new tab)". A step Over
  stops at reads "Step 45: Initialize before = 2, then a sequence point
  (Over stops here)", and a closing entry "Over runs on to the next
  expression" (a first version ran the tag into its words, "Overruns").
  The Over key's tooltip reads "Step over: run to the last step of before
  = *value, through any call it makes (O)". Reset, Back and Run keep their
  names on small laptops, and the step counter its word.
- **Reduced motion:** nothing new moves. Every count of the walk is the
  same with motion allowed as with reduced motion.
- **Tests:** the engine's tests (114, four of them new) check every
  description against the step that follows in every program they run to
  the end, the lessons' 1,242 steps among them, and the words of the
  common kinds; the JSON test checks schema version 9 and the new field.
  The site's tests (163) check the panel's "next" entry, that Over's mark
  is where Over stops at every note of every lesson that ends without a
  stop, and that a run's last step has no mark. The engine's clippy run,
  the site's four clippy runs and the WebAssembly stepper's tests pass.

### Left for later

- **Below 1181 px** the note still sits under the tape and the cells;
  below 1024 px the control strip still takes two rows.
- **On the phone** the "Next" line pushes the expression panel's current
  step below the window's foot at 37 more notes (above).
- **One printf** with five conversions, at its last step, still has its
  current step below the foot at each laptop size, and at the "Signed
  overflow" stop the leading callout now does the same at 1366x768.
- **The description** quotes source, not values: "compare `i < 3`" says
  what will be compared, not that it is 0 < 3. Saying the values would
  repeat the step it announces.
- **The cells under the head** still name a shadowing object `x`; the
  tape, Table 1 and the note call it `x #2`.

## Part 10: a newcomer's whole-site pass (2026-11)

Parts 4 to 9 each looked at one view. This part looks at the whole site
once more, as a programmer who has never studied compilers would on a
first visit: the landing, Table 1, the abstract machine, the compiler
stages and the WebAssembly view, in that order. It then fixes the four
findings that most hurt a first visit and lists the rest. The work is on
`agent/site-newcomer-pass`.

### Method

The before build is `main` at `9e93d97a5`; the after builds are this
branch. Both were built with `web/build.sh` as in Parts 7 to 9: Trunk
without `--release`, the dev profile at `opt-level=s`. Playwright scripts
drove Chromium with reduced motion unless noted:

- **the review:** a first visit at 1440x900, 1366x768, 1024x768 and
  390x844 in both themes. It opens the page (at first paint and once the
  engine has loaded, and the whole page at 1440 and 390 px), Table 1, the
  hero's primary key "Step through it", the machine after 6 and 36 steps,
  the compiler pipeline (six stages, and all eleven at 1440x900 dark), the
  WebAssembly view, "How it works" and the glossary, recording sideways
  scroll and console errors. Steps are pressed with a pointer at the Step
  key's place: a Playwright locator click scrolls the sticky key into view
  first, which moved the page up 424 px and looked like a bug the site does
  not have;
- **the probe:** the four changes below at the same eight configurations,
  and with motion allowed at 1440x900 light, 1366x768 dark and 390x844
  dark: where the workbench lands after a lesson is chosen with the
  pointer and with the keyboard (sampled from 50 ms to 2 s), where the
  focus goes and where Tab and `→` go next; the blank tape's card, its fit
  and when it goes; the rows of the WebAssembly strip at instruction 125
  and at the end of the printf lesson (6,177); and the WCAG contrast of
  each new text node against its composited background;
- **the subset:** 28 small programs opened into the machine with **Open
  .c**, one per claim of "Limits of this model" (§3 of "How it works");
- **the lesson walk:** Part 9's walk, every note of the twelve machine
  lessons, before and after at 1440x900 light and after at 390x844 dark,
  comparing each note's title, lead and value flow.

The screenshots are in [`screenshots/newcomer/`](screenshots/newcomer/),
named `<item>-<width>-<theme>-before.png` and `…-after.png`, or without
the suffix where the view did not change.

### What a newcomer sees

**First impression.** The landing says what the site is and what to do
first. The eyebrow names it ("an interactive introduction to compilers
and the C abstract machine"), the headline asks the question the site
answers, and three numbered keys give an order: pick a lesson, step
through it, follow a program through the compiler. Figure 0 draws the
whole path at a glance: one statement, run by the abstract machine along
the top and carried down by the compiler through tokens, syntax tree,
types, IR and assembly to machine code, each station a link to its view.
The lede is dense (six lines of monospace that name the abstract machine,
a tape, IR, assembly and machine code at once), and Figure 0's stations
are words a newcomer may not know; both are left for later.
[`landing-1440-dark`](screenshots/newcomer/landing-1440-dark.png),
[`landing-390-light`](screenshots/newcomer/landing-390-light.png).

**Visual quality.** The machine and the compiler stages read as one
instrument: the same black screens in both themes, numbered keys, figure
and table captions, vermilion for "now", and syntax colours that agree
between the C listing and the outputs. Spacing and type are consistent
from view to view. Three places looked unfinished: the tape before the
first step (finding 2), the WebAssembly strip, whose status took a row of
its own on every laptop (finding 3), and, below 1181 px, the same strip
on two rows, as Part 9 left the machine's below 1024 px.
[`machine-1440-dark`](screenshots/newcomer/machine-1440-dark.png),
[`machine-code-1440-dark`](screenshots/newcomer/machine-code-1440-dark.png).

**Teaching path.** The order is clear inside each view. The compiler's
eleven stages are numbered keys under "Front end", "Middle end" and "Back
end"; each stage says what it takes in, what it gives out and why a
compiler has it, gives one example from the program ("In this program"),
and ends with a key to the next stage, the last one back to the first.
The machine's notes say what each step did, which rule of C it followed,
why it matters and what comes next. Between the views the order is
looser: the hero's third key and lesson 13 open the pipeline at its first
stage, but the "Compiler pipeline" key opens it at stage 7, bcc IR (left
for later).

**Friction.** Choosing a lesson in Table 1 did nothing a reader could see
(finding 1), and "Limits of this model" said that much of what the
lessons show was outside the machine (finding 4). Smaller ones are left
for later below.

### Findings

#### 1. Choosing a lesson did nothing in sight

Table 1 sits between the landing and the workbench, so a lesson chosen
there loaded into a workbench out of sight: at 1440x900, choosing lesson 5
left the page where it was, with the workbench's top 1,362 px below the
window's top. Only the row's colour changed. On a phone the workbench was
further still.

**Fixed.** Choosing a lesson brings the workbench to the top of the
window, at the place the hero's keys and the skip link bring it to (52 px
from the top on a laptop, under the header; 12 px on a phone, whose header
scrolls away). It glides, as the page's links do, and jumps with reduced
motion. The lesson loads after that first scroll, and its listing and tape
then take their own heights; the browser's scroll anchoring moved the page
by 43 px to keep what it had anchored on in place, so the page scrolls
there once more when the lesson has loaded. The keyboard focus goes to the
key of the view the lesson opened in, at the head of the control strip:
`→` steps at once from there, and Tab goes on to the strip's controls. The
focus ring shows when the lesson was chosen with the keyboard and not with
the pointer. The probe found the workbench at 52 px (12 px on the phone)
for a machine lesson chosen with the pointer, one chosen with the
keyboard and lesson 13, which opens the pipeline, at every configuration;
with motion allowed it is there by 1.2 s.
[`lesson-1440-light`](screenshots/newcomer/lesson-1440-light-before.png)
([after](screenshots/newcomer/lesson-1440-light-after.png)).

#### 2. The tape before the first step looked broken

Before the first step Figure 1 was a black field with "NOT YET RUN" in
small grey capitals, and the playhead and its flag drawn across the first
call's name, which read as a drawing that had failed to load. Nothing on
the tape said what it would hold or how to start; the note beside it does,
but the tape is the largest thing on the screen.

**Fixed.** A card over the blank tape says "Nothing recorded yet", then
"Press Step or → to run the first step", the Step key drawn as it is in
the strip, and what will fill the tape: a column a step, the program's
objects as lanes from the step their lifetime begins with their values
along them, and a note for each step saying what it did and which rule of
C it followed. It goes at the first recorded step and stays gone when
Back returns to step 0, since the tape is no longer blank; it comes back
for each program loaded. The tape under it still takes the pointer. On a
phone the tape before the first step is 40 to 80 px tall, too short for
the card, which ran over the cells and Figure 2 in a first version; up to
820 px the card follows the blank tape in the deck instead.
[`blank-tape-1366-light`](screenshots/newcomer/blank-tape-1366-light-before.png)
([after](screenshots/newcomer/blank-tape-1366-light-after.png)),
[`blank-tape-1024-dark`](screenshots/newcomer/blank-tape-1024-dark-before.png)
([after](screenshots/newcomer/blank-tape-1024-dark-after.png)),
[`blank-tape-390-light`](screenshots/newcomer/blank-tape-390-light-before.png)
([after](screenshots/newcomer/blank-tape-390-light-after.png)).

#### 3. The WebAssembly strip took two rows on every laptop

The control strip is at most 1,402 px wide. The WebAssembly view puts two
more keys in it than the machine (Out and Next line), the "Start at main"
switch and a longer status; with the mode keys' full names, from 1440 px
up, that came to 1,519 px, so the status ("Ready Instruction 125") took a
second row of its own and the strip was 103 px tall instead of 64.

**Fixed.** In the WebAssembly view Reset, Back and Run show their icons
alone at every width, as the machine's do below 1181 px, and the counter
drops its word "Instruction", as the machine's drops "Step" there; the
note under the strip heads with the same number ("Instruction 125"). The
keys' names and the word stay for screen readers, and the keys' names in
their tooltips. The mode keys keep their full names, so they do not
change under the pointer when the view changes. At 1440x900 and 1366x768
the strip keeps to one row, at instruction 125 and at the end of the
printf lesson, 6,177 (a first version, with only the icons, took a second
row again from four digits).
[`wasm-strip-1440-dark`](screenshots/newcomer/wasm-strip-1440-dark-before.png)
([after](screenshots/newcomer/wasm-strip-1440-dark-after.png)).

#### 4. "Limits of this model" said the opposite of the lessons

§3 of "How it works" still described the machine before the lessons on
strings, printf, `#define`, the integer types and floating point. It said
that the preprocessor, other integer types, char pointers and strings,
library calls and floating point were outside the subset, and that its
lesson opened in the compiler pipeline instead; lessons 2, 3, 5, 6 and 8
show each of these running in the machine.

**Fixed.** The section now lists what the machine runs (the integer types
from char to unsigned long long and the three floating types at their
x86-64 sizes; pointers to numbers, chars and structures; arrays of
numbers or chars, and string literals;
structures, nested or through pointers; const, casts and sizeof;
file-scope and static objects; functions that return a number or a
structure; if and else, while and for; printf, puts and putchar after
`#include <stdio.h>`; `#define` of a name as one number, character or
string) and, among what it does not, the constructs a reader is likely to
write: switch, do, break, continue and goto; `?:` and the comma operator;
typedef, enum, union and `_Bool`; void and static functions; pointers to
pointers or to functions; arrays of pointers, of structures or of arrays;
hexadecimal and octal constants; a call inside a larger expression; nested side
effects such as `a = b++`; other library functions; and more of the
preprocessor. It says what happens to such a program: it stops before its
first step, and its note says why, or, for the preprocessor, sends the
reader to the Preprocessor stage. A second paragraph says that the
machine's left-to-right operand order is one C allows, citing C99 §6.5p3
(checked against N1256, PDF p. 79).

Each claim was checked by opening a program into the machine. Seven
ran to their end: `char *` into a string, arrays of `double`, `float` and
`long` in one program, `long double`, `if`/`else`, `unsigned long long` with `signed char`
and `unsigned short`, `->` on a pointer to a structure, and a function
returning a structure. Each of the others stopped at Note 0, "This program
is outside the supported subset": `switch`, `do`, `break`, `continue`,
`goto`, `?:`, the comma operator, `typedef`, `enum`, `union`, `_Bool`, a
void function, a static function, an array of pointers, an array of
structures, `int x = f(1) + 1`, `return n * f(n - 1)`, `a = b++` and
`getchar`; a function-like macro and `#include <stdlib.h>` stopped at "The
machine starts after the preprocessor".
[`limits-1440-dark`](screenshots/newcomer/limits-1440-dark-before.png)
([after](screenshots/newcomer/limits-1440-dark-after.png); the after
shot predates a last change of its fourth sentence from the end, from
"with a button to the Preprocessor stage" to "its note sends the reader to
the compiler's Preprocessor stage", since the stop's note names the stage
in its explanation but has no button).

#### Intended changes to the text

- §3 of "How it works", "Limits of this model", is rewritten (finding 4)
  and gains the paragraph on operand order.
- The blank tape's card is new (finding 2).
- The WebAssembly strip's counter no longer shows its word "Instruction",
  which screen readers still read (finding 3).

Nothing else: every lesson's title in Table 1 is the same, and every
title, lead and value flow of the 1,254 notes of the twelve machine
lessons is the same before and after at 1440x900 light.

#### Checks

- **No sideways scroll** at any step of the review or the probe in any
  configuration, nor at any note of the lesson walk, and no page logged an
  error.
- **Contrast:** the card's text is on the tape's black screen in both
  themes: its label 6.31:1, its lead's Step key (the strip's key colours)
  6.06:1, its body 6.88:1. The rewritten §3 is 10.59:1 in the light theme
  and 9.40:1 in the dark. The lowest text the probe measured is still §3's
  number in the accent ink in the light theme, 4.99:1, as in Part 9.
- **Keyboard and screen readers:** choosing a lesson with Enter moves the
  focus to the view's key with its focus ring; `→` from there steps the
  machine. The card is text in Figure 1, after the tape. The
  WebAssembly keys keep their names, and the counter its word, for screen
  readers.
- **Reduced motion:** the page jumps to the workbench instead of gliding;
  nothing else new moves.
- **Tests:** the site's tests (163 and 2) and its four clippy runs
  (native, and wasm32 for hydrate, worker and compiler) pass.

### Left for later

- **The "Compiler pipeline" key opens at stage 7, bcc IR**, not at the
  first stage, unless the pipeline is reached through the hero's third key
  or lesson 13. A newcomer who tries the views in order meets the IR before
  tokens and the syntax tree. Opening at stage 1 would follow the path, but
  for most lessons stage 1's output is the source itself, and the IR is
  what readers who already know the front end come for.
  [`pipeline-opens-1440-light`](screenshots/newcomer/pipeline-opens-1440-light.png).
- **A stage's output starts below the fold** at 1440x900 (about 700 px
  down): the heading, the hand-off sentence, "In this program", the in,
  out and why lines and the reading line come first. A phone already shows
  the output right under the stage keys; a laptop could do the same, or
  fold the explanation.
  Same screenshot.
- **The reading line appears twice** in the pipeline on a laptop: "Point at
  a line to read what it links to." under the source and above the output.
- **Below 1181 px the WebAssembly strip still takes two rows**, as the
  machine's does below 1024 px.
  [`wasm-strip-1024-light`](screenshots/newcomer/wasm-strip-1024-light.png).
- **A preprocessor stop has no button** to the Preprocessor stage: its
  note names the stage in "How C defines this step", but the button the
  steps of `#include <stdio.h>` and `#define` carry is not shown at the
  stop, though the README says the stop has one.
- **The landing's words for the compiler's forms** (tokens, syntax tree,
  IR) are links to the views in Figure 0 but not to their glossary entries,
  which exist; and the lede is six dense lines. On a phone Figure 0
  scrolls sideways (it says so).
- **Figure 2 and Table 1 before the first step** are a sentence and an
  empty table head; the blank tape's card now explains the screen, but the
  table could say that it fills as objects begin.
- **The view keys are a tab list without arrow-key movement**: the arrow
  keys step the machine everywhere on the page, so a screen-reader user
  who expects the tab pattern's arrows moves the machine instead.

## Part 11: the newcomer pass's "left for later" (2026-11)

Part 10 ended with eight things left for later. This part takes all eight,
in the order of what they cost a first visit: the pipeline's opening stage,
its output below the fold, the doubled reading line, the landing's jargon,
the preprocessor stop's missing button, the empty Table 1 and Figure 2, the
WebAssembly strip's second row below 1181 px, and the view keys' tab
pattern. The work is on `agent/site-part11`.

### Method

The before build is `main` at `23541a8e8` (Part 10 merged). The after
builds are this branch at `0a6290ac7` (the probe at every configuration)
and at `fcf37926e`, which adds the empty figures' lines for a stopped run
and the new test (the strip, contrast and the lesson walk, and the probe
again at 1440x900 light and 1366x768 dark); the branch's last commit only
moves Table 1's empty state into a component of its own. Both were built with `web/build.sh` as in Parts 7
to 10: Trunk without `--release`, the dev profile at `opt-level=s`.
Playwright scripts drove Chromium with reduced motion:

- **the probe:** one script for the eight items, at 1440x900, 1366x768,
  1024x768 and 390x844 in both themes, on both builds. It reads the
  landing's lede (its lines and links) and focuses its first term, records
  Table 1 and Figure 2 before the first step, the view keys' roles and what
  `→` does on one, opens the pipeline with its view key and records the
  stage it opens at, presses `→` on the selected stage key, then shows all
  eleven stages and records where each one's output starts, its width and
  every reading line on screen, with `-O0` and `-O2` side by side and with
  the pointer on a source line; it loads a program with a function-like
  macro into the machine and presses the stop's button if there is one; and
  it measures the WebAssembly strip. It records sideways scroll and console
  errors throughout;
- **the strip:** the control strip of the machine and of the WebAssembly
  view at 821, 900, 1024, 1100, 1180 and 1366 px, with the widest status a
  lesson reaches put in its place (a four-digit instruction count, a
  stop's status); the change was first tried by injecting its CSS into the
  before build;
- **contrast:** the WCAG contrast of each new or moved text against its
  composited background, at 1440x900 in both themes, 1366x768 dark,
  1024x768 light and 390x844 in both themes;
- **the lesson walk:** Part 9's walk, every note of the twelve machine
  lessons, before and after at 1440x900 light and after at 390x844 dark,
  comparing each note's title, lead and value flow.

The screenshots are in [`screenshots/part11/`](screenshots/part11/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours to keep them small.

### Findings

#### 1. The pipeline opened at stage 7

The view key "Compiler pipeline" opened at stage 7, bcc IR, while the
hero's third key and lesson 13 open at stage 1, so a newcomer who tried the
views in order met the IR before tokens and the syntax tree.

**Fixed.** Every way in now opens at stage 1, Logical lines, until the
reader picks another stage, which the pipeline then keeps as before. The
brief allowed the first stage whose output differs from the source
instead, which is stage 2 for any program without a line splice, but
skipping stage 1 would make the opening stage depend on the program and
leave the stage keys' numbering to explain itself. Stage 1 says what
translation phases 1 and 2 do, and its "In this program" line says that
no line ended in a backslash, so the output is the source again; that is
the point a newcomer should take from it. For readers who came for the IR,
each front-end stage before the last (whose Next is already bcc IR) has a
second key beside Next, "Skip to bcc IR", unlit so Next stays the one to
press; it moves the focus to the IR's stage key. Figure 0's IR station
still opens the IR directly. The probe found stage 1 open at every
configuration, where the before build opened stage 7.
[`pipeline-opens-1440-light`](screenshots/part11/pipeline-opens-1440-light-before.png)
([after](screenshots/part11/pipeline-opens-1440-light-after.png)).

#### 2. The output started below the fold

At 1440x900 a stage's output started 659 to 732 px down, under the
heading, the command, the hand-off, Next, "In this program", the in, out
and why lines, the hover note and the reading line; at 1366x768 the same
659 to 732 px left at most 109 px of output in the window, and at 1024x768
it started 968 to 1,042 px down, below the window's foot at every stage.

**Fixed.** Where the pipeline's column is at least 840 px wide (in windows
from 1181 px, where the listing narrows to its laptop width) the card splits
under the stage's name: the output on the left, beside the source it links
to, and the notes on the right in a column of 20rem: the hand-off with
Next (and Skip), "In this program", the in, out and why lines, the command
that prints the stage, and the hover note with the output's toggles. Both
stay readable: the notes keep their type sizes (the hand-off is 20 px
instead of 22), and the output's editor keeps its height. The output now
starts 403 px down at every stage at 1440x900 and 1366x768, so 365 px of
it shows at 1366x768. It is narrower: 646 px wide at 1440 (990 before) and
593 at 1366 (937): a line longer than about 70 characters, such as the
IRs' folded target line, scrolls sideways in the editor. `-O0` and `-O2` side by side need the width, so then the
card keeps the stacked layout. A narrower card (the 1024x768 laptop's,
whose output is 556 px wide) stacks the stage as a phone does, the output first, but under the
stage's name, with the reading line above it: the output starts 507 to 551
px down at 1024x768. The phone is unchanged (498 px).
[`stage-bcc-ir-1440-dark`](screenshots/part11/stage-bcc-ir-1440-dark-before.png)
([after](screenshots/part11/stage-bcc-ir-1440-dark-after.png)),
[`pipeline-opens-1024-light`](screenshots/part11/pipeline-opens-1024-light-before.png)
([after](screenshots/part11/pipeline-opens-1024-light-after.png)).

The brief also suggested folding the explanation behind a disclosure.
That would bring the output up only after a first visit, and a newcomer is
the reader who needs the explanation beside the output, so the two columns
were chosen.

#### 3. The reading line showed twice

"Point at a line to read what it links to." showed under the listing and
over the output on every laptop, and both changed together as the pointer
moved.

**Fixed.** Where the listing stands beside the pipeline (from 821 px) only
the line over the output shows; it sits right above the output, where a
reader pointing at a source line looks for what it became. A phone keeps
both, since its listing and its output are a screen or more apart. The
probe found one reading line on screen at every stage of every laptop
configuration (two before) and two on the phone, as before.
[`stage-pointing-1366-light`](screenshots/part11/stage-pointing-1366-light-before.png)
([after](screenshots/part11/stage-pointing-1366-light-after.png)).

#### 4. The landing's words for the compiler's forms

The lede named IR, assembly and machine code with nothing to say what they
are, and took six dense lines on every laptop. Figure 0's station names
link to the views, not to their definitions.

**Fixed.** The lede now names every form Figure 0 draws, tokens, a syntax
tree, IR, assembly and machine code, each a link to its entry under
Compiler terms in the glossary (Table 2). While a mouse is over one, or the
keyboard focus is on it, its definition shows under the word in the
glossary's own words, the way the narration's terms show theirs; Escape
hides it, and the pointer can move onto it. A touch screen has no hover, so
a tap follows the link to the entry. A screen reader reads the definition
as the link's description. Below 821 px the definition shows under the
lede, full width, so it never runs off a phone. The lede is shorter and its
measure wider (70ch instead of 62ch): five lines at 1440, 1366 and 1024 px
instead of six, nine on a phone instead of ten. It keeps what it said, but
for three words: programs are no longer "real", bcc no longer "written" in
Rust, and the rules no longer say "exactly" when each effect happens (the
lede still says they are rules for when it happens). Figure 0's station
names still open their views.
[`landing-1440-light`](screenshots/part11/landing-1440-light-before.png)
([after](screenshots/part11/landing-1440-light-after.png)),
[`landing-term-1440-dark`](screenshots/part11/landing-term-1440-dark.png),
[`landing-390-light`](screenshots/part11/landing-390-light-before.png)
([after](screenshots/part11/landing-390-light-after.png)).

#### 5. A preprocessor stop had no button

A program with a directive the machine does not run (a function-like
macro, `#if`, another header) stops at Note 0, "The machine starts after
the preprocessor". Its explanation, the README and "Limits of this model"
all sent the reader to the Preprocessor stage, but the button that the
steps of `#include <stdio.h>` and `#define` carry was not shown: the page
shows it for a step of kind `directive` or one that cites a clause of
§6.10, and the stop was neither.

**Fixed.** The stop now cites C99 §6.10p2 beside §5.1.1.2p1: "A directive
is a line whose first token is #; it ends at the line's new-line", which is
what the paragraph says of a preprocessing directive (checked against
N1256, pp. 146-147, PDF pp. 158-159). That makes the stop a step of the
preprocessor's, so its note shows the button. The probe loaded `#define
TWICE(x) ((x) * 2)` at every configuration: the button showed, and pressing
it opened stage 3, Preprocessor. The README and §3 of "How it works" now
say the stop has a button; the site's test of the stop checks it.
[`preprocessor-stop-1440-light`](screenshots/part11/preprocessor-stop-1440-light-before.png)
([after](screenshots/part11/preprocessor-stop-1440-light-after.png)).

#### 6. Table 1 and Figure 2 before the first step

Before the first step Figure 2 said "This step is not part of an
expression", and Table 1 was a table head with no rows.

**Fixed.** Each now has a card in the blank tape's manner (its accent rule
on the left and its capitals), in the figures' own colours: "Nothing wired
yet: each full expression the machine evaluates is drawn here as a
circuit: its operands feed its operators, and the wires fill with values
step by step", and "No objects yet: each object gets a row here when its
lifetime begins: its type and size, where it lives, the step that created
it and its value at the playhead". Table 1's card shows whenever no object
exists yet. Where the run stopped before its first step, as at a
preprocessor stop, the cards would promise what will not come, so a plain
line says that nothing was evaluated and no object began its lifetime.
[`empty-figures-1366-dark`](screenshots/part11/empty-figures-1366-dark-before.png)
([after](screenshots/part11/empty-figures-1366-dark-after.png)).

#### 7. The WebAssembly strip below 1181 px

Between 821 and 1180 px the WebAssembly view's control strip needed about
1,105 px (the view keys 298, the transport 499, "Start at main" 142, the
status 124, and their gaps) and took two rows, 99 px tall, below about
1,140 px.

**Fixed, down to 1024 px.** Between 821 and 1180 px the view keys drop the
numbers in their squares, which only echo their order (a phone drops them
too), and the strip's gaps and the keys' padding tighten. The view keys
take 214 px and the transport 463, so the strip keeps to one row, 64 px
tall, at 1180, 1100 and 1024 px, a four-digit instruction count included.
The machine's strip gains the same room: at 900 px it keeps to one row
(two before), and from 1100 px a stop's status no longer takes a second
row. Below 1024 px the WebAssembly strip still takes two rows: it would
need its keys' words to go, and Over, Out and Next line keep theirs because
their icons alone do not say which is which.
[`wasm-strip-1024-light`](screenshots/part11/wasm-strip-1024-light-before.png)
([after](screenshots/part11/wasm-strip-1024-light-after.png)).

#### 8. The view keys' tab pattern

The view keys were a tab list (`role="tablist"`) whose tabs all took Tab
and none took the arrow keys, which step the machine wherever the focus
is. The pipeline's stage keys were a tab list in the same way.

**Fixed, in two ways.** The view keys switch the whole workbench, and the
focus is put on one on purpose: choosing a lesson focuses its view's key
so that `→` steps at once (Part 10, finding 1). Making them real tabs would
have taken `→` away from the machine there, and a screen reader would have
promised arrows that step the machine instead. So they are no longer a tab
list: they are a group of keys, "View", each pressed or not
(`aria-pressed`), as the lesson cards and the memory display's keys are,
and `→` on one still steps the machine. The WebAssembly view's memory
region keys were a tab list in the same way and change the same way. The
stage keys are another matter: the pipeline view steps nothing, so the
arrows are free there, and they are now a tab list with the pattern's
keys. Only the selected key takes Tab; `←` and `→` (and `↑` and `↓`)
select the stage before or after, around the ends, `Home` and `End` the
first and last, and the focus follows the selection. The stage card is the
tab panel, labelled by the selected key. The probe pressed `→` on stage
1's key at every configuration and found stage 2 selected and focused,
the other ten keys out of the tab order, and the panel labelled by stage
2's key; `→` on the machine's view key stepped to Note 1, as before.

#### Intended changes to the text

- The landing's lede is rewritten (finding 4) and its five words for the
  compiler's forms gain their definitions.
- Each front-end stage before the last gains the key "Skip to bcc IR"
  (finding 1).
- The preprocessor stop cites §6.10p2, "Preprocessing directives", and
  shows the button "See the real preprocessor: open the Preprocessor
  stage" (finding 5); §3 of "How it works" now says its note "has a button
  to" the stage rather than "sends the reader to" it.
- Figure 2 and Table 1 gain their cards, or the plain lines at a stop
  before the first step (finding 6).

None in the lessons' notes: every title, lead and value flow of the 1,254
notes of the twelve machine lessons is the same before and after, at
1440x900 light and, against the same before walk, at 390x844 dark.

#### Checks

- **No sideways scroll** at any step of the probe or the strip runs in
  any configuration, nor at any note of the lesson walk, and no page logged
  an error.
- **Contrast:** nothing new is below AA. The lowest new text is the empty
  figures' label in the accent ink on the device in the light theme,
  4.99:1, the same ink and ground as the section numbers Part 10 measured;
  in the dark theme it is 6.87:1. The figures' card text is 10.59:1 (light)
  and 9.40:1 (dark); the lede's terms 15.82:1 and 13.34:1, and their
  definitions 10.59:1 and 9.40:1; "Skip to bcc IR" 15.82:1 and 13.34:1;
  the notes column's command line 5.89:1 and 6.22:1, as before.
- **Keyboard and screen readers:** the view keys are a group named "View"
  of three keys, each read as pressed or not; `→` on one steps the machine.
  The stage keys are a tab list of which only the selected key takes Tab;
  the arrow keys, Home and End move the selection and the focus, and the
  card is the tab panel labelled by the selected key. "Skip to bcc IR"
  moves the focus to the IR's stage key, since the key itself goes as the
  stage changes. Each of the lede's terms is a link described by its
  definition (`aria-describedby`), which shows on focus and hides on
  Escape or blur. The reading lines stay hidden from screen readers, as
  before; the hover note says how to use them.
- **Reduced motion:** nothing new moves.
- **Tests:** the site's tests (164, one of them new: the skip key at
  exactly the front-end stages before the last, and each stage card's tab
  panel and the stage keys' one tab stop) and its four clippy runs (native,
  and wasm32 for hydrate, worker and compiler) pass. The test of the
  preprocessor stop checks that its note links to the Preprocessor stage.
  The engine is unchanged.

### Left for later

- **A narrow card** (the 1024x768 laptop's, and from 821 to 1180 px)
  stacks the stage with the output first, so its hand-off, Next and Skip
  come after the output, below the window's foot; the stage keys and the
  heading name the stage above it.
- **The output's column** is 646 px wide at 1440 and 593 px at 1366, so a
  line longer than about 70 characters, such as the IRs' folded target
  line, scrolls sideways in the editor. Giving the notes less than 20rem
  would wrap the hand-off into more lines than the output gains.
- **Below 1024 px** the WebAssembly strip still takes two rows (at 900 and
  821 px), as the machine's does at a stop's status.
- **Figure 0's stations** still open their views and carry no definitions;
  the lede carries them for the same five words.
- **The cells under the head** still say "No objects yet. Entering a
  block puts the objects it declares on the tape" at a stop before the
  first step, where Figure 2 and Table 1 now say that the run stopped.
- **On a phone** a definition shows under the lede, not under its word, and
  only for keyboard focus or a mouse; a tap follows the link to the
  glossary, which takes the reader away from the landing.
