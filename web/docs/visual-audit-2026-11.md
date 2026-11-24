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
593 at 1366 (937), so a line longer than about 70 characters, such as the
IRs' folded target line, ran past the column's edge; the review's round
wraps it (see [Review, round 1](#review-round-1)). `-O0` and `-O2` side by side need the width, so then the
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
as the link's description. Up to 1180 px, where the hero is one column,
the definition shows under the lede, full width; from 1181 px, where the
lede is the hero's left column, under the word. (This part first put it
under the word from 821 px, where a word at the end of a line ran it off
the window: see [Review, round 1](#review-round-1).) The lede is shorter and its
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
it opened stage 3, Preprocessor, with the focus on its key (since the review's
round, as for the button of a `#define` step). The README and §3 of "How it works" now
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
keys. Only the selected key takes Tab; `←` and `→` select the stage
before or after, around the ends, `Home` and `End` the first and last,
and the focus follows the selection; `↑` and `↓` scroll the page, as the
pattern has it for a horizontal tab list (this part first took them too). The stage card is the
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
  before the first step (finding 6); at such a stop the cells under the
  head now say "The machine stopped before its first step, so no object
  began its lifetime." instead of "No objects yet. Entering a block puts
  the objects it declares on the tape." (review, round 1).

None in the lessons' notes: every title, lead and value flow of the 1,254
notes of the twelve machine lessons is the same before and after, at
1440x900 light and, against the same before walk, at 390x844 dark.

#### Checks

- **No sideways scroll** at any step of the probe or the strip runs in
  any configuration, nor at any note of the lesson walk, and no page logged
  an error. The probe's widths (390, 1024, 1366, 1440) missed the lede's
  definitions running off the window from 821 to 965 px, which the review
  found; the sweep of [Review, round 1](#review-round-1) covers every width.
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
- **Below 1024 px** the WebAssembly strip still takes two rows (at 900 and
  821 px), as the machine's does at a stop's status.
- **Figure 0's stations** still open their views and carry no definitions;
  the lede carries them for the same five words.
- **Up to 1180 px** a definition shows under the lede, not under its
  word, and on a phone only for keyboard focus or a mouse; a tap follows the link to the
  glossary, which takes the reader away from the landing.

### Review, round 1

The review of the branch found one blocker, a trade-off worth fixing, and
three small things. The before build is the branch at `d14a7f89e`; the
after build is the round's fixes, built as above (Trunk without
`--release`, the dev profile at `opt-level=s`).

#### 1. The lede's definitions ran off the window from 821 to 965 px

From 821 px each definition was a 340 px box under its word, and nothing
kept it inside the window. Up to 1180 px the hero is one column and the
lede's third line ends at "a syntax tree, IR,", so at 834 and 900 px IR's
definition reached 966 px: the window cut it mid-word and the page scrolled
132 px sideways while it showed. The probe's widths (390, 1024, 1366 and
1440 px) all happened to fit.

**Fixed.** Up to 1180 px the definition spans the lede under it, as it
did below 821 px; from 1181 px, where the lede is the hero's left column
and narrower than half the window, it stays under its word, where a
340 px box always fits. A sweep focused each of the five terms at every
window width from 320 to 1440 px, in 1 px steps, in both themes (5,605
definitions a theme) and checked that the box lies inside the window, the
page has no sideways scroll, no ancestor's overflow clips the box, its
text fits in it, and nothing covers its corners or centre: no failures in
either theme, the closest edge 18 px from the window's (Token's at
320 px). The same sweep over 800 to 1000 px on the before build failed 173
times, from 821 to 965 px.
[`landing-term-834-light`](screenshots/part11/landing-term-834-light-before.png)
([after](screenshots/part11/landing-term-834-light-after.png)),
[`landing-term-834-dark`](screenshots/part11/landing-term-834-dark-after.png).

#### 2. Long lines of the output were cut

The output's column beside the notes is 646 px wide at 1440 and 593 px at
1366, about 65 characters, and the listings did not wrap, so their long
lines ran past its edge. On each lesson's first screen at 1366 px, 27 of
209 bcc IR lines and 34 of 221 LLVM IR lines were cut (none of the
assembly's), and in the lessons that call printf the cut took the
argument that mattered, as in Floating point's
`%v10 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, double noundef %v8)`.

**Fixed.** The read-only listings now wrap, as the source already did. A
wrapped line's later rows hang two tab stops in from the line's own
indent (Monaco's `deepIndent`), and only
its first row has a line number, so it reads as one line; the token
colours are those of the line. The line links are decorations of the whole
line, so pointing at any row of a wrapped line reads that line and marks
its source, and a source line's mark covers every row of the output lines
it links to. Where the page scrolls to show a linked or followed line, it
now measures the whole wrapped line rather than one row. (Corrected in
[Part 12](#part-12-the-output-listings-and-the-steps-explanations-2026-11):
Monaco guessed each listing's tab size from its text, so the hang was 8
columns in some listings, past where the next instruction starts, but 4
in others, where it starts; and the editor's own scroll to a linked line
still stopped at the last line's first row. Part 12 fixes both.) Narrowing the
notes was the other way: below 20rem they wrap the hand-off into more
lines than the output gains, and at 1024 px and on a phone the output is
narrower still, where the wrap helps as much. A wrapped line takes more
rows, so fewer lines fit on a first screen.

Counted again at 1366x768 light over the twelve machine lessons' bcc IR,
LLVM IR and x86-64 assembly, a line counting as cut when any of its rows
runs past the editor's visible width: on the first screens 0 of 159, 0 of
179 and 0 of 200 lines are cut (26, 21 and 0 of them wrap); scrolling on
through the listings with the wheel, 0 of 524, 285 and 451 lines (26, 36
and 5 wrap). Pointing at the second row of Floating point's wrapped
`printf` call (LLVM IR line 14) read "Line 14 of the LLVM IR came from
source line 8." and marked source line 8, as pointing at its first row
does, on this build and the before build.
[`stage-llvm-ir-1366-light`](screenshots/part11/stage-llvm-ir-1366-light-before.png)
([after](screenshots/part11/stage-llvm-ir-1366-light-after.png)),
[`stage-llvm-ir-1366-dark`](screenshots/part11/stage-llvm-ir-1366-dark-after.png),
[`stage-llvm-ir-390-light`](screenshots/part11/stage-llvm-ir-390-light-before.png)
([after](screenshots/part11/stage-llvm-ir-390-light-after.png)),
[`stage-wrapped-row-1366-light`](screenshots/part11/stage-wrapped-row-1366-light.png).

#### 3. Small things

- **`↑` and `↓` on the stage keys** selected the stage after or before.
  WAI-ARIA's pattern takes only `←` and `→` for a horizontal tab list, so
  `↑` and `↓` now scroll the page there; the probe pressed both on a stage
  key at every configuration and the selection stayed.
- **The focus after the machine's button to the Preprocessor stage** fell
  to the page's body, since the button goes with the machine's view. It now
  moves to the Preprocessor stage's key once the pipeline is drawn, as
  "Skip to bcc IR" does for its stage, both at a preprocessor stop and at
  a `#define` step; the probe found it there at every configuration.
- **"Press Step" at a stop.** The review read the blank tape's card, which
  says to press Step, at a stop before the first step, where Step is
  disabled. The card is hidden there (the review's probe read the hidden
  card's text); the probe found no visible card at every configuration.
  The line that did show the wrong promise was the cells', "No objects
  yet. Entering a block puts the objects it declares on the tape", which
  Part 11 had left for later: at such a stop it now says "The machine
  stopped before its first step, so no object began its lifetime."

#### Checks

- **The probe** above, extended with the vertical arrows, the focus after
  both buttons to the Preprocessor stage and the cells' line at a stop,
  ran at 1440x900, 1366x768, 1024x768, 900x800, 834x1112 and 390x844 in
  both themes: no sideways scroll at any step, no console error or warning,
  the WebAssembly strip in one row from 1024 px, and every result above.
- **The lesson walk** at 1440x900 light: every title, lead and value flow
  of the 1,254 notes of the twelve machine lessons is the same as in the
  walks of `main` and of the branch before the round, with no sideways
  scroll at any note and no page error.
- **Contrast** is unchanged: no new colours. The definition under the lede
  at 834 px is 10.59:1 (light) and 9.40:1 (dark), its word 15.82:1 and
  13.34:1; the cells' new line 5.89:1 and 6.22:1.
- **Tests:** the site's tests (164, and the prerender's 2) and its four
  clippy runs (native, and wasm32 for hydrate, worker and compiler) pass.
  The worker run prints the engine's existing unfulfilled lint expectation
  in `src/abstract_machine/parser.rs`, as before; the engine is unchanged.

## Part 12: the output listings and the steps' explanations (2026-11)

Part 11's review left three things about the wrapped output listings, and
Part 11 itself left four for later. This part fixes the listings, takes
three of the four, and then walks three lessons once more as a programmer
who has never studied compilers or the C standard, to improve the steps
whose notes said least about what happened to memory and values. The work
is on `agent/site-part12`.

### Method

The before build is the review's build of #427 at `e5fffb38f`, whose
`web/` and engine are those of `main` at `b41e293e0` (Part 11 merged; the
merge's other PR, #428, changed only the compiler). The after builds are
this branch: `a1` at `300ff8c5d` (the first version of every change) for
the first measurements and the first lesson walk, and the final build at
`8007aaa10` for everything below unless noted (the branch's later commits
only format code and write this part). Both were built with `web/build.sh`
as in Parts 7 to 11: Trunk without `--release`, the dev profile at
`opt-level=s`. Playwright scripts drove
Chromium with reduced motion, one browser at a time:

- **the listings:** Floating point's LLVM IR, pointing at source lines 8,
  10 and 13 in turn (the review's reproduction) at 1366x768, 1024x768 and
  900x800, checking that every row of every marked line lies inside the
  editor; and the hang and the breaks of every wrapped line of Floating
  point's bcc IR and LLVM IR and Structs' bcc IR, alone and with `-O0` and
  `-O2` side by side, at 1366x768 (two columns) and 834x1112 (stacked);
- **the probe:** at 1440x900, 1366x768, 1024x768, 900x800, 834x1112 and
  390x844 in both themes (both builds at 1024x768 light, 900x800 dark,
  834x1112 light and 390x844 light): the lede's word IR with the mouse, the
  keyboard (focus, Tab, Escape) and, on the phone, taps (on the word, again,
  elsewhere, and on the definition's link); Figure 0's stations; the stage
  card at Logical lines and at bcc IR, where its Next and Skip keys and its
  output sit and in what order its parts come; the WebAssembly strip; and
  sideways scroll and console errors throughout;
- **the definitions:** Part 11's sweep, every window width from 320 to
  1440 px in 1 px steps, focusing each of the lede's five words, now also
  checking that the definition sits under its word;
- **contrast:** the WCAG contrast of the new and moved text at 1440x900,
  1024x768 and 390x844 in both themes;
- **the notes:** the engine's notes for every lesson, dumped from a test
  before and after (title, lead, explanation, why it matters, citations,
  what comes next), and Part 9's lesson walk, every note of the twelve
  machine lessons at 1440x900 light and 390x844 dark on both builds, now
  also recording the "Why it matters" note each step shows.

The screenshots are in [`screenshots/part12/`](screenshots/part12/), named
`<item>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours to keep them small.

### The output listings

#### 1. A wrapped row hung 4 columns in some listings and 8 in others

A wrapped row hangs two tab stops past its line's indent (Monaco's
`deepIndent`), but Monaco guessed each listing's tab size from the text its
model was made with, so the same line hung differently from listing to
listing: at 1366x768, Floating point's LLVM IR hung 8 columns at `-O0` and
4 at `-O2`, and Structs' bcc IR hung 4 columns alone, where its function's
instructions start, and 8 at `-O2`.

**Fixed.** The editors keep the tab size of 4 the site sets instead of
guessing (`detectIndentation: false`). Monaco keeps that setting in one
configuration for every editor, so the source takes it too; its lessons
indent by 4 already. Every wrapped row now starts at the second tab stop
past its line's indent: column 8 for a line at the margin, and for LLVM
IR's instructions, indented by 2, also column 8, six columns past them.
The measurement found the same hang in every listing of the three, alone
and side by side, at both sizes, where the before build had 4 in some and
8 in others.
[`compare-structs-1366-light`](screenshots/part12/compare-structs-1366-light-before.png)
([after](screenshots/part12/compare-structs-1366-light-after.png)).

#### 2. Revealing a linked line stopped at its first row

Pointing at a source line scrolls the listing to the lines it links to.
The editor revealed whole lines, which Monaco takes as the last line's
first row, so a last line that wraps was left with its later rows below
the editor's foot: after pointing at Floating point's source lines 8, 10
and then 13, the second row of LLVM IR line 19 (`noundef %v15)`) was half
hidden at 1366x768, 1024x768 and 900x800.

**Fixed.** The editor reveals a range from the first line's start to the
last line's end, which Monaco measures to the last row; a run too tall for
the editor shows its first line whole. The editor's scroll to a followed
line (the current instruction of the WebAssembly view) measures its whole
line in the same way. The reproduction now finds every row of lines 17 to
19 inside the editor at all three sizes.
[`reveal-1366-light`](screenshots/part12/reveal-1366-light-before.png)
([after](screenshots/part12/reveal-1366-light-after.png)).

#### 3. A number split at its point

Monaco may break a row after a full stop, so a narrow editor split
numbers and names that hold one: in the side-by-side columns at 1366x768
and stacked at 834x1112, Floating point's LLVM IR broke the string
constants `c"0.1 + 0.2 == 0.3 is %d\0A\00"`, `c"0.1 + 0.2 is %.17g\0A\00"`
and `c"ten 0.1s make %.17g\0A\00"` after `0.`, the review saw
`3.000000e-01` split the same way, and the source at 1366x768 split its own
line 12, `double sum = 0.1 + 0.` over `2;`.

**Fixed.** Every editor breaks after the characters Monaco's default list
has for Latin text except the full stop (`wordWrapBreakAfterCharacters`),
so numbers, `p.x`, `@.str.5` and `%v.x` stay whole; the source takes the
same rule, since its numbers split the same way. A piece too long for a row
still breaks, so no line runs past the editor: the measurement found no
number split and no row past the editor's edge in any of the listings or
the source.
[`compare-llvm-ir-1366-light`](screenshots/part12/compare-llvm-ir-1366-light-before.png)
([after](screenshots/part12/compare-llvm-ir-1366-light-after.png)).

#### 4. Part 11's account of the wrap

Part 11's "Review, round 1" said a wrapped row hangs "past where the next
instruction starts" and that the page "measures the whole wrapped line"
when it scrolls to a linked line. The first was true only of the listings
whose tab size Monaco had guessed as 4, and the second of the page's
scroll but not of the editor's own. That paragraph now says so and points
here.

### Part 11's "left for later"

#### 5. A narrow stage card put Next below the output

From 821 to 1180 px, where the pipeline stands beside the listing, the
stage card is narrow and stacks: the output came first and the hand-off,
with Next and Skip, after it, below the window's foot. Next sat 1,089 px
down at 1024x768, 1,137 px at 900x800 and 1,385 px at 834x1112.

**Fixed.** There the hand-off and its keys come before the output, as one
compact line (the hand-off at 18 px instead of 22): Next now sits 545 px
down at 1024x768, 544 px at 900x800 and 551 px at 834x1112, and 501, 500
and 551 px at bcc IR, which has no Skip. The rest of the notes (the
command, "In this program", the in, out and why lines) still follow the
output. The cost is the output's start: 99 px lower, at 651 px at
1024x768 (607 at bcc IR), which leaves 117 px of it in that window; the
keys that lead on are the part a reader needs before scrolling. The
keyboard order already had the keys before the output, so now it follows
what the eye sees. The phone keeps its order (the output first, under the
channel keys), and so does the wide card from 1181 px, whose notes column
already has the hand-off beside the output. A sticky Next and Skip was the
other way; it would cover the output's last lines while the reader reads
them.
[`stage-card-1024-light`](screenshots/part12/stage-card-1024-light-before.png)
([after](screenshots/part12/stage-card-1024-light-after.png)),
[`stage-card-834-light`](screenshots/part12/stage-card-834-light-before.png)
([after](screenshots/part12/stage-card-834-light-after.png)).

#### 6. The lede's definitions: under their words, and on a tap

Up to 1180 px a definition spanned the lede under it, since a box under
its word could run off the window (Part 11, review round 1), and a tap on
a phone followed the link to the glossary, away from the landing.

**Fixed.** The page now places each definition under its word at every
width: up to 340 px wide, as wide as the lede allows, and moved left as far
as keeps it inside the lede. A tap on a word shows its definition instead
of following the link, with a link to the entry inside it, "Read it in the
glossary"; a second tap on the word, or a tap on that link, goes to the
entry, and a tap anywhere else hides the definition. The mouse and the
keyboard work as before: hover or focus shows the definition, Escape hides
it, and Enter follows the word's link. The link inside the definition is
for the pointer only (`tabindex="-1"`, `aria-hidden`), so Tab goes from one
word to the next and a screen reader still reads the definition, without
the link's words, as the word's description; the word's own link goes to
the same entry. Before the page has loaded, a definition spans the lede
as before. The sweep found every definition under its word and inside the
window at every width from 320 to 1440 px in both themes (5,605 a theme),
the closest edge still 18 px from the window's, at 320 px. The probe's taps
on the phone showed the definition with the page's address unchanged,
followed the link on the second tap, and hid it on a tap elsewhere.
[`landing-term-900-dark`](screenshots/part12/landing-term-900-dark-before.png)
([after](screenshots/part12/landing-term-900-dark-after.png)),
[`landing-tap-390-light`](screenshots/part12/landing-tap-390-light-before.png)
([after](screenshots/part12/landing-tap-390-light-after.png)).

#### 7. Figure 0's stations

Figure 0's station names open their views, and said nothing of what each
form is.

**Fixed, for the mouse and screen readers.** Each station's link has a
title: the glossary's definition of its form, the same words the lede
shows, or for Types, which has no entry, what the stage does ("Semantic
analysis gives every name its declaration and every expression a type").
A screen reader reads it as the link's description. A touch screen and the
keyboard do not show a title; the lede above the figure shows the same
five definitions to both, so the stations do not repeat its machinery.

#### 8. The WebAssembly strip below 1024 px

Not fixed. At 900 and 834 px the strip still takes two rows (99 px): the
view keys (214 px), the transport (463), "Start at main" (142) and the
status (120) take 939 px before their gaps, more than the strip has. Part 11 already took the
numbers from the view keys and the words from Reset, Back and Run; what is
left would be the words of Over, Out and Next line, whose icons alone do
not say which is which, or the switch's label. A second row of the strip
costs 35 px and hides nothing.

### The steps' explanations

#### Method

The engine's notes for the first lesson (Objects, loops and calls, 198
steps), the pointers lesson (Pointers and arrays, 141) and the printf
lesson (Hello, world: printf and strings, 87) were read in order, with
Floating point's (231) for the kinds of step those three lack. At each
kind of step the question was whether a programmer who has not studied
compilers or the C standard would know, from the lead and the
explanation, what happened to memory and values and why C says so. Most
steps already do: a read names the object and the value, a store the value
before and after, a call the copies its parameters get, pointer arithmetic
the elements it moves, and each topic's "Why it matters" note says why the
rule exists. Four kinds of step did least, and three of them are the ones
the pointers lesson is about.

#### 9. Indirection: `*p` said "an lvalue designating"

`*value` in the first lesson (9 steps) and `*first` in the pointers lesson
(4) read: "`*value` follows the pointer to the object it points to,
values[0] (in main)", and then "`value` holds a pointer to values[0] (in
main), so `*value` (indirection) is an lvalue designating values[0] (in
main)". That is right but leans on the standard's words, and the step had
no "Why it matters" note: nothing said why following a pointer matters,
which is the point of `bump(cursor)`.

**Changed.** The lead says what `*value` now is: "`*value` follows the
pointer `value` to the object it points to: `*value` now stands for
values[0] (in main) itself." (Round 1 of the review dropped an "in" after
"the pointer": an expression does not hold a pointer, it is one.) The explanation says it in other words:
"`value` holds a pointer to values[0] (in main). The unary `*` operator
(indirection) turns a pointer back into the object it points to, so
`*value` is an lvalue: another name for values[0] (in main), the same place
in memory, not a copy of it. Nothing is read or written yet: the operator
around it decides which." A new note says why it matters: "A pointer holds
an address, and `*` is how a program uses it: reading `*p` reads the object
p points to, and storing into `*p` changes that object, not a copy. That is
how a function given a pointer changes its caller's objects. `*` needs a
pointer to an object that still exists: through a null or dangling pointer
it is undefined behavior, and this machine stops there." C99 §6.5.3.2p4
says the result "is an lvalue designating the object" and that `*` on an
invalid pointer is undefined, and its footnote 87 names a null pointer and
the address of an object after its lifetime (N1256, p. 79, PDF p. 91); the
machine stops on both (`Undefined::NullIndirection`,
`Undefined::AfterLifetime`).
[`note-indirection-1440-light`](screenshots/part12/note-indirection-1440-light-before.png)
([after](screenshots/part12/note-indirection-1440-light-after.png)),
[`note-indirection-1366-dark`](screenshots/part12/note-indirection-1366-dark-before.png)
([after](screenshots/part12/note-indirection-1366-dark-after.png)).

#### 10. A subscript said "a[i] means *(a + i)" in the abstract

In the printf lesson `length` counts letters with `text[count]`, where
`text` is a pointer, not an array. Its steps read "Subscripting counts
elements from the pointer: it moves 0 elements and designates name[0] (in
main)" and "Subscripting: a[i] means *(a + i). Starting from a pointer to
name[0] (in main), …", with letters that are not the program's, and no
note on why indexing a pointer works or what happens past the end.

**Changed.** The step quotes the program's own subscript and names both
values: the lead is "`text[count]` means `*(text + count)`: it moves 0
elements and designates name[0] (in main)", and the explanation
"Subscripting is pointer arithmetic followed by `*`: C defines
`text[count]` as `*(text + count)`. Here `text` gives a pointer to name[0]
(in main) and the index is 0, so the machine moves 0 elements and
designates name[0] (in main). Nothing is read yet: the operator around it
decides whether that element is read or stored into." A new note:
"Subscripting is pointer arithmetic: `a[i]` is `*(a + i)` whether `a` is an
array, which decays to a pointer first, or a pointer, so a function given
only a pointer can index its caller's array. Nothing checks the index:
reading or storing outside the array is undefined behavior, and this
machine stops on it, where a compiled program may read or overwrite
whatever lies there." C99
§6.5.2.1p2 defines `E1[E2]` as `(*((E1)+(E2)))` (p. 70, PDF p. 82), and
§6.5.6p8 leaves a result outside the array, or a `*` one past its end,
undefined (p. 83, PDF p. 95), as the step already cited. The quoted text is
the program's, from the expression's two operands (as the stop at a null
pointer's subscript already quotes it); without it the step keeps the
general `a[i]`. An index that is not a primary expression keeps its
parentheses, so `v[i << 1]` reads "`*(v + (i << 1))`" (corrected in
[Review, round 1](#review-round-1-1)).
[`note-subscript-390-dark`](screenshots/part12/note-subscript-390-dark-before.png)
([after](screenshots/part12/note-subscript-390-dark-after.png)).

#### 11. A return said "the objects of its frame cease to exist"

A return from a function other than main read "The return hands 3 back to
the caller of bump, and the objects of its frame cease to exist", with no
note. The explanation already named the call's new value and the objects
that end, but neither said why it is safe for `bump` to return a value
that lives in its own frame, which is the question a newcomer asks of a
frame that is about to vanish.

**Changed.** The lead names the call and the objects that end: "The return
hands 3 back as the value of `bump(cursor)`, and the frame of bump ends:
`value` and `before` cease to exist." A new note: "A function's parameters
and automatic locals (those not declared static) last only while its call
runs; a static local lasts the whole run. return copies the value out
before the frame ends, so the caller gets it safely; returning the address
of an automatic local instead would hand back a dangling pointer, since
the object it points to no longer exists." (Both as corrected in [Review,
round 1](#review-round-1-1): the first version said "its parameters and
locals cease to exist", which is false of a static local.) The step now
also cites C99 §6.2.4p2, "The value of a pointer becomes indeterminate
when the object it points to reaches the end of its lifetime" (p. 32, PDF
p. 44), beside §6.8.6.4p3 and §6.2.4p5. A return from main is unchanged;
its note is the exit status.
[`note-return-1440-light`](screenshots/part12/note-return-1440-light-before.png)
([after](screenshots/part12/note-return-1440-light-after.png)).

#### 12. A declaration's initializer said the declaration created the object

The step before an initializer (`` `total` receives its initial value``)
read "The value after `=` becomes the first value of `total`, the object
this declaration just created", two steps after "Begin the lifetime of
`total`" said the object was created at the block's brace (Part 7), and
its explanation opened "In a declaration, `total` is not an expression
that is evaluated", which leaves a newcomer to guess what it is instead.

**Changed.** The lead ends "…, the object this declaration names." The
explanation says what the name is and what comes next: "Here `total` is
not an expression to evaluate but the name being declared, so nothing is
read from it: its object already exists, and this step only marks it as
the one the initializer fills. The initializer after `=` is evaluated
next, converted to the object's type as an assignment would convert it,
and stored as its first value." For a char array initialized by a string
literal the last sentence is "The string literal after `=` comes next, and
its characters become the first values of the array's elements", citing
C99 §6.7.8p14 instead of §6.7.8p11, and for a structure initialized from
another "The structure after `=` is evaluated next, and its value, every
member included, becomes the object's first value", citing §6.7.8p13
(both p. 126, PDF p. 138); before, both cited the scalar paragraph.

#### 13. Two glossary entries

The new notes name indirection and a dangling pointer, and the glossary
(Table 2) had neither, so they gain entries, which the notes' first use of
each word links to, as for every term: **Indirection**, "The unary *
operator: *p is the object p points to, not a copy of it, so reading *p
reads that object and storing into *p changes it" (§6.5.3.2p4), and
**Dangling pointer**, "A pointer to an object whose lifetime has ended,
such as an automatic (non-static) local of a function that has returned.
Its value is indeterminate, and using it is undefined" (§6.2.4p2; "an
automatic (non-static)" added in round 1 of the review). The second also
marks the word "dangling" in the note on lifetimes, which a step that ends
a block shows.

#### Intended changes to the text

No lesson's title in Table 1 and no step's title change. The changes are
these four kinds of step, in every lesson where they occur (the twelve
machine lessons, and the pipeline lesson's machine run):

| Step | Lead | Explanation | Why it matters | Citations |
| --- | --- | --- | --- | --- |
| `*p` (dereference) | new | new | new note | same |
| `a[i]` (subscript) | new | new | new note | same |
| return from a function other than main | new | same | new note | adds §6.2.4p2 |
| `x` receives its initial value | new last words | new | none, as before | §6.7.8p14 or p13 for an array or a structure |

Counted over the engine's dump, the notes that change, of each lesson's:
Objects, loops and calls 21 of 198 (9 initializers, 9 dereferences, 3
returns); Hello, world 8 of 87 (3, 4 subscripts, 1); Names for values 4 of
98; Recursion and frames 8 of 79 (4, 4 returns); Integer promotions 3 of
35; Unsigned numbers 11 of 184 (8, 3 subscripts); Signed overflow 2 of 38;
Floating point 8 of 231; Pointers and arrays 8 of 141 (3, 4 dereferences,
1 return); Structs and static storage 8 of 135 (4, 2 dereferences, 2
returns); Reading past the end 1 of 21; Uninitialized reads 1 of 7: 83 of
the 1,254. Nothing else in any note changes: no other title, lead,
explanation, note, citation or "next" line, and the Designate lead of a
read, a store, `&` and `++` is as before.

Every one of these 83 notes, and only they, differs in the lesson walk:
at 1440x900 light and at 390x844 dark, 83 of the 1,254 notes differ from
the before build, the same 83 the dump lists. In the walk's fields that is
the lead of all 83 (50 initializers, 15 dereferences, 11 returns, 7
subscripts) and the "Why it matters" note where a lesson first meets the
three new ones (3 dereferences, 5 returns, 2 subscripts); no title and no
value flow changes. The walk of the first build, `a1`, and of the final
build are the same note for note.

#### Checks

- **No sideways scroll** at any step of the probe in any of the twelve
  configurations, in the listings' measurements, at any width of the sweep,
  or at any note of the lesson walks, and no page logged an error or a
  warning.
- **Contrast:** the definition's link is the accent ink on the device,
  bold at 11.5 px: 4.99:1 in the light theme, as Part 10 measured that ink
  on that ground, and 6.87:1 in the dark. The rest is unchanged: the
  definition 10.59:1 and 9.40:1, its word 15.82:1 and 13.34:1, the hand-off
  at 18 px 15.82:1 and 13.34:1, Next 6.06:1 and Skip 15.82:1 and 13.34:1.
- **Keyboard and screen readers:** focus on a lede word shows its
  definition, Tab goes to the next word and hides it, Escape hides it, and
  Enter follows the link; the definition's own link takes no Tab stop and
  is hidden from screen readers, and the word's description is the
  definition alone. In the narrow stage card the Next and Skip keys now come
  before the output both on screen and in the Tab order. Figure 0's
  stations are described by their titles. The new notes' words for
  indirection and a dangling pointer are glossary terms, read and focused
  like the others.
- **Reduced motion:** nothing new moves. The reveal of a linked line glides
  as before and jumps with reduced motion.
- **Tests:** the site's tests (164, and the prerender's 2) and its four
  clippy runs (native, and wasm32 for hydrate, worker and compiler) pass;
  the worker run prints the engine's existing unfulfilled lint expectation
  in `src/abstract_machine/parser.rs`, as before. The engine's tests (115)
  and its clippy pass as `pages.yml` runs them; the tests now check the
  new leads, the three new notes, the return's citation, and the
  initializer's paragraph for a char array and a structure.

### Left for later

- **The WebAssembly strip below 1024 px** still takes two rows (finding 8).
- **The narrow stage card's output** starts 99 px lower than before at
  1024x768, so 117 px of it is in that window at the first stage. Folding
  the hand-off into the stage's heading line would win some of it back.
- **Figure 0's stations** show their definitions to the mouse and to
  screen readers only; a tap or the keyboard opens the view, as before.
- **Other steps a newcomer may stumble on:** the integer promotion of a
  char reads "Promote char 65 to int" where the read before it said 65
  ('A'), and the comparison after it reads "Compare 65 != 0" for
  `text[count] != '\0'`; naming the characters there would join the loop in
  the printf lesson to its source. Entering a block still explains, in its
  first sentence, that this machine leaves out variable-length arrays,
  which a newcomer does not need there.

### Review, round 1

The review of the branch at `9f70fa181` found two blockers, both in the
new notes' words about C, and three small things on the page. The before
build is the review's build of that head; the after build is the round's
fixes at `24b06cadf` plus the focus fix below, built as above (Trunk
without `--release`, the dev profile at `opt-level=s`).

#### 1. A subscript with a compound index was quoted as another expression

The subscript's lead spliced the program's base and index into
`*(base + index)` without parentheses, so `v[i << 1]` read "means
`*(v + i << 1)`", a shift of a pointer, and `row[n - 1]` read
`*(row + n - 1)`, which forms `row + n` first. The lessons only index with
a name, so neither the walk nor the tests met it, but the workbench runs
whatever the reader types. The stop at a null pointer's subscript, older
than this part, did the same.

**Fixed.** C99 §6.5.2.1p2 defines `E1[E2]` as `(*((E1)+(E2)))` (p. 70,
PDF p. 82). The index now keeps its parentheses unless it is a primary
expression (§6.5.1: a name, a constant, a string literal or an expression
already in parentheses), so `v[i << 1]` reads "`*(v + (i << 1))`" and
`v[3]` still "`*(v + 3)`". The base needs none: it is a postfix
expression, which binds tighter than `+`, and its text keeps the
parentheses it was written with, so `(p + 1)[2]` reads "`*((p + 1) + 2)`"
and `t.a[i]` "`*(t.a + i)`". The lead, the explanation and the null
pointer's stop share one spelling. A new engine test pins eight
subscripts (`v[i << 1]`, `v[n == 1]`, `v[n - 1]`, `(p + 1)[2]`, `t.a[i]`,
`v[(i)]`, `v[3]` and `v['\0']`) in both the lead and the explanation, and
the null pointer's test now expects `*(q + (i + 1))`.

#### 2. A return said every local ends, in the lesson about static locals

The return's new lead said "the frame of moved ends: its parameters and
locals cease to exist", and its note "A function's parameters and locals
last only while its call runs". In Structs and static storage, `moved`
declares `static int total;`, which lives for the whole run (C99
§6.2.4p3), as that lesson's own notes say.

**Fixed.** The lead names the objects that end, as the explanation already
did: "…and the frame of moved ends: `p` and `dx` cease to exist" (one
object takes "ceases"; past three, "its parameters and automatic locals
cease to exist"). The note: "A function's parameters and automatic locals
(those not declared static) last only while its call runs; a static local
lasts the whole run. return copies the value out before the frame ends,
so the caller gets it safely; returning the address of an automatic local
instead would hand back a dangling pointer, since the object it points to
no longer exists." A new engine test returns twice from a function with a
static local and checks that neither the lead nor the explanation names
it, and pins the singular and the summed-up forms.

The same search over the other new or changed words found one more claim
of each kind. The glossary's **Dangling pointer** said "such as a local
of a function that has returned"; it now says "an automatic (non-static)
local". No other new text quotes an expression it builds: the compound
assignment's `x = x op (e)` already parenthesizes its right operand, and
`*p`'s quoted operand is the program's own text, parentheses and all.

#### 3. Small things

- **The stop banner below Next.** From 821 to 1180 px the narrow stage
  card gave the hand-off (with Next) and the "Compilation stopped" banner
  the same order, so the hand-off came first. The hand-off now comes after
  the banner and before the output: at 1024x768 with a program bcc
  rejects, the banner sits at 490 px, the hand-off at 694, Next at 748 and
  the output at 855 (before: the hand-off at 490, the banner at 589). A
  sweep of 13 widths from 390 to 1440 px, in both themes, found the banner
  first at every width and no sideways scroll.
  [`err-1024-light`](screenshots/part12/err-1024-light-before.png)
  ([after](screenshots/part12/err-1024-light-after.png)).
- **A tap on a definition's text hid it.** The tap moved the focus from
  the word to the page, and the word's `focusout` hid the definition. A
  press inside the word, its definition included, now keeps the definition
  open until the press's click; if the focus then lands anywhere else, by
  Tab for instance, the definition hides as before. The touch probe at
  390x844 (both themes), 834x1112 and 1024x768: a tap on IR's text leaves
  it open and the page where it was, the next tap on the word follows the
  link, and Tab from there shows only Assembly's definition.
- **A definition kept its place after a resize.** An open definition now
  takes its place again when the window resizes or the phone rotates. The
  probe opened IR at 390 px and narrowed the window to 320 px, then rotated
  390x844 to 844x390 and back: each time the box sat under the word, inside
  the window and uncovered.
- **Wording.** `*(q + 0)`'s lead now reads "follows the pointer `(q + 0)`"
  rather than "the pointer in `(q + 0)`", and the subscript's note says
  "reading or storing outside the array is undefined behavior" where it
  said any index outside was, since `&a[n]` is defined (§6.5.3.2p3). The
  editor's comment on the listings' hang now says where a wrapped row goes
  on: two tab stops past its line's indent, which is column 8 both for an
  unindented line and for LLVM IR's 2-space instructions.

#### Checks

- **The notes:** the engine's dump differs from the round's before in 34
  notes, 33 of the twelve machine lessons' 1,254 and the pipeline lesson's
  return: the lead of 15 dereferences, the lead and note of 12 returns, and
  the note of 7 subscripts. The lesson walk at 1440x900 light and 390x844
  dark finds the same 28 of its 1,254 notes changed in both (the walk
  records a note only where a lesson first shows it), no title or value
  flow changed, no sideways scroll and no page error.
- **The probe** of this part at 1440x900, 1366x768, 1024x768, 900x800,
  834x1112 and 390x844 in both themes reads as before at every
  configuration but one figure: the glossary's longer Dangling pointer entry
  moves IR's entry 20 px down the page. No sideways scroll, no console
  error or warning.
- **The definitions' sweep**, every third width from 320 to 1440 px in both
  themes (1,870 checks a theme): no failures, the closest edge still 18 px
  from the window's.
- **Tests:** as `pages.yml` runs them, the engine's tests (117) and its
  clippy pass, as do the site's tests (164, and the prerender's 2), the
  WebAssembly stepper's (79) and the site's four clippy runs; the worker
  run prints the engine's existing unfulfilled lint expectation in
  `src/abstract_machine/parser.rs`, as before.

#### Left for later

- **A screen reader on a touch screen** (the review's finding 5, not
  verified on a device): TalkBack's double tap arrives as touch pointer
  events, so its first activation may show the definition, which the
  reader already heard as the description, instead of following the link.
- **The source's indent guides** now sit at 4-column stops for a pasted
  program indented by 2 or by tabs, since the tab size is fixed; the
  lessons indent by 4.
- **Seen on `main` too:** at 1024 px the listing's header with an edited
  program overflows into the pipeline column; at 1180 px the stage keys
  "Preprocessing tokens" and "Preprocessor" truncate; and from 760 to
  820 px (an iPad in portrait) the card still puts the output first, with
  Next 1,261 px down.

## Part 13: the C a beginner types next (2026-11)

The last review found that a reader who edits a lesson soon meets refusals
for C that every textbook's first chapters use: `&&` and `||`, the bitwise
operators, `?:`, the comma operator in a `for` loop, hexadecimal and octal
constants, a call inside a larger expression, `i[v]`, pointers to pointers,
arrays of pointers and function pointers. This part measures how much of
that the machine refused, runs the most valuable constructs with the same
narration as the rest (what happened to memory and values, the C99 rule,
why it matters, and what comes next), adds two lessons that teach what the
other twelve machine lessons do not, and checks every new construct in the
machine view, the expression panel and the stage views. The work is on
`agent/machine-subset`.

### Method

- **The corpus.** One small program per construct, written as a textbook
  or a beginner would, is in [`corpus/`](corpus/): 32 programs (six for
  the bitwise operators, six for `&&` and `||`, three for `?:`, two for the
  comma operator, three for constants, four for calls, one for `i[v]`, five
  for pointers to pointers and arrays of pointers, two for function
  pointers). A test steps each one to its end, and
  [`corpus/results-before.txt`](corpus/results-before.txt) and
  [`corpus/results-after.txt`](corpus/results-after.txt) are its output on
  `main` and on this branch. The 30 programs the branch runs were also
  compiled with GCC 13 and run: every exit status and every output
  agrees.
- **The notes.** A throwaway engine test dumped every step of every lesson
  (title, lead, explanation, why it matters, citations, what comes next,
  and whether a sequence point follows) on `main` and on the branch.
- **The builds.** The before build is `main` at `4d7c18a25` (Part 12
  merged), the after build this branch at ``c6f29c6dd``, both with
  `web/build.sh` as in Parts 7 to 12: Trunk without `--release`, the dev
  profile at `opt-level=s`.
- **The browser.** Playwright drove Chromium with reduced motion, one
  browser at a time, at 1440x900, 1366x768, 1024x768 and 390x844 in both
  themes, on both builds. Each program was loaded through the page's own
  Open .c, as a reader's file would be (typing or pasting it lets the
  editor's auto-indentation and closing braces change it), run with Run
  edited code and stepped to the step named below, recording the note,
  the value flow, the cells under the head, Table 1, Figure 2 and its
  outline for screen readers, sideways scroll and console errors. The two
  new lessons were also opened from Table 1 and shown in the compiler
  pipeline and WebAssembly views, and the contrast of the new text was
  measured against its composited background.
- **The lesson walk.** Part 9's walk, every note of every machine lesson,
  on both builds at 1440x900 light and 390x844 dark.

The screenshots are in [`screenshots/part13/`](screenshots/part13/), named
`<scene>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours to keep them small.

### The corpus, before and after

| Construct | Program | Before: where the engine refused it | After |
| --- | --- | --- | --- |
| Bitwise | [`b01_and_mask.c`](corpus/b01_and_mask.c) | parser, 3:21: expected `;` | runs (15 steps), returns 1 |
| Bitwise | [`b02_or_set.c`](corpus/b02_or_set.c) | lexer, 3:21: unsupported character | runs (15 steps), returns 9 |
| Bitwise | [`b03_xor_toggle.c`](corpus/b03_xor_toggle.c) | lexer, 3:24: unsupported character | runs (15 steps), returns 6 |
| Bitwise | [`b04_not_complement.c`](corpus/b04_not_complement.c) | lexer, 3:20: unsupported character | runs (17 steps), returns 240 |
| Bitwise | [`b05_compound_bits.c`](corpus/b05_compound_bits.c) | lexer, 3:11: unsupported character | runs (20 steps), returns 7 |
| Bitwise | [`b06_is_odd.c`](corpus/b06_is_odd.c) | parser, 3:11: expected `)` | runs (13 steps), returns 1 |
| `&&`, `\|\|` | [`l01_and_range.c`](corpus/l01_and_range.c) | parser, 3:15: expected `)` | runs (19 steps), returns 1 |
| `&&`, `\|\|` | [`l02_or_either.c`](corpus/l02_or_either.c) | lexer, 4:18: unsupported character | runs (24 steps), returns 1 |
| `&&`, `\|\|` | [`l03_and_value.c`](corpus/l03_and_value.c) | parser, 4:22: expected `;` | runs (25 steps), returns 0 |
| `&&`, `\|\|` | [`l04_null_guard.c`](corpus/l04_null_guard.c) | parser, 4:16: expected `)` | runs (18 steps), returns 0 |
| `&&`, `\|\|` | [`l05_while_and.c`](corpus/l05_while_and.c) | parser, 4:18: expected `)` | runs (76 steps), returns 2 |
| `&&`, `\|\|` | [`l06_not_and.c`](corpus/l06_not_and.c) | parser, 4:16: expected `)` | runs (17 steps), returns 1 |
| `?:` | [`c01_max.c`](corpus/c01_max.c) | lexer, 4:21: unsupported character | runs (24 steps), returns 9 |
| `?:` | [`c02_abs_return.c`](corpus/c02_abs_return.c) | lexer, 2:18: unsupported character | runs (16 steps), returns 6 |
| `?:` | [`c03_printf_choice.c`](corpus/c03_printf_choice.c) | lexer, 5:45: unsupported character | runs (23 steps), returns 0 and prints `1 item` |
| Comma | [`k01_for_two.c`](corpus/k01_for_two.c) | parser, 5:15: expected `;` | runs (114 steps), returns 4 |
| Comma | [`k02_comma_value.c`](corpus/k02_comma_value.c) | parser, 3:19: expected `)` | runs (21 steps), returns 20 |
| Constants | [`h01_hex.c`](corpus/h01_hex.c) | type checker, 2:16: octal and hexadecimal integer constants are outside this subset; write the value in decimal | runs (8 steps), returns 255 |
| Constants | [`h02_octal.c`](corpus/h02_octal.c) | type checker, 2:23: octal and hexadecimal integer constants are outside this subset; write the value in decimal | runs (8 steps), returns 493 |
| Constants | [`h03_hex_unsigned.c`](corpus/h03_hex_unsigned.c) | type checker, 2:24: octal and hexadecimal integer constants are outside this subset; write the value in decimal | runs (21 steps), returns 1 |
| Calls | [`f01_call_sum.c`](corpus/f01_call_sum.c) | type checker, 6:38: calls must be whole expressions; operand sequencing is outside this subset | runs (26 steps), returns 13 |
| Calls | [`f02_call_condition.c`](corpus/f02_call_condition.c) | parser, 7:20: expected `)` | runs (26 steps), returns 1 |
| Calls | [`f03_call_argument.c`](corpus/f03_call_argument.c) | type checker, 8:29: nested function calls are outside this subset | runs (17 steps), returns 0 and prints `9` |
| Calls | [`f04_call_nested.c`](corpus/f04_call_nested.c) | type checker, 6:26: nested function calls are outside this subset | runs (17 steps), returns 12 |
| `i[v]` | [`i01_index_reversed.c`](corpus/i01_index_reversed.c) | type checker, 4:16: subscript requires a pointer to an arithmetic type and a side-effect-free integer index | runs (21 steps), returns 30 |
| Pointers | [`p01_pointer_to_pointer.c`](corpus/p01_pointer_to_pointer.c) | parser, 4:10: expected an identifier; keyword or unsupported type found | runs (25 steps), returns 5 |
| Pointers | [`p02_out_parameter.c`](corpus/p02_out_parameter.c) | parser, 1:15: expected an identifier; keyword or unsupported type found | runs (29 steps), returns 3 |
| Pointers | [`p03_array_of_pointers.c`](corpus/p03_array_of_pointers.c) | type checker, 5:10: arrays of pointers are outside this subset | runs (36 steps), returns 20 |
| Pointers | [`p04_array_of_strings.c`](corpus/p04_array_of_strings.c) | type checker, 4:11: arrays of pointers are outside this subset | runs (83 steps), returns 0 and prints `Ada Grace Linus` |
| Pointers | [`p05_strings_unsized.c`](corpus/p05_strings_unsized.c) | type checker, 4:21: an array of unknown size needs a string literal initializer in this subset; give its size | runs (23 steps), returns 0 and prints `Wed` |
| Function pointers | [`g01_function_pointer.c`](corpus/g01_function_pointer.c) | parser, 6:9: expected an identifier; keyword or unsupported type found | refused: expected an identifier |
| Function pointers | [`g02_function_table.c`](corpus/g02_function_table.c) | parser, 10:9: expected an identifier; keyword or unsupported type found | refused: expected an identifier |

On `main` all 32 were refused before their first step. Eight stopped in
the lexer, which did not know `~`, `|`, `^`, `?` or `:`; fourteen in the
parser, which read `&` and `&&` as nothing it knew and so expected a `)`
or `;`, and found no name after a second `*` or in `int (*f)(…)`; and ten
in the type checker, the part of the parser that gives each expression its
type: octal and hexadecimal constants, calls inside expressions, `i[v]`,
arrays of pointers and an array whose length comes from its list. None
stopped in the stepper: the subset is enforced before the first step. On
the branch 30 run to the end GCC gives them; the two with
function pointers are still refused (see "Left for later").

### What changed

#### 1. Bitwise operators show their bits

`&`, `^`, `|` and `~`, and `&=`, `^=` and `|=`, are steps of their own,
after the integer promotions and the usual arithmetic conversions they
need. Each writes its operands' bits: `13 & 3` reads "`&` keeps only the
bits that are 1 in both operands: 0000 1101 & 0000 0011 = 0000 0001, which
is 1", with as many groups of four as the largest value needs and at least
eight bits, all of the type's bits when a value is negative, and in the
lead `1111 … 1111 0000` for the middle of a 32-bit pattern that repeats its
top bit. `~` says it works on the promoted operand ("flips each of its 32
bits") and why the result is negative in an int ("~x is -x - 1" in two's
complement) or the largest value minus x in an unsigned type. A compound
`mode &= ~WRITE` names the bits it combines after the conversion
(4294967293, not -3). The constraint for a floating or pointer operand is
reported as one (C99 §6.5.10p2, §6.5.3.3p1). New "why it matters" note:
flags and masks, and `&` against `&&`.
[`bits-1366-light`](screenshots/part13/bits-1366-light-after.png)
([before](screenshots/part13/bits-1366-light-before.png)).

#### 2. `&&` and `||` stop early, and the page shows what they skipped

The test of the first operand is a step of its own: "Left of && is false:
skip the right, result 0", with the lead "`i < 3` is 0, so `i < 3 &&
scores[i] != 5` is 0 whatever its right operand would give, and
`scores[i] != 5` is never evaluated". It ends at a sequence point, so the
expression panel's dashed line and the tape's taller clock pulse mark it
like the end of a statement, and it cites §6.5.13p4 (or §6.5.14p4), the
value rule §6.5.13p3 when the left operand settles the result, and Annex
C's list of sequence points. When the left operand does not settle it, a
second step turns the right operand into the `0` or `1` that `&&` yields
("`x < 10` is 1: && yields 1"). The engine records the nodes it skipped,
and Figure 2 draws them faint, their gates dashed and their wires dotted,
with "skipped" on the wire into the operator; the outline screen readers
hear says "skipped, never evaluated" where other nodes say "not evaluated
yet". The value flow reads "0 counts as false so skip the right result
0". New note: the null and bounds guards short-circuiting makes safe.
[`logic-1440-light`](screenshots/part13/logic-1440-light-after.png)
([before](screenshots/part13/logic-1440-light-before.png); [Figure
2](screenshots/part13/logic-schematic-1440-light-after.png);
[`logic-390-light`](screenshots/part13/logic-390-light-after.png)).

#### 3. `?:` evaluates only the operand it chooses

The same test, "Condition of ?: is false: choose `0`", then a step that
gives the conditional its value, converted to the operands' common type
when they differ (`c ? 1 : 2.5` yields 1.0, citing §6.5.15p5); a null
pointer constant beside a pointer gives a null pointer. The operand not
chosen is skipped in Figure 2 as for `&&`.
[Figure 2 at 1024 px](screenshots/part13/ternary-schematic-1024-light-after.png).

#### 4. The comma operator

`for (i = 0, j = 3; i < j; i++, j--)` runs: the store into `i` no longer
claims to end the full expression, and a comma step ("Comma: `i = 0` is
done, go on to `j = 3`") marks the sequence point between the two
operands. In parentheses, `(x + 1, x * 10)` discards 3 and yields 20.
[`comma-1366-dark`](screenshots/part13/comma-1366-dark-after.png)
([before](screenshots/part13/comma-1366-dark-before.png); [Figure
2](screenshots/part13/comma-schematic-1366-dark-after.png)).

#### 5. Octal and hexadecimal constants

`0xFF` reads "The hexadecimal constant 0xFF is the int value 255", and its
explanation computes the value in its base ("15 × 16 + 15 = 255") and
gives the type list octal and hexadecimal constants have, which includes
the unsigned types: `0xFFFFFFFF` is an unsigned int where `4294967295` is a
long, and the note says so. `09` and `0x` are refused with what is wrong.

#### 6. Pointers to pointers and arrays of pointers

A declarator takes up to four `*`, and an array may hold pointers; `T
name[] = {…}` takes its length from the list. `**pp`'s second step names
`*pp` as the pointer it follows (the old wording stripped every `*` and
said `*pp`). Under the head, `p` reads `→x ◂pp` and `pp` reads `→p`, so
the chain reads in both directions; Table 1 spells the types `int **` and
`int *[3]` (24 bytes); an array of pointers' cells each hold an arrow
(`→a`, `→b`, `→c`), and an array of strings' cells point into each
literal (`→"Grace"[0]`).
[`pp-1440-light`](screenshots/part13/pp-1440-light-after.png)
([before](screenshots/part13/pp-1440-light-before.png); [the
cells](screenshots/part13/pp-headrow-1440-light-after.png); [Table
1](screenshots/part13/pp-datasheet-1440-light-after.png);
[`pp-390-dark`](screenshots/part13/pp-390-dark-after.png)),
[`slots-1024-dark`](screenshots/part13/slots-1024-dark-after.png)
([before](screenshots/part13/slots-1024-dark-before.png); [the
tape](screenshots/part13/slots-deck-1440-light-after.png)),
[`strings-390-dark`](screenshots/part13/strings-390-dark-after.png)
([before](screenshots/part13/strings-390-dark-before.png); [the
cells](screenshots/part13/strings-headrow-390-dark-after.png)).

#### 7. Calls inside expressions, and `i[v]`

A call may now be an operand of any expression, an argument of another
call or an element of an initializer list: `square(2) + square(3)` and
`printf("%d\n", square(3))` run, each call folded in the expression panel
as before. `i[v]` is `*(i + v)` and runs (C99 §6.5.2.1p1).

#### 8. Two new lessons

- **"Short-circuit evaluation: &&, || and ?:"** ends the first part of
  Table 1: a search whose last test never reads past the array, a `?:`
  that picks a null pointer, a null test that keeps `*` from running and a
  `?:` that picks printf's text. It is the first lesson whose run depends
  on an operand not being evaluated.
- **"Bits and masks"** follows the unsigned lesson: three flags, each a
  `#define` of a hexadecimal constant, set with `|=`, cleared with `&=
  ~`, flipped with `^=` and tested with `&`, printed with `%x`.

Their comments stand on lines of their own and take at most 36 columns,
since the compiler view's listing holds about 37 at 1366 px and the
machine's about 40. Their LLVM IR comes from `web/pipeline/generate.sh`
with a dev-profile bcc, whose output for the other lessons was byte for
byte the committed one, and their modules from `web/wasm/generate.sh` with
wasi-sdk 34, which also rebuilt the other modules unchanged. The first
part's caption now reads "Objects, output, names for values, calls, and the
order things happen in."
[`lesson-short-circuit-1366-dark`](screenshots/part13/lesson-short-circuit-1366-dark-after.png)
(in the [compiler pipeline](screenshots/part13/lesson-short-circuit-compiler-1366-dark-after.png)
and the [WebAssembly view](screenshots/part13/lesson-short-circuit-wasm-1366-dark-after.png)),
[`lesson-bits-and-masks-390-light`](screenshots/part13/lesson-bits-and-masks-390-light-after.png)
(in the [compiler pipeline](screenshots/part13/lesson-bits-and-masks-compiler-390-light-after.png)).

#### 9. The glossary and "Limits of this model"

The glossary defines short-circuit evaluation (§6.5.13p4), the bitwise
operators (§6.5.10p4) and two's complement (§6.2.6.2p2). "Limits of this
model" (§3 of "How it works") now lists what runs, every operator family,
octal and hexadecimal constants, pointers to pointers, arrays of pointers
and calls anywhere in an expression, and what does not: switch, do, break,
continue and goto; typedef, enum, union and _Bool; void and static
functions; function pointers; arrays of structures or of arrays;
subtracting or ordering two pointers; an assignment or increment used as
an operand (except a statement's commas and one update in a condition);
and the other library functions. A new engine test runs a program for
each claim of what runs and checks a refusal for each claim of what does
not.

### Checks

- **The corpus:** 30 of the 32 programs run to the end GCC gives them
  (`results-after.txt`); every step's "what comes next" named the step
  that followed it (the engine test `the_corpus_of_beginner_programs_runs`
  checks this for every program).
- **The notes:** the engine's dump of every lesson is the same, byte for
  byte, before and after (952 KB). The browser walk at 1440x900 light and
  390x844 dark finds none of the twelve machine lessons' 1,254 notes
  changed in its title, lead, value flow, "why it matters" note or next
  step, no sideways scroll and no page error; at 390 px, 144 notes' next
  step sits 1 px higher, as the two new lessons lengthen Table 1 above the
  workbench. The two new lessons walk 112 and 54 notes, every current line
  in view, with no sideways scroll.
- **The scenes:** at the four sizes in both themes, every scene reached
  its step on the after build and was refused before its first step on
  the before build. No sideways scroll, no console error or warning.
  Figure 2 marks 11 nodes skipped at `&&`'s last test and 7 at `?:`'s,
  with one "skipped" label each, and its outline names each of them
  "skipped, never evaluated". Both lessons open in the compiler pipeline
  and the WebAssembly view without a stop or sideways scroll.
- **Contrast:** the "skipped" label and the skipped operands' text measure
  10.59:1 in the light theme and 9.40:1 in the dark one at every size;
  the new leads and value flows use the existing colours (15.82:1 and
  13.34:1).
- **Keyboard, screen readers and motion:** the change adds no control.
  Figure 2's drawing stays hidden from screen readers, whose outline
  carries the skipped state; the new styles have no transition, and every
  run used reduced motion.
- **Tests:** as `pages.yml` runs them, the engine's tests (129, 12 of them
  new) and its clippy pass, as do the site's tests (164, and the
  prerender's 2), the WebAssembly stepper's (79) and the site's four
  clippy runs; the worker run prints the engine's existing unfulfilled
  lint expectation in `src/abstract_machine/parser.rs`, as before.
  `rustfmt +nightly --check` passes on every touched engine file, and on
  the site's files every changed line is formatted (the files themselves
  are not, on `main` either); `git diff --check` is clean.

### Left for later

- **Function pointers** stay outside the subset. They need a value that is
  a function, a declarator grammar with parentheses (`int (*f)(int,
  int)`), a call through a pointer, and a way to draw a pointer whose
  target is code rather than an object, which the tape and the cells have
  no lane for.
- **Pointer subtraction and ordering** (`p - q`, `p < q`) still refuse; a
  beginner's two-pointer loop compares with `!=`, which runs.
- **An assignment or increment as an operand** (`a = b++`, `while ((c =
  next()) != 0)`) still refuses: the machine would need to narrate
  unsequenced side effects, and to stop where C leaves them undefined.
- **Constant expressions in static initializers** (`static int mask = 1
  << 3;`) still take only a constant.
- **Narrow listings:** the listing holds 37 to 40 columns at 1366 and
  1440 px, so the existing lessons' trailing comments, and three lines of
  code in each new lesson, wrap; the new lessons keep their comments short
  instead.

### Review, round 1

The review of the branch at `cd04dd1b2` found three blockers, all in what
the machine said about C's sequence points, and five small things. Every
value, output, exit status and stop it checked was right. The probe
programs and their engine dumps are in the review's scratch directory; the
fixes below are checked against the same programs (82 across its three
probe directories), 27 more of my own for the cases the fixes open, and
GCC 14 (`-std=c99`).

#### 1. An update's store was placed at the end of the condition

In `while (i-- > 0 && n < 5)`, the update's step said the store into `i`
completes "by the next sequence point, the end of the condition". The next
sequence point is the end of `i-- > 0`, `&&`'s first operand (C99
§6.5.13p4, p. 89, PDF p. 101), as the very next `&&` step says. The text
predates this part; letting `&&`, `||` and `?:` into a condition made it
reachable.

**Fixed.** The update now records the sequence point that next follows it,
and its step names it: the end of the first operand of `&&`, `||` or `?:`
(§6.5.14p4, §6.5.15p4), the end of a comma operator's left operand
(§6.5.17p2, p. 94, PDF p. 106), the point just before a call whose
argument it is in (§6.5.2.2p10, p. 72, PDF p. 84), or, everywhere else,
the end of the condition, a full expression (§6.8p4, p. 131, PDF p. 143).
Nested operators take the nearest: in `(x && i++ == 2) && x` it is the end
of the outer `&&`'s first operand. Each step cites the paragraph it names.
A new engine test runs eight conditions, one per context, and checks that
the next step marked as a sequence point is the one the update named: the
`&&` or `||` test, the `?:` test, the comma, the call or the condition.

#### 2. The comma operator's note said declarations give no order

"The commas between a call's arguments or in a declaration only separate,
so they give no order and no sequence point." That is right for
arguments, and wrong for declarations: C99 §6.8p3 evaluates a block's
initializers "in the order that declarators appear", and each initializer
is a full expression (§6.8p4; Annex C also lists the end of a full
declarator).

**Fixed.** The note now names three commas: the comma operator, a
sequence point; the commas between a call's arguments, which give the
arguments no order and no sequence point between them; and the commas
between declarators, where in `int a = 1, b = a + 1;` inside a function
each initializer is a full expression, evaluated in declarator order, so b
is 2. (The machine still takes one declarator per declaration.) An engine
test pins the note.

#### 3. Refusals of valid conditions said there was no sequence point

`if (i++ && i++)`, `if (i++ == 0 && i == 1)`, `while (i++ < 3 && i != 2)`
and `while (i-- > 0 && f(1))` are all defined, but the machine refused
them "without a sequence point between them". Looking at why, I found the
opposite hole too: an update in a call's argument was not checked at all,
so `while (f(i--) > i)`, which is undefined, ran until the step limit.

**Fixed, by running them.** The machine makes an update's store at once,
and `&&`, `||`, `?:` and the comma operator already evaluate in their
order, so it runs any condition in which a sequence point separates each
update from every other access that could reach its variable. The check
walks the condition: the operands of `&&`, `||` and the comma operator are
separated, the first operand of `?:` from the other two (and those two
are never both evaluated), and a call's arguments from the call; every
other operator's operands are unsequenced with each other (C99 §6.5p2-3,
Annex C). Two variables may each be updated, as in `i++ < j--`. What
remains is refused with a reason that is true:

- a read or second update of the variable that no sequence point
  separates from the update, as in `i++ + i` or `f(i--) > i`: "this
  condition updates `i` and also reads or updates it with no sequence
  point between the two; C99 makes that undefined whenever both are
  evaluated (§6.5p2)";
- a `*`, `[]`, `->` or call that no sequence point separates from it, as in
  `i++ == *p` or `i-- > f(1)`: "this machine does not track which objects
  *, [], -> and calls reach, so in a condition that updates `i` it accepts
  them only where a sequence point separates them from the update". That
  is a limit of the model and says so.

The update's own step keeps "Nothing else in the condition touches `i`"
when that is so, and otherwise says every other access that could reach
`i` is separated from it by a sequence point or never evaluated with it.
All six review probes the old check refused for a false reason now run,
with GCC's output and exit status; `i++ < f()` with `f` reading the
global `i` stays refused as the model's limit.

#### 4. Small things

- **`*pp++` on a `char **`** was refused "use (*p)++ to increment the
  pointed-to int". The message now says `*p++` means `*(p++)`, an
  increment nested in an operand, and offers `(*p)++` for the object `p`
  points to or `p++` to move `p`.
- **Limits of this model** said a condition "may update one variable". It
  now says a condition may update variables, as in `i-- > 0 && n < 5`, if
  a sequence point separates each update from every other use of its
  variable, from `*`, `[]` and `->`, and from any call it is not an
  argument of. The limits test runs the new claim and refuses its
  counterexamples.
- **The comma step** said a plain read like `j` in `(j, i < 3)` "was
  evaluated only for its side effects". It now says its value is thrown
  away, "so only its side effects, if it has any, matter".
- **Figure 2** chose "choose the second" or "choose the third" for `?:` by
  looking for ": true" in the step's English title. It now reads the first
  operand's value, as the "counts as true" label beside it does.
- **The Annex C gloss** read as a complete list of sequence points but
  left some out. It now lists them all: each call after its arguments, the
  first operands of `&&`, `||`, `?:` and the comma operator, the end of
  each full declarator and full expression, and the library's (before a
  function returns, after each printf or scanf conversion, around a qsort
  or bsearch comparison).

The search the brief asked for, of every narration and refusal sentence
about sequence points or evaluation order, against C99 §5.1.2.3, §6.5p2-3
and Annex C, found one more overstatement: the `&&` and `||` test said the
left operand's effects are complete "before anything else happens". In
`g() + (a && b)`, `g()` may run first; the sentence now says "before
anything after it is evaluated". The rest hold: the full-expression
sentences, the call's "in an order C leaves unspecified … a sequence point
follows", the unspecified order of `+`'s and `=`'s operands, and the
sequence-point note.

#### Checks

- **The notes:** the 14 existing lessons' engine dumps are byte-identical
  to `origin/main`'s; none of their notes made one of the false claims
  (Unsigned's `k-- > 0` is an update whose next sequence point is the end
  of the condition, and nothing else touches `k`). Bits is identical to
  the review's dump. Short-circuit evaluation differs in 5 notes, the
  explanation of each `&&` test: "before anything else happens" became
  "before anything after it is evaluated". Nothing else in either lesson
  changed.
- **The corpus:** all 32 programs give the same result as before (30 run
  to GCC's result, the 2 with function pointers are refused), with the
  same step counts.
- **The probes:** of the review's 82 programs and my 27, every one the
  machine runs gives GCC 14's output and exit status, and every step's
  "what comes next" named the step that followed it. Against the review's
  dumps, 8 programs changed outcome: the 6 above now run, and `i++ < f()`
  and `*pp++` are refused with their new messages.
- **Tests:** as `pages.yml` runs them, the engine's tests (132, 3 of them
  new and 4 updated for the wider rule and the new messages) and its
  clippy pass, as do the site's tests (164, and the prerender's 2), the
  WebAssembly stepper's (79) and the site's four clippy runs; the worker
  run prints the engine's existing unfulfilled lint expectation in
  `src/abstract_machine/parser.rs`, as before. `cargo +nightly fmt
  --check` passes for the engine and the site, and `git diff --check` is
  clean.

## Part 14: pointer idioms, assignments as operands and pointer differences (2026-11)

Part 13 left the machine refusing the C a beginner meets right after
arrays: `while (*s++)`, `a[i++] = x`, `*p++ = c`, `x = y = 0`, `while ((c =
next()) != 0)`, `p - q` and `p < q`. Each needs the machine to know which
object an lvalue reaches, since C's rule for side effects (C99 §6.5p2) is
about objects, not names: `*p = i++` is fine when `p` points elsewhere and
undefined when it points to `i`. This part makes the machine decide that on
the objects its accesses actually reach, runs every case that is defined,
stops every case that is undefined with what went wrong, and stops too
where C leaves the order open and the order would decide the result. It
adds a lesson on walking a string with a pointer, and clears the small
things earlier reviews left. The work is on `agent/machine-subset-2`.

### Method

- **The corpus.** Seventeen programs join the 32 of Part 13 in
  [`corpus/`](corpus/): five that walk arrays and strings with `*p++`
  (`w…`), four with an assignment or increment as an operand (`a…`), three
  with pointer differences and orders (`d…`), three whose accesses C leaves
  undefined (`u…`) and two whose result depends on an order C leaves open
  (`o…`). The corpus test now expects each `u…` program to stop as undefined
  and each `o…` program to stop for its order.
  [`corpus/results-part14-before.txt`](corpus/results-part14-before.txt) and
  [`corpus/results-part14-after.txt`](corpus/results-part14-after.txt) are
  the engine's results on `main` and on this branch.
- **The probes.** The 156 probe programs of the two reviews of Part 13, and
  95 of my own (60 for the new constructs, 35 that try to slip an aliasing
  clash past the check: two pointers to one object, a pointer to a variable
  the expression also updates by name, `**pp` reaching `p`, a call writing
  what a sibling operand reads, a sequence point inside one operand and not
  the other). Each was stepped to its end by a throwaway engine test that
  checks every step's "what comes next" against the step that followed, and
  every program the engine runs was compiled with GCC 14 (`gcc -B …/gcc14…
  -std=c99`) and run.
- **The notes.** The same throwaway test dumped every step of every lesson
  (title, lead, explanation, why it matters, citations, what comes next,
  whether a sequence point follows) on `main` and on the branch.
- **The builds.** The before build is the deployed site,
  <https://bcc.benbwall.com/>, which is `main` at `4bebb980f` (Part 13
  merged). The after build is this branch, built with `web/build.sh`
  without `--release`: the dev profile at `opt-level=s`, as in Parts 7 to
  13.
- **The browser.** Playwright drove Chromium with reduced motion, one
  browser at a time, at 1440x900, 1366x768, 1024x768 and 390x844 in both
  themes, on both builds. Each program was loaded through the page's own
  Open .c and run with Run edited code, then stepped to its scene, recording
  the note, Figure 2's outline for screen readers, the contrast of the
  note's title, lead and fault text against their backgrounds, sideways
  scroll and console errors. The new lesson was also opened from Table 1 and
  shown in the compiler pipeline and WebAssembly views.
- **The lesson walk.** Part 9's walk, every note of every machine lesson,
  on both builds at 1440x900 light and 390x844 dark.

The screenshots are in [`screenshots/part14/`](screenshots/part14/), named
`<scene>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours.

### The corpus, before and after

| Construct | Program | Before: where the engine refused it | After |
| --- | --- | --- | --- |
| `*p++` | [`w01_strlen.c`](corpus/w01_strlen.c) | 4:13: "this machine does not track which objects *, [], -> and calls reach" | runs (93 steps), returns 0 and prints `5` |
| `*p++` | [`w02_strcpy.c`](corpus/w02_strcpy.c) | 7:18: expected `)` | runs (98 steps), returns 0 and prints `copy 5` |
| `*p++` | [`w03_sum_postfix.c`](corpus/w03_sum_postfix.c) | 5:21: unsupported operand types (`p < v + 3`) | runs (77 steps), returns 15 |
| `*p++` | [`w04_store_and_move.c`](corpus/w04_store_and_move.c) | 4:5: "unparenthesized *p++ … is outside this subset" | runs (40 steps), returns 123 |
| `*p++` | [`w05_reverse_in_place.c`](corpus/w05_reverse_in_place.c) | 5:19: unsupported operand types (`lo < hi`) | runs (116 steps), returns 4321 |
| Assignment as operand | [`a01_chained.c`](corpus/a01_chained.c) | 4:11: expected `;` | runs (14 steps), returns 14 |
| Assignment as operand | [`a02_postfix_value.c`](corpus/a02_postfix_value.c) | 4:10: expected `;` | runs (21 steps), returns 56 |
| Assignment as operand | [`a03_assign_in_condition.c`](corpus/a03_assign_in_condition.c) | 5:25: expected `]` (`text[position++]`) | runs (114 steps), returns 3 and prints `abc` |
| Assignment as operand | [`a04_if_assign.c`](corpus/a04_if_assign.c) | 4:12: expected `)` | runs (19 steps), returns 4 |
| Pointer difference | [`d01_difference.c`](corpus/d01_difference.c) | 5:23: unsupported operand types | runs (48 steps), returns 32 |
| Pointer order | [`d02_order_loop.c`](corpus/d02_order_loop.c) | 5:26: unsupported operand types | runs (101 steps), returns 62 |
| Both | [`d03_count_and_reverse.c`](corpus/d03_count_and_reverse.c) | 7:33: unsupported operand types | runs (129 steps), returns 0 and prints `4` and `dcba` |
| Undefined | [`u01_alias_increment.c`](corpus/u01_alias_increment.c) | 4:11: expected `;` | stops at step 16: unsequenced accesses to `i` |
| Undefined | [`u02_alias_subscript.c`](corpus/u02_alias_subscript.c) | 4:15: expected `]` | stops at step 16: unsequenced accesses to `i` |
| Undefined | [`u03_difference_of_objects.c`](corpus/u03_difference_of_objects.c) | 6:17: unsupported operand types | stops at step 24: subtracting unrelated pointers |
| Open order | [`o01_printf_arguments.c`](corpus/o01_printf_arguments.c) | 3:25: "expected a subset expression" | stops at step 15: two calls to `next()` |
| Open order | [`o02_global_and_call.c`](corpus/o02_global_and_call.c) | **ran (17 steps) and returned 1; GCC 14 returns 2** | stops at step 15: `bump()` and `g` |

On `main` sixteen of the seventeen were refused before their first step,
and the seventeenth ran to an answer GCC does not give: `g + bump()`, with
`bump` incrementing `g`, returned 1, from reading `g` first, where the code
GCC makes reads `g` after the call and returns 2. On the branch the twelve defined programs
run to GCC's output and exit status, the three undefined ones stop where
the behavior becomes undefined, and the two whose result an open order
decides stop there. The 32 programs of Part 13 give the same results, step
for step.

### What changed

#### 1. The machine checks C's sequencing rule on the objects it reaches

C99 §6.5p2 lets an expression store into an object at most once between
two sequence points, and read it then only to compute the value stored.
Which object `*p`, `a[i]` or `s->m` reaches is known only as the program
runs, so a check over names, like Part 13's, could only refuse them. The
new [`sequencing.rs`](../../src/abstract_machine/sequencing.rs) logs each
read and store of a full expression with the expression-tree node that made
it, and decides from the tree alone whether a sequence point separates two
of them: the first operand of `&&`, `||` and `?:` and the left operand of a
comma operator are complete before the rest of their operator; a call's
arguments are complete before its body runs, and its body before its value
is used; an operator's operands are evaluated before it computes its value,
but a store nested in an operand is a side effect, which C completes only by
the next sequence point; every other pair of operands is unsequenced. The
machine stops at the first access that meets an earlier one to the same
object with nothing between them, one of the two a store:

- `*p = i++` with `p == &i`: "`*p = i++` stores into `i`, and `i++` stores
  into it too, with no sequence point between the two";
- `return a[i++];` with `a == &i`: "`a[i++]` reads `i`, and `i++` stores
  into it too";
- `(*p)++ + (*q)++` with `p == q`, `*pp = p++` with `pp == &p`, `a[i] =
  a[j]++` with `i == j`, and the old exchange trick `*a ^= *b ^= *a ^= *b`.

Each stop is titled "Undefined behavior: unsequenced accesses" and cites
§6.5p2. The same programs with the pointers aimed elsewhere run to GCC's
result. Where a variable is named twice with nothing between, as in `i =
i++`, `a[i] = i++` or `x += x++`, the check over names still refuses the
program before its first step; it now lets a sequence point inside the
right operand order an update before the store, so `i = (i++, i + 10)` and
`x = id(x++)` run (12 and 2, as GCC gives).

One case C99's wording does not settle stops as a limit of the model, not
as undefined behavior: a store that reads what it stores into to find where
to store, as `a[a[0]] = 1` does when `a[0]` is 0.

[`unsequenced-1366-dark`](screenshots/part14/unsequenced-1366-dark-after.png)
([before](screenshots/part14/unsequenced-1366-dark-before.png);
[`unsequenced-390-dark`](screenshots/part14/unsequenced-390-dark-after.png)).

#### 2. `*s++`, `a[i++]` and `*p++ = x` run

With the check on objects, an update no longer needs to stand alone. `while
(*s++)`, `while (v[i++] < 1)`, `*p++ = 1;`, `total += *p++` and the
two-pointer reversal `*lo++ = *hi; *hi-- = t;` all run. The update's step
says what it yields and when its store completes, as in a condition before,
and now says how the clash is ruled out: "Which object a `*`, `[]`, `->` or
call reaches is known only as the program runs, so this machine compares
every access in the condition that no sequence point separates from this
update with the object it actually reaches". The `*` that follows it says
that `*to++` is `*(to++)`: "`to++` yields a pointer to word[0], the value
`to` held before its update: `++` binds tighter than `*`, so `*to++` is
`*(to++)`, and the `*` uses what `to++` yields", citing §6.5.2.4p2 beside
§6.5.3.2p4. `*p++;` as a statement runs too, where Part 13 refused it.
[`walk-1440-light`](screenshots/part14/walk-1440-light-after.png)
([before](screenshots/part14/walk-1440-light-before.png);
[`walk-390-dark`](screenshots/part14/walk-390-dark-after.png)).

#### 3. Assignments as operands

`=` and the compound assignments are now operators of the expression
grammar, grouping from the right below `?:`, so `x = y = 0`, `a = b++`,
`int y = x += 1;`, `f(x = 3)`, `return g = v;`, `while ((c = next()) != 0)`
and `if ((n = half(9)) > 0)` run. As a whole statement an assignment is the
same store as before, so no lesson's steps change; nested in another
expression its store leaves the value stored for the expression around it,
and its step says so: "The assignment stores 4 into `n`, replacing ?. As an
operand, `(n = half(9))` yields that value, 4, to the condition", then in
the explanation that an assignment is no lvalue, that its store completes
by the next sequence point (the end of the condition, of the full
expression, of `&&`'s first operand, or the point just before a call), and
how a clash is ruled out, citing §6.5.16p3. An update or assignment in an
initializer list is refused, since C99 leaves the order of their side
effects unspecified (§6.7.8p23).
[`copy-1366-light`](screenshots/part14/copy-1366-light-after.png)
([before](screenshots/part14/copy-1366-light-before.png); [Figure
2](screenshots/part14/copy-schematic-1366-light-after.png)).

#### 4. An order C leaves open stops the run

A call's body runs before or after the other operands as a whole (C99
§6.5p3, §6.5.2.2p10). When the call and another unsequenced operand use the
same object and one of them changes it, each order gives another result,
and compilers differ: GCC 14 prints `2 1` for `printf("%d %d\n", next(),
next())` and returns 2 for `g + bump()` where `bump` increments `g`, where
`main`'s machine, evaluating left to right, gave 1. Every call now records
what its body read and stored outside itself, library calls a write to
standard output and reads of the strings they take; when a sibling operand,
or another call, conflicts with it, the machine stops: "Stopped: the result
depends on an open order", with "the call `bump()` stores into `g`, and
another operand, `g`, reads it; C99 leaves unspecified which of the two is
evaluated first, and the result depends on it, so compilers may differ:
this machine stops rather than choose an order for you", and an explanation
that suggests storing the call's result in a variable first. Calls that only
read, calls on different objects, and a call whose value an assignment
stores (`g = bump()`) run.
[`order-390-light`](screenshots/part14/order-390-light-after.png)
([before](screenshots/part14/order-390-light-before.png)).

#### 5. Pointer subtraction and ordering

`p - q` of two pointers into one array counts the elements between them, a
`ptrdiff_t` (`long` on x86-64): "Pointer difference: q - p = 3", with "`q -
p` counts the elements from the right pointer's place to the left's in `v`:
4 - 1 = 3, a ptrdiff_t", citing §6.5.6p9, and a note of its own about
counting elements, not bytes. `p < q`, `<=`, `>` and `>=` compare pointers
into one array by subscript, one past the end counting as the length, and
pointers to members of one structure by declaration (§6.5.8p5). Pointers to
different types are a constraint violation (§6.5.6p3, §6.5.8p2); pointers
into different arrays, different objects, or a null pointer stop as
undefined: "Undefined behavior: subtracting unrelated pointers" or
"ordering unrelated pointers".
[`difference-1024-dark`](screenshots/part14/difference-1024-dark-after.png)
([before](screenshots/part14/difference-1024-dark-before.png)).

#### 6. A lesson: "Walking a string with a pointer"

The new lesson follows "Pointers and arrays" in Table 1's part on memory.
It copies `"walk"` into a char array with the loop every C text shows,

```c
while ((*to++ = *from++) != '\0')
    ;
```

where `*to++` follows the old value of `to` while `to` moves on, and the
assignment hands the char it stored to the comparison, so the copied null
character ends the loop. `length` then walks the copy with `while (*s)
s++;` and returns `s - start`, a `ptrdiff_t` that the return converts to
int. It is the first lesson in which an update and an assignment are
operands, and the first with a pointer difference: 150 steps, ending
`walk has 4 chars`, as GCC prints. Its comments stand on lines of their own
and take at most 36 columns. Its LLVM IR comes from
`web/pipeline/generate.sh` with a dev-profile bcc and its module from
`web/wasm/generate.sh` with wasi-sdk 34; both tools reproduced every
unchanged lesson byte for byte.
[`lesson-strings-1366-dark`](screenshots/part14/lesson-strings-1366-dark-after.png)
(its [pointer
difference](screenshots/part14/lesson-strings-difference-390-dark-after.png),
in the [compiler pipeline](screenshots/part14/lesson-strings-compiler-1366-dark-after.png)
and the [WebAssembly view](screenshots/part14/lesson-strings-wasm-1366-dark-after.png)).

#### 7. Small things from earlier reviews

- **Limits of this model** (§3 of "How it works") now lists subtracting and
  ordering pointers and assignments, compound assignments, `++` and `--`
  anywhere in an expression except an initializer list, and a new paragraph
  says what the machine does with C's sequencing rules: it checks every read
  and store against the object it reaches, stops where two of them clash as
  undefined behavior, refuses before the first step where a variable named
  twice clashes, and stops on a store that reads where it stores. The
  paragraph on evaluation order says that the machine stops where the order
  would decide the result. The limits test runs a program for each claim and
  stops or refuses each counterexample.
  [`limits-1440-light`](screenshots/part14/limits-1440-light-after.png)
  ([before](screenshots/part14/limits-1440-light-before.png)).
- **The Annex C gloss** lists every formatted input/output function of
  7.19.6 and 7.24.2 for the sequence point after each conversion
  specifier, and the sequence points just before and after each call qsort
  or bsearch makes to its comparison function and between that call and
  any movement of the objects passed to it (7.20.5).
- **Long trailing comments.** The constants, floating point, preprocessor,
  pointers, promotion and unsigned lessons put each trailing comment on the
  line above the code it describes, at most 36 columns wide, so none wraps
  into a second row without its `//`. Every lesson's notes are unchanged;
  the preprocessor lesson's refusal, a line splice, moves from line 8 to
  line 11. The six lessons' IR line tables and modules were regenerated; the
  IR itself is unchanged.
  [`comments-1366-light`](screenshots/part14/comments-1366-light-after.png)
  ([before](screenshots/part14/comments-1366-light-before.png)).
- **Escape** now hides a landing word's definition that a tap on its text
  kept open, wherever the focus is.
- **The return's lead** said "its parameters and automatic locals cease to
  exist" for four or more objects even when they were all parameters; it
  now names the kind it ended.
- **Shadowed names** no longer repeat: a return that ends a parameter `x`
  and a block's `x` says "the two objects named `x` cease to exist".

### Checks

- **Correctness against GCC 14.** Of the 251 probe programs (the reviews'
  156 and my 95), the 154 the engine runs to the end give GCC 14's output
  and exit status, every one. Of the rest, 20 of mine stop as undefined
  behavior, each a true clash on one object (`*p = i++` with `p == &i`,
  `(*p = 1) + x` with `p == &x`, two pointers to one `x` both
  incremented, `**pp = (*p)++`, `a[i] = a[j]++` with `i == j`, an ordering
  of pointers into two arrays, …); nine stop because an open order would
  decide the result, each of which GCC 14 resolves differently from a
  left-to-right reading or could; two stop on a store that reads where it
  stores; and the source check refuses nine, all undefined (`i = i++`, `a[i]
  = i++`, `x += x++`, `i = ++i + 1`, `p = p++ + 1`, `x = (x = 1) + 1`,
  `printf("%d %d\n", x, x++)`, `a[i++] = i * 10`) or a constraint violation
  (subtracting an `int *` and a `char *`). Against `main`, the reviews'
  probes changed outcome only where this part means them to: 18 that were
  refused now run to GCC's result (10), stop as undefined (5) or stop for
  an open order (3); 2 are still refused, now because they name a variable
  twice (`a[i] = i++`, `i += i++`) rather than for their syntax; and 4 that
  `main` ran left to right now stop for their order: `g + f1()` and `x +
  set(&x)`, which gave 1 where GCC gives 2 and 8, and `f1() + f2()`
  writing one global and `printf("a") + printf("b")`, where GCC happened to
  agree.
- **Descriptions.** Every step of every probe, corpus program and lesson
  named, in its "what comes next", the step that followed it; the engine
  tests check this for every program they run.
- **The notes.** The engine's dump of every lesson is byte for byte the
  same on `main` and on the branch, apart from the new lesson and the
  preprocessor lesson's refusal, whose position moves with its comments.
  The browser walk at 1440x900 light and 390x844 dark compares the
  title, lead, value flow, "why it matters" note and next step of each of
  the fourteen machine lessons' 1,406 notes: none changed, at either size;
  the new lesson walks 150 notes. No sideways scroll and no page error in
  either walk, on either build.
- **The scenes.** At the four sizes in both themes, every scene reached its
  step on the after build, where the before build refused the program
  before its first step, except `o02`, which ran to its left-to-right
  answer. No sideways scroll, no console error or warning. Figure 2 wires a
  nested `=` like any operator, its value flowing to `!=`, and its outline
  for screen readers reads "=: 99".
- **Contrast:** the new stops' titles and leads use the existing note
  colours (15.82:1 light, 13.34:1 dark); their callouts measure 11.52:1 in
  the light theme and 13.34:1 in the dark one.
- **Keyboard, screen readers and motion.** The change adds no control: the
  new lesson is a card like the others, a button that Enter opens. Escape
  now hides a landing word's definition kept open by a tap on its text
  (checked at 390x844 with touch: shown, kept, hidden on Escape; on the
  before build it stayed). The new note text is plain text in the existing
  live region, and every run used reduced motion.
- **Tests:** as `pages.yml` runs them, the engine's tests (137: 5 new,
  and 8 updated for what now runs, among them the corpus test and the
  limits test) and its clippy pass, as do the site's tests (164, one
  updated, and the prerender's 2), the WebAssembly stepper's (79) and the
  site's four clippy runs; the worker run prints the engine's existing
  unfulfilled lint expectation in `src/abstract_machine/parser.rs`, as
  before. `rustfmt +nightly --check` passes for every engine file; in the
  site's files every changed line is formatted (the files themselves are
  not, on `main` either). `git diff --check` is clean.

### Left for later

- **Function pointers** stay outside the subset. Beside the declarator
  grammar (`int (*f)(int, int)`, `int (*ops[2])(int, int)`) and calls
  through a pointer, they need a value that designates a function in the
  wire format the page reads, and a way for the tape and the cells to draw
  a pointer whose target is code rather than an object; that is a change to
  the page's drawing as much as to the engine, and it did not fit this part.
- **Constant expressions in static initializers** (`static int mask = 1 <<
  3;`) still take only a constant: the machine would need to evaluate them
  before `main`, with C's rules for constant expressions (§6.6), and narrate
  that evaluation in the static object's single step.
- **An initializer list's expressions** may not update objects, and a list
  element that reads an element initialized earlier in the same list (`int
  a[2] = {1, a[0]};`) reads what the machine's order stored, where C leaves
  the order of initialization and evaluation unspecified (§6.7.8p23,
  footnote 133). Calls in a list are checked against one another for an open
  order.
- **A store that reads where it stores** (`a[a[0]] = 1` when `a[0]` is 0)
  stops as a limit of the model. C11 orders the operands' value computations
  before the store and so defines it; C99's wording ("the prior value shall
  be read only to determine the value to be stored") does not settle it.
- **A loop's empty body**, `while (...) ;`, still shows its implicit block
  being entered and left on each pass, as every substatement's block is.
- **The lessons' opening comments** still take a line of up to 75 columns,
  which wraps into a second row in the listing; only trailing comments were
  moved.

### Review, round 1

The review of the branch at `5970b2dc9` found three blockers, two of them
false statements about C and one a hole in the sequencing check, and six
small things. Its 161 probe programs (in its scratch directory, with every
step of each dumped) are the base for the checks below, with 292 more of
my own: 270 that put each of the ten compound assignment operators against
an update of its left operand hidden behind a comma, `&&`, `||`, `?:`, a
call's argument, a nested assignment or a pointer alias, with the defined
neighbours of each, and 22 for the member stores of the third blocker.
Every program the engine runs was compiled with GCC 14 (`-std=c99 -Wall
-Wsequence-point`) and run; GCC's `-Wsequence-point` and Clang 23's
`-Wunsequenced` were read as hints only.

#### 1. A compound assignment ran undefined programs

`x += (x++, 5)` ran and returned 7, as did `x += (x++ && 1)`, `x -= x++ ?
1 : 2`, `x += id(x++)` and, through a pointer, `*p += (x++, 5)` with `p ==
&x`. C99 §6.5.16.2p3 (p. 93, PDF p. 105) makes `E1 op= E2` the same as
`E1 = E1 op (E2)` except that the lvalue `E1` is evaluated once, so the
compound assignment's read of `E1` is an operand of `op`, and §6.5p3
leaves the order of `op`'s operands unspecified: the read may come before
`E2` starts, with the `x++` and no sequence point between them, which
§6.5p2 makes undefined. The comma inside `E2` orders only `E2`'s own
parts. The machine took the read as part of the operator, after `E2`, so
the sequence point seemed to order the update before it; its own step
said `x += (x++, 5)` means `x = x + ((x++, 5))`, which it refused.

**Fixed, in both checks.** The check over names refuses a compound
assignment whose right operand updates the variable on its left by name,
behind a sequence point or not: "this += reads `x` and its right operand
updates `x`: `E1 += E2` means `E1 = E1 + (E2)` with E1 evaluated once
(§6.5.16.2p3), and C99 leaves the order of +'s operands unspecified
(§6.5p3), so the read may come before that update with no sequence point
between them, which C99 makes undefined (§6.5p2)". The run-time check
treats a store inside a compound assignment's right operand as
unsequenced with the compound assignment's own access, whatever sequence
points lie between, so `*p += (x++, 5)` with `p == &x` stops at the
compound assignment as "Undefined behavior: unsequenced accesses". What
stays defined still runs: a sequence point inside the left operand
(`*(x++, &x) += 1`, 14 in the probe), an update of another object
(`x += (y++, 2)`, `*p += id(y++)`), a read of the left operand's object
on the right (`x *= (x, 3)`, `x += id(x)`), and a call that touches the
object only in its body (`g += bump()`, 11 as GCC, Clang and tcc give;
see "Left for later"). (Round 2 makes it stop.) `++` and `--` are
unaffected: their operand is the object itself.

Of my 270 compound probes, the 80 that update the variable by name are
refused (70 of them ran before), the 100 that reach it through a pointer,
subscript or member stop as undefined (all 100 ran before), and the 90
defined ones run to GCC's output and exit status. GCC warns on all 80 by
name and on 30 of the 100 by alias; it warns on none of the 90.

#### 2. The open-order stop said the order changes the result

The stop for a call and another operand that share an object said "the
two orders give different results here", its diagnostic "the result
depends on it", and the limits text "the order decides the result". For
`inc() + inc()` both orders give 3, and `printf("a") + printf("a")` prints
`aa` either way. C99 leaves the order unspecified (§6.5p3; §6.5.2.2p10,
p. 72, PDF p. 84), and the machine cannot cheaply tell whether the two
orders agree, so stopping stays the teaching choice; the claim was what
was wrong.

**Fixed.** The title is "Stopped: the result can depend on an open order";
the lead says "A compiler may run the call before or after the other
operand, and the result can depend on which runs first, so the machine
stops rather than pick one"; the diagnostic says "the result can depend on
it, so compilers may differ"; and the limits text says "the order can
decide the result, so the engine stops wherever that could happen rather
than choose". The stop still calls the order unspecified, not undefined.

#### 3. A defined member store was called undefined

`s.a = (s = t, 5)` was refused before its first step: "this expression
updates `s` and also reads or updates it with no sequence point between
the two; C99 makes that undefined". The machine runs the same shape with
plain variables, `i = (i = 3, i + 10)`, and with a member on both sides.

What C99 says: §6.5p2 lets an object be modified at most once between two
sequence points. `s = t` modifies `s`, and so `s.a`, before the comma's
sequence point; the outer assignment modifies `s.a` after it, since its
store (§6.5.16p3, p. 91, PDF p. 103) cannot come before the value it
stores, which exists only once the comma has completed. The left operand
`s.a` designates a member of `s` without reading it (§6.5.2.3p3, p. 73,
PDF p. 85). That is the reading the machine already takes for `i = (i++,
i + 10)`, and C11 states it outright: the store "is sequenced after the
value computations of the left and right operands" (C11 §6.5.16p3, p.
101, PDF p. 119). So the member form is defined exactly when the scalar
form is.

**Fixed.** The check over names treats an assignment to `s.m`, `s.in.m`
and so on as it treats one to `s`: an update of `s` that the right operand
completes at a sequence point of its own comes before the store, while
one with no sequence point between is still refused, as is the same
update under a compound assignment (`s.a += (s = t, 5)`, by item 1) or
beside a read (`s.a = (s = t, 5) + s.a`). `s.a = (s = t, 5)` now runs to
GCC's 54, and so do five refused variants (a nested member, a call's
argument, `&&`, `?:`, and a read of `s.a` after the comma); `ps->a = (s =
t, 5)` and `s.in.a = (s.in = t.in, 6)` ran already. GCC 14 prints "may be
undefined" for these struct forms but not for the scalar ones; Clang 23
warns on neither.

#### 4. Smaller things

- **`(s = t).a`** was refused as if it stored into an assignment's value.
  It reads a member of a non-lvalue structure, which C allows
  (§6.5.2.3p3). The refusal now says what is missing: "a member of an
  assignment's value, as in (s = t).a, is outside this subset; assign
  first, then read the member of the structure assigned to", and the
  limits text excepts it from "anywhere in an expression".
- **An initializer list's open order** named `g` in `int a[2] = {inc(),
  g};` "another operand" and cited §6.5p3. It now says "another
  initializer", and the stop cites §6.7.8p23 (p. 128, PDF p. 140): "The
  order in which any side effects occur among the initialization list
  expressions is unspecified".
- **Output before the stop.** The open-order stop is found once both
  calls have run, so `printf("a") + printf("a")` shows `a` first. Its
  explanation now says so: "It finds the clash only once both have run,
  so what the first of them did, such as printing, has already happened
  here."
- **The new lesson's comments** read "*p++ reads a char and moves p on"
  (the side effect of `p++` may come before the read, so "then" was
  wrong) and "Copy, including the '\0'". Line numbers are unchanged; the
  lesson's module was rebuilt with wasi-sdk 34, which first reproduced
  every committed module byte for byte, and only `strings.wasm` changed.
- **A block without braces** was narrated as "a { … } compound statement".
  A loop body, an if branch or a for statement is a block whether or not
  braces are written (§6.8.4p3, p. 133, PDF p. 145; §6.8.5p5, p. 135, PDF
  p. 147), and the step now says so: "Execution enters a block with no
  braces: a loop body is a block of its own, a new scope", citing the
  paragraph. A for statement's objects begin their lifetime "as the for
  statement begins" rather than "at this brace".
- **A quote over several lines** kept its line break and indentation, as
  in step 148 of the new lesson. Quoted code now reads as one line, as
  "what comes next" already did.

#### What changed in the results

- **The review's 161 probes:** 126 are byte for byte the same. Ten change
  outcome as item 1 means: `p4/f06` and `p5/g01`, `g03` to `g06` and `g10`
  are refused, and `p5/g02`, `g07` and `g08` stop as undefined. Two run
  that were refused (item 3): `p4/f04` and `p2/d10`, both 54 as GCC
  gives. Three refusals of `(s = t).a` change wording (`p2/u09b`,
  `u09d`, `p4/f08`). The 13 open-order stops change wording (item 2;
  `x47` now names an initializer and cites §6.7.8p23). Seven change only
  in the narration of item 4 (braceless blocks and one-line quotes).
- **The corpus:** every program has the same outcome and step count.
  [`corpus/results-part14-after.txt`](corpus/results-part14-after.txt)
  changes only in the wording of `o01` and `o02`; eight programs' steps
  change only in the narration of item 4 (braceless blocks, and in `p04` a
  for statement's lifetime sentence).
- **The lessons:** every lesson has the same steps, values and outcome.
  Eight lessons' notes change, and only these: the new lesson's eight
  braceless blocks and its `printf` quote; the `printf` quote spread over
  lines in the bits and logic lessons; and a for statement's lifetime
  sentence in the constants, floating point, objects, overflow and
  unsigned lessons.
- **"What comes next"** named the step that followed in every step of
  every probe, corpus program and lesson.

#### Checks

- **Engine tests:** 140 pass (three new: compound reads against the right
  operand, member stores after a sequenced update, and the small fixes;
  the open-order test now checks the reworded stop, `inc() + inc()` and
  the initializer list), and the engine's clippy passes, as `pages.yml`
  runs them.
- **Site:** as `pages.yml` runs them, the site's tests (164, and the
  prerender's 2), the WebAssembly stepper's (79) and the site's four
  clippy runs pass; the worker run prints the engine's existing
  unfulfilled lint expectation in `src/abstract_machine/parser.rs`, as
  before. CI builds the site; I did not build it locally.
- **Formatting:** `rustfmt +nightly --check` passes for every engine file,
  and `git diff --check` is clean.
- **Browser:** not rerun. The page code is unchanged apart from two
  sentences of the limits text; the new wording reaches the page through
  the engine's notes, which the dumps above cover.

#### Left for later

- **A compound assignment and a call** that changes its left operand, as
  in `g += bump()`, runs, reading `g` after the call as GCC and Clang do.
  Only C99 leaves this open: the read of `g` is an operand of `+`
  (§6.5.16.2p3), whose order with the call §6.5p3 leaves unspecified, so
  the read may come before the call too. C11 settles it: §6.5.16.2p3 makes
  the compound assignment "a single evaluation" with respect to the call,
  and its store follows `E2`'s value, call included (§6.5.16p3), so the
  read follows the call as well and the result is fixed. The machine does
  not stop there as it does for `g + bump()`. (Round 2 makes it stop.)
- **A member of a call's or an assignment's value**, `f().a` or `(s =
  t).a`, is still outside the subset.

### Review, round 2

The re-check of the branch at `da6231a0c` found the three blockers of round
1 fixed, and two new ones, both about what the machine says rather than
what it computes, and one small thing. Its 99 new probe programs (sets
`q1` to `q7`, with GCC 14's and Clang 23's output beside them) and the 17
lessons were dumped again before and after the fixes below, together with
round 1's 453 probes, its 49 corpus programs and its 17 lessons: 635 runs
in all.

#### 1. A compound assignment's refusal called defined programs undefined

Round 1's refusal of a compound assignment whose right operand updates its
left operand's variable ended "the read may come before that update with
no sequence point between them, which C99 makes undefined (§6.5p2)". In
`x += (0 && x++)`, and in `x += (c ? 1 : x++)` with `c` 1, the update is
never evaluated, so nothing breaks §6.5p2 and the programs are defined;
GCC gives 1 and 2. The refusal of `=` already said C99 makes such a clash
undefined "whenever both are evaluated", and round 1's rewording lost
that.

**Fixed.** The refusal stays, as a limit of the model, but now says
"C99 makes that undefined whenever the update is evaluated (§6.5p2)".

#### 2. The limits text said the engine stops where it ran

Round 1's limits text says that when a call and another operand use the
same object and one of them changes it, "the order can decide the result,
so the engine stops wherever that could happen rather than choose". But
`g += bump()`, where `bump` multiplies `g` by 10 and returns 5, ran to 15
and did not stop. In C99 the compound assignment's read of `g` is an
operand of `+` (§6.5.16.2p3), and §6.5p3 leaves its order with the call
open, so the read may come before the call, giving 6, or after it, giving
15. C11 settles it at 15 (see round 1's "Left for later", corrected
above).

**Fixed in the engine.** The run-time rule that a store in `E2` is
unsequenced with the compound assignment's read of `E1` now covers a
store a call in `E2` makes in its body. Such a run stops at the existing
open-order stop, whose diagnostic now names the compound assignment's
read: "the call `bump()` stores into `g`, and `g += bump()` reads it as
the left operand of its `+` (§6.5.16.2p3); C99 leaves unspecified which
of the two is evaluated first, and the result can depend on it". The
limits text names `g += f()` beside `g + f()`. A call that only reads the
object (`g += get()`), or stores into another one, still runs.

#### 3. Small thing

"What comes next" before leaving a block that ends no lifetimes cited
§6.8.2p2, the compound statement's paragraph, even for a block without
braces. It now cites the paragraph the step that entered the block cites:
§6.8.5p5 for a for statement or a loop body, §6.8.4p3 for an if branch.

#### What changed in the results

- **All 635 runs:** 507 are byte for byte the same.
- **The refusal's wording (item 1):** 108 refusals change only in that
  sentence: 17 in `q1`, the 2 defined programs of `q3`, and 89 of round
  1's probes. No run still says "which C99 makes undefined".
- **The open-order stop (item 2):** 20 programs that ran now stop there,
  and every earlier step of each is unchanged. They are `q4`'s `g +=
  bump()`, `*p += bump()`, `g -= setg(5)`, `gs.a += bumps()`, `x +=
  wr(&x)`, `g += (bump(), 1)`, `g += id(bump())` and `g *= bump()`;
  round 1's `c19` and `g09`; and its ten `d_*_callbody` probes, one per
  compound operator. `q4`'s `g = g + bump()` and `g += bump() + g`
  stopped before and still do, and `g += readg()`, whose call only reads `g`,
  still runs.
- **"What comes next" (item 3):** the dumps record its text, which is
  unchanged, but not its citation. A separate pass over all 635 runs
  found 143 "leave the block" lines. The 75 at a closing brace still cite
  §6.8.2p2. The 68 without braces each cite the paragraph their entry
  step cites: §6.8.5p5 for 67 of them, §6.8.4p3 for one.
- **Values, outcomes and step counts:** the same everywhere else,
  lessons and corpus included.

#### Checks

- **Engine tests:** 140 pass, and the engine's clippy passes, as
  `pages.yml` runs them. The compound-assignment test now checks the
  refusal's caveat on both defined programs, the open-order stop for `g
  += bump()` and `*p += (bump(), 1)`, and that `g += get()` and a call
  that stores into another object still run; the review-fixes test checks
  the citations of "what comes next" before leaving a loop body and an if
  branch without braces.
- **Site:** as `pages.yml` runs them, the site's tests (164, and the
  prerender's 2), the WebAssembly stepper's (79) and the site's four
  clippy runs pass; the worker run prints the engine's existing
  unfulfilled lint expectation in `src/abstract_machine/parser.rs`, as
  before. CI builds the site; I did not build it locally.
- **Formatting:** `cargo +nightly fmt --check` passes, and `git diff
  --check` is clean.
- **C11:** the reading in round 1's corrected "Left for later" was checked
  against N1570 §6.5.16.2p3 (pp. 103-104; PDF pp. 121-122), §6.5.16p3
  (p. 101; PDF p. 119) and §6.5.2.2p10 (p. 82; PDF p. 100).
- **Browser:** not rerun. The page code is unchanged apart from the
  example in one sentence of the limits text; the new wording reaches the
  page through the engine's diagnostics and notes, which the dumps above
  cover.

## Part 15: pointers to functions and constant static initializers (2026-11)

Parts 13 and 14 left two things a beginner meets soon after arrays outside
the machine: pointers to functions, which every C text uses for callbacks
and dispatch tables, and a static initializer that computes its value, as in
`static int mask = 1 << 3;`. This part runs both. A pointer to a function is
a new kind of value, and the page draws it in its own way everywhere a value
appears: the tape, the cells, Table 1, Figure 2, and the listing, where an
arrow from the pointer's name leads to the function it points to. The work is
on `agent/machine-subset-3`.

### Method

- **The corpus.** Nine programs join the 49 of Parts 13 and 14 in
  [`corpus/`](corpus/): five more with pointers to functions (`g03` to `g07`:
  a typedef, a callback, `(*f)(x)`, comparisons with a function and with
  null, and a static dispatch table), two static initializers that compute
  (`s01`, `s02`), and two undefined calls (`u04` through a null pointer,
  `u05` through a pointer cast to the wrong type).
  [`corpus/results-part15-before.txt`](corpus/results-part15-before.txt) and
  [`corpus/results-part15-after.txt`](corpus/results-part15-after.txt) are
  the engine's results on `main` and on this branch.
- **The probes.** 77 programs of my own: 52 for pointers to
  functions (24 that run, 7 undefined, 1 indeterminate read and 20 that C or
  the subset refuses), 20 for constant expressions in static initializers
  and 5 for the open-order stop's naming; and, as a regression set, the 247
  probes of Part 14's second and third reviews. A throwaway engine test
  stepped each to its end, checking every step's "what comes next" against
  the step that followed, and every program the engine runs was compiled
  with GCC 14 (`gcc -B …/gcc14… -std=c99`) and run.
- **The notes.** The same throwaway test dumped every step of every lesson
  (title, lead, explanation, why it matters, citations, what comes next,
  whether a sequence point follows) on `main` and on the branch.
- **The builds.** The before build is the deployed site,
  <https://bcc.benbwall.com/>, which is `main` at `6fb057ac7` (Part 14
  merged). The after build is this branch, built once at the end with
  `web/build.sh` without `--release`: the dev profile at `opt-level=s`, as in
  Parts 7 to 14.
- **The browser.** Playwright drove Chromium with reduced motion, one
  browser at a time, at 1440x900, 1366x768, 1024x768 and 390x844 in both
  themes, on both builds. Each corpus program was loaded through the page's
  own Open .c and run with Run edited code, then stepped to its scene,
  recording the note, the listing's marks, the cells' and Table 1's text
  for screen readers, Figure 2's outline, the contrast of the new visuals
  against their composited backgrounds, running animations, sideways scroll
  and console errors.
- **The lesson walk.** Part 9's walk, every note of every machine lesson,
  on both builds at 1440x900 light and 390x844 dark, comparing each note's
  title, lead, value flow, "why it matters" note and next step.

The screenshots are in [`screenshots/part15/`](screenshots/part15/), named
`<scene>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours.

### The corpus, before and after

| Construct | Program | Before: where the engine refused it | After |
| --- | --- | --- | --- |
| Pointer to a function | [`g01_function_pointer.c`](corpus/g01_function_pointer.c) | 6:9: expected an identifier | runs (18 steps), returns 5 |
| Table of pointers | [`g02_function_table.c`](corpus/g02_function_table.c) | 10:9: expected an identifier | runs (23 steps), returns 5 |
| typedef | [`g03_typedef.c`](corpus/g03_typedef.c) | 1:1: expected an arithmetic type or struct | runs (39 steps), returns 25 |
| Callback | [`g04_callback.c`](corpus/g04_callback.c) | 1:15: expected an identifier | runs (35 steps), returns 40 |
| `(*f)(x)` and `&f` | [`g05_star_call.c`](corpus/g05_star_call.c) | 6:9: expected an identifier | runs (28 steps), returns 42 |
| `==` and null | [`g06_compare_null.c`](corpus/g06_compare_null.c) | 6:9: expected an identifier | runs (49 steps), returns 8 |
| Static table | [`g07_static_table.c`](corpus/g07_static_table.c) | 6:5: expected an identifier | runs (73 steps), returns 0 and prints `11` and `5` |
| Constant expression | [`s01_mask.c`](corpus/s01_mask.c) | 1:21: "a static object's initializer must be a constant … and no arithmetic on it" | runs (7 steps), returns 9 |
| Constant expressions in a list | [`s02_computed_table.c`](corpus/s02_computed_table.c) | 1:18: the same | runs (12 steps), returns 6 |
| Undefined | [`u04_null_call.c`](corpus/u04_null_call.c) | 1:5: expected an identifier | stops at step 6: call through a null pointer |
| Undefined | [`u05_wrong_type.c`](corpus/u05_wrong_type.c) | 1:1: expected an arithmetic type or struct | stops at step 10: call through the wrong function type |

On `main` all eleven were refused before their first step, by the parser
for every pointer to a function (it found no name after `(` in `int (*f)(…)`,
and did not know `typedef`) and by the static initializer's rule for the two
constant expressions. On the branch the nine defined programs run to GCC
14's output and exit status, and the two undefined ones stop where the call
would go wrong. The other 47 programs of Parts 13 and 14 give the same
results, and the engine's dump of every one of their steps is byte for byte
the same.

### What changed

#### 1. A pointer to a function is a value of its own

The engine had one kind of pointer, to an object, drawn as an arrow into the
cells. A pointer to a function points to no object: it records which
function a call runs. It is now a value kind of its own (`Value::Function`
in the engine, `function` in the JSON trace, whose schema is now version
10), with the types that hold one: `int (*f)(int)`, a table `int
(*ops[2])(int, int)`, a pointer to such a pointer `int (**pp)(int)`, and a
parameter `int (*f)(int)`. Table 1 and the cells spell the type as C does,
`int (*)(int)` and `int (*[2])(int, int)`, 8 bytes per pointer.

- **Declaration and typedef.** The declarator grammar takes `(*name)(…)`,
  `(**name)(…)`, `(*name[N])(…)` and `(*name[])(…)` before a list, in
  declarations, parameters, prototypes and type names (`(int (*)(int))` in
  a cast, `sizeof(int (*)(int))`). A file-scope `typedef int (*binop)(int,
  int);` names the type, and `binop op`, `binop ops[4]` and `binop *p` use
  it; a typedef declares no object, so it is no step (C99 §6.7.7p3). A
  parameter written as a function, `int f(int)`, is refused with the
  pointer it adjusts to (§6.7.5.3p8).
- **The conversion.** A function's name used as an operand is a step of its
  own: "`square` becomes a pointer to the function", whose lead says "A
  function's name used as an operand designates the function, and C
  converts it to a pointer to it" and whose explanation says why: "A
  function is code, not an object: it has no value to read and cannot be
  copied, so wherever its name is an operand, other than after & or sizeof,
  C converts it to a pointer to the function", naming the function's type
  and the line that defines it (§6.3.2.1p4). `&square` is one step, "Take
  the address of the function square", which yields the same pointer
  without the conversion (§6.5.3.2p3). A call written `square(3)` makes the
  same conversion; the machine shows it as a step only where the pointer is
  kept or passed on, so no existing lesson's steps change, and the
  explanation says so.
- **The call.** `fp(x)`, `(*fp)(x)`, `ops[i](x)` and `(square)(x)` call
  through a pointer. The call step is titled "Call square through `fp`
  (frame #2)", leads with "`fp` yields a pointer to square, so the call runs
  square in a new frame of its own", and explains how the call finds the
  function: the expression before the parentheses gave a pointer to the
  function, the call runs the function it points to exactly as `square(…)`
  would, in compiled code by jumping to the address where the function's
  machine code starts, and the pointer's type promised a function of the
  type the function has, so the arguments convert to its parameters
  (§6.5.2.2p1, p3). It notes that C leaves the order of evaluating that
  expression and the arguments open and that the machine evaluates it
  first (§6.5.2.2p10). `*fp` is a step, "`*fp` designates the function
  square", that says the designator converts straight back to the same
  pointer, which is why `(*fp)(…)` and `fp(…)` call the same function
  (§6.5.3.2p4).
- **Comparison and null.** `fp == square`, `f != g` and `fp == 0` compare
  ("Compare pointers to functions: f == square → 1", citing §6.5.9p6), a
  null pointer to a function counts as false in a condition and a pointer
  to a function as true, "a pointer to a function, which is never null,
  counts as true" (§6.3.2.3p3). A static pointer starts null.
- **What C refuses.** Each is a constraint violation, refused before the
  first step with what is wrong: storing a pointer to a function of another
  type ("this is an int (*)(int, int), where an int (*)(int) is needed: a
  pointer to a function converts only to a pointer to a function of the
  same type, whose parameters and return type agree", §6.5.16.1p1,
  §6.7.5.3p15), a call with the wrong number of arguments (§6.5.2.2p2),
  calling something that is not a pointer to a function (§6.5.2.2p1),
  arithmetic, ordering, `++` and `--` on one (§6.5.6p2, §6.5.8p2), `==`
  against a pointer of another type (§6.5.9p2), `sizeof` of a function
  (§6.5.3.4p1), and assigning to a function's name.
- **Undefined calls.** A call through a null pointer stops: "Undefined
  behavior: call through a null pointer", with "A null pointer points to no
  function, and C defines a call only of a function, so this call has no
  meaning", citing §6.3.2.3p3. C99 has no sentence that says so; the call
  is undefined because §6.5.2.2p3 defines a call only of the function its
  expression denotes, and §4p2 makes behavior undefined where no definition
  is given, so the stop says "has no meaning" rather than quoting a rule
  that does not exist. `(*fp)(x)` with `fp` null stops at the `*`, which
  §6.5.3.2p4 and its footnote 87 make undefined. A call through a pointer
  that a cast converted to another function type stops as "Undefined
  behavior: call through the wrong function type", naming both types
  ("`f` points to add, a function of type int (int, int), but its own type
  says it points to a function of type int (int)"), citing §6.5.2.2p9; the
  cast itself is a step that says C lets the pointer be converted and back
  again unchanged (§6.3.2.3p8), and casting back before the call runs. An
  uninitialized pointer to a function stops at its read, as any
  indeterminate read does.
- **A note.** New "why it matters" note: a pointer to a function lets a
  program choose at run time which code runs, as a table or a callback such
  as qsort's does, and its type is a promise that a null or mistyped call
  breaks.

[`callback-1366-dark`](screenshots/part15/callback-1366-dark-after.png)
([before](screenshots/part15/callback-1366-dark-before.png)),
[`nullcall-390-dark`](screenshots/part15/nullcall-390-dark-after.png)
([before](screenshots/part15/nullcall-390-dark-before.png)),
[`wrongtype-1024-dark`](screenshots/part15/wrongtype-1024-dark-after.png)
([before](screenshots/part15/wrongtype-1024-dark-before.png)).

#### 2. The page draws a pointer to a function, and links it to the listing

- **The tape and the cells** write the value as `→square()`: the arrow of a
  pointer, and the parentheses of a call, since it leads to code. It is set
  in a colour of its own, the colour of code, with a dotted rule under it in
  the cells, distinct from the object colours and from the orange of a
  write, in both themes. A table's cells each hold one.
- **Table 1** writes the same value, in the same colour, and its title says
  "Points to the function square, defined on line 5".
- **Figure 2** writes the value on the wire as `&square`, as `&` and the
  name would make it, in the colour of code, and the outline screen readers
  hear reads "square: a pointer to the function square". The call's node
  takes the pointer as its first input, so the wire from `f` into `f()`
  shows which function the call will run.
- **The listing** marks each function a live object points to: its name in
  its definition is underlined with the same dotted rule, and before it the
  pointers that lead there are written, `f →` or `ops[0] →` and `ops[1] →`
  on two functions, with a hover text "ops[1] points to the function sub".
  A pointer to an object points into the cells; a pointer to a function now
  points into the code.
- **Screen readers.** Each cell that holds one is read as "f, int (*)(int),
  8 bytes: a pointer to the function square, defined on line 5"; the arrow
  in the listing is drawn by the editor and is not read, so the cell, the
  table and the outline carry the same facts in words.
- **The glossary** defines a function designator (§6.3.2.1p4), a pointer to
  a function (§6.2.5p20) and a constant expression (§6.6p2).

[`table-1440-light`](screenshots/part15/table-1440-light-after.png)
([before](screenshots/part15/table-1440-light-before.png)); [the
cells](screenshots/part15/callback-headrow-1440-light-after.png), [a table's
cells](screenshots/part15/table-headrow-1024-dark-after.png), [Table
1](screenshots/part15/table-datasheet-1440-light-after.png), [the
listing](screenshots/part15/callback-sourcepane-1366-light-after.png), [Figure 2 at
`add`](screenshots/part15/designator-schematic-1024-light-after.png) and [at
`*f`](screenshots/part15/star-schematic-1366-dark-after.png);
[`compare-390-light`](screenshots/part15/compare-390-light-after.png)
([before](screenshots/part15/compare-390-light-before.png)),
[`table-390-dark`](screenshots/part15/table-390-dark-after.png).

#### 3. Constant expressions in static initializers

`static int mask = 1 << 3;`, `int table[] = { 1+1, 2*2 };`, `double third =
1.0 / 3;`, `int big = 10 > 3 ? 100 : 200;`, `char c = 'a' + 2;` and a
static local `static int count = 10 * 10;` run. The engine works each
initializer out while it compiles the program, by the same rules as the
running program (§6.6p11): the integer promotions and usual arithmetic
conversions, unsigned wraparound, casts between arithmetic types, `&&`,
`||` and `?:` evaluating only what they need. The static object's step says
what happened: "Its initializer `1 << 3` is a constant expression: the
compiler works it out while it translates the program, by the same rules
the running program would use, giving 8. The value is fixed before the
program starts, and no step of the run computes it", citing §6.6p2, p8 and
§6.7.8p4, with a new note on why a static initializer must be constant.
A static table of pointers to functions says that each function's name
"is an address constant: a function exists before the program starts, so
a pointer to it is known while the program is translated" (§6.6p9). A
constant as written, `int n = -5;`, is narrated as before.
[`mask-1440-dark`](screenshots/part15/mask-1440-dark-after.png)
([before](screenshots/part15/mask-1440-dark-before.png)).

What is not a constant expression is refused with the reason: a call or an
assignment (§6.6p3), the comma operator, a read of an object, which C99
refuses even for a `const` one (§6.6p8; GCC accepts that as an extension,
which §6.6p10 allows, and the message says so), and a value the type cannot
hold or that has none, such as `2147483647 + 1`, `1 / 0`, `1 << 40` or `-1
<< 2` (§6.6p4). GCC 14 compiles the last four with a warning, which is the
diagnostic the constraint requires; the machine refuses them rather than
show a value C does not define.

#### 4. The minor findings of Part 14's last review

- **N1.** The open-order stop named the object where the call's access
  starts, so a callee that assigns a whole structure was said to store into
  `s.a` when the caller read `s.b`. It now names where the two accesses
  meet: "the call `f()` stores into `s.b`, and another operand, `s.b`,
  reads it", and the whole object when both reach all of it ("the call
  `g()` stores into `s`, and the call `f()` stores into it too"). Of the
  review's 247 probes (its round-2 and round-3 sets), exactly its three
  `name` and `m26` probes change, each now naming `s.b`.
- **N2.** Part 14's round-1 section listed `g += bump()` under what still
  runs; it now carries the same "(Round 2 makes it stop.)" as its "Left for
  later".

#### 5. "Limits of this model"

§3 of "How it works" now lists among what runs: static initializers that
are constant expressions, string literals or the names of functions; and
pointers to functions, declared directly or with a typedef, kept in
variables, tables and parameters, called as `fp(x)` or `(*fp)(x)`, compared
and tested for null. Among what does not: a typedef of anything but a
pointer to a function; a function that returns a pointer to a function, a
structure member that points to one, and a pointer to a library function; a
static pointer initialized to an object's address. The limits test runs a
program for each new claim, stops on both undefined calls, and refuses each
new counterexample.
[`limits-1440-light`](screenshots/part15/limits-1440-light-after.png)
([before](screenshots/part15/limits-1440-light-before.png)).

#### 6. What did not change

- **The lessons' opening comments** still wrap in the narrow listing. Each
  is one or two lines of up to 75 columns; rewrapping them to 36 columns
  adds lines, which moves every later line of the lesson, and with it the
  lesson's precomputed LLVM IR line tables, its WebAssembly module's line
  table and the site tests that pin lesson lines. That is not cheap, so it
  is left for later (below).
- **No new lesson.** "Callbacks: passing a function to a function" would
  teach what no lesson does, but its LLVM IR needs a native bcc build with
  LLVM, which this machine's disk and the build lock did not leave room
  for beside the site build; the corpus's `g04_callback.c` and
  `g07_static_table.c` show the same steps through Open .c meanwhile.

### Checks

- **Correctness against GCC 14.** Every program the engine runs gives GCC
  14's output and exit status: the 24 pointer-to-function probes, the 14
  constant-expression probes, all 51 corpus programs that run, and the 126
  of the 247 review probes that run. Every undefined probe stops at the call
  or `*` that C leaves undefined, with the messages above, where GCC's
  builds crash with SIGSEGV on the null calls and return arbitrary statuses
  (203, 0 and 11) on the mistyped ones.
- **Descriptions.** Every step of every probe, corpus program and lesson
  named, in its "what comes next", the step that followed it; the engine
  tests check this for every program they run, the six new ones included.
- **The notes.** The engine's dump of every lesson is byte for byte the
  same on `main` and on the branch. The browser walk compares 1,571 notes of
  the fifteen machine lessons at each size: none changed in its title,
  lead, value flow, "why it matters" note or next step, at either size, and
  neither build scrolled sideways or raised a page error. The one intended
  change a reader can see in an existing lesson is a new glossary link: the
  words "constant expression" in a static object's explanation now link to
  the glossary's new entry.
- **Regressions.** Of the review's 247 probes, 244 have byte-identical
  dumps; the other three are N1's, which now name `s.b`.
- **The scenes.** At the four sizes in both themes, every scene reached its
  step on the after build, where the before build refused the program
  before its first step. No sideways scroll, no console error or warning,
  no running animation. The listing marked the function each live pointer
  leads to wherever that line was on screen (on a phone the listing
  follows the current line, so the definition is often scrolled away, and
  the cells carry the same facts).
- **Contrast.** The function-pointer value measures, at its lowest across
  sizes and themes, 7.64:1 in the cells, 7.05:1 in Table 1 and on Figure
  2's wires, 10.73:1 on the tape, and 13.56:1 for the listing's label; the
  notes' text keeps 15.82:1 (light) and 13.34:1 (dark).
- **Keyboard, screen readers and motion.** The change adds no control. A
  cell holding a pointer to a function is read as "f, int (*)(int), 8
  bytes: a pointer to the function square, defined on line 5", Table 1's
  value has the same words as its title, and Figure 2's outline reads
  "f: a pointer to the function square". The new styles have no
  transition, and every run used reduced motion.
- **Tests:** as `pages.yml` runs them, the engine's tests (146: six new,
  and eight updated: the limits and corpus tests for what now runs, three
  for the JSON schema's version 10, and three whose refusals now run or
  read differently)
  and its clippy pass, as do the site's tests (167, three new, and the
  prerender's 2), the WebAssembly stepper's (79) and the site's four clippy
  runs. The worker run no longer prints the unfulfilled lint expectation in
  `src/abstract_machine/parser.rs`: the type it was on now has a variant of
  a type's size, so the expectation is gone. `cargo +nightly fmt --check`
  passes for every engine file; in the site's files every changed line is
  formatted. `git diff --check` is clean.

### Left for later

- **A lesson on callbacks**, `apply(int (*f)(int), int x)` and a dispatch
  table, with its LLVM IR from `web/pipeline/generate.sh` and its module
  from `web/wasm/generate.sh`.
- **The lessons' opening comments**: rewrap them to 36 columns and
  regenerate the line tables and modules, as Part 14 did for the trailing
  comments.
- **Pointers to functions in more places**: as structure members (a
  "vtable"), as return values (`int (*pick(int))(int)`), and pointing to
  the library's functions (`puts`), all refused with a message today; a
  typedef of other types.
- **Static pointers to objects**: `static int *p = &x;` and `int *p = a;` at
  file scope are address constants too (§6.6p9), and are still refused.
- **A direct call's conversion**: `square(3)` converts `square` to a pointer
  before calling, which the machine shows only where the pointer is kept or
  passed on, so a learner sees the conversion only in programs with
  pointers to functions.

### Review, round 1

The review of the branch at `e7dcc2267` found four blockers: CI's site
build failed on a stack overflow, two explanations said false things about
C, and valid null pointer constants were refused as constraint violations.
It also listed six small things (N-a to N-f). Its 132 probe programs, in
its scratch directory with every step of each dumped, are the base for the
checks below, with the implementer's 386 (24 constant expressions, 52
pointers to functions, 5 for N1, 247 regression programs and the 58 of the
corpus), the 17 lessons, and 17 new ones of my own for the forms of `*`, the
null pointer constants and the new refusals. My dump harness reproduces the
review's 149 dumps of the head byte for byte. Every program the engine runs
was compiled with GCC 14 (`-std=c99 -pedantic -O0`) and run.

#### 1. The site build overflowed its stack

The engine test `limits_stop_source_compilation_execution_memory_and_recursion`
parses `((…(1)…))` until the parser's nesting limit of 64 stops it. Each
level of parentheses repeats the frames of `expression`, `expression_inner`
and `prefix`, and in a debug build each frame holds every temporary of its
function: on `main` that was 30 KB a level (21.8 KB of it `prefix`), and the
function-pointer branches pushed the whole test past the 2 MiB a test
thread gets. Measured by setting `RUST_MIN_STACK` for that one test on the
same debug test binary, `main` fails at 1850 KiB and passes at 1900 KiB,
and the reviewed head failed at 2000 KiB and passed at 2048 KiB.

**Fixed.** `prefix` now only dispatches: the unary operators, a
parenthesized expression, the primary expressions and the postfix operators
are helpers kept out of line (`#[inline(never)]`), and so is typing a
binary operator, which `expression_inner` did inline. A level now takes 7.3
KB (`prefix` 1.1 KB, `expression_inner` 2.7 KB, `expression` 0.7 KB and the
parenthesized expression's helper 2.9 KB). The test fails at 450 KiB and
passes at 500 KiB, 76% under the default. The test's depth and
the stack size are unchanged. The refactor alone changed none of the 552
dumps.

#### 2. The step for `*` before a function quoted C that is not C

The step was written by stripping one `*` from the source: for `**sq` it
said "`*sq` holds a pointer to the function sq … That is why `(**sq)(…)`
and `*sq(…)` call the same function", but `*sq(…)` applies `*` to what
`sq` returns, a constraint violation (§6.5.3.2p2). `*&sq` gave `&sq(…)`,
the address of a value (§6.5.3.2p1), and `*sq` said a function "holds" a
pointer, though a function is no object.

**Fixed, from the expression's structure.** The compiler records how the
operand of `*` gives its pointer (a function's name, `*` of a function
designator, an object, or any other expression) and whether it is a
postfix expression, which a call's parentheses can follow as written
(§6.5.2p1). The step quotes `*` and its operand as written, and
parenthesizes the operand for the call when it needs it:

- `*sq`: "`sq` designates the function sq, and like any function designator
  it converts to a pointer to that function. … `(*sq)(…)` and `sq(…)` call
  the same function."
- `**sq`: "`*sq` designates the function sq too, and converts to a pointer
  to it. … `(**sq)(…)` and `(*sq)(…)` call the same function", and `***sq`
  likewise one level up.
- `*&sq`: "`&sq` yields a pointer to the function sq. … `(*&sq)(…)` and
  `(&sq)(…)`".
- `*fp`: "`fp` holds a pointer to the function sq. … `(*fp)(…)` and
  `fp(…)`"; `**fp` as `**sq`.
- `*(int (*)(int))sq` and `*(c ? sq : sq)` keep their operand's parentheses:
  `((int (*)(int))sq)(…)` and `(c ? sq : sq)(…)`.

Each ends "However many * a program writes, each only shows the reader
that a pointer is being followed", citing §6.5.3.2p4 and §6.3.2.1p4. The
same facts give the call through a pointer its lead ("`f` holds a pointer
to square", "`(sq)` designates sq and converts to a pointer to it", "`(*&fp)`
yields a pointer to sq") and "what comes next", which decided "designates"
by looking for a `*` in the text: `(f)` now designates its function, and
`(*pp)`, an object that holds the pointer, points to it. `&*sq` and `*&fp`
run as before.

No other narration builds code by editing source text: the rest quote whole
expressions or operands as written, and a call's tree label adds `()` only
after a postfix expression.

#### 3. Null pointer constants other than a literal 0 were refused

C99 §6.3.2.3p3 makes any integer constant expression with the value 0, or
such an expression cast to `void *`, a null pointer constant. The checks
took only a literal `0`, so `int (*p)(int) = (1 - 1);`, `p = (1 - 1)` and
`p != (1 - 1)` were refused as constraint violations whose message said
they broke the rule; `int *q; q != (1 - 1)` was refused as unsupported.

**Fixed, wherever a literal 0 was accepted:** an initializer, an
assignment, `==` and `!=`, `?:`, a call's argument and a static initializer,
for pointers to objects as to functions. An integer constant expression
(§6.6p6) is checked by its form: integer and character constants and
`sizeof` joined by operators, with a floating constant only as a cast's
operand, so `(int)0.5` is one and `(int)(0.5 + 0.1)` is not, as GCC
agrees. `(void *)0` is a cast of its own, the only pointer to void in the
subset, with a step titled "Cast to void *: a null pointer", whose lead says
"`(void *)0` converts the null pointer constant 0 to void *, which gives a
null pointer: it points to no object and no function". Any other `(void
*)` cast is refused as outside the subset, as is arithmetic on `(void *)0`.
Comparisons name the constant as written: "`(1 - 1)` is an integer constant
expression with the value 0, so here it is a null pointer constant", and
"the right is the null pointer constant `(1 - 1)`, which converts to a null
pointer". A literal `0` keeps its old sentences, so no lesson changes. Two
null pointers, which the review's `(void *)0` probes now reach, "compare
equal exactly when both are null pointers, or both point to the same
place", citing §6.5.9p6.

#### 4. "A pointer to a function is never null"

The comparison with a null pointer said "A pointer to a function is never
null" in the step that compared one, `b`, with `a`.

**Fixed.** It now says "A pointer that points to a function never compares
equal to a null pointer, so comparing a pointer with null tells whether
there is a function to call through it" (§6.3.2.3p3). A condition says "a
pointer that points to a function is not null, so it counts as true".

#### 5. Smaller things

- **N-a.** The null-call stop now cites §4p2, the clause Part 15 added and
  never used, and its message says why: "the call `p(…)` goes through a
  null pointer, which points to no function (C99 §6.3.2.3p3). A call's
  first expression denotes the function it calls (§6.5.2.2p3), and C
  defines no call of anything else, so this one is undefined by the
  omission of any definition (§4p2)".
- **N-b.** `*` of a null pointer to a function stops under a title of its
  own, "Undefined behavior: * of a null pointer to a function", led by "A
  null pointer points to no function, so there is no function designator
  for `*` to yield", in place of the null object dereference's.
- **N-d.** The refusals of operators on a pointer to a function cite each
  operator's own paragraph: `+` §6.5.6p2, `-` §6.5.6p3, the orderings
  §6.5.8p2, and the shifts, bitwise and multiplicative operators theirs.
- **N-e.** A declaration of `void (*v)(void)` is refused for its void
  ("other types (void, _Bool, ...) are outside this subset") rather than as
  an unexpected expression; `fp == (void *)0` runs (item 3). The rest of
  N-e are explicit refusals and stay.
- **N-c and N-f** are left (see below).

#### What changed in the results

- **The lessons:** all 17 dumps are byte for byte the same, as are the
  247 regression programs, the 24 constant-expression probes and N1's 5.
- **The review's 132 probes:** 75 are the same. Five that were refused run
  to GCC's exit status: `p1/b28` (`p == (void *)0`, 0), `p3/d10` (`0L`,
  `(1 - 1)` and `0x0`, 10), `p4/e01` (an object pointer against `(1 - 1)`,
  1), `p4/e02` and `p4/e07` (1 each). Fourteen refusals or stops change
  only their message (items 2 and 5: `p1/b10`, `b11`, `b14`, `b29`, `b30`,
  `b39`, `b40`, `p2/c22`, `c24`, `c25`, `c27`, `p3/d07`, `p4/e06`, `e09`).
  The other 38 change only in the narration of items 2 to 4.
- **The implementer's pointer-to-function probes:** 22 of 52 are the same;
  seven refusals and stops change their message, and 23 change only in the
  narration. **The corpus:** every program has the same outcome and step
  count; `u04`'s stop changes its message, which
  [`corpus/results-part15-after.txt`](corpus/results-part15-after.txt)
  records, and the seven pointer-to-function programs change only in the
  narration.
- **My 17 probes:** the six that use a null pointer constant other than `0`
  (`(1 - 1)`, `'\0'`, `0L`, `(void *)0`, `(2 * 0)` in a static table,
  `(int)0.5`) run to GCC's exit status; `(void *)1`, an int from `(void
  *)0`, `(void *)0 + 1` and `(int)(0.5 + 0.1)` are refused, where GCC
  rejects or warns on each.
- Every program of the probe sets above that the engine runs and GCC 14
  compiled, 158 in all, gives GCC's exit status.

#### Checks

- **Stack:** 450 KiB fails and 500 KiB passes for the limit test, against
  1850 and 1900 KiB on `main`.
- **Engine tests:** 149 pass, three new: the forms of `*`, the null
  pointer constants, and null pointers to functions told apart from
  pointers to functions. Two change for the new wording (`f` holds its
  pointer; `*` of a null pointer to a function has its own cause). The
  engine's clippy passes, as `pages.yml` runs them.
- **Site:** as `pages.yml` runs them, the site's tests (167, and the
  prerender's 2), the WebAssembly stepper's (79) and the site's four
  clippy runs pass. CI builds the site; I did not build it locally.
- **Formatting:** `rustfmt +nightly --check` passes for every changed Rust
  file, and `git diff --check` is clean.
- **Browser:** not rerun. The page code changes only in a comment; the new
  wording reaches the page through the engine's notes, which the dumps
  cover.

#### Left for later

- **N-c, two spellings of one pointer:** the tape, cells and Table 1 write
  `→twice()`, Figure 2's wires and the expression panel `&twice`. Choosing
  one, or adding a legend line, is a design change to the page that needs
  its own browser walk.
- **N-f, the listing's label wraps a short definition** at 1024 and 1440
  pixels. It stays readable and nothing scrolls sideways; moving the label
  out of the line is a layout change for the same walk.

## Part 16: enumerations, switch, typedefs and pointers to functions in structures (2026-11)

Part 15 left four things a beginner writes soon after pointers to
functions, all refused before their first step: an enumeration, `enum color
{ RED, GREEN = 5, BLUE };`, with the `switch` that usually reads one; a
typedef of anything but a pointer to a function; a structure member that
points to a function, which is how C writes a dispatch table; and a function
that returns a pointer to a function. This part runs all of them, explains
each step in the notes' usual order (what happened to memory and values,
the rule of C99, why it matters, what comes next), and fixes the verb of
the `*` step for ordinary pointers that Part 15's review found. The work is
on `agent/machine-subset-4`.

### Method

- **The corpus.** Fourteen programs join the 58 of Parts 13 to 15 in
  [`corpus/`](corpus/): five with enumerations (`e01` to `e05`: constants
  and an object, a `switch` with `break` and `default`, a constant as an
  array's size, a `for` over an enumeration, and cases that fall through),
  one `break` out of a loop (`n01`), four typedefs (`t01` to `t04`: scalars
  and a block-scope typedef, a structure without a tag and a self-referring
  one, an array and a pointer, an enumeration without a tag), three with
  pointers to functions in new places (`v01` a structure member called
  through `.` and `->`, `v02` a function that returns one, `v03` a typedef'd
  dispatch structure), and one undefined call through a null member (`u06`).
  [`corpus/results-part16-before.txt`](corpus/results-part16-before.txt)
  and [`corpus/results-part16-after.txt`](corpus/results-part16-after.txt)
  are the engine's results on `main` and on this branch.
- **The probes.** 65 programs of my own: 30 for enumerations, `switch` and
  `break`, 13 for typedefs, 18 for pointers to functions as members and
  return values, and 4 for the verb of `*`; 55 run and 10 are refused or
  stop. As a regression set, the 299 probe programs of Part 15's two
  reviews. A throwaway engine test stepped each to its end, checking every
  step's "what comes next" against the step that followed, and every
  program was compiled with GCC 14 (`gcc -B …/gcc14… -std=c99 -pedantic
  -O0`) and run.
- **The notes.** The same throwaway test dumped every step of every lesson
  (title, lead, explanation, why it matters, citations, what comes next,
  whether a sequence point follows) on `main` and on the branch.
- **The builds.** The before build is the deployed site,
  <https://bcc.benbwall.com/>, which is `main` at `26aa3d018` (Part 15
  merged). This machine had 10 GB of disk free, less than a site build
  needs while keeping 8 GB spare, so the after build is the one CI made for
  this branch's pull request at `c56fc3a70` (`sh build.sh`, the release
  build the site deploys), downloaded as its Pages artifact and served
  locally. A first pass used CI's build of `e94df8058`; what it found is
  below ("Found in the browser"), and every scene and the 1440x900 walk
  were run again on the final build.
- **The browser.** Playwright drove Chromium with reduced motion, one
  browser at a time, at 1440x900, 1366x768, 1024x768 and 390x844 in both
  themes, on both builds. Each scene's program was loaded through the
  page's own Open .c and run with Run edited code, then stepped to its
  scene, recording the note, the cells' text for screen readers, Table 1's
  rows, the tape's labels, Figure 2's legend, the contrast of the new
  visuals against their composited backgrounds, running animations,
  sideways scroll and console errors.
- **The lesson walk.** Part 9's walk, every note of every machine lesson,
  on both builds at 1440x900 light and 390x844 dark, comparing each note's
  title, lead, value flow, "why it matters" note and next step.

The screenshots are in [`screenshots/part16/`](screenshots/part16/), named
`<scene>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours.

### The corpus, before and after

| Construct | Program | Before: where the engine refused it | After |
| --- | --- | --- | --- |
| Enumeration | [`e01_enum_color.c`](corpus/e01_enum_color.c) | 3:1: expected an arithmetic type or struct | runs (21 steps), returns 1 and prints `0 5 6` |
| `switch` on an enumeration | [`e02_enum_switch.c`](corpus/e02_enum_switch.c) | 3:1: the same | runs (41 steps), returns 3 and prints `4` |
| Constant as an array's size | [`e03_enum_array_size.c`](corpus/e03_enum_array_size.c) | 1:1: the same | runs (129 steps), returns 14 |
| `for` over an enumeration | [`e04_enum_loop.c`](corpus/e04_enum_loop.c) | 1:1: the same | runs (91 steps), returns 40 |
| Falling through cases | [`e05_fallthrough.c`](corpus/e05_fallthrough.c) | 1:1: the same | runs (21 steps), returns 2 |
| `break` out of a loop | [`n01_break_loop.c`](corpus/n01_break_loop.c) | 7:13: unsupported statement | runs (71 steps), returns 1 |
| typedef of scalars | [`t01_typedef_scalar.c`](corpus/t01_typedef_scalar.c) | 1:9: "a typedef in this subset names a pointer to a function" | runs (29 steps), returns 25 |
| typedef of structures | [`t02_typedef_struct.c`](corpus/t02_typedef_struct.c) | 1:16: expected an identifier | runs (37 steps), returns 32 |
| typedef of an array and a pointer | [`t03_typedef_array.c`](corpus/t03_typedef_array.c) | 1:9: the typedef message | runs (33 steps), returns 104 |
| typedef of an enumeration | [`t04_typedef_enum.c`](corpus/t04_typedef_enum.c) | 1:9: expected an arithmetic type or struct | runs (52 steps), returns 1 |
| Member that points to a function | [`v01_struct_member.c`](corpus/v01_struct_member.c) | 10:9: "a member that points to a function is outside this subset" | runs (49 steps), returns 15 |
| Function that returns one | [`v02_returns_pointer.c`](corpus/v02_returns_pointer.c) | 9:11: "a function that returns a pointer to a function is outside this subset" | runs (41 steps), returns 20 |
| Dispatch structure | [`v03_dispatch_table.c`](corpus/v03_dispatch_table.c) | 15:15: the member message | runs (49 steps), returns 5 and prints `5` |
| Undefined | [`u06_null_member_call.c`](corpus/u06_null_member_call.c) | 2:9: the member message | stops at step 10: call through a null pointer |

On `main` all fourteen were refused before their first step. On the branch
the thirteen defined programs run to GCC 14's output and exit status, and
`u06` stops at the call, where GCC's build crashes with SIGSEGV. The other
58 programs give exactly the results of Part 15; the engine's dump of every
one of their steps is byte for byte the same, except `g03` and `u05`, whose
declaration of an object written with a typedef name now says so (item 2).

### What changed

#### 1. Enumerations

`enum color { RED, GREEN = 5, BLUE };` declares a type and three constants,
at file scope or in a block, with or without a tag, and as a typedef.
`enum color c = BLUE;`, `enum color *p`, `enum color palette[3]`, a member
`enum color c;`, a parameter and a return type all work.

- **The declaration is a step that creates nothing.** At file scope it runs
  before `main`, after the directives and before the static objects; in a
  block, where execution reaches it. "Declare the constants of `enum
  color`", led by "`enum color` declares three constants, RED = 0, GREEN = 5
  and BLUE = 6: names for int values, which are not objects and take no
  storage." The explanation gives each constant's value and where it came
  from ("RED is the first constant and has no =, so it is 0; GREEN = 5 gives
  it the value 5; BLUE has no =, so it is the previous constant, GREEN, plus
  1: 6"), says that nothing is created or stored and that the compiler puts
  a constant's value wherever its name appears, gives the constants' scope,
  and says which integer type the enumeration is compatible with, citing
  §6.7.2.2p3, §6.4.4.3p2, §6.7.2.2p4 and §6.2.1p7. Its event kind is new,
  `enumeration`, in the colour of declarations; the JSON trace's schema is
  now version 11, which adds it, each object's and member's enumeration,
  and the trace's list of enumerations.
- **A constant reads nothing.** "Evaluate the enumeration constant BLUE",
  led by "`BLUE` is an enumeration constant of enum color: it stands for
  the int value 6, so nothing is read from memory", and explained with
  where the value came from and that its type is int "even though enum
  color itself is compatible with unsigned int" (§6.4.4.3p2, §6.7.2.2p3).
  Constants work in expressions, case labels, array sizes (`int v[SIZE];`,
  now any integer constant expression), static initializers and other
  constants' values (`B = A + 10`).
- **An enumeration's objects compute as GCC's do.** C leaves the compatible
  integer type to the implementation (§6.7.2.2p4); GCC chooses `unsigned
  int` unless a constant is negative, and `int` otherwise, and so does the
  machine, so `enum e x = A; x - 1 > 0` is 1 here as in GCC's build. An
  object's declaration says so: "Its type is an enumeration: enum color
  itself is compatible with unsigned int: … So it may hold any value of
  that type; C does not check that it holds one of the constants RED, GREEN
  and BLUE."
- **The page names the constant.** The tape, the cells and Table 1 write an
  enumeration object's value with the constant it holds, `6 BLUE`, as they
  write a char with its character, `65 'A'`; a value no constant has, such
  as `enum color odd = 7`, reads as `7`. Its type is spelled `enum color`.
- **What C refuses**, each as a constraint violation with its paragraph: a
  constant's value that is not an integer constant expression or does not
  fit in an int, including one that would be the previous constant plus 1
  (§6.7.2.2p2); two definitions of one enumeration (§6.7.2.3p1); a tag used
  for both a structure and an enumeration (§6.7.2.3p2); `enum tag` before
  the enumeration is complete, as in its own list (§6.7.2.3p3); a constant
  that repeats a name of the same scope (§6.7p3). A constant that would
  hide a name of an enclosing scope, or a name that would hide a constant,
  is refused as outside the subset.
- **A note.** New "why it matters" note: an enumeration names related
  integer values so a program can say `state == RUNNING`; the names are
  constants, not variables, and C does not check that an object holds one
  of them.

[`enum-1440-light`](screenshots/part16/enum-1440-light-after.png)
([before](screenshots/part16/enum-1440-light-before.png)), [the tape](screenshots/part16/enumvalue-deck-1440-dark-after.png),
[the cells](screenshots/part16/enumvalue-headrow-1440-dark-after.png) and
[Table 1](screenshots/part16/enumvalue-datasheet-1440-light-after.png) at
`GREEN`, with `c` holding `6 BLUE`.

#### 2. switch and break

`switch` runs with its `case` and `default` labels written directly in its
block, the form nearly every program uses, and `break` leaves a `switch`, a
`while` or a `for`.

- **The switch's step** is a branch: "switch on 1: jump to `case SQUARE`",
  led by "`switch (s)` has the value 1: `case SQUARE` matches, so control
  jumps to the statement after that label", or "no case matches, jump to
  `default`", or "no case matches, skip the body". The explanation says
  that the value is promoted and compared with each label's constant,
  converted to the same type, and that execution then "runs on through the
  statements that follow, past any later labels, until a break leaves the
  switch or its body ends" (§6.8.4.2p4-5). Falling through to the next case
  was no step at first; the first review round gave it one (below).
- **`break`** is "break: leave the switch" or "break: leave the loop"
  (§6.8.6.3p2). Leaving a block by a `break` ends the lifetimes of its
  objects, so each block the `break` leaves is a step of its own first, as
  at its closing brace.
- **What C refuses:** a controlling expression that is not an integer
  (§6.8.4.2p1), a case label that is not an integer constant expression, two
  labels with the same value after conversion, and a second `default`
  (§6.8.4.2p3), a label outside a switch (§6.8.1p2), and a `break` outside a
  switch or loop (§6.8.6.3p1). A label nested deeper in the body, and a
  declaration directly in the switch's block (which the switch jumps past),
  are refused as outside the subset.
- **A note** on why falling through is a classic bug.

[`switch-390-dark`](screenshots/part16/switch-390-dark-after.png)
([before](screenshots/part16/switch-390-dark-before.png)),
[`break-1024-light`](screenshots/part16/break-1024-light-after.png)
([before](screenshots/part16/break-1024-light-before.png)).

#### 3. typedef of any type

A typedef now names any type the subset has, at file scope or in a block
(§6.7.7p3): scalars (`typedef double celsius;`), pointers (`typedef const
char *string;`), arrays (`typedef int row[3];`), structures, tagged or not
(`typedef struct { int x, y; } point;`, and `typedef struct node node;`
before `struct node` is complete), enumerations, other typedef names, and
pointers to functions as before. A block's typedef ends with its block.

- **No step.** A typedef declares no object, so, as in Part 15, nothing runs
  where it is written; a typedef that defines an enumeration is that
  enumeration's step, which says the typedef "names a type and creates no
  object either".
- **The declaration says it.** An object declared with a typedef name says
  so: "Its declaration spells the type with the typedef name `celsius`,
  which line 1 declared as a synonym for double: a typedef names a type and
  creates no object, so `warm` is simply an object of that type", citing
  §6.7.7p3, with a new note on what a typedef is for. The cells and Table 1
  spell the type C resolves it to (`double`, `int[3]`, `struct node`), as
  they already did for a pointer to a function; a structure or an
  enumeration without a tag has no name but its typedef name, so `point p`
  is spelled `point`.
- **What is refused:** a typedef of a `const` type, of a function type
  (`typedef int fn(int);`), of an array of unknown size, two typedef names
  in one declaration, and a pointer to or an array of a typedef'd array; a
  typedef name repeated in one scope is a constraint violation (§6.7p3).

[`typedef-1366-dark`](screenshots/part16/typedef-1366-dark-after.png)
([before](screenshots/part16/typedef-1366-dark-before.png)), [Table
1](screenshots/part16/typedef-datasheet-1440-light-after.png).

#### 4. Pointers to functions as members and return values

- **Members.** `struct ops { int (*apply)(int); int count; };` lays the
  member out as the 8-byte pointer it is, spelled `int (*)(int)` in Table 1.
  A structure's initializer, an assignment to the member, a copy of the
  structure and a static structure all carry the pointer. `s.apply(3)`,
  `p->apply(3)`, `(*s.apply)(3)` and `o.inner.f(1)` call through it: "Call
  twice through `s.apply`", led by "`s.apply` holds a pointer to twice". A
  table member, `int (*ops[2])(int, int)`, works too. A null member stops
  the call as Part 15's null call does; an indeterminate one stops at its
  read.
- **Return values.** `int (*pick(int up))(int)`, and `operation find(char
  name)` with a typedef, return a pointer to a function, or a null one from
  `return 0;`. `pick(1)(10)` calls what the call returned: "Call decrement
  through `pick(0)`", led by "`pick(0)` yields a pointer to decrement".
  Returning a pointer of another function type is a constraint violation
  (§6.5.16.1p1 through §6.8.6.4p3).

[`member-1366-light`](screenshots/part16/member-1366-light-after.png)
([before](screenshots/part16/member-1366-light-before.png)), [the
listing](screenshots/part16/member-sourcepane-1440-light-after.png),
[`returns-1024-dark`](screenshots/part16/returns-1024-dark-after.png)
([before](screenshots/part16/returns-1024-dark-before.png)).

#### 5. The verb of `*` for ordinary pointers

The `*` step said "holds" for every operand. It now uses the rule #436 gave
pointers to functions: an object holds its pointer ("`p` holds a pointer to
`x`", "`*pp` holds …"), any other expression yields one ("`(t + 1)` yields
a pointer to t[1]", "`&p` yields …"). The step also quoted its operand by
stripping a `*` from its own source, which misquoted a parenthesized one:
`(*(*&p))(4)` said "`*(*&p)` designates `p`" for the inner `*&p`. It now
quotes the operand as written, "`*&p` designates `p`". Part 15 said no
narration still built code by editing source text; this one did, and now
none does.

[`yields-1440-light`](screenshots/part16/yields-1440-light-after.png)
([before](screenshots/part16/yields-1440-light-before.png)).

#### 6. Part 15's page nits

- **N-c, two spellings of one pointer.** Figure 2's wires write a pointer
  as `&` would make it, and the tape, the cells and Table 1 draw it as an
  arrow, for pointers to objects as to functions; choosing one would change
  every panel. Instead Figure 2 now adds one line under the drawing whenever
  a wire carries a pointer: "A wire writes a pointer as & would make it,
  &square; the tape, the cells and Table 1 draw the same pointer as an
  arrow, →square()." It is left out for a pointer into a string literal, which the other panels
  spell by the literal's text.
  [Figure 2](screenshots/part16/legend-schematic-1024-light-after.png)
  ([before](screenshots/part16/legend-schematic-1024-light-before.png)).
- **N-f, the listing's label wraps a short definition.** Left as it was. The wrap comes from a long
  label, `ops[0], ops[1] →`, written into a short line; this part's new
  labels (`s.apply →`, `step →`) fit on their lines at every size checked.
  Moving the label out of the line is the layout change Part 15 described,
  and is still left for later (below).

#### 7. "Limits of this model" and the glossary

§3 of "How it works" now lists among what runs: enumerations and their
constants, usable wherever C takes a constant; a typedef of any of the
subset's types, at file or block scope; functions that return an
enumeration or a pointer to a function; pointers to functions in structure
members, called as `s.f(x)` or `p->f(x)`; and `switch` with its labels
directly in its block, and `break`. Among what does not: `do`, `continue`
and `goto`, and a case label nested deeper in a switch's body; a function
that returns any other pointer. The limits test runs a program for each new
claim and refuses each new counterexample.
[`limits-1440-light`](screenshots/part16/limits-1440-light-after.png)
([before](screenshots/part16/limits-1440-light-before.png)).

The glossary defines an enumeration constant (§6.4.4.3p2), a typedef name
(§6.7.7p3) and a case label (§6.8.4.2p5).

#### 8. What did not change

- **No new lesson.** "Callbacks: passing a function to a function" needs its
  LLVM IR from a native bcc build with LLVM, which needs at least 14 GB of
  free disk; this machine had 10 GB, so the lesson is left for later again.
  The corpus's `g04_callback.c` and `v03_dispatch_table.c` show the same
  steps through Open .c meanwhile.
- **The lessons' opening comments** still wrap in the narrow listing (Part
  15's reason stands).

### Checks

- **Correctness against GCC 14.** Every program the engine runs gives GCC
  14's output and exit status: the 13 new corpus programs that run, the 55
  probes that run, and the two review probes that now run (`b3/e01_enum`,
  1; `rv-p3/d22_fp_return`, 3). The undefined ones stop where C leaves the
  behavior undefined: `u06` and a pointer returned null and then called at
  the call, where GCC's builds crash with SIGSEGV, and an uninitialized
  member at its read. The ten refused probes are refused for the rule they
  break where GCC rejects or warns: a duplicate case after conversion, two
  calls of the wrong function type, a constant used after its block, `enum
  e` inside its own list, a floating case label, and an implicit constant
  past `INT_MAX` (GCC 14 warns under `-pedantic`, the diagnostic the
  constraint requires); a parameter named like a constant, which C allows,
  is refused as outside the subset.
- **Descriptions.** Every step of every probe, corpus program and lesson
  named, in its "what comes next", the step that followed it; the engine
  tests check this for every program they run, the eight new ones included.
- **The notes.** The engine's dump of every lesson is byte for byte the
  same on `main` and on the branch. The browser walk compares 1,571 notes of
  the fifteen machine lessons at each size, 1440x900 light and 390x844
  dark: none changed in its title, lead, value flow, "why it matters" note
  or next step, and neither build scrolled sideways or raised a page error.
  The changes a reader can see in an existing lesson are a link and a line:
  "typedef name", in the unsigned lesson's declaration of a `size_t`
  object, now links to the glossary's new entry, and Figure 2 adds its
  legend under any expression whose wire carries a pointer to an object or
  a function, as several of the pointer lessons' expressions do.
- **Regressions.** Of Part 15's review probes, 289 of 299 have
  byte-identical dumps. Of the other ten, three are item 5's verb and
  quotation (`b2/n04`, `n10`, `rv-p1/b34`), three gain item 3's typedef
  sentence (`b2/n09`, `rv-p3/d08`, `d11`), two now run (above), one is
  refused with a clearer message (`rv-p1/b32`, a typedef of a function
  type), and one, a recursion that ends at the engine's 8 MB trace limit
  (`rv-p1/b17`), now reaches it at step 422 rather than 423, since each
  retained state is a few bytes larger. That is the limit, not the
  program's behavior.
- **The scenes.** At the four sizes in both themes, every scene reached its
  step on the after build, where the before build refused all but two of
  the programs before their first step. No sideways scroll, no console
  error or warning, no running animation.
- **Contrast.** Figure 2's new legend measures, at its lowest across sizes
  and themes, 5.89:1 for its text (12 px) and 8.38:1 for its code; an
  enumeration's constant is written in the value's own style, whose
  contrast is unchanged; the notes' text keeps 15.82:1 (light) and 13.34:1
  (dark), and the pointer-to-function visuals of Part 15 keep their ratios
  (7.05:1 at their lowest).
- **Keyboard, screen readers and motion.** The change adds no control. A
  cell of an enumeration is read as "c, enum color, 4 bytes: 6 BLUE", Table
  1's value has the same words, and Figure 2's legend is text in the figure,
  after the outline screen readers hear. The new styles have no
  transition, and every run used reduced motion.
- **Stack.** The limits test's stack need is unchanged: on the debug test
  binary it passes at 500 KiB and aborts at 480 KiB, on `main` and on the
  branch, well under 1 MiB. The new statements parse through `statement`,
  whose frame is unchanged, and the new expression kind adds no frame.
- **Tests:** as `pages.yml` runs them, the engine's tests (157: eight new,
  and six updated: the limits test for what now runs, the constraints test
  whose typedef case now runs, and the three JSON schema tests and the
  structure layout test for schema version 11) and its clippy pass, as do
  the site's tests (171, four new: an enumeration's objects and members,
  the switch's chips, Figure 2's legend and a member's mark in the listing;
  two changed their example of an unsupported statement from `switch` to
  `do`; and the prerender's 2), the WebAssembly stepper's (79) and the
  site's four clippy runs. CI's root jobs (Tests, Clippy, both Windows
  jobs, the WebAssembly explorer) pass. `cargo +nightly fmt --check` passes
  for every engine file; in the site's files every changed line is
  formatted. `git diff --check` is clean.

#### Found in the browser

The first pass over CI's build found four things, fixed before the final
screenshots:

- **A switch's value flow** read "1 counts as true", the chips of an `if`'s
  condition. It now reads "1 → jumps to → case SQUARE", or `default`, or
  "past its body".
- **An enumeration's declaration** had no value flow; it now draws "enum
  color → declares → RED = 0, GREEN = 5, BLUE = 6", as a `#define` draws
  its replacement.
- **The listing** did not mark a function a structure member points to; it
  now writes `s.apply →` before `twice`, as it does for a variable or a
  table element.
- **Figure 2's legend** spelled a pointer into a string literal as the
  other panels do not; such a pointer is now left out of it.

### Left for later

- **A lesson on callbacks**, `apply(int (*f)(int), int x)` and a dispatch
  table, with its LLVM IR from `web/pipeline/generate.sh` and its module
  from `web/wasm/generate.sh`, once a native build has room.
- **The lessons' opening comments**: rewrap them to 36 columns and
  regenerate the line tables and modules.
- **More of the switch**: a case label inside a nested block or statement
  of the body (Duff's device), and a declaration in the switch's block;
  `continue`, `do` and `goto`.
- **Pointers returned in general**: a function returning a pointer to an
  object (`int *f(void)`), with the lifetime checks that needs, and a
  pointer to a library function.
- **Static pointers to objects**: `static int *p = &x;` (§6.6p9), still
  refused.
- **Hiding a name**: an enumeration constant or a typedef name that hides a
  name of an enclosing scope, or is hidden by one, which C allows and the
  subset refuses.
- **A direct call's conversion**, Part 15's last item, stands.
- **N-f, the listing's label**: draw it outside the line, in the margin or
  under the definition, so a long one cannot wrap a short line.

### Review, round 1

The review of the branch at `ddc0d709c` found four blockers: a typedef'd
array parameter was not adjusted to a pointer, a `break` claimed later
labels that did not exist, an enumeration's object said C promised it
unsigned int's range, and Figure 2 drew the wrong enumeration's constants
when two shared a name. It also found one should-fix (N1, the limits text)
and seven nits. The review's own probe files went when its scratch
directory was cleaned, so I rewrote the 86 its report names or describes
(`k04` to `k10` for typedef'd parameters, `s01` to `s38` for `switch` and
`break`, `e01` to `e22` for enumerations, `t01` to `t14` for typedefs, and
`f` and `b` programs for members and `break` out of loops). They join the
65 probes and 72 corpus programs of Part 16, the 299 regression probes of
Part 15's reviews and the 17 lessons: 539 programs in all. A throwaway
engine test dumped every step of each on `ddc0d709c` and on the fixed
branch, and every program was compiled with GCC 14 (`-std=c99 -pedantic
-O0`) and run.

#### 1. A typedef'd array parameter was not adjusted

After `typedef int row[3];`, the parameter of `int first(row r)` kept the
type `int[3]`. C99 §6.7.5.3p7 adjusts it to `int *`, as it adjusts `int
r[3]`. So `int (*fp)(int *) = first;` was refused as a constraint
violation that it is not, `first(t)` as an argument that does not match,
and `int size(row r) { return sizeof r; }` would have given 12, not 8.

**Fixed by refusing it as the plain form is refused.** The subset has no
array parameters: `int r[3]` was already refused with "array parameters are
outside this subset; write the pointer they adjust to, such as `char *s`".
The check now looks at the parameter's type as well as for a `[` after its
name, so `row r` gets the same message, in a definition, a prototype or the
parameter list of a pointer to a function. A parameter of function type
(§6.7.5.3p8) cannot come from a typedef, since a typedef of a function type
is refused where it is declared.

#### 2. `break` claimed later labels

Every `break` in a switch said "the statements after the break, including
those under later case labels, do not run. Without the break, execution
would run on into the next label's statements", even as the body's last
statement, in the common `default: r = 2; break; }`.

**Fixed, from the body as compiled.** When the switch's body is complete,
each of its `break`s records what follows it: how many labels come after
it, else whether any statement does. Leaving a block and an `if`'s jump
over its `else` branch do not count, since neither runs anything. The text
is one of:

- a later label: "…the statements after the break, including those under
  the later label, do not run. Without the break, execution would run on
  past that label into its statements" (or "the later labels" and "the next
  label" for more than one);
- only statements: "…and the statements after the break, to the end of the
  body, do not run";
- nothing: "…control jumps to just after its body. Nothing in the body
  follows the break, so it skips nothing: reaching the body's closing brace
  would leave the switch the same way".

#### 3. An enumeration's object promised unsigned int's range

The declaration of `enum color c` reused unsigned int's width text: "4
bytes on x86-64, 0 to 4294967295; C promises only 0 to 65535". C99
§6.7.2.2p4 promises an enumerated type only a type that can represent its
constants' values, which may be a char.

**Fixed.** It reads "an enum color object (4 bytes on x86-64, 0 to
4294967295; C promises only that its type can represent its constants'
values, so at least 0 to 6)", "-3 to 11" for constants from -3 to 11, and
"its constant's value, 4" for one constant. Round 2 changed "it holds" to
"its type can represent": the object may hold any value of the type, such
as 42 after `enum color c = 42;`. The next sentence lost its two colons in a
row: "Its type, enum color, is an enumeration, compatible with unsigned
int: C lets each implementation choose an integer type that holds every
constant, and GCC chooses int when a constant is negative and unsigned int
otherwise, as here, where none is". An array of an enumeration's objects
says it of "Its element type" and of "each element", where it had said
"Its type" of the array.

#### 4. Figure 2 drew the wrong enumeration's constants

The value flow found the enumeration by the name in the step's title, the
first match. In sibling blocks that each declare `enum e`, legal since each
tag's scope ends with its block (§6.2.1p4), the second declaration drew "enum
e → declares → A = 1, B = 2", the first one's constants.

**Fixed.** The step's range is the declaration's own, so the value flow
takes the enumeration whose declaration covers the step's range: the second
step now draws "C = 10, D = 20", and the same holds for `typedef enum { … }
t;` in two blocks.

#### 5. The limits text, and the nits

- **N1, the limits text.** "a typedef of any of these types" now reads "a
  typedef of any of these types but a const one", since `typedef const int
  cint;` stays refused. A typedef'd array as a parameter is refused like any
  array parameter (item 1), which the text never listed as in the subset.
- **Fall-through is a step.** Reaching a label from the statement before it
  is "Run on past `case 2:`", led by "The statement before `case 2:` has
  finished, and no break came, so execution runs on past the label into the
  statements after it", and explained by §6.8.1p4: a label "does not alter
  the flow of control". A run of labels, as in `case 2: case 3:`, is one
  step, and the step before it says "Next: run on past `case 2:` into the
  statements after it". The switch's jump to a label lands past that step,
  so it appears only where execution falls through. `e05_fallthrough.c`
  now has 21 steps, not 20, which
  [`corpus/results-part16-after.txt`](corpus/results-part16-after.txt)
  records.
- **Chip grammar.** A switch that skips its body reads "42 → skips → its
  body".
- **`compatible_text`** says "GCC chooses int when a constant is negative,
  as one is here, and unsigned int otherwise" (item 3).
- **The glossary's case label** is "A place in a switch's body, such as
  case 3:, that the switch can jump to: it jumps to the case label whose
  constant equals its value, or else to the default: label, which has no
  constant".
- **The doc comment** of `static_storage` is back on it, and
  `enumeration_name` has only its own.
- **Dead code.** The `labelled && at("}")` test inside the body's loop and
  the switch step's branch for a promotion that changed the value are gone,
  and "What changed" above no longer says the step gives one.
- **A parameter of enumerated type** is named as declared: "The argument
  for `c` converts to the parameter's type, enum color (compatible with
  unsigned int)".

#### What changed in the results

- **The lessons:** all 17 dumps are byte for byte the same, as are 298 of
  the 299 regression probes; the other, `b3/e01_enum`, changes only in the
  enumeration step's GCC sentence.
- **The corpus and Part 16's probes:** 66 of the 72 corpus programs are the
  same, the 58 of Parts 13 to 15 among them. The other 6, all enumeration
  programs, and 23 of the 65 probes change in the narration of items 2, 3
  and 5; only `e05_fallthrough` and `p30_default_first` change their step
  count, by their fall-through steps.
- **The review's probes:** 42 of the 86 change. Four that were refused or
  mis-refused (`k04`, `k06`, `k07`, and `k08`, whose prototype `int
  first(row);` was refused as differing from `int first(int *r)`) are now
  refused as array parameters; the rest change only in the narration of
  items 2, 3 and 5, and seven gain fall-through steps.
- **Explained in full.** A script undid each intended change in the after
  dumps (removed the fall-through steps and restored the "next" before
  them, and put back the old break, width, type, GCC and parameter
  sentences). Every one of the 72 changed dumps then equals its before
  dump, except the four refusals above.
- **GCC 14.** Each of the 365 programs that the engine runs to its end
  gives GCC 14's output and exit status, and GCC compiles each one. The 41
  that stop (undefined behavior, an indeterminate read or a resource limit)
  and the 133 refused stop or are refused as on `ddc0d709c`, except the four
  refusals of item 1.

#### Checks

- **Engine tests:** 160 pass, three new: a typedef'd array is no parameter
  type (B1), break and fall-through say what the body holds (B2 and the
  fall-through step), and an enumeration's type promises only its
  constants' range (B3, with the parameter's name). One changed for the new
  type sentence. The engine's clippy passes.
- **Site:** as `pages.yml` runs them, the site's tests (172, one new for
  sibling enumerations and one extended for the skipped body; and the
  prerender's 2) and the WebAssembly stepper's (79) pass. CI's Lint site
  job failed on the first push, `c102fbc76`: clippy's `similar_names`
  refused the fall-through step's bindings `them` and `they` in the
  abstract machine's lint run. They are now `phrase` and `pronoun`, and the
  five clippy runs (the abstract machine's, the site's native one and its
  hydrate, worker and compiler wasm32 ones) pass locally.
- **Formatting:** `rustfmt +nightly --check` passes for every changed
  engine file; in the site's files every changed line is formatted. `git
  diff --check` is clean.
- **Browser:** CI's Pages build of `c102fbc76` (the rename after it changes
  no behavior), served locally, at 1440x900 light and 390x844 dark with
  reduced motion. The second of two sibling `enum e` declarations draws
  "enum e → declares → C = 10, D = 20", and the `typedef enum` pair "t →
  declares → C = 10, D = 20"
  ([`review1-sibling-390-dark`](screenshots/part16/review1-sibling-390-dark.png)).
  `switch (42)` with no matching label draws "42 → skips → its body".
  `e05_fallthrough.c` reaches "Run on past `case LOW:`" at step 16
  ([`review1-fallthrough-1440-light`](screenshots/part16/review1-fallthrough-1440-light.png)).
  The final break's note says nothing follows it and names no later label,
  `c`'s declaration reads round 1's "C promises only that it holds the
  values of its constants, 0 to 6", and the limits text reads "a typedef of
  any of these types but a const one". No sideways scroll, console error or
  warning, or running animation. Round 2's wording, "C promises only that
  its type can represent its constants' values, so at least 0 to 6", was not
  in that build; Part 17 checked it in the browser on the deployed site
  (`main` at `6ccc7d72a`, 1440x900 light).

## Part 17: do, continue, goto, hidden names and pointers returned (2026-11)

Part 16 left the rest of C's control flow refused before a program's first
step: `do`, `continue` and `goto`, and a case label anywhere but directly in
a switch's block, which Duff's device needs. It also left a function that
returns a pointer to an object refused, and a use of a name that another
object shares explained no differently from any other use. This part runs
all of them and explains each step in the notes' usual order (what happened
to memory and values, the rule of C99, why it matters, what comes next). A
jump is where the abstract machine's rules for objects show most: leaving a
block ends its objects' lifetimes, entering one begins them, and a jump that
passes a declaration passes its initializer. The work is on
`agent/machine-subset-5`.

### Method

- **The corpus.** Thirteen programs join the 72 of Parts 13 to 16 in
  [`corpus/`](corpus/): three loops (`j01` a `do` that counts the digits of
  0, so its body runs once before any test, `j02` a `continue` in a `for`,
  `j03` one in a `do`), three gotos (`j04` out of two nested loops, `j05` to
  one cleanup point per function, `j06` back over a declaration), two
  switches with labels inside the body (`j07` Duff's device, `j08` a label in
  a block that declares an object), a name an inner block hides (`m01`), two
  functions that return pointers to objects (`q01` to an element of a static
  array, `q02` to one of its arguments), an undefined program (`u07` uses a
  pointer to a returned function's local) and one that stops at an
  indeterminate read (`x01` reads an object whose initializer a goto jumped
  past; a new prefix, since the machine stops there without calling the
  read undefined, as it does for any indeterminate read).
  [`corpus/results-part17-before.txt`](corpus/results-part17-before.txt) and
  [`corpus/results-part17-after.txt`](corpus/results-part17-after.txt) are
  the engine's results on `main` and on this branch.
- **The probes.** 65 programs of my own: 41 for `do`, `continue`, `goto`,
  nested labels, hidden names and pointer returns, then 24 adversarial ones
  (a goto two blocks deep, out of a switch, back into a loop's body from
  after the loop, to a label that runs on into case labels, in a recursive
  function; `continue` in a `do` inside a `for`; a case label on a loop and a
  default inside an `if`; three levels of hidden names and a body's `i`
  hiding its `for`'s; `*f() += 40`, `f()[0]`, `f()->x`; a pointer to a
  parameter or to a block's object returned). As a regression set, the 668
  programs Part 16's reviews kept (Part 16's corpus and probes, its review
  probes, the 299 of Part 15's reviews and the lessons) and today's 17
  lessons and 72 corpus programs: 757 in all. A throwaway engine test dumped
  every step of each (title, lead, explanation, note, citations, what comes
  next, whether a sequence point follows) on `main` and on the branch, and
  every program was compiled with GCC 14 (`gcc -B …/gcc14… -std=c99
  -pedantic -O0`) and run.
- **The builds.** The before build is the deployed site,
  <https://bcc.benbwall.com/>, which is `main` at `6ccc7d72a` (Part 16
  merged). This machine had 9 GB of disk free, less than a site build needs
  while keeping 8 GB spare, so the after build is the one CI made for this
  branch's pull request (`sh build.sh`, the release build the site deploys),
  downloaded as its Pages artifact and served locally: CI's build of
  `028975772`, the branch's head before this document and the screenshots,
  which change no code.
- **The browser.** Playwright drove Chromium with reduced motion, one browser
  at a time, at 1440x900, 1366x768, 1024x768 and 390x844 in both themes, on
  both builds. Each scene's program was loaded through the page's own Open
  .c and run with Run edited code, then stepped to its scene, recording the
  note, the listing's marks, Figure 2's value flow, the cells' text for
  screen readers, Table 1's rows, the contrast of the new visuals against
  their composited backgrounds, running animations, sideways scroll and
  console errors.
- **The lesson walk.** Part 9's walk, every note of every machine lesson, on
  both builds at 1440x900 light and 390x844 dark, comparing each note's
  title, lead, value flow, "why it matters" note and next step.

The screenshots are in [`screenshots/part17/`](screenshots/part17/), named
`<scene>-<width>-<theme>-before.png` and `…-after.png`, saved with 256
colours.

### The corpus, before and after

| Construct | Program | Before: where the engine refused it | After |
| --- | --- | --- | --- |
| `do` | [`j01_do_digits.c`](corpus/j01_do_digits.c) | 4:5: unsupported statement | runs (28 steps), returns 1 |
| `continue` in a `for` | [`j02_continue_for.c`](corpus/j02_continue_for.c) | 6:13: unsupported statement | runs (190 steps), returns 35 |
| `continue` in a `do` | [`j03_continue_do.c`](corpus/j03_continue_do.c) | 4:5: unsupported statement | runs (95 steps), returns 53 |
| `goto` out of two loops | [`j04_goto_out.c`](corpus/j04_goto_out.c) | 8:17: unsupported statement | runs (203 steps), returns 5 |
| `goto` to a cleanup point | [`j05_goto_cleanup.c`](corpus/j05_goto_cleanup.c) | 5:9: unsupported statement | runs (52 steps), returns 2 and prints `ok 3`, `bad -2` |
| `goto` back over a declaration | [`j06_goto_back.c`](corpus/j06_goto_back.c) | 3:6: "use of an undeclared identifier" (the label) | runs (77 steps), returns 4 |
| Duff's device | [`j07_duff_device.c`](corpus/j07_duff_device.c) | 8:13: unsupported statement (the `do`) | runs (106 steps), returns 15 |
| A case label in a block | [`j08_case_in_block.c`](corpus/j08_case_in_block.c) | 8:5: "a case or default label nested inside another statement of the switch's body is outside this subset" | runs (23 steps), returns 20 |
| A hidden name | [`m01_hidden_name.c`](corpus/m01_hidden_name.c) | ran (36 steps), returns 121 | the same, and each use says which `x` it reaches |
| A pointer to an element returned | [`q01_returns_element.c`](corpus/q01_returns_element.c) | 3:6: "functions return an arithmetic type, an enumeration, a structure or a pointer to a function in this subset" | runs (92 steps), returns 80 |
| A pointer argument returned | [`q02_returns_argument.c`](corpus/q02_returns_argument.c) | 1:6: the same | runs (39 steps), returns 30 |
| Undefined: a dangling pointer | [`u07_dangling_return.c`](corpus/u07_dangling_return.c) | 1:6: the same | stops at step 13, the pointer's first use |
| Stops: a jump past an initializer | [`x01_jump_past_initializer.c`](corpus/x01_jump_past_initializer.c) | 2:5: unsupported statement | stops at step 4, the read of `count`, indeterminate |

On `main` twelve of the thirteen were refused before their first step. On
the branch the eleven defined programs give GCC 14's output and exit status.
`u07` stops at its first use of the returned pointer, where GCC's build
crashes with SIGSEGV; `x01` stops at its read of `count`, which the jump into
its block left indeterminate (GCC's build returns 253). The 72 programs of
Parts 13 to 16 give exactly the results of Part 16; the engine's dump of
every one of their steps is byte for byte the same, except `e05`, whose
fall-through step now says "no break left the switch" (item 7).

### What changed

#### 1. `do`

`do statement while (expression);` runs, with a braced body or not, and
`break` and `continue` work in it.

- **The body runs before any test, and a step says so.** The `do` keyword is
  a step of its own: "do: run the body before any test", led by "A do
  statement runs its body first and tests `n != 0` only after it, so the
  body runs at least once", and explained against `while` and `for`, which
  test before each pass (§6.8.5.2p1, §6.8.5p4). The loop's body is then a
  block entered on each pass, as a `while`'s is.
- **The test.** "Condition is true: run the body again" or "Condition is
  false: leave the loop", citing §6.8.5.2p1. A true test jumps straight back
  to the body; there is no separate "jump back" step, since nothing else
  runs between the test and the body.
- **A note.** New "why it matters" note on C's three loops: while tests
  before each pass, so its body may never run; do tests after, so its body
  runs at least once; for puts a loop's start, test and step on one line;
  break and continue work in all three.

[`do-1440-light`](screenshots/part17/do-1440-light-after.png)
([before](screenshots/part17/do-1440-light-before.png)),
[the test](screenshots/part17/dotest-390-dark-after.png).

#### 2. `continue`

`continue;` in a `while`, a `do` or a `for`, inside any blocks, ifs and
switches of the body.

- **Which lifetimes it ends.** A continue ends the blocks it is in, the
  loop's body last, each in a step of its own before it jumps, as `break`
  does (Part 16): "`continue` leaves the block, so `square`, declared inside
  it, ceases to exist", explained as "The continue leaves this block on its
  way to the end of the loop's body. With the block left, `square` reaches
  the end of its lifetime…".
- **Where it goes.** "continue: end this pass through the body", led by
  "`continue` skips the rest of this pass through the for loop's body, so
  control goes to the for statement's third clause, `i++`, and then to the
  condition" (in a `while`, "goes straight to the condition `i < 6`"; in a
  `do`, "goes to the test `i < 5` after the body"; in `for (;;)`, "starts the
  next pass"). The explanation gives §6.8.6.2p2's own reading, a jump to a
  label just before the body's closing brace, and says how it differs from
  break. The step after it is the third clause's, or the test's.
- **What C refuses:** a continue outside a loop, including in a switch that
  is in no loop, is a constraint violation (§6.8.6.2p1).

[`continue-1366-dark`](screenshots/part17/continue-1366-dark-after.png)
([before](screenshots/part17/continue-1366-dark-before.png)).

#### 3. `goto` and labels

`goto name;` and `name:` on any statement, forward and backward, out of and
into blocks.

- **Leaving blocks.** A goto first leaves, one step each, every block it is
  in that the label is not: "`goto done;` leaves the block that begins on
  line 4, so `cell`, declared inside it, ceases to exist", and for a for
  statement's own block "leaves the for statement on line 3, so `row`…".
  The explanation says the label is not inside that block, and that the goto
  leaves the blocks one at a time, innermost first.
- **The jump.** "goto `done`: jump ahead to line 12" (or "jump back"), led by
  "`goto done;` jumps to the statement labeled `done:` on line 12, later in
  `main`; execution goes on from there", explained by §6.8.6.1p2 and by a
  label's function scope (§6.2.1p3), with how many steps before it left
  blocks.
- **Entering blocks.** A jump into a block begins the lifetimes of the
  block's objects, as entering at its brace does (§6.2.4p5), but runs none
  of the initializers it passes (§6.8p3): "The label is inside the block
  that begins on line 3, which the jump enters from outside… `count` begins
  to exist here, indeterminate. The jump passed its declaration, so no
  initializer ran, but its name is in scope at the label and can be
  assigned…" (the wording of round 1, below, which also tells apart the
  declarations after the label). `x01` then stops at its read of `count`.
- **Passing a declaration in the same block.** Jumping ahead past `int x =
  4;` brings `x`'s name into scope without its initializer: "The jump passes
  the declaration of `x` on line 4: … no initializer of that declaration
  runs, so its value is still indeterminate." Jumping back before a
  declaration takes the name out of scope while the object lives on, and
  reaching the declaration again is said as such: "A goto went back over
  this declaration, so `square` is declared here again: … its initializer
  stores a value again", or, without an initializer, "its value becomes
  indeterminate again" (§6.2.4p5). Without that, `j06`'s second pass would
  have found its object "already declared".
- **The listing draws the jump.** While a goto leaves blocks and when it
  jumps, the listing marks its label, `done:`, with a dotted underline, and
  writes before it where the goto is: `goto, line 8 →`, the idiom the
  listing uses for `s.apply →`. Figure 2 draws "goto done → jumps to → done:
  line 12".
- **What C refuses:** a goto to a label its function does not have
  (§6.8.6.1p1) and a label repeated in one function (§6.8.1p3), both as
  constraint violations.
- **A note** on goto: jumping freely makes code hard to follow, so C
  programs keep goto for leaving nested loops at once or for one cleanup
  point; a jump never suspends the rules for objects.

[`goto-1440-light`](screenshots/part17/goto-1440-light-after.png)
([before](screenshots/part17/goto-1440-light-before.png)), [the
listing](screenshots/part17/goto-sourcepane-1440-light-after.png), [leaving a
block](screenshots/part17/gotoleave-sourcepane-1024-dark-after.png),
[`gotoinside-390-dark`](screenshots/part17/gotoinside-390-dark-after.png)
([before](screenshots/part17/gotoinside-390-dark-before.png)),
[`gotoback-1366-light`](screenshots/part17/gotoback-1366-light-after.png).

#### 4. Case labels anywhere in a switch's body

A case or default label may be inside any block or statement of the switch's
body (§6.8.4.2p2-3), as in Duff's device, where the labels sit inside a `do`.
The subset refuses them only where C does: outside a switch (§6.8.1p2), and
inside a nested switch they belong to that switch.

- **The switch's jump enters the blocks around the label.** Its explanation
  adds what entering does: "The label is inside the block that begins on
  line 5, which the jump enters from outside… `scale` begins to exist here,
  indeterminate", so a following `scale = 20;` is fine and a read before it
  stops.
- **Falling through to a label first in a block** reads "Execution has
  entered the block that `default:` begins, and no break left the switch, so
  execution runs on past the label…", since no statement came before it.
- **A declaration directly in the switch's block** stays refused, as outside
  the subset: entering the body would have to create it.

[`duff-1440-dark`](screenshots/part17/duff-1440-dark-after.png)
([before](screenshots/part17/duff-1440-dark-before.png)),
[`caseblock-1024-light`](screenshots/part17/caseblock-1024-light-after.png)
([before](screenshots/part17/caseblock-1024-light-before.png)).

#### 5. Which `x` a use reaches

The tape, the cells and Table 1 already told a hidden name's objects apart
(`x` and `x #2`). The notes now say which one each use reaches, and why,
wherever another live object of the call, or the file, has the name:

- **Before the inner declaration:** "The block's own x, declared on line 6,
  already exists, since entering its block created it, but its name's scope
  begins only at its declaration, which execution has not reached: until
  then `x` still means the one declared on line 2" (§6.2.1p7).
- **After it:** "Another object named x is alive here too, the x declared on
  line 2. This `x` is the one declared on line 6, in a block inside that
  one's scope: an inner declaration hides an outer one for the rest of its
  block, so this use reaches the inner object and not the outer one, which
  still holds 1" (§6.2.1p4). The other object is "the x declared at file
  scope, on line 1" or "as a parameter of `f`" where it is one of those.
- The page writes this `x` as `x #2`, as the tape does; the other is named
  by its line, so that the two never share a label in one sentence.
- **A note** on hidden names and GCC's `-Wshadow`.
- An enumeration constant or a typedef name that hides another name, or is
  hidden, stays refused as Part 16 left it.

[`hidden-1440-light`](screenshots/part17/hidden-1440-light-after.png)
([before](screenshots/part17/hidden-1440-light-before.png)), [Table
1](screenshots/part17/hidden-datasheet-1440-light-after.png).

#### 6. Functions that return pointers to objects

`int *best(void)`, `char *skip(char *s)`, `const char *name(int i)`,
`struct point *get(void)` and a typedef'd pointer return type all run, as do
`*f() = 0`, `*f() += 40`, `f()[0]` and `f()->x`. A typedef of an array as a
return type is refused as the constraint violation it is (§6.7.5.3p1).

- **A dangling result.** When a function returns a pointer to one of its own
  automatic objects, its return step says so: "`make` returns a pointer to
  its own `local`, which ends with the call, so the caller receives a
  dangling pointer", explained by §6.2.4p2, by which the value of every
  pointer to an object becomes indeterminate when the object's lifetime
  ends, and by Annex J.2, which lists using that value, even to store or
  compare it, as undefined (round 1 added J.2; §6.2.4p2 alone says only
  "indeterminate").
- **The machine stops at the first use,** not at the return: "Undefined
  behavior: use of a dangling pointer", "The pointer pointed to an object of
  a call that has returned. When that object's lifetime ended, the pointer's
  value became indeterminate, and C gives no meaning to using it", citing
  §6.2.4p2 and J.2. A statement that only discards the result, `make();`,
  runs on, since it does not use the value.

[`dangling-1366-light`](screenshots/part17/dangling-1366-light-after.png)
([before](screenshots/part17/dangling-1366-light-before.png)),
[the stop](screenshots/part17/danglingfault-390-dark-after.png),
[`pointerreturn-1024-dark`](screenshots/part17/pointerreturn-1024-dark-after.png).

#### 7. Part 16's nits

- **The next line for a run of labels** says "run on past `case 2: case 3:`
  into the statements after them", and "after it" for one label.
- **"No break came"** is now "no break left the switch", true when an inner
  loop's or switch's break ran.
- **An unnamed array parameter's refusal**, as in `int first(row);`, points
  at `row` (2:11), where it pointed at the `)` after it (2:14). An unnamed
  `const int` parameter's refusal moves the same way.
- **One constant:** "C does not check that it holds its only constant,
  ONLY", where it said "one of its constant, ONLY".
- **The round-1 bullet's browser claim.** Part 16's "Review, round 1" said
  the browser showed `c`'s declaration reading "C promises only that its type
  can represent its constants' values, so at least 0 to 6"; the round-1 pass
  had seen round 1's "C promises only that it holds the values of its
  constants, 0 to 6". This pass checked round 2's wording in the browser on
  the deployed site, `main` at `6ccc7d72a`, at 1440x900 light, where it reads
  as the bullet says
  ([`enumdeclare-1440-light`](screenshots/part17/enumdeclare-1440-light-before.png)),
  and the bullet now says which build showed which.

#### 8. "Limits of this model" and the glossary

§3 of "How it works" now lists among what runs: functions that return "a
pointer to one of these objects or to a function"; and "if and else, while,
do and for, switch with its case and default labels anywhere in its block,
break, continue, and goto with the labels it jumps to". Among what does not,
"do, continue and goto, and a case label nested deeper in a switch's body"
and "a function that returns any other pointer" are gone, and "a declaration
directly in a switch's block" is new. The limits test runs a program for
each new claim and refuses the new counterexamples (a declaration directly
in a switch's block, a function returning `void *`).
[`limits-1440-light`](screenshots/part17/limits-1440-light-after.png)
([before](screenshots/part17/limits-1440-light-before.png)).

The glossary defines goto (§6.8.6.1p2) and continue (§6.8.6.2p2).

#### 9. What did not change

- **No new lesson.** A lesson needs its LLVM IR from `web/pipeline/
  generate.sh`, which needs a native bcc build with LLVM and at least 14 GB
  of free disk, and its WebAssembly module from `web/wasm/generate.sh`; this
  machine had 9 GB, falling to 7.8 GB while another build ran, so neither
  "Callbacks: passing a function to a function" nor a "Loops three ways"
  lesson was written. The corpus's `j01` to `j03` show the three loops
  through Open .c meanwhile.
- **The lessons' notes.** No lesson uses `do`, `continue`, `goto`, a hidden
  name or a pointer return, so none of their steps changed (below).

### Checks

- **Correctness against GCC 14.** Every program the engine runs gives GCC
  14's output and exit status: the 11 new corpus programs that run, and the
  55 of the 65 probes that run. The undefined ones stop where C
  leaves the behavior undefined: a use of a returned pointer to a callee's
  local or parameter (`u07`, and probes that store, compare or dereference
  it), where GCC's builds crash or happen to return 0; and a read of an
  object whose initializer a jump passed, or that a jump back made
  indeterminate again, where GCC's builds return whatever the stack held
  (253, 254, or the earlier pass's 5). The refused probes are refused for the
  rule they break, where GCC rejects them too: a goto to a missing label and
  a repeated label.
- **Descriptions.** Every step of every probe, corpus program and lesson
  named, in its "what comes next", the step that followed it; the engine
  tests check this for every program they run, the nine new ones included.
- **The notes.** Of the 757 regression programs, 717 have byte-identical
  dumps, the 17 lessons among them. A script undid this part's intended
  wording changes in the other 40 (the fall-through lead, "after them", and
  the single constant) and 32 then equal their `main` dumps. The other eight
  are explained: two now run (`rv437/s33`, a `continue`, and `s34`, a nested
  label); three refusals point at the parameter's type (item 7); and three
  recursions that end at the engine's 8 MB trace limit reach it two to four
  steps sooner (`r-rv-p1/b17` at 420 of 422, `r-rv-p3/d03` at 477 of 480,
  `d04` at 496 of 500), since each object now records its block and its
  declaration's line; their steps before the limit are byte for byte the
  same. That is the limit, not the programs' behavior. The browser walk
  compares 1,571 notes of the fifteen machine lessons at each size, 1440x900
  light and 390x844 dark: none changed in its title, lead, value flow, "why
  it matters" note or next step, and neither build scrolled sideways or
  raised a page error. The only intended change to an existing note is
  Part 16's round-1 bullet in this document (item 7).
- **The scenes.** At the four sizes in both themes, every scene reached its
  step on the after build, where the before build refused all but two of the
  programs (`m01` and `e01`) before their first step. No sideways scroll, no
  console error or warning, no running animation. The label's mark fits its
  line at every size, `goto, line 2 →` included at 390x844.
- **Contrast.** The label's mark, `goto, line 8 →`, measures 14.63:1 at its
  lowest across sizes and themes, and the label it underlines 16.47:1, both
  on the listing's black screen; Figure 2's goto chips use the existing chip
  styles; the notes' text keeps 15.82:1 (light) and 13.34:1 (dark), and
  Figure 2's legend keeps 5.89:1 for its text and 8.38:1 for its code.
- **Keyboard, screen readers and motion.** The change adds no control. The
  label's mark is a decoration of the listing's text, which screen readers
  read as before; its words, "goto, line 8 →", are also its hover text's
  subject ("The goto on line 8 jumps ahead to this label"), and the note says
  the same in words. The new styles have no transition, and every run used
  reduced motion.
- **Stack.** The limits test's stack need is unchanged: on the debug test
  binary it passes at 500 KiB and aborts at 490 KiB, on `main` and on the
  branch, well under 1 MiB. Labels parse in a loop, one frame for a whole
  run of them, and the new statements parse through `statement`, whose frame
  is unchanged.
- **Tests:** as `pages.yml` runs them, the engine's tests (169: nine new,
  and three changed: the limits test for what now runs and what stays
  refused, the switch test whose nested label and `continue` now compile,
  and the corpus test, which expects an `x` program to stop at its
  indeterminate read) and its clippy pass, as do the site's tests (173, one
  new for the goto's mark in the listing and
  its value flow; one changed its example of an unsupported statement from
  `do` to an array of arrays; and the prerender's 2), the WebAssembly
  stepper's (79) and the site's four clippy runs. `cargo +nightly fmt
  --check` passes for every engine file; in the site's files every changed
  line is formatted. `git diff --check` is clean.

### Left for later

- **A lesson on callbacks**, and one on the three loops, once a native build
  with LLVM has room (14 GB of free disk).
- **The lessons' opening comments**: rewrap them to 36 columns and
  regenerate the line tables and modules (Part 15's reason stands).
- **A declaration directly in a switch's block**, which the switch's jump
  passes: entering the body would begin its lifetime, as entering any other
  block a jump lands in now does.
- **`f().m`**, a member of a structure a call returns, still refused; `f()->m`
  runs.
- **Hiding a name with a constant or a typedef name**, Part 16's item, stands.
- **Static pointers to objects**: `static int *p = &x;` (§6.6p9), still
  refused.
- **A direct call's conversion**, Part 15's item, stands.
- **N-f, the listing's label**: draw it outside the line, so a long one
  cannot wrap a short line; the goto's `goto, line 8 →` is short, but the
  same idea would serve it.
- **break's leave steps** still say "The block ends", where continue's and
  goto's now say which statement leaves the block; changing break's would
  change Part 16's notes, so it was left out of this part.
- **A hidden `static` local** (round 1): name it among the objects a use's
  name hides, once the machine records where its scope ends.

### Review, round 1

The review of the branch at `a48a2f56a` found two blockers, both false
narration about jumps, a citation that claimed more than its paragraph, six
minor findings and some nits. Its 120 probes (`a` to `i`: gotos into, out
of and back over blocks, `continue`, Duff's device, hidden names and pointer
returns) join this part's 65 probes and 13 corpus programs, the 72 corpus
programs before them, the 17 lessons and the regression probes of Parts 15
and 16: 1,142 programs in all. A throwaway engine test dumped every step of
each on `a48a2f56a` and on the fixed branch, and the review's probes, this
part's probes and the corpus were compiled with GCC 14 (`-std=c99 -pedantic
-O0`) and run.

#### 1. A jump into a block said it passed the declarations after its label

A jump into a block said of every object of the block that "the jump passed
their declarations, so no initializer ran". For an object declared after
the label that is false. In

```c
goto in;
{
    int a = 1;
in: ;
    int b = 2;
    return b;
}
```

the jump said so of `a` and `b`, and three steps later the machine ran
`b`'s declaration and its initializer. A switch's jump to a case label in a
block said the same.

**Fixed.** Entering a block still begins the lifetimes of all its objects
(§6.2.4p5); only the declarations before the label are said to be passed:
"`a` and `b` begin to exist here, indeterminate. `a` is declared before the
label: the jump passed its declaration, so no initializer ran, but its name
is in scope at the label and can be assigned; reading it first would read
an indeterminate value. `b` is declared after the label, so the jump did
not pass it: execution reaches it as usual, and only then does its name
come into scope and any initializer run" (§6.8p3). With objects of only
one kind, the step does not name them twice: "The jump passed its
declaration…", or "Its declaration comes after the label…". A new engine
test runs this program as a goto and as a switch, and a block whose only
object comes after the label.

#### 2. A read after a goto back said nothing had been stored

After a goto back over `int k;` made `k` indeterminate again, its read
stopped with "The read would use a value the program never set" and
"nothing has been stored in `k` yet", though the program had stored 4 in it
on the pass before. (GCC 14's build returns that 4.)

**Fixed.** The machine keeps what such a declaration discarded, apart from
the objects' metadata, and the read now says what happened: "a value was
stored in `k` earlier, but execution then reached the declaration of `k` on
line 4 again, and a declaration without an initializer makes the value
indeterminate each time it is reached", led by "The program stored a value
in this object earlier, but reaching its declaration again, with no
initializer, made the value indeterminate once more", citing §6.8p3. A read
of an object that held nothing before the jump back still says "nothing has
been stored". The engine test checks both messages, not only the kind.

#### 3. The dangling pointer's citation

The return step and the stop called any use of a dangling pointer undefined
on the authority of §6.2.4p2, which says only that the pointer's value
becomes indeterminate. Annex J.2 lists "The value of a pointer to an object
whose lifetime has ended is used (6.2.4)" as undefined (p. 492; PDF p.
504). Both steps now cite J.2 beside §6.2.4p2, the return step no longer
cites §6.2.4p2 twice, and §6.2.4p2's gloss says what the paragraph does:
referring to an object outside its lifetime is undefined, and every pointer
to it becomes indeterminate. Item 6 above now cites J.2 as well.

#### 4. The minor findings

- **`break;` advice outside a switch.** A label at a block's end, as in
  `{ goto L; L: }`, was told "write `break;` (or an empty statement, `;`)",
  and there `break;` would itself break §6.8.6.3p1. Only a run of labels
  with a case or default label among them, which must be in a switch, gets
  that advice now; others get "write an empty statement, `;`, after the last
  label".
- **The hover on a goto on its label's line.** `loop: n++; if (n < 3) goto
  loop;` said "jumps ahead" in the listing's hover, from comparing lines
  only. It compares positions now, and says "jumps back"; a new site test
  covers it.
- **"Variably modified type".** The jump's note and §6.8.6.1p1's gloss now
  say what C forbids as the paragraph does: a jump into the scope of an
  identifier with a variably modified type, such as a variable-length array.
- **The limits text** lists among what functions return "a pointer to one
  of these objects, to a pointer or to a function", since `int **f(void)`
  runs; the limits test runs one.
- **A `static` local that a hidden name hides** is still not named. The
  machine resolves a static local's name when it compiles the use and
  keeps no record at run time of where its scope ends, so a note that named
  it could name one declared in a block that has ended. Left for later.

The nits (generic lifetime sentences after leaving a block with no objects,
the generic "until something is stored" beside "it held 50", `int x = x;`'s
note, the repeated hiding paragraph, `continue` through a switch, and the
listing's window at 390 px) are left as they were.

#### What changed in the dumps

Of the 1,142 programs, 1,094 dump byte for byte as before, the 17 lessons
among them. The other 48 change only where intended:

- **26 jumps into blocks** (the corpus's `j08` and `x01`, 13 of the review's
  probes, 7 of this part's, and four copies of the two corpus programs) say
  item 1's new text. All but three have objects only before the label, so
  they read "The jump passed its declaration…"; `e09` and `s01` name `a`
  before and `b` after the label, and `a09` reads "Its declaration comes
  after the label".
- **17 dangling-pointer programs** (the corpus's `u07`, 10 of the review's
  probes, 4 of this part's, and two copies of `u07`) cite J.2 at the return,
  and at the stop where they stop.
- **2 reads after a goto back** (the review's `g06`, this part's `g08`) stop
  with item 2's text.
- **1 refusal** (`l02`, `{ goto L; L: }`) gets the new advice.
- **2 recursions** that end at the engine's 8 MB trace limit reach it one
  step sooner (`r-rv-p1/b17` at 419 of 420, `r-rv-p3/d03` at 476 of 477):
  each retained step now carries the machine's (empty) list of discarded
  values, 24 bytes. Their steps before the limit are the same.

No other program's result changed: every one finishes, stops or is
refused as before, at the same step. Of the 283 programs compiled with GCC
14 (the review's 120, this part's 65, the 85 corpus programs and this
part's own copy of its 13), the 218 that finish in the engine give GCC's
exit status, and the 15 of them that print give its output.

#### Checks

As `pages.yml` runs them: the engine's tests (171: two new, for item 1 and
the `break;` advice; the goto-back test now checks item 2's messages, and
the limits test runs a function that returns `int **`) and its clippy pass;
the site's tests (174, one new for the hover) and the prerender's 2; the
WebAssembly stepper's 79; and the site's four clippy runs, all clean.
`cargo +nightly fmt --check` passes for every changed engine file, every
changed line in the site's files is formatted, and `git diff --check` is
clean. This round took no screenshots: its visible changes are notes'
text and one hover's word, which the dumps and the site test cover.

### Review, round 2

The re-check of the branch at `2450a4d54` found one blocker: round 1's fix
for item 2 held for one jump back but not for two.

#### 1. A value stored two passes back was said never to have been stored

In

```c
int n = 0;
again:;
int k;
if (n == 2) return k;
if (n == 0) k = 4;
n++;
goto again;
```

the read of `k` on the third pass stopped with "nothing has been stored in
`k` yet" and "The read would use a value the program never set", though the
first pass stored 4 in it (GCC 14's build returns that 4). The machine kept,
for each object, only what it held just before the latest pass over its
declaration, and on the third pass that was already indeterminate. The
third pass's declaration step said "it held no value yet" for the same
reason. The opposite mistake came from the same record: no store ever
dropped it, so a structure that was reached again and then assigned a copy
whose member `a` was never set stopped at `s.a` with "a value was stored in
`s.a` earlier, but execution then reached the declaration of `s` on line 5
again", though the copy two steps before had stored that indeterminate
member.

**Fixed.** The machine now keeps a list of leaves, each an object and a
member or element of it, that held a stored value until a pass over their
object's declaration without an initializer made them indeterminate
(§6.8p3). A leaf stays on the list over further passes and leaves it when
something is stored there, by an assignment, an initializer or a copy of a
whole structure. So the first program stops with "a value was stored in `k`
earlier, but execution then reached the declaration of `k` on line 4
again…", citing §6.8p3, and its third declaration step says "it held no
value: the value stored in it earlier became indeterminate when execution
reached this declaration on an earlier pass… so it still holds none" (round
3 dropped "last", which was false from the fourth pass on). The copy's member
gets the usual "nothing has been stored in `s.a` yet", and the copy's own
step already says "Indeterminate members are copied as they are"
(§6.2.6.1p6). A new engine test runs a store and then two jumps back; a
store, a jump back, a new store and a jump back, and a store after the last
pass, which reads back; and the structure copy.

The list holds two numbers per leaf in place of a copy of each object's
old value. When no goto has gone back over a declaration it is empty, so
each retained step still carries 24 bytes for it, and the two recursions
that end at the trace limit end at the same step as after round 1 (`b17`
at 419, `d03` at 476).

#### 2. J.2 at a block's dangling pointer

Round 1 left a pointer to an object of a block that has ended citing only
§6.2.4p2 at its stop. That stop now cites J.2 beside it, as the stop for a
call's dangling pointer does.

The minor finding on order (a jump's note lists the objects declared after
its label in the order they were created, not in source order) is left as
it was.

#### What changed in the dumps

The same 1,142 programs, the re-check's 28 new probes, and 13 new probes of
this round's (repeated passes over scalars, structure members, arrays,
pointers and recursion) were dumped before and after the fix. Of the 1,142,
1,140 dump byte for byte as before, the 17 lessons among them. The others
change only where intended:

- **2 reads after repeated gotos back** (the re-check's
  `b2_twice_store_once`, its declaration step too, and
  `b2_struct_copy_indet`) stop with item 1's text.
- **2 dangling pointers to a block's object** (the review's `g12`, which
  the re-check's probes repeat, and this part's `a16`) cite J.2 at the
  stop.
- **7 of this round's 13 probes** change from "nothing has been stored" to
  item 1's text; the other 6 dump as before.

No program's result or step count changed. Of the 324 programs compiled
with GCC 14, the 235 that finish in the engine give GCC's exit status, and
the 15 of them that print give its output.

#### Checks

As `pages.yml` runs them: the engine's tests (172, one new) and its clippy
pass; the site's tests (174) and the prerender's 2; the WebAssembly
stepper's 79; and the site's four clippy runs, all clean. `cargo +nightly
fmt --check` passes for every changed engine file, and `git diff --check`
is clean.

## Part 18: a lesson on callbacks (2026-11)

Parts 15 and 16 taught the machine pointers to functions but left the lesson
that would show them. A lesson needs its LLVM IR and line tables from a
native bcc build with LLVM and its WebAssembly module from wasi-sdk, and
three times (Parts 15, 16 and 17) the disk was too short for that build
beside the site's. With about
18 GB free, this part adds the lesson "Callbacks: passing a function to a
function" and checks it in every view. The work is on
`agent/lesson-callbacks`.

### The lesson

It follows "Walking a string with a pointer" in Table 1's part on memory,
as lesson 13
([`table1-1024-dark`](screenshots/part18/table1-1024-dark.png)):

```c
// f points to a function that takes
// an int and returns an int.
int sum_of(const int *v, int n,
           int (*f)(int)) {
    int total = 0;
    // A call through the pointer f:
    for (int i = 0; i < n; i++)
        total += f(v[i]);
    return total;
}

int main(void) {
    int v[2] = {3, 4};
    // A function's name becomes a
    // pointer to the function:
    int a = sum_of(v, 2, twice);
    int b = sum_of(v, 2, square);
    printf("%d %d\n", a, b);
    return 0;
}
```

`twice` and `square` stand above it. The run is 187 steps and prints `14
25`, as GCC 14 does. A first draft summed three elements in 247 steps,
longer than any other lesson, so the array has two. Its comments stand on
lines of their own, and every line takes at most 36 columns, so nothing
wraps in the narrow listing. The steps that carry the lesson:

- **Note 15, "`twice` becomes a pointer to the function".** A function is
  code, not an object, so its name used as an operand converts to a
  pointer to it (C99 §6.3.2.1p4, p. 46; PDF p. 58), a value of a type
  derived from the function's type (§6.2.5p20, pp. 35-36; PDF pp. 47-48).
  Its note says why that matters: a table of pointers or a callback lets a
  program choose at run time which code runs. Its value flow was empty, where
  an array's decay two steps earlier draws "v int[2] → decays to → &v[0] int
  *"; it now draws "twice → becomes → &twice", so the pointer can be seen
  appearing
  ([`machine-designator-1440-light`](screenshots/part18/machine-designator-1440-light.png)).
- **Note 16, the call of `sum_of`.** Its parameter `f` gets a copy of the
  pointer, as `v` gets a copy of `&v[0]`: the value flow reads "&v[0] →
  copied to → v", "2 → copied to → n" and "&twice → copied to → f", typed
  `int (*)(int)`.
- **Note 43, "Call twice through `f` (frame #3)".** The expression before
  the parentheses gave a pointer to the function, and the call runs the
  function it points to (§6.5.2.2p1 and p3, p. 71; PDF p. 83). In compiled
  code, the explanation says, the call jumps to the address the pointer
  holds rather than to one fixed when the program was linked. Under the
  head, screen readers hear `f` as "f, int (*)(int), 8 bytes: a pointer to
  the function twice, defined on line 5"
  ([`machine-call-1366-dark`](screenshots/part18/machine-call-1366-dark.png)).
- **Note 126, "Call square through `f` (frame #6)".** The same line, in
  the second call of `sum_of`, runs another function
  ([`machine-square-390-dark`](screenshots/part18/machine-square-390-dark.png)).

The engine's narration needed no change: Part 15's steps already say what
happened to memory and values, the rule, why it matters and what comes
next for each of these. The value flow of note 15 is the one change to the
machine view, and no other lesson has such a step.

### The indirect call in the compiler views

Line 20, `total += f(v[i]);`, is the one place each view shows the call
through a pointer:

- **bcc IR**, line 32: `v13 = call_indirect v2(v12) : (i32 noundef) ->
  i32`, where `v2` is the parameter `f`
  ([`compiler-bcc-ir-1366-dark`](screenshots/part18/compiler-bcc-ir-1366-dark.png)).
- **LLVM IR**, line 32: `%v13 = call i32 %v2(i32 noundef %v12)`, a call of
  a value rather than of a name like `@sum_of`
  ([`compiler-llvm-ir-1024-light`](screenshots/part18/compiler-llvm-ir-1024-light.png)).
  At `-O2` bcc inlines `sum_of` into `main`, where the pointer is a known
  function, so those two copies call `@twice` and `@square` directly; the
  standalone `sum_of` keeps its `call i32 %v2`, since any caller could pass
  any function.
- **x86-64 assembly**, line 79: `callq *%r13`, where the stage's summary
  lists the calls as "*%r13, sum_of and printf"
  ([`compiler-x86-64-assembly-1366-dark`](screenshots/part18/compiler-x86-64-assembly-1366-dark.png)).
- **WebAssembly**: with both compilers, the step reaches `call_indirect`
  with C line 20
  ([`wasm-clang-1440-light`](screenshots/part18/wasm-clang-1440-light.png),
  [`wasm-bcc-390-dark`](screenshots/part18/wasm-bcc-390-dark.png)).

The WebAssembly view's step for `call_indirect` used to say "`call` pops
the arguments", with a sentence on the index after it. It now explains the
instruction itself: "Call `$twice` through the table with `3`", led by
"`call_indirect` pops a table index, `1`, and calls the function stored at
that index of the table, here `$twice`. A C function pointer compiles to
such an index, so this one instruction runs whichever function the pointer
names. It first checks that `$twice` has the type the instruction names,
and traps if not; then it pops the arguments…", and it links the Tables
concept beside Calls and frames. wasi-libc's `printf` calls its stream's
write function the same way, so ten steps in the seven lessons that print
now read the same way: seven "Call `$__stdout_write` through the table with
…", and three, the flush of `stdout` when the program exits, "Call
`$__stdio_write` through the table with …, `0`, `0`". No other step of
theirs changed.

### How the artifacts were made

As in Parts 13 and 14: `web/pipeline/generate.sh` with a dev-profile bcc
(`cargo build --bin bcc-rust`; the script takes any bcc through `BCC`, so
no release build was needed) and `web/wasm/generate.sh` with wasi-sdk 34
(clang 23.1.0). Before the lesson was added, both rewrote every other
lesson's files byte for byte: `git status` showed only the new lesson's
`web/pipeline/callbacks/` and `web/wasm/callbacks.wasm`, and both scripts'
`--check` pass. bcc's own binary of the lesson prints `14 25` and exits 0
at `-O0` and `-O2`.

### Checks

- **GCC 14.** `gcc -B …/gcc14…/14/ -std=c99 -pedantic -Wall -Wextra` (cc1
  14.2.0) compiles the lesson without a warning, and at `-O0` and `-O2` it
  prints `14 25` and exits 0, as the machine, bcc and both WebAssembly
  modules do.
- **The lesson walk.** Part 9's walk at 1440x900 light and 390x844 dark,
  on the deployed site (`main` at `0d5cf90f6`) and on CI's Pages build of
  this branch at `6ae8af61f`: the 15 existing machine lessons' 1,571 notes
  are the same in title, lead, value flow, "why it matters" note and next
  step at both sizes. The new lesson walks 188 states with its current line
  and lead in view at each, and each but the last names the step that
  follows. Neither build scrolled sideways or raised a page error.
- **The scenes.** On the same build, at 1440x900, 1366x768, 1024x768 and
  390x844 in both themes, the machine reached notes 15, 43 and 126, the
  three compiler stages showed their indirect call with no stop, and the
  WebAssembly view
  reached `call_indirect` with each compiler. No sideways scroll, no
  console error or warning.
- **Contrast.** The notes' text measures 15.82:1 (light) and 13.34:1
  (dark); the WebAssembly step's explanation 10.59:1 and 9.40:1.
- **Tests,** as `pages.yml` runs them: the engine's 173 and its clippy, the
  site's (one new, that the lesson's `call_indirect` runs `$twice` twice
  and `$square` twice with the new text, and one extended, for the
  conversion's value flow; and the prerender's 2), the
  WebAssembly stepper's 79, and the site's four clippy runs. The site's
  tests also check that the lesson's module and LLVM IR match its source.
  `rustfmt +nightly --check` reports nothing on the changed lines (the
  site's files have older unformatted lines, as on `main`), and `git diff
  --check` is clean.

### Left for later

- **A dispatch table lesson**: `int (*ops[2])(int, int)` indexed by a
  command, the other common use, which the corpus's `g02` and `g07` show
  through Open .c.
- **The function's type in that value flow**: the engine records no type
  on the conversion, so the chips read "twice → becomes → &twice" without
  "int (int)" and "int (*)(int)" beneath them, as the call's chips have.
- Part 17's list stands, less the callbacks lesson: among others, the
  lessons' opening comments, static pointers to objects, hiding a name with
  a constant or a typedef name, the listing's label, and a lesson on the
  three loops. Part 17 finished a function returning a pointer to an object
  and hidden names, which Part 16's list carried.

## Part 19: loops three ways, and the LLVM IR's block count (2026-11)

Part 17 taught the machine `do` and `continue`, but no lesson used either:
the corpus's `j01` to `j03` showed them only through Open .c. This part adds
the lesson "Loops three ways: while, do and for", fixes the LLVM IR stage's
count of basic blocks, which the review of Part 18 found too low, and
corrects Part 18's record. The work is on `agent/site-part19`.

### The lesson

It ends Table 1's first part, after "Short-circuit evaluation", as lesson 6
([`table1-1024-dark`](screenshots/part19/table1-1024-dark.png)). It computes
one sum, 1 + 3, three times, and each loop skips 2 with `continue`:

```c
int a = 0;          // while
int i = 0;
while (i < 3) {
    i++;
    if (i == 2)
        continue;
    a += i;
}

int b = 0;          // do
int j = 0;
do {
    j++;
    if (j == 2)
        continue;
    b += j;
} while (j < 3);

int c = 0;          // for
for (int k = 1; k <= 3; k++) {
    if (k == 2)
        continue;
    c += k;
}
printf("%d %d %d\n", a, b, c);
```

(The comments here are shortened; the lesson's own say where each loop
tests and where its `continue` goes.) A first draft put each loop in its own
function and made four passes: 312 steps, more than any other lesson, so
the loops moved into `main` and make three passes each. The run is 215
steps, second to "Floating point" (230), and prints `4 4 4`. Every line
takes at most 36 columns. The engine needed no change: Part 17's steps
already say where each loop tests and where each `continue` goes.

- **Note 47, `continue` in the while**: "`continue` skips the rest of this
  pass through the while loop's body, so control goes straight to the
  condition `i < 3`" (§6.8.6.2p2), with the note on C's three loops
  ([`machine-whilecontinue-1366-dark`](screenshots/part19/machine-whilecontinue-1366-dark.png)).
- **Note 80, "do: run the body before any test"**: "A do statement runs its
  body first and tests `j < 3` only after it, so the body runs at least
  once" (§6.8.5.2p1)
  ([`machine-do-390-dark`](screenshots/part19/machine-do-390-dark.png)).
  Notes 98 and 115, "Condition is true: run the body again", come after the
  body, where the while's and the for's tests come before it.
- **Note 110, `continue` in the do**: "…goes to the test `j < 3` after the
  body".
- **Note 176, `continue` in the for**: "…goes to the for statement's third
  clause, `k++`, and then to the condition"; notes 177 to 179 then run
  `k++` and jump back to the test
  ([`machine-forcontinue-1440-light`](screenshots/part19/machine-forcontinue-1440-light.png)).

### The three loops in the compiler views

The same three shapes appear in every view. Clicking line 26, `} while (j
< 3);`, lights up the do loop's test in each:

- **bcc IR**: 17 basic blocks. The while's test block, `block1`, comes
  first, and its `continue` block, `block3`, is `jump block1(v7, v3)`,
  straight back to the test. The do's body, `block6`, comes first and its
  test block, `block9`, last, with `brif v22, block6(v16, v20), block10`
  back to the body (lines 41 to 43 for line 26). The for's `continue`
  block, `block13`, jumps to `block15`, which runs `k++` and then jumps to
  the test, `block11`
  ([`compiler-bcc-ir-1440-light`](screenshots/part19/compiler-bcc-ir-1440-light.png)).
- **LLVM IR**: the same 17 blocks as `br` instructions, with `; preds`
  comments that name each block's predecessors: `block9:  ; preds =
  %block7, %block8` is where the do's two paths through its body meet
  ([`compiler-llvm-ir-1024-light`](screenshots/part19/compiler-llvm-ir-1024-light.png)).
  At `-O2` bcc keeps the three loops but turns the do's and the for's `if
  (…) continue;` into a `select`, so the do loop is one block that branches
  to itself: 8 blocks.
- **x86-64 assembly**: labels and jumps. The do's test is `cmpl $3, %eax`
  and `jl .LBB0_5`, back to the body
  ([`compiler-x86-64-assembly-1440-light`](screenshots/part19/compiler-x86-64-assembly-1440-light.png)).
  The native back end also copies the while's and the for's test to the end
  of each pass (`cmpl $3, %eax; jl .LBB0_2`), so in assembly all three
  loops end a pass with a conditional jump back; the while's `continue`
  still jumps to the test at the top, `je .LBB0_1`.
- **WebAssembly**: in Clang's module the while and the for are each a
  `block` around a `loop` whose test leaves with `br_if 1` and whose pass
  ends with `br 0` back to the test; the do is a bare `loop` whose pass ends
  with `br_if 0` back to the top
  ([`wasm-clang-1366-light`](screenshots/part19/wasm-clang-1366-light.png)).
  bcc's module builds the same tests from `if` and `br`
  ([`wasm-bcc-390-dark`](screenshots/part19/wasm-bcc-390-dark.png)).

The WebAssembly view's step for `loop` said "A C `for` or `while` loop
becomes a `loop` whose body ends in a conditional branch back to the top".
Unoptimized, neither compiler's module does that; only a do loop does. The
step now reads "Without optimization, a C `while` or `for` loop tests its
condition at the top of the `loop` and ends each pass with a `br` back to
that test; a `do` loop tests at the bottom, so its pass ends in a
conditional branch back to the top". That sentence is the one change to
the other lessons' WebAssembly steps: every "Enter loop" step, in the
lessons' loops and in the C library's, reads it now. A new site test steps
the lesson's Clang module and checks its three `loop`s, six plain branches
back (three passes of the while and of the for) and two taken `br_if`s
back (the do's second and third passes), and its output.

### The LLVM IR's block count

The LLVM IR stage's "In this program" line, and the `-O0`/`-O2` comparison
under it, counted a block only when its label line ended in `:`. LLVM
writes `; preds = …` after the label of every block that another block
branches to, so only each function's entry block was counted: the callbacks
lesson read "27 instructions in 4 basic blocks" where its listing has 8.
`Census::of` now takes a line whose first word ends in `:` and is followed
by nothing or a comment as a label. The bcc IR's count was right: its
labels never carry a comment.

A new site test checks the count for every lesson at `-O0` and `-O2`, in
bcc IR and in LLVM IR, against the listing's terminators (each basic block
ends in exactly one), and pins the callbacks lesson's line: "Here the LLVM
IR is 27 instructions in 8 basic blocks, with 1 stack slot and 3 loads or
stores". In the browser the callbacks lesson reads that line, and the loops
lesson reads 17 blocks at `-O0` and 8 at `-O2` in both IRs, at every size.

What the LLVM IR stage now says, lesson by lesson (blocks at `-O0` / `-O2`;
the instruction, stack slot and memory counts did not change):

| Lesson | Before | After |
| --- | --- | --- |
| Objects, loops and calls | 2 / 2 | 6 / 5 |
| Hello, world | 2 / 2 | 5 / 8 |
| Names for values | 1 / 1 | 5 / 4 |
| Recursion and frames | 2 / 2 | 4 / 4 |
| Short-circuit evaluation | 1 / 1 | 14 / 8 |
| Unsigned numbers wrap | 1 / 1 | 8 / 7 |
| Signed overflow | 1 / 1 | 5 / 4 |
| Floating point | 1 / 1 | 5 / 4 |
| Pointers and arrays | 2 / 2 | 5 / 8 |
| Walking a string with a pointer | 2 / 2 | 8 / 10 |
| Callbacks | 4 / 4 | 8 / 13 |
| Before the compiler: the preprocessor | 1 / 1 | 6 / 1 |

The other six (Integer promotions, Bits and masks, Structs, Reading past
the end, Uninitialized reads and From source to a running program) have
one block per function and read as before. Where the two counts now differ,
the comparison gains a line, "Basic blocks: 8 at -O0, 13 at -O2.", and its
bars show the new counts; a count that falls also gets "Fewer blocks means
simpler control flow."

### Part 18's record

Part 18 is corrected where the review of #440 found it wrong: three of the
ten changed `printf` steps call `$__stdio_write`, the flush at exit, not
`$__stdout_write`; there were 15 existing machine lessons, not 16; the
engine's tests on the merged head number 173, not 160; its introduction
counts three deferrals, not two; and its "Left for later" no longer lists
a function returning a pointer to an object and hidden names, which Part 17
finished.

### How the artifacts were made

As in Part 18: `web/pipeline/generate.sh` with a dev-profile bcc (`cargo
build --bin bcc-rust`) and `web/wasm/generate.sh` with wasi-sdk 34 (clang
23.1.0). Before the lesson was added, both scripts' `--check` passed on
`main`, so they reproduce every existing lesson byte for byte; with it, `git
status` showed only `web/pipeline/loops/` and `web/wasm/loops.wasm`.

### Checks

- **Agreement.** The machine, bcc's binary at `-O0` and `-O2`, bcc's
  WebAssembly module and Clang's (each run under Node's WASI), and GCC 14
  (`gcc -B …/gcc14…/14/ -std=c99 -pedantic -Wall -Wextra`, cc1 14.2.0, no
  warnings) at `-O0` and `-O2` all print `4 4 4` and exit 0.
- **The lesson walk.** Part 9's walk at 1440x900 light and 390x844 dark,
  on the deployed site (`main` at `c6f91935d`) and on CI's Pages build of
  this branch at `f30b3259f`: the 16 existing machine lessons' 1,759 notes
  are the same in title, lead, value flow, "why it matters" note and next
  step at both sizes. The new lesson walks 216 states with its current line
  and lead in view at each but note 0, which has no current line in any
  lesson. Neither build scrolled sideways or raised a page error. The
  machine's notes are the walk; the summary counts above and the `loop`
  step's sentence are the only intended changes elsewhere.
- **The scenes.** On the same build, at 1440x900, 1366x768, 1024x768 and
  390x844 in both themes, the machine reached notes 47, 80, 110 and 176;
  the bcc IR, LLVM IR and assembly stages found the do loop's test with no
  stop; and the WebAssembly view reached the do's `loop` with each
  compiler. No sideways scroll, no console error or warning.
- **Contrast.** The notes' text measures 15.82:1 (light) and 13.34:1
  (dark); the WebAssembly step's explanation 10.59:1 and 9.40:1.
- **Screen readers.** The lesson adds no control. Its card reads as the
  others do, and in the third loop the cells under the head read "a, int,
  4 bytes: 4" … "k, int, 4 bytes: 2, read by this step", one per object,
  so `i`, `j` and `k` are told apart by name.
- **Tests,** as `pages.yml` runs them: the engine's 173 and its clippy; the
  site's 177 (two new: the block count and the loops lesson's WebAssembly)
  and the prerender's 2; the WebAssembly stepper's 79; and the site's four
  clippy runs, all clean. The site's tests also check that the lesson's
  module and LLVM IR match its source. `rustfmt +nightly --check` reports
  nothing on the changed lines, and `git diff --check` is clean. Both
  generators' `--check` pass.

### Left for later

- **The assembly's copied tests**: the native back end's copy of a while's
  or a for's test to the end of each pass is visible in the listing but
  explained nowhere; the assembly stage could say so for a loop.
- **The `loop` step at `-O2`**: the new sentence describes unoptimized
  modules, which are all the WebAssembly view shows today.
- **A `switch` in LLVM IR**: the census counts a `switch`'s case lines as
  instructions. No lesson has one.
- **`continue`'s explanation** still says "the next pass creates fresh
  ones" when the blocks it left declared nothing (Part 17's nit).
- Part 18's list stands: a dispatch table lesson and the function's type
  in the conversion's value flow, with Part 17's list (the lessons' opening
  comments, static pointers to objects, hiding a name with a constant or a
  typedef name, the listing's label, and others).

## Part 20: a claims audit of Parts 11 to 19, and the review of Part 19 (2026-11)

Parts 11 to 19 taught the machine a dozen constructs and added five
lessons, each sentence checked by its own part's review but never against
the compilers as a whole. This part audits every claim they added, in the
manner of [the claims audit](claims-audit.md), and fixes the three nits the
review of Part 19 (#441) left. The work is on `agent/site-part20`; the
claims audit's new section, [Parts 11 to 19](claims-audit.md#parts-11-to-19-2026-11-23),
holds the full table.

### The review's three nits

- **The WebAssembly view's provenance.** It said Clang built every
  lesson's module "without a C library". The nine lessons that include
  `<stdio.h>` are WASI commands linked with wasi-libc
  (`web/wasm/generate.sh`), so for them it now reads "The lesson calls
  `printf`, so Clang linked it with wasi-libc, the C library for WASI,
  whose `$_start` calls `main`", and the sentence about bcc's `$_start`
  keeps its "also" only there. The other ten keep "without a C library, so
  the module starts in `main`".
- **The `loop` step.** "Its pass ends in a conditional branch back to the
  top" was a literal `br_if` only in Clang's module. It now reads "a branch
  back to the top that is taken only while the condition holds: a `br_if`,
  or a `br` inside an `if`", true of both modules.
- **The loops' own clauses.** No step cited §6.8.5.1 (`while`) or §6.8.5.3
  (`for`), though note 176's "goes to `k++`" rests on §6.8.5.3p1. Two new
  clauses, checked with `search-standard` (both p. 136, PDF p. 148), are
  now cited beside §6.8.5p4 on each loop's test and jump back, on each
  `continue` (a `do`'s cites §6.8.5.2p1), on the step before a test
  ("test the `for` condition"), and on a for statement's block, whose first
  clause's names §6.8.5.3p1 scopes to the loop. A new engine test pins
  them for a while, a for and a for without a condition.

### The claims audit

52 claims: 36 hold, 12 over-promised and 4 were wrong. Probes `c29` to
`c37` in `web/docs/claims/` show what GCC 14, Clang 23.1.5 and bcc do. The
fixes:

- **Calls through a bad pointer to a function.** The note and the stop
  said a compiled program "usually crashes". A null call crashes without
  optimization, but Clang and bcc at `-O2` exit normally (44 and 0), and a
  call through the wrong type runs on and prints 21. Both now say so.
- **Static initializers.** "Reading a variable … is refused": GCC and
  Clang compile `static const int a = 1; int b = a;` silently, even with
  `-pedantic`, as §6.6p10 lets them. The note and the machine's refusal now
  say so.
- **What a function pointer is in compiled code.** Every call through a
  pointer said the pointer "holds the address where twice's machine code
  starts", which the WebAssembly view contradicts two tabs away (an index
  into a table). It now gives both.
- **Types that differ on Windows.** ptrdiff_t and size_t are long types on
  x86-64 Linux, but long long ones for `x86_64-pc-windows-msvc`; both now
  say "on x86-64 Linux" (size_t's was older text).
- **Glosses.** Three of the 74 new clause glosses left out cases of their
  paragraphs (`?:` with void, `==` with a pointer to void, two pointers
  equal across adjacent arrays), and the glossary's "no current machine
  uses" the other representations was not checkable; it now says C23
  requires two's complement.
- **Smaller ones.** The loop note now says a for's `continue` runs its
  step first; an enumeration without a name was "The type  itself" and is
  now "The enumeration's type"; the switch and hiding notes name Clang's
  options beside GCC's; and a refusal still said "nested
  assignments/increments … are rejected", which Parts 13 and 14 ended.

What holds includes every C rule the 17 new notes state, the "Limits of
this model" paragraph (36 programs through the engine, each run or refused
as it says), the landing, the five lesson cards, the WebAssembly view's
`call_indirect` step, the enumeration types (GCC, Clang and bcc agree on
`c29`), and the open-order stop (GCC prints 11 where Clang and bcc print
1). All five new lessons print what the machine prints under the three
compilers at `-O0` and `-O2`, with no warning under `-std=c99 -pedantic
-Wall -Wextra`.

### Checks

- **The lesson walk.** The engine's narration of every lesson was written
  out before (`main` at `40c2e52bd`) and after the changes: title, lead,
  explanation, "why it matters" note, clauses and next step for each of
  the 18 lessons the machine runs (the preprocessor lesson stops before its
  first step on both). Every lesson keeps its step count. The text changed
  only where intended: Loops three ways, notes 47, 80, 110 and 176 (the
  note on loops); Callbacks, notes 15 and 98 (the note on pointers to
  functions) and 43, 71, 126 and 155 (a call through `f`); Walking a
  string, note 145 (ptrdiff_t); Unsigned, note 146 (size_t). Clauses
  changed on the loop steps of the 11 lessons with a while or a for. This
  walk reads the engine directly, not the page, so it does not compare the
  value flow, which no change touches.
- **The scenes.** CI's Pages build of `07036a217`, at 1440x900 and 390x844
  in both themes: loops note 176
  ([`machine-loops-176-1440-light`](screenshots/part20/machine-loops-176-1440-light.png))
  cites §6.8.6.2p2, §6.8.5.3p1 and §6.8.5p5; callbacks note 43 reads "In
  x86-64 code … ; in WebAssembly it holds an index into a table of
  functions"
  ([`machine-callbacks-43-390-dark`](screenshots/part20/machine-callbacks-43-390-dark.png));
  and the WebAssembly view's provenance and "Enter loop" step read as
  above with each compiler for the loops lesson
  ([`provenance-loops-clang-390-dark`](screenshots/part20/provenance-loops-clang-390-dark.png),
  [`provenance-loops-bcc-1440-light`](screenshots/part20/provenance-loops-bcc-1440-light.png),
  [`wasm-loops-bcc-390-light`](screenshots/part20/wasm-loops-bcc-390-light.png),
  [`wasm-loops-clang-1440-dark`](screenshots/part20/wasm-loops-clang-1440-dark.png))
  and for the objects lesson, which has no C library
  ([`provenance-objects-clang-1440-dark`](screenshots/part20/provenance-objects-clang-1440-dark.png)).
  The first build showed bcc's module of the loops lesson without the
  sentence's "also": the provenance read the C library from the shown
  module's command line, not from the lesson. It now reads the lesson. No
  sideways scroll, no console error or warning. The changed text keeps its
  elements and colors, so the contrast was not measured again.
- **Agreement.** The five new lessons under the machine, GCC 14, Clang
  23.1.5 and bcc at `-O0` and `-O2`, as in the claims audit.
- **Tests,** as `pages.yml` runs them: the engine's 175 (two new: the
  loops' clauses and an enumeration without a name; the `continue` test now
  checks each loop's own clause) and its clippy; the site's 177 and the
  prerender's 2; the WebAssembly stepper's 79; the site's four clippy runs,
  all clean. Both generators' `--check` pass with a dev bcc and wasi-sdk
  34. `cargo +nightly fmt --check` passes for the engine, rustfmt reports
  nothing on the site's changed lines, and `git diff --check` is clean.

### Left for later

- **bcc and `-Wimplicit-fallthrough`.** bcc does not know the option, so
  the switch note names GCC's and Clang's only.
- **`x & 1` for a negative x** assumes two's complement; the note leaves
  that unsaid, as every machine the site describes uses it.
- **A Playwright walk of this change.** The walk above reads the engine's
  steps; Part 9's walk of the page was not run, since the page shows those
  same strings and nothing in its layout changed.
- Part 19's list stands: the assembly's copied loop tests, the `loop` step
  at `-O2`, a `switch` in LLVM IR, `continue`'s "fresh ones", a dispatch
  table lesson and the function's type in the conversion's value flow,
  with Part 17's list behind them.

## Part 21: the remaining loose explanations (2026-11)

The reviews of Part 20 (#442) and of its results file (#443) left six
non-blocking findings, all fixed here on `agent/site-part21`. The claims
audit's [Part 21's follow-ups](claims-audit.md#part-21s-follow-ups-2026-11-23)
holds the evidence. Citations were checked with `search-standard`, and the
compiler claims against GCC 14, Clang 23.1.5 and a dev build of bcc in C99.

### The fixes

- **The `?:` gloss** (§6.5.15p3) let any pointer pair with a pointer to
  void. The paragraph allows only a pointer to an object or incomplete
  type, so a function pointer is excluded. GCC (`-pedantic`) and Clang
  warn about `c ? fp : v` but not about an object pointer. The gloss now
  says so. The `==` constraint's gloss (§6.5.9p2) also left out
  incomplete types, and now includes them.
- **Pointer equality** (§6.5.9p6) now follows the paragraph: a pointer to
  an object equals a pointer to a subobject at its start, and a pointer
  one past an array equals a pointer to the start of a different array
  that immediately follows it.
- **A call through the wrong function type.** The note on pointers to
  functions and the stop at such a call gave "the caller and the function
  disagree about where the arguments and the result are" as the reason.
  A new probe, `c38`, calls `int (*)(int)` as `unsigned (*)(unsigned)`.
  Both types use the same registers, and the probe prints 42 under all
  three compilers at `-O0` and `-O2`. Both texts now say the call is
  undefined (§6.5.2.2p9) even when it seems to work, and give the
  disagreement only as one possible consequence. A test pins the stop's
  wording.
- **Sizes "on x86-64".** For `x86_64-pc-windows-msvc`, a long and an
  unsigned long are 4 bytes and a long double is 8 (LLP64). For
  x86_64-linux-gnu they are 8 and 16 (LP64). Each statement of a size,
  range, alignment, format or type choice that differs between the two
  now says "on x86-64 Linux", as ptrdiff_t and size_t already did; later
  steps that only repeat a type already stated are not tagged (review
  round 2, "Left for later"). Review round 1
  (below) finished the engine's part of that. It covers:
  - every object's declaration;
  - `sizeof`;
  - a structure's layout and a structure assignment;
  - the cast to char;
  - the notes on promotion, pointer arithmetic, int's range, padding and
    the floating sizes;
  - the glosses of §6.7.2.1p12 and §F.2p1;
  - the glossary's alignment, unsigned type and floating type;
  - the datasheet's byte counts;
  - "Limits of this model";
  - the rank panel's sentences that depend on long's width.

  Statements about the processor itself are unchanged, and are the same
  on every x86-64 target: its shift and division instructions, its
  default NaN, FLT_EVAL_METHOD 0 and two's complement.
- **`run.sh`** now takes three things:
  - `STD`, the language standard. It defaults to `c17`, the standard
    `results.txt` was made with. A default run does not give that file
    back: the claims audit lists what differs (corrected in review round
    1, below).
  - `EXTRA`, options added after `-Wall -Wextra`.
  - The names of the probes or lessons to run.

  The script's header and both audit method sections document it.
- **The warning rows.** `results-part21.txt` records two runs:
  - `c34` with `-Wimplicit-fallthrough`: GCC and Clang warn, and bcc
    reports the option unknown.
  - `c35` with `-Wshadow`: all three warn.

  The `c34` and `c35` rows now cite the file that backs each half.

### Checks

- **The lesson walk.** A throwaway test stepped all 19 lessons on `main`
  (`d623744e2`) and on this branch, plus a wrong-type call. It wrote each
  step's title, lead, explanation, note, clauses, next step, diagnostics
  and output, and each cited clause's gloss. Every step count is the same.
  The preprocessor lesson is refused before its first step on both
  ("line splicing is outside this subset"). Every changed line differs
  only by the added " Linux", except the note on pointers to functions
  (Callbacks, notes 15 and 98) and the wrong-type call's stop. Explanations
  changed at these steps:

  | Lesson | Steps | What |
  | --- | --- | --- |
  | Bits and masks | 7, 26 | an unsigned int and an int declared |
  | Reading past the end | 2, 12 | an array of ints, an int * |
  | Callbacks | 4, 11, 18, 23, 94, 101, 106 | an array of ints, ints |
  | Names for values | 6, 10, 15, 84 | ints, the cast to char |
  | Floating point | 9, 13, 17, 44, 62, 67, 212, 216 | ints, doubles |
  | Hello, world | 11, 16 | ints |
  | Uninitialized reads | 2, 3 | ints |
  | Short-circuit evaluation | 7, 17, 84 | an array of ints, an int, an int * |
  | Loops three ways | 4, 8, 72, 76, 134, 139 | ints |
  | Objects, loops and calls | 2, 6, 16, 23, 33, 39, 87, 93, 141, 147 | ints, an array of ints, an int * |
  | Signed overflow | 2, 7 | ints |
  | From source to a running program | 2, 6 | ints |
  | Pointers and arrays | 2, 15, 19, 31 | an array of ints, int *s, an int |
  | Integer promotions | 6 | an int |
  | Recursion and frames | 2, 12, 25, 38 | ints |
  | Walking a string | 6, 10, 91 | char *s |
  | Structs and static storage | 2, 5, 10, 44, 59, 76, 119 | struct layouts, a struct point *, a structure assignment |
  | Unsigned numbers wrap | 6, 22, 70, 80, 85, 144, 146, 148, 156 and 162 | unsigned ints, an array, the size_t object (unsigned long), sizeof of int[3], an int |

  Notes changed at these steps. The site shows a note the first time
  its topic appears.

  | Note | Lessons and steps |
  | --- | --- |
  | Pointers to functions | Callbacks 15, 98 |
  | Int's range | Callbacks 48, 76, 132, 161; Objects 52, 65, 78, 106, 119, 132, 160, 173, 186; Signed overflow 21; From source 11, 18; Pointers and arrays 49, 73, 97, 121; Integer promotions 14, 24, 33; Recursion 17, 30, 43, 59, 66, 73; Structs 23, 29, 38, 57, 74, 99, 105, 114, 126, 129, 133 |
  | Pointer arithmetic | Reading past the end 16; Objects 20, 71, 125, 179; Pointers and arrays 23, 55, 79, 103, 127 |
  | Promotion | Names for values 89; Hello 26, 41, 56, 71; Integer promotions 10, 13, 20, 23, 32; Walking a string 22, 37, 52, 67, 82 |
  | Floating sizes | Floating point 17, 44, 62, 212 |
  | Padding | Structs 76 |

  Only two cited glosses changed in any lesson: §6.7.2.1p12 in Structs
  and static storage, and §F.2p1 in Floating point. No lesson cites the
  `?:`, `==` or pointer equality glosses, so those were checked against
  their paragraphs only. No clause list or next step changed.
- **Tests,** as `pages.yml` runs them:

  | Check | Result |
  | --- | --- |
  | Engine tests | 175 passed (one test also checks the stop's new wording) |
  | Engine clippy | clean |
  | Site tests | 177 passed, plus the prerender's 2 |
  | WebAssembly stepper tests | 79 passed |
  | Site clippy: native, and wasm32 `hydrate`, `worker` and `compiler` | all four clean |
  | `web/wasm/generate.sh --check` (wasi-sdk 34) | passes |
  | `web/pipeline/generate.sh --check` (this branch's dev bcc) | passes |
  | `cargo +nightly fmt --check` | passes |
  | `git diff --check` | clean |

  rustfmt reports nothing on the site's changed lines.

### Left for later

- **Doc comments.** The engine's own doc comments still say "on x86-64",
  for example `integers.rs`'s "Size in bytes". The site never shows them.
- **§6.5.9p7** (an object outside an array counts as an array of one) has
  no gloss of its own. No step cites it.
- **A Playwright walk.** Part 9's walk of the page was not run: the page
  shows the strings the lesson walk compared, and no layout changed.
- Part 20's list stands.

### Review, round 1

The review of #445 found two blockers and three nits, all fixed here.

- **LP64 facts with no target.** The engine still stated a long's range
  and a constant's type without naming x86-64 Linux, in arithmetic ("The
  result fits in a long (-9223372036854775808 to 9223372036854775807)"),
  in initialization and assignment ("it fits in -9223372036854775808 to
  9223372036854775807") and for integer constants ("The integer constant
  3000000000 has type long"). Each is false on `x86_64-pc-windows-msvc`.
  A throwaway engine test then ran programs using `long`, `unsigned long`,
  `long long`, `long double`, large constants, `sizeof`, `size_t` and
  `ptrdiff_t` through every step kind, before and after, and every
  sentence it printed was read. These now name x86-64 Linux when they
  depend on the data model:
  - a long's or an unsigned long's range, wherever a step states it:
    arithmetic, unsigned wrapping, conversions on initialization,
    assignment and cast, increments, compound assignment and the stops
    for signed overflow, a too-wide shift and a floating value out of
    range;
  - its width: the bitwise operators' and `~`'s "64 bits", a shift's
    "64-bit unsigned long" and the "modulo 2^64" of unsigned wrapping and
    conversion;
  - the type a constant or the usual arithmetic conversions choose: a
    constant's type when its value needs more than 32 bits, a comparison's
    common type, and "is wider (64 bits against 32)" or "no wider (64 bits
    each)";
  - `sizeof`'s size, an array's element size and a member's offset, when
    the type is or holds a long, an unsigned long or a long double;
  - a long double's 64 bits of precision, its largest value in the stop
    for a constant out of range, and an integer it holds exactly only
    because of those 64 bits;
  - the §6.3.1.8p1 gloss's "long long and unsigned long give unsigned
    long long".

  A bare "fits" ("4294967295 fits in long") and a constant's or
  conversion's type are tagged only for a value beyond 32 bits, which a
  4-byte long would not hold, so `10L` and `a + 1` read as before. Facts
  about char, short, int and their unsigned types, and about float and
  double, are the same on every x86-64 target and are unchanged. A new
  test pins one sentence of each kind. Clang 23.1.5 confirms each changed
  claim with `_Static_assert`s on both targets: `LONG_MAX`, `ULONG_MAX`,
  `sizeof(long)`, `sizeof(long double)`, `LDBL_MANT_DIG`, the member
  offsets of `struct { char c; long l; long double d; unsigned long u; }`,
  the types of `3000000000`, `2147483648`, `040000000000`,
  `18446744073709551615u` and `0xFFFFFFFFFFFFFFFF`, of `1L + 1u` and
  `1LL + 1UL`, and of `size_t` and `ptrdiff_t`. Converting
  18446744073709551615u to long double draws
  `-Wimplicit-const-int-float-conversion` only for the Windows target.
- **`run.sh`'s reproducibility.** A default run does not give
  `results.txt` back. The claims audit now lists what differs: for Clang,
  the values the undefined probes `c03b`, `c04b` and `c07` print, the
  `indeterminate` lesson's exit status at `-O0`, and every probe and
  lesson added since; for GCC and bcc, also the `indeterminate` and
  `bounds` exit statuses, a warning's line number, bcc's
  `-Wuninitialized` warnings and the temporary paths in two errors.
- **Nits.**
  - `web/README.md` says x86-64 Linux width.
  - The §F.2p1 gloss followed only half the paragraph. Annex F makes long
    double "an IEC 60559 extended format, else a non-IEC 60559 extended
    format, else the IEC 60559 double format" (`search-standard`, N1256
    p. 444; PDF p. 456). The gloss now says "an extended format if there
    is one, else double precision: on x86-64 Linux the x87 80-bit format,
    and with Microsoft's compiler double".
  - `sizeof`'s result type is labelled `size_t`, not `unsigned long`. The
    site shows the label only for conversions, so no lesson changes.

#### Checks

- **The lesson walk.** The same throwaway test stepped all 19 lessons
  before and after. Every step count is the same, and no step's title,
  lead, explanation, note, clause list, next step, diagnostic or output
  changed. Two cited glosses changed: §6.3.1.8p1 in Bits and masks,
  Callbacks, Floating point, Objects, Signed overflow, From source,
  Pointers and arrays, Integer promotions, Recursion, Structs and
  Unsigned numbers wrap; and §F.2p1 in Floating point.
- **Tests,** as `pages.yml` runs them, with `ci.yml`'s pipeline check:

  | Check | Result |
  | --- | --- |
  | Engine tests | 176 passed (175 and the new test) |
  | Engine clippy | clean |
  | Site tests | 177 passed, plus the prerender's 2 |
  | WebAssembly stepper tests | 79 passed |
  | Site clippy: native, and wasm32 `hydrate`, `worker` and `compiler` | all four clean |
  | `web/wasm/generate.sh --check` (wasi-sdk 34) | passes |
  | `web/pipeline/generate.sh --check` (this branch's dev bcc) | passes |
  | `cargo +nightly fmt --check` | passes |
  | `git diff --check` | clean |

#### Left for later

- **Type labels.** The engine models `size_t` as an unsigned long and
  `ptrdiff_t` as a long, so a step's upcoming label can still read
  "cast `(q - p)` from `long` to `int`", and `sizeof n` for a `size_t n`
  says "the size of an unsigned long". Both are the types on x86-64
  Linux, and the step's sentence names it, but the label does not.
- **Values.** Every value the machine shows for a long or a long double
  (a sum, a rounded quotient, the low bits a conversion keeps) is the
  value on x86-64 Linux; the steps tag the facts the values follow from,
  not each value.
- **Repeated types (review round 2).** Later steps that repeat a type the
  machine already stated, without restating the rule, are not tagged,
  though each is false on x86_64-pc-windows-msvc, where `3000000000` and
  `2147483648` are long long: "The value to store is the long
  3000000000", "Unary minus negates its long operand: -(2147483648)",
  the printf check's "`3000000000`, is a long", "`3000000000 > 2` mixes
  an int and a long", "`(long)3e9` … converts to the long 3000000000"
  (whose conversion is undefined on Windows), and upcoming labels such as
  "cast `18446744073709551615u` from `unsigned long`". No lesson reaches
  them; edited code does.

## Part 22: the compiler views, with fresh eyes (2026-11)

Parts 11 to 21 worked on the abstract machine's explanations. The compiler
views had their last visual pass in Part 10: bcc IR, the ABI-lowered IR,
LLVM IR, the x86-64 assembly, the machine code and the WebAssembly text,
all coloured by the Monaco grammars in `web/editor/`. This part walks them
as a newcomer would, on `agent/site-part22`, and fixes the ten problems
that hurt most.

### Method

The deployed site (`main` at `d7d3e133a`) was driven with Playwright in
Chromium, one browser at a time, with reduced motion. For each lesson the
script opened the compiler pipeline and took the stage card at every
back-end stage (bcc IR, ABI-lowered IR, LLVM IR, x86-64 assembly, machine
code), each at `-O0` and `-O2`, and then the WebAssembly view:

- at 1440x900, every lesson, in both themes (418 states);
- at 1366x768 and 390x844, in both themes, five lessons that cover the
  shapes the views take (Objects, Floating point, Callbacks, Structs and
  From source to a running program): 220 states.

On each state it recorded whether the page scrolls sideways, every element
of the stage card past the window's right edge, the WCAG contrast of every
token colour Monaco drew against the screen, the stage's explanatory text,
and how far the page jumped when the stage changed. No state scrolled
sideways, no page logged an error, and every token colour is at least
6.88:1 on the light theme's screen (`#0b0b0b`) and 7.12:1 on the dark
theme's (`#050505`): those are the grey of comments and directives, and
every other colour is 8.96:1 or more. I read the
screenshots at full size.

The fixes were checked against the output they describe: the dev build's
listings for every lesson at both levels (`web/src/explorer.rs` natively),
the bundled LLVM IR, `middle-end.md` for bcc IR, the LLVM Language
Reference for `load`, `align` and `nsw`, and the Intel SDM (Vol. 2) for the
encodings, as each item says.

### The problems, worst first

1. **Nothing said how to read a line, or what the colours mean.** The only
   legend under the output was for the links to the source. A newcomer
   met `v3 = iadd.i32 nsw v1, v2`, `%v1 = load i32, ptr %v0, align 4`,
   `movl %eax, (%rdi)` and `5  89 07  movl %eax, (%rdi)` with no key, and
   the colours (orange, blue, teal, khaki, orchid, pink) were never named
   ([before](screenshots/part22/before-bcc-ir-1440-dark.png)). **Fixed:**
   under the output of each of the five stages a figure, "How to read a
   line", writes one line of that notation in its colours, says what it
   does, lists each part beside its meaning, and keys the view's other
   colours with a sample of each
   ([after](screenshots/part22/after-bcc-ir-1440-dark.png),
   [phone](screenshots/part22/after-machine-code-390-dark.png)). The
   examples are lines of the first lesson's output, and a test pins each
   one to that output and each key's sample to some lesson's view. The
   claims:
   - bcc IR: `nsw` lets bcc assume the sum fits, since signed overflow is
     undefined (`middle-end.md`, "Undefined behaviour": C lowering sets
     `nsw` on signed `+ - *` because of C99 §6.5p5, and an overflowing
     `nsw` add is poison).
   - LLVM IR: `load <ty>, ptr <pointer>, align <n>` reads a `<ty>` at the
     pointer, and `align` promises the address's alignment (LangRef,
     `load`); `#0` names the attribute group the file ends with
     (`attributes #0 = { "frame-pointer"="all" }` in every lesson).
   - Assembly: AT&T's `l` suffix is a 32-bit operand; the destination is
     last (GNU as, "AT&T Syntax versus Intel Syntax").
   - Machine code: `89 /r` is `MOV r/m32, r32`, and the ModRM byte `07`
     (mod 00, reg 000, r/m 111) names `eax` and the memory at `rdi` in
     64-bit mode (SDM Vol. 2, MOV, and Table 2-2).
   The ABI-lowered IR uses bcc IR's figure, since it writes the same
   notation.
2. **The machine code's jumps went to labels the listing never showed.**
   At `-O2` in Objects, `2a  7c 14  jl .LBB1_3` jumps to a `.LBB1_3`
   that only the assembly view has, and the offsets skip from `33` to
   `40` with nothing between. **Fixed:** each label one of the function's
   instructions names is a line of its own before the instruction it
   labels, as in the assembly, and each run of alignment padding is one
   line, `34  (12 bytes of padding)`
   ([after](screenshots/part22/after-machine-code-1440-dark.png)). The
   bytes agree: `7c 14` is `jl` with a distance of 0x14 from the next
   instruction at 0x2c, so it lands at 0x40, where `.LBB1_3:` now stands
   (SDM Vol. 2, Jcc: the offset is relative to the next instruction).
   Every padding line in every lesson is followed by a label, as the
   assembly's `.p2align` precedes one. Neither kind of line counts as an
   instruction, so "23 instructions encode to 95 bytes" is unchanged
   (a test).
3. **On a phone each machine-code row broke in two,** the mnemonic at the
   right edge and its operands under the bytes column
   ([before](screenshots/part22/before-machine-code-390-dark.png)).
   **Fixed:** the machine code alone no longer wraps; it scrolls sideways
   inside its frame, as its fallback text already did, and the page does
   not ([after](screenshots/part22/after-machine-code-390-dark.png)). On a
   card narrower than 560 px the figure of item 1 puts each meaning under
   its part.
4. **The assembly never said which operand is the destination.**
   `movl %eax, (%rdi)` reads backwards to anyone who knows Intel's
   syntax. **Fixed:** the explanation names AT&T syntax, which GCC and
   Clang print by default, its `%`, `$` and parentheses, that the
   destination comes last (`addl $1, %eax` adds 1 to eax, a line of
   Objects), and the size suffixes b, w, l and q that most integer
   instructions carry (jumps such as `jl` and floating-point instructions
   such as `movsd` are named differently; GNU as says instructions *may*
   carry one; review round 1).
5. **The machine code said only calls show zero bytes.** A string
   literal's and a floating constant's address do too: Floating point's
   `leaq .str(%rip), %rdi` is `48 8d 3d 00 00 00 00` and
   `movsd .LCPI0_0(%rip), %xmm0` ends in four zero bytes. **Fixed:** the
   explanation says so, says a jump within the function is complete (its
   last bytes are the distance to its label, counted from the end of the
   jump), and says why the offsets skip.
6. **The ABI-lowered IR's struct notation was unexplained.** Structs
   shows `function @moved(ptr registers(i64, align 4), i64, i32 noundef)`,
   and the explanation said only that structs are "where this stage earns
   its keep" ([before](screenshots/part22/before-abi-1440-dark.png)).
   **Fixed:** it names the System V ABI, says what the IR's
   `ptr aggregate(…)` and `ptr result(…)` mean, and reads that signature,
   as `middle-end.md` defines `registers(…)`: the 8-byte `struct point`
   travels as one `i64` in a register (`movq %rdi, -32(%rsp)` in the
   machine code), and the result comes back in one 64-bit register
   (`movq -16(%rsp), %rax`) through a parameter that is not passed
   ([after](screenshots/part22/after-abi-1440-dark.png)).
7. **`-O2` made code bigger and the note did not say why.** In
   Callbacks the LLVM IR grows from 27 instructions to 52, and the note
   said only that `sum_of` is no longer called, which reads as a saving
   ([before](screenshots/part22/before-optimizer-1366-dark.png)). The
   `-O2` IR shows why: `main` holds a copy of `sum_of`'s loop for each of
   its two calls, and `sum_of` is still defined, since another file could
   call it. **Fixed:** where the code grew and a function was inlined,
   the note ends "That is why there is more code at -O2: each inlined
   call gets its own copy of the body, and a function that other files
   could call is kept as well." It appears in Hello, Pointers (LLVM IR
   only), Walking a string and Callbacks, each of which inlines a
   function that stays defined
   ([after](screenshots/part22/after-optimizer-1366-dark.png)).
8. **An indirect call's register was named as a function.** Callbacks'
   assembly at `-O2` read "with calls to `*%r13`, `*%rax` and `printf`"
   ([before](screenshots/part22/before-asm-example-390-dark.png)).
   **Fixed:** "with calls to `printf` and 3 calls through a function
   pointer", the three `callq *` lines of that listing (a test;
   [after](screenshots/part22/after-asm-example-390-dark.png)).
9. **The same kind of token had different colours in different views.**
   The WebAssembly text drew a function's name where it is defined or
   called off-white, where every other view uses orchid; `i32.add` was
   all instruction colour, though bcc IR's `iadd.i32` colours its type
   blue; `offset=` and `align=` were as loud as an instruction; and LLVM
   IR's `%slot0` was a plain value's teal, though bcc IR's `slot0` is a
   stack slot's lavender. **Fixed** in the grammars, so each kind of
   token has one colour in every view. `style.css` now names the code
   colours as `--syntax-*` tokens, which the figure and the fallback
   highlighting use, beside `themes.js`'s palette.
10. **Two listings were both "Listing 2"** in the WebAssembly view: the C
    source and the module's text, because the source's chip rule also
    matched the text's. **Fixed:** the C source is Listing 1 there, as in
    the other modes, and the text Listing 2.

### Checks

- **The lesson walk.** A throwaway engine test (never committed) stepped
  all 19 lessons and wrote each step's title, lead, explanation, note,
  clauses and next step: 1981 notes over 18 lessons, with the
  preprocessor lesson refused before its first step ("line splicing is
  outside this subset"), as on `main`. Nothing the abstract machine shows
  changed: this branch leaves `src/`, the engine and the machine view's
  code (`web/src/machine*`, `engine.rs`, `glossary.rs` and the narration,
  tape, cells and evaluation panels) byte-identical to `main`, so no note
  can differ.
- **Tests,** as `pages.yml` runs them:

  | Check | Result |
  | --- | --- |
  | Engine tests | 176 passed |
  | Engine clippy | clean |
  | Site tests | 181 passed (177 and four new), plus the prerender's 2 |
  | WebAssembly stepper tests | 79 passed |
  | Site clippy: native, and wasm32 `hydrate`, `worker` and `compiler` | all four clean |
  | `web/wasm/generate.sh --check` (wasi-sdk 34) | passes |
  | `web/pipeline/generate.sh --check` (this branch's dev bcc) | passes |
  | `git diff --check` | clean |

  `cargo +nightly fmt --check` passes for the repository's crate. In
  `web/` it reports the same hunks on `main` as on this branch: none of
  this branch's lines.
- **The browser.** Less than 14 GB was free, so the site was not built
  here: the walk ran on CI's Pages build of `d84d123b9` (run 45292587212),
  served locally, with the same script as before, at 1440x900 (every
  lesson, dark), and in both themes at 1440x900, 1366x768, 1024x768 and
  390x844 (the five lessons): 594 states. No state scrolled sideways, no
  page logged an error or a warning, every back-end stage showed the
  figure, which never passed the window's right edge, and every colour in
  the code and in the figure is at least 6.88:1 on the light theme's
  screen and 7.12:1 on the dark one's (the grey of comments, directives,
  offsets and the figure's meanings; every other colour 8.96:1 or more).
  The figure adds no control: its parts list is named "The line, part by
  part" and its key "Other colours in this view" for screen readers, and
  its example is plain text in its colours. The narrow-card layout of
  item 3 came after that build. It was checked first on that build with
  only its CSS added (390x844 dark, the Objects lesson), then on CI's
  build of `a2a53a1e1` (run 45305513222): the Callbacks lesson at
  390x844 in both themes, 1024x768 light and 1440x900 light, 44 states,
  each with the figure, none with an error or sideways scroll, and every
  colour at least 6.88:1.

### Left for later

- **The assembly's directives.** At `-O0` a function's assembly is about
  a third `.cfi_*` lines, and every listing opens with five directives
  before the first label. The figure's key says what a directive is, but
  they could fold behind a chip, as the target lines do.
- **The WebAssembly text** has no "How to read a line" of its own; its
  stack-machine notation is explained step by step in the WebAssembly
  view's notes.
- **Fixed examples.** Each figure's example comes from the first lesson,
  whatever program is on screen. Taking it from the output on screen
  would need a reader for each notation that finds a line of each shape.
- **The editor's height at 1366x768** is the minimum, 360 px, about 15
  lines; the figure now fills the column under it, but the output shows
  less than at 1440x900.
- **Touch wording.** "Point at a line" and "Point at a source line" also
  mean a tap on a phone, which the text does not say.
- **The before walk** at 1366x768 and 390x844 covered five lessons, not
  all nineteen; the 1440x900 walk covered every lesson in both themes.

### Review, round 1

- **Size suffixes.** The assembly explanation said every instruction's
  last letter gives its size, which `jl`, `jne`, `sete`, `movsd`,
  `ucomisd` and `cvttsd2si` on the site contradict. It now says most
  instructions that move or compute integers carry one, and names jumps
  and floating-point instructions as exceptions.
- **`%slot0`.** The LLVM IR key now says `%slot0` is the *address* of
  the memory `alloca` reserves, as LLVM IR uses it. The bcc IR key, where
  `slot0` names the frame region itself, is unchanged.
- **Left for later, from the review:** a swipe hint for the machine
  code on phones; the figure's height on phones (490 to 565 px); saying
  in the key that labels and padding lines are not bytes; a reason for
  the assembly and machine code growing at `-O2`; the inlining note's
  "kept" clause on user code that inlines a `static` function.

## Part 23: Part 22's "Left for later" (2026-11)

Part 22 (#454) gave every back-end stage a "How to read a line" figure and
left a list for later, which its review added to. This part does that
list, on `agent/site-part23`, in the brief's order.

### What changed

1. **The WebAssembly text has a "How to read a line" of its own.** The
   WebAssembly view puts the figure after the module's provenance
   ([before](screenshots/part23/before-wasm-1440-dark.png),
   [after](screenshots/part23/after-wasm-1440-dark.png)). Its line is a
   load or a store, the instructions that show the stack machine and
   linear memory together: `i32.load offset=4 align=4` reads "Pop an
   address, read the 32-bit integer 4 bytes past that address, and push
   it." Each part is glossed: the type, the instruction (pop an address
   off the operand stack, read memory there and push the value; a store
   pops a value, then an address), `offset=` (added to the address) and
   `align=` (a hint that the address is a multiple of 4, which changes
   nothing the instruction does). Its key names `$v0` (a local, kept
   outside linear memory), `$__stack_pointer` (the global that holds the
   top of the C stack, which lives in linear memory), `$main`,
   `local.get`, a constant and bcc's `;; block0` comments. The claims
   were checked against the WebAssembly specification as
   webassembly.github.io serves it (3.0, 2026-11-23):
   - text format, "Memory Instructions": "The offset defaults to 0, the
     alignment to the storage size of the respective memory access", and
     `align=` is written in bytes, the binary holding its exponent; bcc
     writes `align=4` always and `offset=` only when it is not 0
     (`src/backend/wasm/text.rs`), and the stepper prints Clang's modules
     without a natural `align=`, which the reader accepts too;
   - execution, `t.load`: "Pop the value (at.const i) from the stack",
     then "Push the value (nt.const c) to the stack", reading at
     `i + ao.offset`; `t.store` pops the value first, then the address;
   - the note under "Memory Instructions": "The alignment memarg.align in
     load and store instructions does not affect the semantics", and an
     unaligned access "must succeed regardless of the annotation";
   - structure: "instructions manipulate values on an implicit operand
     stack"; "The locals declare a list of mutable local variables",
     separate from a memory, "a list of raw uninterpreted bytes".
2. **The assembly folds its `.cfi_` directives.** At `-O0` they were a
   quarter of the listing, 38 of Callbacks' 147 lines, and half of a short
   function's (8 of `twice`'s 16 lines, from `twice:` to `.cfi_endproc`;
   [before](screenshots/part23/before-asm-1440-dark.png)). The view now
   folds them, as the IRs fold their target lines, into one line at the
   top that says how many there are and what they are for:
   `# 38 .cfi_ directives: call-frame information for debuggers and
   unwinding, folded (Show call frames to read them)`
   ([after](screenshots/part23/after-asm-1440-dark.png)). Pointing at it
   explains the table the assembler makes of them (`.eh_frame`, which bcc's
   own assembler writes in `src/backend/native/object/unwind.rs`): where
   each function's caller's frame and saved registers are, at each
   instruction, for a debugger's backtrace and for unwinding. "Show call
   frames" puts them back, and the explanation under the view says they
   are folded. The links to the source follow the fold (a test), and the
   counts in the `-O2` note still come from the full listing.
3. **Phones.** The figure is a disclosure, open at first but closed on a
   screen 600 px wide or less, where it was 490 to 565 px tall: closed, it
   is a 50 px strip under the output, "How to read a line", with a
   triangle that opens it (its title is a 40 px target on a touch screen)
   ([before](screenshots/part23/before-machine-code-390-dark.png),
   [after](screenshots/part23/after-machine-code-390-dark.png)). The
   machine code, which scrolls sideways inside its frame, says so on a
   card 560 px wide or less: "Swipe the listing sideways: the
   instructions run on past its right edge" ("Scroll" with a pointer). The
   pointing hints, under the source and over the output, say "Tap a line"
   and "Tap a source line" where the screen has no pointer that hovers, as
   the WebAssembly listing's legend already did.
4. **The figure's example comes from the program on screen.** It always
   showed a line of the first lesson, whatever program was on screen
   ([before](screenshots/part23/before-figure-1440-dark.png): Callbacks
   shows Objects' `movl %eax, (%rdi)`). Each notation now has a reader that
   takes apart only the line shapes it can gloss exactly, and the figure
   shows the most instructive line of the first function that has one, or
   else the first lesson's line
   ([after](screenshots/part23/after-figure-1440-dark.png)):
   - bcc IR: `int` arithmetic with `nsw` (`iadd`, `isub`, `imul` on
     `.i32`);
   - LLVM IR: a load of an integer, pointer or floating type, or else
     `int` arithmetic with `nsw` (`add`, `sub`, `mul`; LangRef: with
     `nsw` the result is poison on signed overflow);
   - assembly: a store first, then arithmetic, then a copy between
     registers, the prologue's `movq %rsp, %rbp` last; integer `mov`,
     `add`, `sub`, `imul`, `and`, `or` and `xor` with an `l` or `q`
     suffix, a register or constant source and a register or memory
     destination;
   - machine code: the same instructions where the bytes are an optional
     REX.W prefix (`48`), one of the opcodes `89`, `01`, `29`, `21`, `09`
     and `31` (`op r/m, reg`), a ModRM byte whose fields agree with the
     operands, and the SIB byte (`24`, for an address in `rsp`) and
     one-byte displacement that its fields call for, each glossed
     (SDM Vol. 2, §2.1, Tables 2-2 and 2-3);
   - WebAssembly: a load, then a store, with an offset first.

   A test reads every lesson's views at both levels: wherever a view has a
   line its reader takes, the example is a line of that view. 209 views
   (190 pipeline views and 19 WebAssembly texts) have 183 examples of
   their own: bcc IR before and after ABI lowering 58 of 76, LLVM IR 35 of
   38, assembly 38 of 38, machine code 33 of 38, WebAssembly 19 of 19. The
   rest have no line of a shape the figure explains (Bits and masks
   computes with bitwise operations, none of them `nsw` arithmetic, and
   its `-O2` LLVM IR is one call to `printf`) and show the first lesson's
   ("Left for later" lists them).
5. **Wording from the reviews.**
   - The machine code's key says a label is "written in for reading: it
     takes no bytes", and gives the padding line its own entry: "filler
     before a label, summed up in one line rather than listed byte by
     byte".
   - **Growth at `-O2` in the assembly and machine code.** The `-O2` note
     found inlining only in the IRs, whose calls it could read; it now
     reads `callq` lines too (and would count a jump to another function
     as a call). Hello, Pointers, Walking a string and Callbacks now say
     why their assembly and machine code grow: the inlined copies
     ([before](screenshots/part23/before-optimizer-1440-dark.png),
     [after](screenshots/part23/after-optimizer-1440-dark.png)). Loops
     three ways grows with nothing inlined (38 instructions to 46): its
     `-O2` IR turns each loop's `if` into a `select`, so the do loop's
     body is one block of six instructions, which the back end copies over
     the jump back (`docs/design/native-backend.md`, "Loop rotation": a
     `jmp` to a block of at most eight instructions that ends in a
     conditional branch takes a copy of it). Its note now says so, when
     the `-O2` assembly has more copied test blocks than the `-O0`
     assembly. Names for values and Short-circuit evaluation have fewer
     instructions but more bytes at `-O2` (75 to 78, 170 to 172): their
     machine code now says an instruction is 1 to 15 bytes long and the
     padding before a label changes with the code ahead of it (in Names
     for values, `-O2` replaces `pushq %rbp`, 1 byte, and `movq %rsp,
     %rbp`, 3, with `subq $8, %rsp`, 4, and `popq %rbp`, 1, with `addq $8,
     %rsp`, 4).
   - **The "kept" clause** now depends on what `-O2` still defines: "a
     function that other files could call is kept as well" only when an
     inlined function is still defined with external (or weak) linkage,
     "the inlined function is kept as well" when only a `static` one is,
     and nothing when none is (a test with a `static` function, kept and
     gone). Every lesson's note reads as before.
   - **`native-backend.md`** said a function that takes its frame address
     keeps a frame pointer "as GCC's and Clang's do", after a list that
     includes `__builtin_setjmp`. It now says Clang keeps one for all
     three, and GCC 14 (and 13.3) at `-O2` for the first two, but for
     `__builtin_setjmp` only pushes `%rbp`, with no `movq %rsp, %rbp`.
     GCC 13.3 and Clang 18.1 were checked here at `-O0` to `-O3`: GCC
     links `%rbp` for `__builtin_setjmp` only at `-O0`; Clang at every
     level.

### Checks

- **Tests,** as `pages.yml` runs them, and its other checks:

  | Check | Result |
  | --- | --- |
  | Engine tests | 176 passed |
  | Engine clippy | clean |
  | Site tests | 191 passed (181 and ten new), plus the prerender's 2 |
  | WebAssembly stepper tests | 79 passed |
  | Site clippy: native, and wasm32 `hydrate`, `worker` and `compiler` | all four clean |
  | `web/wasm/generate.sh --check` (wasi-sdk 34) | passes |
  | `web/pipeline/generate.sh --check` (this branch's dev bcc) | "Pipeline outputs are up to date." |
  | `cargo +nightly fmt --check`, root and `web/` (CI's "Site formatting") | clean |
  | `git diff --check` | clean |

- **The lesson walk.** Nothing the abstract machine shows changed: this
  branch leaves `src/`, the engine and the machine view's code
  (`web/src/machine*`, `engine.rs`, `glossary.rs`, `snapshot.rs` and the
  narration, tape, cells and evaluation panels) byte-identical to `main`,
  so no machine note can differ. The `-O2` notes of every lesson's
  back-end stages were written out by a throwaway test (never committed)
  and compared with the page dump of Part 22's review (`a2a53a1e1`): 202
  of their 228 sentences are as before, and the 26 new ones are all in
  the assembly and machine code, as item 5 says: "is no longer called"
  in the seven lessons that inline a call (Objects, Hello, Pointers,
  Walking a string, Callbacks, Structs and From source to a running
  program), the copies' reason in the four of them that grow, loop
  rotation in Loops three ways, and longer encodings in Names for values
  and Short-circuit evaluation.

- **The browser.** Less than 14 GB was free, so the site was not built
  here: the walk ran on CI's Pages build of `0dc70faaf` (run
  45342929486), served locally, with Playwright in Chromium, one browser
  at a time, under a 1.5 GB scope with a memory watchdog. For six lessons
  (Objects, Hello, Loops three ways, Floating point, Callbacks and
  Structs) it took each back-end stage at `-O0` and `-O2` and the
  WebAssembly view, at 1440x900, 1024x768 and 390x844 (as a touch
  screen), in both themes: 396 states. No state scrolled sideways, no
  page logged an error or a warning, and no part of a figure passed its
  frame or the window. Every colour in the figures is at least 6.88:1 on
  the light theme's screen and 7.12:1 on the dark one's, the WebAssembly
  figure's included; the scroll hint is 9.4:1 or more and the "Show call
  frames" chip 11.25:1 or more. Each figure token whose text the view
  also colours has the view's colour (the script's one exception is an
  offset `0` it took for a displacement `0`). The figure was open at
  every state at 1440 and 1024 px and closed at every state at 390 px,
  50 px tall; a tap opens it (632 px for Callbacks' machine code) and it
  stays open as the stage changes. Its title is the summary "How to read
  a line"; its lists keep their names, "The line, part by part" and
  "Other colours in this view"; the chip is a button named "Show call
  frames" with `aria-pressed`; the hint's arrow is hidden from screen
  readers. At 390 px every state said "Tap" and "Swipe". An element
  screenshot in Playwright turns off the touch emulation for the rest of
  the page (a probe: `(hover: none)` matches before one and not after),
  so the walk's phone states after its first screenshot read "Point at";
  the phone screenshot here is a page screenshot.
  An earlier walk, on CI's build of `b5136387a`, found the WebAssembly
  figure painted on the device's grey, which the view gives every child
  (1.06:1 in the light theme): `.wasm-view > .notation` now keeps its
  strip of screen.

### Left for later

- **More shapes for the readers.** The assembly reader takes `mov` and
  the two-operand ALU instructions only, so Callbacks' `twice` explains
  `movq %rdi, %rax` rather than its `shll $1, %edi`; shifts, `lea`,
  `cmp` and jumps could be glossed too. 26 of the 209 views still show
  the first lesson's line: both bcc IRs where no line is `nsw` `int`
  arithmetic (Unsigned numbers, Bits and masks, Walking a string and
  Reading past the end at both levels, Integer promotions at `-O2`), the
  LLVM IR of Bits and masks and of Integer promotions at `-O2`, and the
  `-O2` machine code of Integer promotions, Bits and masks, Floating
  point, Reading past the end and Uninitialized reads, whose
  instructions are other opcodes or name registers past `rdi`.
- **Loop rotation** is recognised from the text: a block that ends in a
  conditional jump and repeats as the end of a longer one. A count from
  the back end itself would be exact.
- **GCC 14** was not at hand: its `__builtin_setjmp` frame at `-O2` is as
  the brief reports it; GCC 13.3 and Clang 18.1 were checked here.
- **The figure remembers nothing.** Opened or closed, it is back to its
  first state on the next visit; the views' other switches do not
  persist either.
- **Clang's WebAssembly module** (the compiler switch in the WebAssembly
  view): the reader accepts the stepper's printout of it (no natural
  `align=`), but the tests and the browser walk read bcc's modules only.
- **The link legend** under the output still says "came from the line
  you point at" on a touch screen.
