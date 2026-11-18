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
  folding (one cell of its values, opened on demand) would fit it.
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
