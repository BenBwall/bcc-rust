---
name: search-standard
description: Look up the C standard in its searchable HTML editions under standards/ (C89, C99, C11, C17, C23). Use to read a clause, paragraph, footnote, or grammar production, to find where a revision says something or compare revisions, or to check the clause and page numbers of a citation.
---

# Search the standard

`standards/` holds an HTML edition of each revision, generated from the PDF
beside it; the PDFs stay the authority. Each HTML file is one large document,
so reach text through `standard.sh` and targeted reads rather than reading a
file whole. Run everything from the repository root.

| Edition | File | Source |
| --- | --- | --- |
| `c89` | `standards/c89-fips160.html` | ANSI X3.159-1989 as adopted by FIPS PUB 160, from OCR of a scan |
| `c99` | `standards/c99-n1256.html` | N1256: C99 with TC1-TC3 |
| `c11` | `standards/c11-n1570.html` | N1570: final C11 draft |
| `c17` | `standards/c17-n2310.html` | N2310: early C23 draft standing in for C17 |
| `c23` | `standards/c23-n3220.html` | N3220: early C2y draft standing in for C23 |

The compiler targets C99; take another edition only when the task names it or
compares revisions.

## Steps

1. **Locate.** `sh .agents/skills/search-standard/standard.sh <edition> <command> <argument>`,
   where `<edition>` is from the table or `all` (every edition, each line
   prefixed with its edition):
   - a clause or paragraph: `standard.sh c99 pages 6.7.2p2`
   - wording: `standard.sh c99 find 'null pointer constant'` (case-insensitive,
     matched on the text without tags, so a phrase may run across `<code>`,
     italics, and links)
   - the same paragraph in every revision: `standard.sh all pages 6.3.2.3p3`;
     clause numbers drift between revisions, so confirm with `para` that each
     hit is the provision you mean
   - a grammar production: `find 'type-specifier:'`; the Annex A copy is
     prefixed with its defining clause, as in `(6.7.2) type-specifier:`
   - a whole section's pages: `grep -n 'data-section="Library"' <file>`
2. **Read.** `standard.sh <edition> para <id>` prints the element's text,
   following a paragraph onto the pages it continues on. For layout (grammar
   indentation, lists, tables), `Read` the file from the reported line with a
   small `limit`.
3. **Cite.** Take the clause and paragraph from the id, and the pages from
   `standard.sh <edition> pages <id>`: one line per page the element starts or
   continues on. Write C99 citations in the form
   [`CONTRIBUTING.md`](../../../CONTRIBUTING.md#citing-the-c-standard)
   prescribes; for example `c99 pages 6.7.2p2` reports PDF pp. 111-112 and
   printed pp. 99-100, giving `C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112.`
   Cite another revision the same way under its own name, as `C11: …`.

Done when the text you rely on was quoted from the file, and every page number
you cite came from `standard.sh pages`.

## Markup

| Element | Markup |
| --- | --- |
| PDF page | `<section class="page" id="page-111" data-page="111" data-printed="99" data-section="Language">` |
| Clause heading | `<h4 id="6.7.2">`; level follows depth. Annexes are `A`, `A.1`; front matter is `foreword`, `introduction` |
| Numbered paragraph | `<div class="para" id="6.7.2p2">`, holding its lists, code, and grammar; front matter uses `foreword-p5` |
| Paragraph continued on a later page | `<div class="para cont" data-of="6.7.2p2">` |
| Printed line | `<span class="l">`; paragraphs keep the PDF's line breaks, one span per line |
| Footnote | `<p class="fn" id="fn104">`, referenced by `<sup><a href="#fn104">` |
| Diff mark (C17 only) | `<ins>` added after C17, `<del>` removed after C17 |
| Grammar | `<div class="syn">`, one `<div>` per line; nonterminals in `<i>`, terminals in `<code>` |
| Code, tables, formulas | `<pre>`, `<code>`; tables and formulas are `<div class="lay">` with spans placed at printed positions |

## Editions differ

- **C89** is the OCR of a scan: no bold, italics, or code font, so code is set
  as prose; some words are misread, so search on a distinctive word rather than
  a long phrase. Its margin numbers count lines, so it has no paragraph ids:
  `find` names the nearest heading. ANSI clause numbers differ from ISO's (the
  language is clause 3, the library clause 4), and the outline lists only the
  headings the OCR recognised.
- **C17** is N2310, the first C2x draft printed as a diff against C17:
  `<ins>` holds C2x additions and `<del>` C17 text that C2x removed.
  `standard.sh c17` reads without the insertions, so it searches and prints
  C17's own wording; read the file itself to see both.
- **C17 and C23** are drafts of the next revision, so their front matter names
  that revision; their clause numbering follows the revision they stand in for.

## Gotchas

- Match ids with fixed strings: in a regex, `id="6.7.2"` also matches
  `id="6.7p2"`. `grep -F 'id="6.7.2"'` or `standard.sh` avoid it.
- Raw-file searches see entities and tags: `<stdio.h>` is `&lt;stdio.h&gt;`,
  and `int` in prose is `<code>int</code>`. `standard.sh find` decodes both.
- A hyphen dropped at a line break is drawn by CSS, so the text has the whole
  word (`implementation`), while a hyphenated compound broken at its hyphen
  keeps it (`storage-class`).
- Unnumbered blocks after a paragraph, such as `Forward references:`, stand on
  their own; they are not part of the numbered paragraph above them.
- `find` names the nearest preceding paragraph or heading, so a match in an
  unnumbered block or a subheading such as `Constraints` reports the paragraph
  before it; confirm with `para` before citing.
- Tables and formulas are laid out by position, so their text reads in
  printed order but loses table structure; check the PDF page for exact
  layout.
