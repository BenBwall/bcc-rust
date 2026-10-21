# Parser study grammar corpus

`python scripts/parser_study_cases.py` writes 150 small, header-free translation
units and `manifest.json` to `.scratch/parser-study/grammar`. It requires only the
Python standard library. The default corpus has 87 **valid**, 21 **extension**,
and 42 **malformed** cases; 16 of the valid cases use a bounded seeded expression
grammar. Sources and manifest use UTF-8 and LF, stable ordering, source hashes,
and no timestamps or absolute paths. Repeating the command produces identical
bytes. `--seed` defaults to `20261021`; `--fuzz-count 0..32` controls the generated
portion. `--output DIR` changes only the destination.

The manifest is the authoritative file list. Each entry records `name`, intended
`classification`, `coverage` tags, `origin`, byte count, and SHA-256. Generated
cases also record grammar depth. The generator overwrites its current filenames
and manifest, preserving other files in the directory. After changing generation
options, select cases from the manifest rather than a directory glob that could
include old generated files.

## Configuration and classification

The benchmark path uses
[`benchmark_context`](../../src/benchmarking/input.rs), which constructs
`CompilerConfiguration::default()` with a fixed source date epoch. The default in
[`configuration/options.rs`](../../src/configuration/options.rs) is **C99, GNU
off, extension policy Allow**. The CLI's separate `gnu17` default does not describe
this path; see [language standards](../../language-standards.md#cli-reference).
Reserved GNU spellings and several later ISO forms remain accepted with this
library configuration. Fixtures use reserved spellings such as `__asm__` and
`__typeof__` so they do not depend on enabling unreserved GNU keywords.

Classification describes intended parser syntax, not whether the complete
program is semantically valid or whether the current parser emits zero
diagnostics. `valid` uses C99 grammar; `extension` exercises supported GNU or
later ISO syntax; `malformed` deliberately breaks a grammar production. These
labels are reviewable intentions, not compiler-derived goldens. Parser comparison
must still compare diagnostics for every category. Undeclared names, impossible
postfix-chain types, incompatible casts, nonconstant expressions, and sequencing
constraints do not make a fixture parser-malformed. In contrast, `a + b = c`
violates the assignment production's unary-expression left operand, and is
intentionally malformed. Type/lvalue/constant-expression constraints belong to
semantic analysis, as specified in the
[parser checklist](../../c99-parser-compliance-checklist.md#non-goals-for-v0).

## Coverage

- `expr-*` covers identifiers, integer/floating/character/string constants,
  adjacent strings, grouping, postfix calls/subscripts/member access, prefix and
  postfix increments/decrements, every unary operator, casts, and both `sizeof`
  forms. Named/abstract function and array pointer declarators exercise type
  operands. Compound literals cover scalar/array/struct types, nested initializers,
  trailing commas, designations, postfix continuation, and `sizeof` ambiguity.
- Every binary precedence level has an unparenthesized repeated-operator chain
  to expose left association. A full precedence ladder and explicitly grouped
  counterpart expose precedence. Assignment fixtures cover all eleven operators
  and right association. Conditional fixtures expose right nesting, nested middle
  operands, an unparenthesized assignment/comma middle, grouped assignment/comma
  on the right, and colon ownership. Comma chains and call arguments distinguish
  expression from assignment-expression boundaries.
- `decl-*`, `init-*`, and `typedef-*` cover function pointers, nested prototypes,
  named/abstract arrays, array parameter qualifiers/static/star, old-style
  definitions, struct/union/enum declarations, bit-fields, scalar/nested list
  initializers, and chained field/array designations. Typedef cases distinguish
  casts from grouping and declarations from expressions, including an ordinary
  binding's own initializer, parameters, prototype scopes, declarator lists,
  nested blocks, `for` declarations, returned-function prototypes, members, and
  labels. Uses after each scope expose restoration errors.
- `stmt-*` covers all statement families: null/expression/compound, mixed block
  items, labels, `if`/`else`, `switch`/`case`/`default`, `while`, `do`/`while`, both
  `for` initializer forms, all optional `for` expression slots, and every jump
  statement. Cases isolate dangling `else`, nested switch ownership,
  switch-inside-loop jumps, conditional case-label colons, implicit selection and
  iteration scopes, and typedef shadowing in controlling expressions.
- `ext-*` covers `_Alignof` type names, GNU alignment expression operands,
  omitted-middle conditionals, nested statement expressions, `__extension__`,
  `__builtin_va_arg`, `__builtin_offsetof`, `__builtin_types_compatible_p`, and
  `__builtin_choose_expr`. Ordinary builtin calls, complex unary forms, generic
  selection, range/obsolete designators, case ranges, local labels, label
  addresses, computed goto, nested functions, assembly operands/goto, and
  expression/type `__typeof__` operands broaden extension boundary coverage.
- `bad-*` breaks operands, separators, member names, conditional colons/right
  operands, grouping/call/subscript/type closers, initializer/designator boundaries,
  and compound literal/statement expression closers. Statement cases break
  conditions, bodies, `else`, `do`'s `while`, `for` slots, case colons, and goto
  targets. Broken builtin/type operands, typedef restoration after recovery, and
  EOF in expressions/statements/initializers/declarators round out recovery.
  Most cases retain a following statement and `int after;` so comparisons expose
  overconsumption as well as diagnostic and location differences. EOF cases
  intentionally have no suffix.
- `fuzz-expression-*` combines a seeded grammar at depths 2, 3, and 4. Its
  productions include unary/binary expressions, conditionals with comma middles,
  assignment, comma, typedef casts, calls, and array compound literals.
  Parenthesized children preserve restricted operand boundaries; assignments
  always target an identifier. Generation makes no type or runtime-safety claim.

Coverage draws on the existing parser fixtures in
[`expressions.rs`](../../src/translation_phases/parsing/tests/expressions.rs),
[`expression_regressions.rs`](../../src/translation_phases/parsing/tests/expression_regressions.rs),
[`statements.rs`](../../src/translation_phases/parsing/tests/statements.rs),
[`statement_regressions.rs`](../../src/translation_phases/parsing/tests/statement_regressions.rs),
and [`gnu.rs`](../../src/translation_phases/parsing/tests/gnu.rs). It supplements
their tree assertions with small differential inputs; it is not an exhaustive
language-mode, extension-policy, semantic, or translation-limit suite. Large
depth/width workloads remain in `scripts/parser_study_inputs.py`.

## Parent harness handoff

The generator neither builds nor invokes the compiler. Once the parent builds
`.scratch/parser-study/study-release`, a focused comparison can run as follows:

```sh
python scripts/parser_study_cases.py
python scripts/parser_study.py diff \
  --binary .scratch/parser-study/study-release \
  --inputs .scratch/parser-study/grammar \
  --files expr-precedence-ladder.c expr-conditional-middle.c typedef-for-scope.c \
  --modes machine recursive growing hybrid:16 \
  --output .scratch/parser-study/grammar-focused-diff.json
```

For the entire corpus, pass every manifest name explicitly; the harness's default
file list belongs to its larger benchmark input directory:

```sh
python - <<'PY'
import json
from pathlib import Path
import subprocess
import sys

directory = Path('.scratch/parser-study/grammar')
manifest = json.loads((directory / 'manifest.json').read_text())
subprocess.run([
    sys.executable, 'scripts/parser_study.py', 'diff',
    '--binary', '.scratch/parser-study/study-release',
    '--inputs', str(directory),
    '--output', '.scratch/parser-study/grammar-diff.json',
    '--modes', 'machine', 'recursive', 'growing', 'hybrid:16',
    '--files', *[case['name'] for case in manifest['cases']],
], check=True)
PY
```

The inspection comparison covers canonical ASTs, source locations, and rendered
diagnostics. Malformed recovery differences are evidence to triage against the
machine parser, not a reason to silently relabel or drop the case. Corpus-local
verification checks generation, manifest integrity, and determinism only;
compiler comparisons and prototype fixes belong to the parent study.
