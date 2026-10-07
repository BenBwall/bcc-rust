# C standards

Free texts of each C standard revision. The published ISO standards are paid
documents; these are the freely available versions that WG14 lists on its
[projects page](https://www.open-std.org/jtc1/sc22/wg14/www/projects). Cite
C99 from `c99-n1256.pdf` as described in
[`../CONTRIBUTING.md`](../CONTRIBUTING.md#citing-the-c-standard).

| File | Revision | Document |
| --- | --- | --- |
| `c89-fips160.pdf` | C89 (ANSI X3.159-1989) | NIST FIPS PUB 160, which adopts ANSI X3.159-1989 |
| `c99-n1256.pdf` | C99 (ISO/IEC 9899:1999) | WG14 N1256: C99 with Technical Corrigenda 1-3 |
| `c11-n1570.pdf` | C11 (ISO/IEC 9899:2011) | WG14 N1570: final C11 draft |
| `c17-n2310.pdf` | C17 (ISO/IEC 9899:2018) | WG14 N2310: first C23 draft, printed as a diff against C17 |
| `c23-n3220.pdf` | C23 (ISO/IEC 9899:2024) | WG14 N3220: early C2y draft, closest free text to C23 |

WG14 lists no free text for C90 (ISO/IEC 9899:1990) or its 1995 amendment.

`c89-fips160.pdf` is the [NIST scan](https://nvlpubs.nist.gov/nistpubs/Legacy/FIPS/fipspub160.pdf)
reduced from 21 MB to 6.8 MB: on every page but the first three, the colour
background and text-colour layers were replaced with plain white and black,
keeping the 1-bit text layer and the searchable OCR text unchanged.

## HTML editions

Each PDF has an HTML edition beside it (`c99-n1256.html` and so on), generated
for reading and searching; agents search them with the `search-standard`
skill. The PDFs stay the authority; regenerate the HTML instead of editing it.

- Each PDF page is one `<section class="page" id="page-N">`, where `N` is the
  1-based PDF page and `data-printed` holds the printed page number, so a
  citation's PDF page is `#page-N`. `data-section` holds the section title
  from the page's running header or footer (`Environment`, `Library`,
  `Index`), so `grep 'data-section="Library"'` finds a section's pages.
- Clauses have their number as an id (`#6.7.2`, `#A.1`), numbered paragraphs
  are `#6.7.2p2` (a paragraph continued on the next page carries
  `data-of="6.7.2p2"` there), and footnotes are `#fn104`. Clause references in
  the text, the contents, and the index link to those ids.
- Paragraphs keep the PDF's line breaks, one justified line per printed line,
  so pages break where the PDF's do in any font. Printing, or saving as PDF,
  reproduces the original pagination: `@page` sets the PDF's page size and each
  page element breaks after itself. On narrow screens the lines reflow instead.
  Tables and formulas keep their printed positions.
- Where the window fits two pages side by side, beside the sidebar or with it
  closed, they show as spreads. A three-way control in the top bar chooses
  single pages (None), even spreads (page 1 alone, then 2-3, 4-5, ...), or odd
  spreads (pages 1-2, 3-4, ...); each edition starts with the spreads that put
  its odd printed page numbers on the right, as in a bound book. The choice is
  a set of CSS radio buttons; the script keeps the current page in place when
  the layout changes.
- A sidebar, toggled from the top bar, has a page strip and the document
  outline (the PDF's bookmarks, or its headings where it has none). The
  sidebar, its tabs, and the collapsible outline are plain HTML and CSS; a
  short script draws the page thumbnails (cloning each page only when its tile
  scrolls into view), follows the current position, and adds a page box to
  the top bar that shows the current PDF page and goes to the one typed into
  it. Without scripts the strip shows page numbers, the page box is hidden,
  and everything else still works.

Editions differ where their PDFs do:

- **C89** comes from the OCR layer of a scan, so it has no bold, italics, or
  code font, some words are misread, and headings are recognised by size and
  clause number (some are missed). Its margin numbers count lines, so it has
  no paragraph ids.
- **C17** is N2310, the first C2x draft printed as a diff against C17: C2x
  additions are `<ins>` (blue, underlined) and C17 text it removed is `<del>`
  (red, struck through), as in the PDF.
- **C17 and C23** are set in Palladio; the pages use a metric-compatible clone
  where one is installed (URW Palladio L, P052, TeX Gyre Pagella) and Times
  otherwise.

The converter is [`../scripts/standards-html`](../scripts/standards-html), a
standalone crate. [`regenerate.sh`](../scripts/standards-html/regenerate.sh)
rebuilds it (without the repository's LTO linker flags, which target the
compiler) and regenerates every edition, or the ones named:

```sh
sh scripts/standards-html/regenerate.sh
sh scripts/standards-html/regenerate.sh c99 c11
```

`--lines <page,...>` prints how the converter classified each line of those PDF
pages (`SPANS=1` adds each span, `RULES=1` the page's rules), `--fonts <page>`
lists a page's fonts, and setting `STANDARDS_HTML_DEBUG=1` adds each block's
expected position (`data-ey`) so layout drift can be measured in a browser.
The converter learns each document's text size, line spacing, margins, and
fonts from the PDF itself, and targets the layout of these WG14 drafts.
