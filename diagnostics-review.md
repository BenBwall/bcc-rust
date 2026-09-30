# Diagnostics review

Reviewed on 2026-10-01 against the current uncommitted diagnostic rewrite. Only new files under tests/ and this explicitly requested report were written. Compiler sources and the concurrent build changes were left untouched. No commit was made.

## Results

- 218 small C fixtures, 218 exact stderr snapshots, and two supporting headers.
- Dispatch coverage: initial processing 1/1; tokenizer 6/7; preprocessor 125/131, plus one variant blocked by a panic; parser 64/73 (60 independently visible messages and four folded follow-on variants).
- `BLESS=1 cargo test --test diagnostics_golden`: all 218 snapshots written; golden test passed; guard failed on two compiler panics and a raw NUL source snippet.
- `cargo test --test diagnostics_golden`: both tests failed. The golden comparison matched all 216 non-crashing fixtures. Only the two panic snapshots differ, because their OS thread IDs change. The guard fails on those same two panics and the NUL byte.
- `cargo clippy --all-targets -- -D warnings`: passed in the scratch copy.
- `rustfmt +nightly --check --edition 2024 tests/diagnostics_golden.rs`: passed.
- `git diff --check`: passed. The new Rust test, coverage metadata, and report prose passed separate trailing-whitespace checks. Golden stderr and literal report output blocks retain original whitespace by design.

The corpus compares exact bytes. There is no path, caret, whitespace, control-byte, or panic normalization and no skipped/ignored guard cases. The two panic .stderr files preserve real captured output; their deliberately unstable thread IDs keep the checks red until compiler recovery is fixed. BLESS rewrites snapshots but does not disable the guard.

The scratch build copied the requested inputs, restored HEAD:build.rs with `use cmake as _;` appended and HEAD:.cargo/config.toml, removed RUSTFLAGS, and used:

```text
BINDGEN_EXTRA_CLANG_ARGS=--target=x86_64-pc-windows-gnu "-resource-dir=C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/x64/lib/clang/19"
```

Sources were recopied before each build. The reviewed diagnostic source hashes remained unchanged between blessing, testing, and clippy. The full variant mapping and snapshot hashes are in [COVERAGE.md](tests/fixtures/diagnostics/COVERAGE.md); [coverage.tsv](tests/fixtures/diagnostics/coverage.tsv) is the compact fixture map.

## Standards reference

Citations were checked against [WG14 N1256, C99 with Technical Corrigenda](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1256.pdf). In particular, §6.10.1p4 is the replacement/arithmetic paragraph; §6.10.4p3 covers line-number digit sequences; §6.10.6p2 lists standard pragma switches; §6.10.8p4 covers predefined names; §6.5.7p3 covers shift counts; §6.5.16p1 supplies assignment grammar; §6.7.4p5 covers repeated inline. Paragraph references use N1256 numbering, not another draft's numbering.

Priorities: P1 violates the no-internals/control-byte rule or crashes; P2 misleads about the error, source position, or recovery; P3 is wording/citation polish. Suggestions describe future source changes; none were applied here. Missing-citation and duplicate-label items are explicitly quality recommendations, not compiler correctness claims.

## Findings index

| Fixture | Priority | Problems |
| --- | --- | --- |
| [pp-line-directive-suffix-panic.c](tests/fixtures/diagnostics/pp-line-directive-suffix-panic.c) | P1 | malformed #line panics |
| [pp-redefinition-of-built-in-macro.c](tests/fixtures/diagnostics/pp-redefinition-of-built-in-macro.c) | P1 | predefined macro redefinition panics before any source diagnostic is printed |
| [tokenizer-unknown-token-nul.c](tests/fixtures/diagnostics/tokenizer-unknown-token-nul.c) | P1 | renderer writes an actual NUL byte; basic-character-set note does not explain the actual character |
| [parser-expected-declaration-continuation-after-declarator-missing-semicolon.c](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator-missing-semicolon.c) | P2 | object declaration is reported as a missing function body; grammar/range explanation lacks a C99 citation |
| [parser-expected-statement-expression-assignment.c](tests/fixtures/diagnostics/parser-expected-statement-expression-assignment.c) | P2 | cannot-assign label points at = rather than the left expression |
| [parser-unicode-identifier.c](tests/fixtures/diagnostics/parser-unicode-identifier.c) | P2 | multiple errors from one malformed construct |
| [pp-binary-minus-overflow.c](tests/fixtures/diagnostics/pp-binary-minus-overflow.c) | P2 | wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-binary-minus-without-rhs.c](tests/fixtures/diagnostics/pp-binary-minus-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-binary-plus-overflow.c](tests/fixtures/diagnostics/pp-binary-plus-overflow.c) | P2 | wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-binary-plus-without-rhs.c](tests/fixtures/diagnostics/pp-binary-plus-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-bitwise-and-without-rhs.c](tests/fixtures/diagnostics/pp-bitwise-and-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-bitwise-not-without-operand.c](tests/fixtures/diagnostics/pp-bitwise-not-without-operand.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-bitwise-or-without-rhs.c](tests/fixtures/diagnostics/pp-bitwise-or-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-bitwise-xor-without-rhs.c](tests/fixtures/diagnostics/pp-bitwise-xor-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-cannot-use-hash-hash-after-function-like-macro-call.c](tests/fixtures/diagnostics/pp-cannot-use-hash-hash-after-function-like-macro-call.c) | P2 | message describes an invocation although the bad token starts a replacement list; grammar/range explanation lacks a C99 citation |
| [pp-colon-without-matching-question-mark.c](tests/fixtures/diagnostics/pp-colon-without-matching-question-mark.c) | P2 | multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c) | P2 | multiple errors from one malformed construct |
| [pp-divide-by-zero.c](tests/fixtures/diagnostics/pp-divide-by-zero.c) | P2 | expression diagnostic highlights the following #endif |
| [pp-divide-overflow.c](tests/fixtures/diagnostics/pp-divide-overflow.c) | P2 | wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-divide-without-rhs.c](tests/fixtures/diagnostics/pp-divide-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-empty-parentheses-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-empty-parentheses-in-preprocessor-expression.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct |
| [pp-equals-without-rhs.c](tests/fixtures/diagnostics/pp-equals-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c) | P2 | multiple errors from one malformed construct |
| [pp-expected-binary-operator-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-expected-binary-operator-in-preprocessor-expression.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct |
| [pp-expected-newline-after-undef-directive.c](tests/fixtures/diagnostics/pp-expected-newline-after-undef-directive.c) | P2 | multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-extra-tokens-after-pragma-once.c](tests/fixtures/diagnostics/pp-extra-tokens-after-pragma-once.c) | P2 | wrong token spelling and caret for extra pragma tokens |
| [pp-greater-than-equals-without-rhs.c](tests/fixtures/diagnostics/pp-greater-than-equals-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-greater-than-without-rhs.c](tests/fixtures/diagnostics/pp-greater-than-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-header-not-found-system.c](tests/fixtures/diagnostics/pp-header-not-found-system.c) | P2 | relative system-header name is described as absolute; grammar/range explanation lacks a C99 citation |
| [pp-left-shift-overflow.c](tests/fixtures/diagnostics/pp-left-shift-overflow.c) | P2 | invalid shift count is described as arithmetic overflow; wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-left-shift-without-rhs.c](tests/fixtures/diagnostics/pp-left-shift-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-less-than-equals-without-rhs.c](tests/fixtures/diagnostics/pp-less-than-equals-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-less-than-without-rhs.c](tests/fixtures/diagnostics/pp-less-than-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-line-directive-is-not-a-simple-digit-sequence.c](tests/fixtures/diagnostics/pp-line-directive-is-not-a-simple-digit-sequence.c) | P2 | valid decimal #line is rejected |
| [pp-line-directive-number-too-large.c](tests/fixtures/diagnostics/pp-line-directive-number-too-large.c) | P2 | out-of-range #line also gets a false non-digit warning |
| [pp-logical-and-without-rhs.c](tests/fixtures/diagnostics/pp-logical-and-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-logical-not-without-operand.c](tests/fixtures/diagnostics/pp-logical-not-without-operand.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-logical-or-without-rhs.c](tests/fixtures/diagnostics/pp-logical-or-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-missing-closing-parenthesis-in-defined-directive.c](tests/fixtures/diagnostics/pp-missing-closing-parenthesis-in-defined-directive.c) | P2 | multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-missing-identifier-in-defined-directive.c](tests/fixtures/diagnostics/pp-missing-identifier-in-defined-directive.c) | P2 | multiple errors from one malformed construct |
| [pp-missing-left-hand-side-of-hash-hash-operator.c](tests/fixtures/diagnostics/pp-missing-left-hand-side-of-hash-hash-operator.c) | P2 | malformed replacement list is only diagnosed on redefinition |
| [pp-missing-newline-after-line-directive.c](tests/fixtures/diagnostics/pp-missing-newline-after-line-directive.c) | P2 | #line loses source spelling, changes the filename too early, and leaks tokens into the parser; multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-missing-on-off-switch-in-s-t-d-c-pragma.c](tests/fixtures/diagnostics/pp-missing-on-off-switch-in-s-t-d-c-pragma.c) | P2 | message says the switch belongs after the bad token itself |
| [pp-missing-opening-parenthesis-in-pragma-operator.c](tests/fixtures/diagnostics/pp-missing-opening-parenthesis-in-pragma-operator.c) | P2 | multiple errors from one malformed construct |
| [pp-missing-right-hand-side-of-hash-hash-operator.c](tests/fixtures/diagnostics/pp-missing-right-hand-side-of-hash-hash-operator.c) | P2 | malformed replacement list is only diagnosed on redefinition |
| [pp-modulo-by-zero.c](tests/fixtures/diagnostics/pp-modulo-by-zero.c) | P2 | expression diagnostic highlights the following #endif |
| [pp-modulo-overflow.c](tests/fixtures/diagnostics/pp-modulo-overflow.c) | P2 | wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-modulo-without-rhs.c](tests/fixtures/diagnostics/pp-modulo-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-multiply-overflow.c](tests/fixtures/diagnostics/pp-multiply-overflow.c) | P2 | wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-multiply-without-rhs.c](tests/fixtures/diagnostics/pp-multiply-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-no-condition-in-if-directive.c](tests/fixtures/diagnostics/pp-no-condition-in-if-directive.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-not-equals-without-rhs.c](tests/fixtures/diagnostics/pp-not-equals-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-right-shift-overflow.c](tests/fixtures/diagnostics/pp-right-shift-overflow.c) | P2 | invalid shift count is described as arithmetic overflow; wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-right-shift-without-rhs.c](tests/fixtures/diagnostics/pp-right-shift-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-ternary-operator-without-mhs.c](tests/fixtures/diagnostics/pp-ternary-operator-without-mhs.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct |
| [pp-ternary-operator-without-rhs.c](tests/fixtures/diagnostics/pp-ternary-operator-without-rhs.c) | P2 | expression diagnostic highlights the following #endif; grammar/range explanation lacks a C99 citation |
| [pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c) | P2 | multiple errors from one malformed construct |
| [pp-token-merging-error.c](tests/fixtures/diagnostics/pp-token-merging-error.c) | P2 | multiple errors from one malformed construct; primary label repeats the headline expectation |
| [pp-unary-minus-overflow.c](tests/fixtures/diagnostics/pp-unary-minus-overflow.c) | P2 | wrong N1256 paragraph for #if replacement/arithmetic; expression diagnostic highlights the following #endif |
| [pp-unary-minus-without-operand.c](tests/fixtures/diagnostics/pp-unary-minus-without-operand.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-unary-plus-without-operand.c](tests/fixtures/diagnostics/pp-unary-plus-without-operand.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [pp-unexpected-end-of-input.c](tests/fixtures/diagnostics/pp-unexpected-end-of-input.c) | P2 | multiple errors from one malformed construct; message repeats expression in expression; grammar/range explanation lacks a C99 citation |
| [pp-unexpected-token-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-unexpected-token-in-preprocessor-expression.c) | P2 | expression diagnostic highlights the following #endif; multiple errors from one malformed construct |
| [pp-unknown-pragma-s-t-d-c-argument.c](tests/fixtures/diagnostics/pp-unknown-pragma-s-t-d-c-argument.c) | P2 | multiple errors from one malformed construct |
| [pp-unterminated-escape-sequence.c](tests/fixtures/diagnostics/pp-unterminated-escape-sequence.c) | P2 | multiple errors from one malformed construct |
| [pp-unterminated-opening-parenthesis-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-unterminated-opening-parenthesis-in-preprocessor-expression.c) | P2 | expression diagnostic highlights the following #endif |
| [tokenizer-angle-header-eof.c](tests/fixtures/diagnostics/tokenizer-angle-header-eof.c) | P2 | unclosed angle header produces misleading include errors and duplicate newline warnings; multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [tokenizer-angle-header-newline.c](tests/fixtures/diagnostics/tokenizer-angle-header-newline.c) | P2 | unclosed angle header is misreported as a missing file; grammar/range explanation lacks a C99 citation |
| [tokenizer-quoted-header-eof.c](tests/fixtures/diagnostics/tokenizer-quoted-header-eof.c) | P2 | unterminated quoted header triggers file lookup and an empty-unit cascade; multiple errors from one malformed construct |
| [tokenizer-unterminated-character.c](tests/fixtures/diagnostics/tokenizer-unterminated-character.c) | P2 | multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [tokenizer-unterminated-string.c](tests/fixtures/diagnostics/tokenizer-unterminated-string.c) | P2 | multiple errors from one malformed construct; grammar/range explanation lacks a C99 citation |
| [parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c](tests/fixtures/diagnostics/parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-enum-specifier-without-name-and-body.c](tests/fixtures/diagnostics/parser-enum-specifier-without-name-and-body.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-curly-brace-in-compound-statement.c](tests/fixtures/diagnostics/parser-expected-closing-curly-brace-in-compound-statement.c) | P3 | grammar/range explanation lacks a C99 citation; primary label repeats the headline expectation |
| [parser-expected-closing-curly-brace-in-initializer-list.c](tests/fixtures/diagnostics/parser-expected-closing-curly-brace-in-initializer-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-curly-brace-in-struct-declaration-list.c](tests/fixtures/diagnostics/parser-expected-closing-curly-brace-in-struct-declaration-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-parenthesis-after-parenthesized-declarator.c](tests/fixtures/diagnostics/parser-expected-closing-parenthesis-after-parenthesized-declarator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-parenthesis-in-statement.c](tests/fixtures/diagnostics/parser-expected-closing-parenthesis-in-statement.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-square-bracket-in-array-designator.c](tests/fixtures/diagnostics/parser-expected-closing-square-bracket-in-array-designator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-square-bracket-in-array-direct-declarator.c](tests/fixtures/diagnostics/parser-expected-closing-square-bracket-in-array-direct-declarator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-closing-square-bracket-in-subscript.c](tests/fixtures/diagnostics/parser-expected-closing-square-bracket-in-subscript.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-colon-in-label.c](tests/fixtures/diagnostics/parser-expected-colon-in-label.c) | P3 | default keyword is not quoted; grammar/range explanation lacks a C99 citation |
| [parser-expected-comma-or-closing-curly-in-enumerator-list.c](tests/fixtures/diagnostics/parser-expected-comma-or-closing-curly-in-enumerator-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-comma-or-closing-parenthesis-in-function-call.c](tests/fixtures/diagnostics/parser-expected-comma-or-closing-parenthesis-in-function-call.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.c](tests/fixtures/diagnostics/parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.c](tests/fixtures/diagnostics/parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-comma-or-semicolon-in-struct-declarator-list.c](tests/fixtures/diagnostics/parser-expected-comma-or-semicolon-in-struct-declarator-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declaration-continuation-after-declarator-long-double.c](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator-long-double.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declaration-continuation-after-declarator-tab.c](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator-tab.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declaration-continuation-after-declarator.c](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.c](tests/fixtures/diagnostics/parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.c](tests/fixtures/diagnostics/parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declarator-in-declaration-include.c](tests/fixtures/diagnostics/parser-expected-declarator-in-declaration-include.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declarator-in-declaration.c](tests/fixtures/diagnostics/parser-expected-declarator-in-declaration.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-declarator-in-typedef.c](tests/fixtures/diagnostics/parser-expected-declarator-in-typedef.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.c](tests/fixtures/diagnostics/parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-function-body.c](tests/fixtures/diagnostics/parser-expected-function-body.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-goto-label.c](tests/fixtures/diagnostics/parser-expected-goto-label.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-member-identifier.c](tests/fixtures/diagnostics/parser-expected-member-identifier.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-opening-parenthesis-in-statement.c](tests/fixtures/diagnostics/parser-expected-opening-parenthesis-in-statement.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-parameter-declaration-after-comma-in-function-declarator.c](tests/fixtures/diagnostics/parser-expected-parameter-declaration-after-comma-in-function-declarator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-semicolon-in-statement.c](tests/fixtures/diagnostics/parser-expected-semicolon-in-statement.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-expected-statement-expression-macro.c](tests/fixtures/diagnostics/parser-expected-statement-expression-macro.c) | P3 | missing macro operand has no expansion context; grammar/range explanation lacks a C99 citation; primary label repeats the headline expectation |
| [parser-expected-statement-expression-operand.c](tests/fixtures/diagnostics/parser-expected-statement-expression-operand.c) | P3 | grammar/range explanation lacks a C99 citation; primary label repeats the headline expectation |
| [parser-expected-statement-expression-operator.c](tests/fixtures/diagnostics/parser-expected-statement-expression-operator.c) | P3 | declaration-only help is weak in an expression statement; grammar/range explanation lacks a C99 citation |
| [parser-expected-statement-expression.c](tests/fixtures/diagnostics/parser-expected-statement-expression.c) | P3 | grammar/range explanation lacks a C99 citation; primary label repeats the headline expectation |
| [parser-expected-statement.c](tests/fixtures/diagnostics/parser-expected-statement.c) | P3 | grammar/range explanation lacks a C99 citation; primary label repeats the headline expectation |
| [parser-inline-specified-twice.c](tests/fixtures/diagnostics/parser-inline-specified-twice.c) | P3 | inline warning lacks a C99 citation |
| [parser-macro-expanded-keyword.c](tests/fixtures/diagnostics/parser-macro-expanded-keyword.c) | P3 | macro diagnostic has no invocation-site label; grammar/range explanation lacks a C99 citation |
| [parser-storage-class-redefinition.c](tests/fixtures/diagnostics/parser-storage-class-redefinition.c) | P3 | prior storage-class location is missing |
| [parser-struct-or-union-specifier-without-name-and-body.c](tests/fixtures/diagnostics/parser-struct-or-union-specifier-without-name-and-body.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-type-qualifiers-without-declarator.c](tests/fixtures/diagnostics/parser-type-qualifiers-without-declarator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-unexpected-end-before-type-specifier.c](tests/fixtures/diagnostics/parser-unexpected-end-before-type-specifier.c) | P3 | primary label repeats the headline expectation |
| [parser-unexpected-end-of-function-declarator-parameter-list.c](tests/fixtures/diagnostics/parser-unexpected-end-of-function-declarator-parameter-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [parser-unexpected-end-of-variadic-function-declarator-parameter-list.c](tests/fixtures/diagnostics/parser-unexpected-end-of-variadic-function-declarator-parameter-list.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-elif-directive-without-if-directive.c](tests/fixtures/diagnostics/pp-elif-directive-without-if-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-else-directive-without-if-directive.c](tests/fixtures/diagnostics/pp-else-directive-without-if-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-error-directive.c](tests/fixtures/diagnostics/pp-error-directive.c) | P3 | #error fixed prefix is not quoted |
| [pp-expected-identifier-in-ifdef-directive.c](tests/fixtures/diagnostics/pp-expected-identifier-in-ifdef-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-expected-identifier-in-ifndef-directive.c](tests/fixtures/diagnostics/pp-expected-identifier-in-ifndef-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-expected-identifier-in-undef-directive.c](tests/fixtures/diagnostics/pp-expected-identifier-in-undef-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-extra-tokens-after-ifdef-directive.c](tests/fixtures/diagnostics/pp-extra-tokens-after-ifdef-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-extra-tokens-after-ifndef-directive.c](tests/fixtures/diagnostics/pp-extra-tokens-after-ifndef-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-extra-tokens-after-include-directive.c](tests/fixtures/diagnostics/pp-extra-tokens-after-include-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-extra-tokens-after-pragma-operator.c](tests/fixtures/diagnostics/pp-extra-tokens-after-pragma-operator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-forced-signed-promotion.c](tests/fixtures/diagnostics/pp-forced-signed-promotion.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-forced-unsigned-promotion.c](tests/fixtures/diagnostics/pp-forced-unsigned-promotion.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-function-call-operator-not-supported-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-function-call-operator-not-supported-in-preprocessor-expression.c) | P3 | wrong N1256 paragraph for #if replacement/arithmetic |
| [pp-header-not-found.c](tests/fixtures/diagnostics/pp-header-not-found.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-hex-escape-sequence-too-large.c](tests/fixtures/diagnostics/pp-hex-escape-sequence-too-large.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-integer-literal-overflow.c](tests/fixtures/diagnostics/pp-integer-literal-overflow.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-invalid-hex-escape-sequence.c](tests/fixtures/diagnostics/pp-invalid-hex-escape-sequence.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-large-unicode-escape-sequence-too-small.c](tests/fixtures/diagnostics/pp-large-unicode-escape-sequence-too-small.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-missing-closing-parenthesis-in-pragma-operator.c](tests/fixtures/diagnostics/pp-missing-closing-parenthesis-in-pragma-operator.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-missing-number-in-line-directive.c](tests/fixtures/diagnostics/pp-missing-number-in-line-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-more-endif-directives-than-if-directives.c](tests/fixtures/diagnostics/pp-more-endif-directives-than-if-directives.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-more-if-directives-than-endif-directives.c](tests/fixtures/diagnostics/pp-more-if-directives-than-endif-directives.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-no-condition-in-elif-directive.c](tests/fixtures/diagnostics/pp-no-condition-in-elif-directive.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-s-t-d-c-pragma-directive-without-argument.c](tests/fixtures/diagnostics/pp-s-t-d-c-pragma-directive-without-argument.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-small-unicode-escape-sequence-too-short.c](tests/fixtures/diagnostics/pp-small-unicode-escape-sequence-too-short.c) | P3 | grammar/range explanation lacks a C99 citation |
| [pp-undefined-identifier-in-preprocessor-expression.c](tests/fixtures/diagnostics/pp-undefined-identifier-in-preprocessor-expression.c) | P3 | wrong N1256 paragraph for #if replacement/arithmetic |
| [tokenizer-unknown-token-backtick.c](tests/fixtures/diagnostics/tokenizer-unknown-token-backtick.c) | P3 | backtick source spelling uses a different quoting style |

## Actual output and suggested changes

All output blocks below are complete captured stderr, including summaries. For the NUL fixture only, the invisible byte is written as `\0` in this report so Markdown remains readable; the .stderr file retains the exact byte 00. Panic thread IDs are the actual IDs in the captured golden files.

### pp-line-directive-suffix-panic.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-line-directive-suffix-panic.stderr)):

```text

thread 'main' (4504) panicked at src\translation_phases\preprocessing.rs:6447:37:
attempt to subtract with overflow
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

- **P1: malformed #line panics.** Suggested fix: Validate the complete digit sequence before accumulating the number; stop after a non-digit. Exclude the number-token NUL terminator and use checked arithmetic. Render LineDirectiveIsNotASimpleDigitSequence at `1u`, with C99 §6.10.4p3, instead of Rust panic output.

### pp-redefinition-of-built-in-macro.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-redefinition-of-built-in-macro.stderr)):

```text

thread 'main' (26888) panicked at src\translation_phases\preprocessing.rs:6263:21:
internal error: entered unreachable code: The case where name is a built-in macro is handled above
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

- **P1: predefined macro redefinition panics before any source diagnostic is printed.** Suggested fix: Stop handling this definition after queuing RedefinitionOfBuiltInMacro, rather than entering the BuiltIn unreachable arm. Print a deterministic source diagnostic for `__LINE__`, citing C99 §6.10.8p4. The panic exposes Rust internals and a changing OS thread ID.

### tokenizer-unknown-token-nul.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-unknown-token-nul.stderr)):

```text
error: unexpected character U+0000 in source
 --> tokenizer-unknown-token-nul.c:1:11
  |
1 | int value;\0
  |           ^ no C token starts with this character
  |
  = note: C99 §5.2.1: `@`, `$`, and `` ` `` are not part of the basic source character set; outside literals and comments they cannot appear

1 error generated.
```

- **P1: renderer writes an actual NUL byte.** Suggested fix: Escape control characters in echoed source lines, not only in the headline. Display the NUL as `\0` or `U+0000` and apply the same display-width mapping to labels. The sibling .stderr contains byte 00; the headline itself is already safe.
- **P3: basic-character-set note does not explain the actual character.** Suggested fix: Replace the fixed note about @, $, and backtick with wording relevant to U+0000, or use a generic statement about accepted source characters. The current note does not support this particular error.

### parser-expected-declaration-continuation-after-declarator-missing-semicolon.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator-missing-semicolon.stderr)):

```text
error: expected a function body, found end of file
 --> parser-expected-declaration-continuation-after-declarator-missing-semicolon.c:3:1
  |
1 | int value
  |     ------ help: add `;` here
  |     |
  |     not a function, so later declarations were read as its parameters
2 | int other;
3 | 
  | ^ expected `{`

1 error generated.
```

- **P2: object declaration is reported as a missing function body.** Suggested fix: Recognize that `value` is an object declarator and recover an omitted declaration terminator. Say expected `;`, found keyword `int` at line 2, with a zero-width insertion after `value` on line 1. Do not parse `int other;` as old-style parameters. The help marker currently underlines the name and newline rather than the insertion point.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-statement-expression-assignment.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-statement-expression-assignment.stderr)):

```text
error: invalid left-hand side of assignment
 --> parser-expected-statement-expression-assignment.c:1:21
  |
1 | int f(void) { a + b = 1; }
  |                     ^ cannot assign to this expression
  |
  = note: C99 §6.5.16: the left operand of an assignment operator is a unary expression, such as a name, `*p`, `a[i]`, or `s.m`

1 error generated.
```

- **P2: cannot-assign label points at = rather than the left expression.** Suggested fix: Highlight `a + b` (columns 15-19) for the left-operand diagnostic, or keep `=` primary with a secondary left-operand label. Say the assignment grammar requires a unary expression on the left; avoid implying semantic lvalue analysis has been performed. Cite C99 §6.5.16p1 for this syntax failure.

### parser-unicode-identifier.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-unicode-identifier.stderr)):

```text
error: missing type specifier
 --> parser-unicode-identifier.c:1:12
  |
1 | int value; λ
  |            ^ expected a type such as `int` before this
  |
  = note: C99 §6.7.2p2: every declaration needs at least one type specifier; C99 removed implicit `int`

error: expected `,`, `=`, `;`, or a function body after the declarator, found end of file
 --> parser-unicode-identifier.c:2:1
  |
2 | 
  | ^ expected one of `,`, `=`, `;`, or `{`

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-binary-minus-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-binary-minus-overflow.stderr)):

```text
error: subtraction overflows in `#if` expression
 --> pp-binary-minus-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-binary-minus-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-binary-minus-without-rhs.stderr)):

```text
error: expected an expression after `-`
 --> pp-binary-minus-without-rhs.c:2:1
  |
2 | #endif
  | ^ `-` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-binary-plus-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-binary-plus-overflow.stderr)):

```text
error: addition overflows in `#if` expression
 --> pp-binary-plus-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-binary-plus-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-binary-plus-without-rhs.stderr)):

```text
error: expected an expression after `+`
 --> pp-binary-plus-without-rhs.c:2:1
  |
2 | #endif
  | ^ `+` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-bitwise-and-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-bitwise-and-without-rhs.stderr)):

```text
error: expected an expression after `&`
 --> pp-bitwise-and-without-rhs.c:2:1
  |
2 | #endif
  | ^ `&` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-bitwise-not-without-operand.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-bitwise-not-without-operand.stderr)):

```text
error: expected an expression after `~`
 --> pp-bitwise-not-without-operand.c:1:7
  |
1 | #if ~)
  |       ^ `~` needs an operand here

error: `#if` with no condition
 --> pp-bitwise-not-without-operand.c:2:1
  |
2 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, as in `#if VERSION >= 2`

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-bitwise-or-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-bitwise-or-without-rhs.stderr)):

```text
error: expected an expression after `|`
 --> pp-bitwise-or-without-rhs.c:2:1
  |
2 | #endif
  | ^ `|` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-bitwise-xor-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-bitwise-xor-without-rhs.stderr)):

```text
error: expected an expression after `^`
 --> pp-bitwise-xor-without-rhs.c:2:1
  |
2 | #endif
  | ^ `^` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-cannot-use-hash-hash-after-function-like-macro-call.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-cannot-use-hash-hash-after-function-like-macro-call.stderr)):

```text
error: `##` cannot follow a function-like macro invocation
 --> pp-cannot-use-hash-hash-after-function-like-macro-call.c:1:13
  |
1 | #define F(a)##a
  |             ^^ pasting onto an invocation is not supported

1 error generated.
```

- **P2: message describes an invocation although the bad token starts a replacement list.** Suggested fix: For this source, report `## cannot start a replacement list` at the definition. The invocation is only when the invalid replacement list is discovered. Validate each fresh definition and use the MissingLeftHandSideOfHashHashOperator explanation, with C99 §6.10.3.3p1.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.3.3p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-colon-without-matching-question-mark.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-colon-without-matching-question-mark.stderr)):

```text
error: `:` without a matching `?`
 --> pp-colon-without-matching-question-mark.c:1:7
  |
1 | #if 1 : 2
  |       ^ no `?` precedes this `:` in the same parentheses

error: expected an operator, found number `2`
 --> pp-colon-without-matching-question-mark.c:1:9
  |
1 | #if 1 : 2
  |         ^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5.15p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.stderr)):

```text
error: expected an operator, found `defined`
 --> pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c:1:7
  |
1 | #if 1 defined X
  |       ^^^^^^^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

error: expected an operator, found identifier `X`
 --> pp-defined-operator-instead-of-binary-operator-in-preprocessor-expression.c:1:15
  |
1 | #if 1 defined X
  |               ^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-divide-by-zero.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-divide-by-zero.stderr)):

```text
error: division by zero in `#if` expression
 --> pp-divide-by-zero.c:2:1
  |
2 | #endif
  | ^ the divisor is zero
  |
  = note: C99 §6.5.5p5: the result of `/` by zero is undefined

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-divide-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-divide-overflow.stderr)):

```text
error: division overflows in `#if` expression
 --> pp-divide-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-divide-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-divide-without-rhs.stderr)):

```text
error: expected an expression after `/`
 --> pp-divide-without-rhs.c:2:1
  |
2 | #endif
  | ^ `/` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-empty-parentheses-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-empty-parentheses-in-preprocessor-expression.stderr)):

```text
error: expected an expression inside `()`
 --> pp-empty-parentheses-in-preprocessor-expression.c:1:6
  |
1 | #if ()
  |      ^ empty parentheses
  |
  = note: C99 §6.10.1p1: the condition must be an integer constant expression

error: unclosed `(` in `#if` expression
 --> pp-empty-parentheses-in-preprocessor-expression.c:2:1
  |
2 | #endif
  | ^ expected `)`

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-equals-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-equals-without-rhs.stderr)):

```text
error: expected an expression after `==`
 --> pp-equals-without-rhs.c:2:1
  |
2 | #endif
  | ^ `==` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.stderr)):

```text
error: expected an operator, found `!`
 --> pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c:1:7
  |
1 | #if 1 ! 2
  |       ^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

error: expected an operator, found number `2`
 --> pp-exclamation-mark-instead-of-binary-operator-in-preprocessor-expression.c:1:9
  |
1 | #if 1 ! 2
  |         ^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-expected-binary-operator-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-expected-binary-operator-in-preprocessor-expression.stderr)):

```text
error: expected an expression after `?`
 --> pp-expected-binary-operator-in-preprocessor-expression.c:1:12
  |
1 | #if (1 ? 2)
  |            ^ `?` needs an operand here

error: expected an operator between the operands of `#if`
 --> pp-expected-binary-operator-in-preprocessor-expression.c:2:1
  |
2 | #endif
  | ^ the expression ends with an operand still unjoined
  |
  = note: C99 §6.10.1p1: the condition must be an integer constant expression

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-expected-newline-after-undef-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-expected-newline-after-undef-directive.stderr)):

```text
error: unexpected identifier `extra` after the macro name in `#undef`
 --> pp-expected-newline-after-undef-directive.c:1:10
  |
1 | #undef F extra
  |          ^^^^^ `#undef` takes one name

error: expected `;`, found keyword `int`
 --> pp-expected-newline-after-undef-directive.c:2:1
  |
1 | #undef F extra
  |               - help: add `;` here
2 | int value;
  | ^^^ expected one of `,`, `=`, `;`, or `{`

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.stderr)):

```text
error: expected an expression after `+`
 --> pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c:2:1
  |
2 | #endif
  | ^ `+` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-extra-tokens-after-pragma-once.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-extra-tokens-after-pragma-once.stderr)):

```text
warning: `#pragma once` in main file
 --> pp-extra-tokens-after-pragma-once.c:1:9
  |
1 | #pragma once extra
  |         ^^^^ only affects files that are included

warning: unexpected identifier `once` after `#pragma once`
 --> pp-extra-tokens-after-pragma-once.c:1:9
  |
1 | #pragma once extra
  |         ^^^^ `#pragma once` takes no arguments

2 warnings generated.
```

- **P2: wrong token spelling and caret for extra pragma tokens.** Suggested fix: Build the extra-token diagnostic with the `extra` token source vectors (t.source_vectors), rather than those for `once`. It should say found identifier `extra` and underline columns 14-18. The independent main-file warning may remain.

### pp-greater-than-equals-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-greater-than-equals-without-rhs.stderr)):

```text
error: expected an expression after `>=`
 --> pp-greater-than-equals-without-rhs.c:2:1
  |
2 | #endif
  | ^ `>=` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-greater-than-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-greater-than-without-rhs.stderr)):

```text
error: expected an expression after `>`
 --> pp-greater-than-without-rhs.c:2:1
  |
2 | #endif
  | ^ `>` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-header-not-found-system.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-header-not-found-system.stderr)):

```text
error: cannot find header `missing-diagnostics-header.h`
 --> pp-header-not-found-system.c:1:10
  |
1 | #include <missing-diagnostics-header.h>
  |          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ not found in any search directory
  |
  = note: the path is absolute; no directory was searched
  = help: add the directory containing it with `--isystem <dir>`

1 error generated.
```

- **P2: relative system-header name is described as absolute.** Suggested fix: Distinguish an absolute include path from an empty configured system search list. Here the name is relative and no system directories are configured: say exactly that, then retain the --isystem help.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-left-shift-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-left-shift-overflow.stderr)):

```text
error: left shift overflows in `#if` expression
 --> pp-left-shift-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P2: invalid shift count is described as arithmetic overflow.** Suggested fix: For the count 64, say `shift count is outside the supported width` and underline `64`; cite C99 §6.5.7p3. Keep an arithmetic-overflow message only for an overflowing signed left-shift value.
- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-left-shift-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-left-shift-without-rhs.stderr)):

```text
error: expected an expression after `<<`
 --> pp-left-shift-without-rhs.c:2:1
  |
2 | #endif
  | ^ `<<` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-less-than-equals-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-less-than-equals-without-rhs.stderr)):

```text
error: expected an expression after `<=`
 --> pp-less-than-equals-without-rhs.c:2:1
  |
2 | #endif
  | ^ `<=` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-less-than-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-less-than-without-rhs.stderr)):

```text
error: expected an expression after `<`
 --> pp-less-than-without-rhs.c:2:1
  |
2 | #endif
  | ^ `<` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-line-directive-is-not-a-simple-digit-sequence.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-line-directive-is-not-a-simple-digit-sequence.stderr)):

```text
warning: `#line` needs a plain decimal line number
 --> pp-line-directive-is-not-a-simple-digit-sequence.c:1:7
  |
1 | #line 12
  |       ^^ not a digit sequence
  |
  = note: C99 §6.10.4p3: the line number is a digit sequence, not any constant

1 warning generated.
```

- **P2: valid decimal #line is rejected.** Suggested fix: For the `#line 12` fixture, trim only the tokenizer sentinel before digit validation. Do not treat the sentinel as C source. A valid digit sequence should produce no diagnostic; this fixture deliberately records the current false warning. Keep the separate `1u` crash case as an invalid-input regression.

### pp-line-directive-number-too-large.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-line-directive-number-too-large.stderr)):

```text
warning: `#line` needs a plain decimal line number
 --> pp-line-directive-number-too-large.c:1:7
  |
1 | #line 2147483648
  |       ^^^^^^^^^^ not a digit sequence
  |
  = note: C99 §6.10.4p3: the line number is a digit sequence, not any constant

warning: line number 2147483648 is out of range
 --> pp-line-directive-number-too-large.c:1:7
  |
1 | #line 2147483648
  |       ^^^^^^^^^^ too large
  |
  = note: C99 §6.10.4p3: the line number must be at most 2147483647

2 warnings generated.
```

- **P2: out-of-range #line also gets a false non-digit warning.** Suggested fix: Remove the NUL sentinel from numeric validation. Keep the out-of-range diagnostic at 2147483648, but do not additionally call that all-digit spelling a non-digit sequence.

### pp-logical-and-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-logical-and-without-rhs.stderr)):

```text
error: expected an expression after `&&`
 --> pp-logical-and-without-rhs.c:2:1
  |
2 | #endif
  | ^ `&&` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-logical-not-without-operand.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-logical-not-without-operand.stderr)):

```text
error: expected an expression after `!`
 --> pp-logical-not-without-operand.c:1:7
  |
1 | #if !)
  |       ^ `!` needs an operand here

error: `#if` with no condition
 --> pp-logical-not-without-operand.c:2:1
  |
2 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, as in `#if VERSION >= 2`

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-logical-or-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-logical-or-without-rhs.stderr)):

```text
error: expected an expression after `||`
 --> pp-logical-or-without-rhs.c:2:1
  |
2 | #endif
  | ^ `||` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-missing-closing-parenthesis-in-defined-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-closing-parenthesis-in-defined-directive.stderr)):

```text
error: expected `)` to close `defined(`, found number `1`
 --> pp-missing-closing-parenthesis-in-defined-directive.c:1:15
  |
1 | #if defined(X 1)
  |               ^ expected `)`

error: expected an expression inside `()`
 --> pp-missing-closing-parenthesis-in-defined-directive.c:1:16
  |
1 | #if defined(X 1)
  |                ^ empty parentheses
  |
  = note: C99 §6.10.1p1: the condition must be an integer constant expression

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-missing-identifier-in-defined-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-identifier-in-defined-directive.stderr)):

```text
error: expected a macro name inside `defined(`, found number `1`
 --> pp-missing-identifier-in-defined-directive.c:1:13
  |
1 | #if defined(1)
  |             ^ expected a macro name
  |
  = note: C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`

error: expected an expression inside `()`
 --> pp-missing-identifier-in-defined-directive.c:1:14
  |
1 | #if defined(1)
  |              ^ empty parentheses
  |
  = note: C99 §6.10.1p1: the condition must be an integer constant expression

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-missing-left-hand-side-of-hash-hash-operator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-left-hand-side-of-hash-hash-operator.stderr)):

```text
error: `##` cannot start a replacement list
 --> pp-missing-left-hand-side-of-hash-hash-operator.c:2:13
  |
2 | #define F(a)##a
  |             ^^ nothing precedes this `##`
  |
  = note: C99 §6.10.3.3p1: `##` needs a token on each side

1 error generated.
```

- **P2: malformed replacement list is only diagnosed on redefinition.** Suggested fix: Validate the first definition as well as redefinitions. In this fixture both definitions are invalid, but only line 2 is reported. The comparison loop should not own initial replacement-list validation; leading whitespace must not hide a first-position `##`. Retain the appropriate C99 §6.10.3.3p1 citation.

### pp-missing-newline-after-line-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-newline-after-line-directive.stderr)):

```text
warning: `#line` needs a plain decimal line number
 --> pp-missing-newline-after-line-directive.c:1:7
  |
1 | #line 5 "file.c" extra
  |       ^ not a digit sequence
  |
  = note: C99 §6.10.4p3: the line number is a digit sequence, not any constant

error: unexpected identifier after `#line`
 --> "file.c":5:18

error: expected `;`, found keyword `int`
 --> "file.c":6:1

2 errors and 1 warning generated.
```

- **P2: #line loses source spelling, changes the filename too early, and leaks tokens into the parser.** Suggested fix: Report `extra` at the physical directive before applying line-control changes; strip the filename literal delimiters and preserve physical-to-presumed source mapping for snippets. Consume the rest of the invalid directive instead of turning `extra` into a declaration. Do not warn that the valid number `5` is non-decimal.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.4p3-4 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-missing-on-off-switch-in-s-t-d-c-pragma.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-on-off-switch-in-s-t-d-c-pragma.stderr)):

```text
error: expected `ON`, `OFF`, or `DEFAULT` after `MAYBE`
 --> pp-missing-on-off-switch-in-s-t-d-c-pragma.c:1:26
  |
1 | #pragma STDC FP_CONTRACT MAYBE
  |                          ^^^^^ expected an on-off switch
  |
  = note: C99 §6.10.6p2: each standard pragma takes an on-off switch

1 error generated.
```

- **P2: message says the switch belongs after the bad token itself.** Suggested fix: Say `expected ON, OFF, or DEFAULT for FP_CONTRACT, found identifier MAYBE`, with backticks around code. Carry the pragma name and the found-token spelling separately; the current payload puts `MAYBE` in the position of the pragma name.

### pp-missing-opening-parenthesis-in-pragma-operator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-opening-parenthesis-in-pragma-operator.stderr)):

```text
error: expected `(` after `_Pragma`, found string literal `"once"`
 --> pp-missing-opening-parenthesis-in-pragma-operator.c:1:9
  |
1 | _Pragma "once"
  |         ^^^^^^ expected `(`
  |
  = note: C99 §6.10.9: write `_Pragma("...")`

warning: `#pragma once` in main file
 --> <pragma string>:1:1
  |
1 | once
  | ^^^^ only affects files that are included

error: expected `)` to close `_Pragma(`, found end of line
 --> pp-missing-opening-parenthesis-in-pragma-operator.c:1:15
  |
1 | _Pragma "once"
  |               ^ expected `)`

2 errors and 1 warning generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-missing-right-hand-side-of-hash-hash-operator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-right-hand-side-of-hash-hash-operator.stderr)):

```text
error: `##` cannot end a replacement list
 --> pp-missing-right-hand-side-of-hash-hash-operator.c:2:14
  |
2 | #define F(a)a##
  |              ^^ nothing follows this `##`
  |
  = note: C99 §6.10.3.3p1: `##` needs a token on each side

1 error generated.
```

- **P2: malformed replacement list is only diagnosed on redefinition.** Suggested fix: Validate the first definition as well as redefinitions. In this fixture both definitions are invalid, but only line 2 is reported. The comparison loop should not own initial replacement-list validation; leading whitespace must not hide a first-position `##`. Retain the appropriate C99 §6.10.3.3p1 citation.

### pp-modulo-by-zero.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-modulo-by-zero.stderr)):

```text
error: remainder by zero in `#if` expression
 --> pp-modulo-by-zero.c:2:1
  |
2 | #endif
  | ^ the divisor is zero
  |
  = note: C99 §6.5.5p5: the result of `%` by zero is undefined

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-modulo-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-modulo-overflow.stderr)):

```text
error: remainder overflows in `#if` expression
 --> pp-modulo-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-modulo-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-modulo-without-rhs.stderr)):

```text
error: expected an expression after `%`
 --> pp-modulo-without-rhs.c:2:1
  |
2 | #endif
  | ^ `%` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-multiply-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-multiply-overflow.stderr)):

```text
error: multiplication overflows in `#if` expression
 --> pp-multiply-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-multiply-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-multiply-without-rhs.stderr)):

```text
error: expected an expression after `*`
 --> pp-multiply-without-rhs.c:2:1
  |
2 | #endif
  | ^ `*` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-no-condition-in-if-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-no-condition-in-if-directive.stderr)):

```text
error: `#if` with no condition
 --> pp-no-condition-in-if-directive.c:2:1
  |
2 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, as in `#if VERSION >= 2`

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-not-equals-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-not-equals-without-rhs.stderr)):

```text
error: expected an expression after `!=`
 --> pp-not-equals-without-rhs.c:2:1
  |
2 | #endif
  | ^ `!=` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-right-shift-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-right-shift-overflow.stderr)):

```text
error: right shift overflows in `#if` expression
 --> pp-right-shift-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P2: invalid shift count is described as arithmetic overflow.** Suggested fix: For the count 64, say `shift count is outside the supported width` and underline `64`; cite C99 §6.5.7p3. Keep an arithmetic-overflow message only for an overflowing signed left-shift value.
- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-right-shift-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-right-shift-without-rhs.stderr)):

```text
error: expected an expression after `>>`
 --> pp-right-shift-without-rhs.c:2:1
  |
2 | #endif
  | ^ `>>` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-ternary-operator-without-mhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-ternary-operator-without-mhs.stderr)):

```text
error: expected an expression after `?`
 --> pp-ternary-operator-without-mhs.c:1:9
  |
1 | #if 1 ? : 2
  |         ^ `?` needs an operand here

error: expected an expression after `:`
 --> pp-ternary-operator-without-mhs.c:2:1
  |
2 | #endif
  | ^ `:` needs an operand here

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-ternary-operator-without-rhs.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-ternary-operator-without-rhs.stderr)):

```text
error: expected an expression after `:`
 --> pp-ternary-operator-without-rhs.c:2:1
  |
2 | #endif
  | ^ `:` needs an operand here

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-tilde-instead-of-binary-operator-in-preprocessor-expression.stderr)):

```text
error: expected an operator, found `~`
 --> pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c:1:7
  |
1 | #if 1 ~ 2
  |       ^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

error: expected an operator, found number `2`
 --> pp-tilde-instead-of-binary-operator-in-preprocessor-expression.c:1:9
  |
1 | #if 1 ~ 2
  |         ^ expected a binary operator before this
  |
  = note: operands in an `#if` expression must be joined by operators

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-token-merging-error.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-token-merging-error.stderr)):

```text
error: pasting `+` and `*` does not give a valid token
 --> pp-token-merging-error.c:2:15
  |
2 | int value = F(+,*);
  |               ^^^ invalid token paste
  |
  = note: C99 §6.10.3.3p3: the result of `##` must be a single valid preprocessing token

error: expected an expression, found `;`
 --> pp-token-merging-error.c:2:19
  |
2 | int value = F(+,*);
  |                   ^ expected an expression

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected an expression. Keep informative related labels and insertion help.

### pp-unary-minus-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unary-minus-overflow.stderr)):

```text
error: negation overflows in `#if` expression
 --> pp-unary-minus-overflow.c:2:1
  |
2 | #endif
  | ^ the result is out of range
  |
  = note: C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 requires constant expressions to stay in range

1 error generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.
- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### pp-unary-minus-without-operand.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unary-minus-without-operand.stderr)):

```text
error: expected an expression after `-`
 --> pp-unary-minus-without-operand.c:1:7
  |
1 | #if -)
  |       ^ `-` needs an operand here

error: `#if` with no condition
 --> pp-unary-minus-without-operand.c:2:1
  |
2 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, as in `#if VERSION >= 2`

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-unary-plus-without-operand.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unary-plus-without-operand.stderr)):

```text
error: expected an expression after `+`
 --> pp-unary-plus-without-operand.c:1:7
  |
1 | #if +)
  |       ^ `+` needs an operand here

error: `#if` with no condition
 --> pp-unary-plus-without-operand.c:2:1
  |
2 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, as in `#if VERSION >= 2`

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 and §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-unexpected-end-of-input.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unexpected-end-of-input.stderr)):

```text
error: unexpected end of file while parsing function-like macro invocation
 --> pp-unexpected-end-of-input.c:2:13
  |
2 | int value = F(
  |             ^ the file ends here

error: expected an expression in expression, found end of file
 --> pp-unexpected-end-of-input.c:3:1
  |
3 | 
  | ^ expected an expression

2 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.
- **P3: message repeats expression in expression.** Suggested fix: Use `expected an expression, found end of file`, or name a specific context such as the initializer. Avoid inserting the generic context string expression into an already complete expected-expression phrase.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.3p4 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-unexpected-token-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unexpected-token-in-preprocessor-expression.stderr)):

```text
error: unexpected string literal `"abc"` in `#if` expression
 --> pp-unexpected-token-in-preprocessor-expression.c:1:5
  |
1 | #if "abc"
  |     ^^^^^ not valid in an integer constant expression
  |
  = note: C99 §6.10.1p1: the condition must be an integer constant expression

error: `#if` with no condition
 --> pp-unexpected-token-in-preprocessor-expression.c:2:1
  |
2 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, as in `#if VERSION >= 2`

2 errors generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.
- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-unknown-pragma-s-t-d-c-argument.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unknown-pragma-s-t-d-c-argument.stderr)):

```text
error: unknown `STDC` pragma `BANANAS`
 --> pp-unknown-pragma-s-t-d-c-argument.c:1:14
  |
1 | #pragma STDC BANANAS ON
  |              ^^^^^^^ not a standard pragma
  |
  = note: C99 §6.10.6: the standard pragmas are `FP_CONTRACT`, `FENV_ACCESS`, and `CX_LIMITED_RANGE`

error: missing type specifier
 --> pp-unknown-pragma-s-t-d-c-argument.c:1:22
  |
1 | #pragma STDC BANANAS ON
  |                      ^^ expected a type such as `int` before this
  |
  = note: C99 §6.7.2p2: every declaration needs at least one type specifier; C99 removed implicit `int`

error: expected `;`, found keyword `int`
 --> pp-unknown-pragma-s-t-d-c-argument.c:2:1
  |
1 | #pragma STDC BANANAS ON
  |                        - help: add `;` here
2 | int value;
  | ^^^ expected one of `,`, `=`, `;`, or `{`

3 errors generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Keep the first concrete failure for this malformed construct and recover without diagnosing consequences as independent mistakes. Avoid reducing a known-invalid expression or passing leftovers from a failed directive into phase 7. Do not suppress unrelated later errors merely because their location matches.

### pp-unterminated-escape-sequence.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unterminated-escape-sequence.stderr)):

```text
warning: no newline at end of file
 --> pp-unterminated-escape-sequence.c:1:20
  |
1 | char *value = "abc\
  |                    ^ the file ends without a newline
  |
  = note: C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character
  = help: add a newline at the end of the file

error: unterminated string literal
 --> pp-unterminated-escape-sequence.c:1:15
  |
1 | char *value = "abc\
  |               ^^^^^ the file ends before the closing `"`

error: expected `,`, `=`, `;`, or a function body after the declarator, found end of file
 --> pp-unterminated-escape-sequence.c:1:20
  |
1 | char *value = "abc\
  |                    ^ expected one of `,`, `=`, `;`, or `{`

2 errors and 1 warning generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Recover the already diagnosed malformed literal/header without emitting an additional missing declaration terminator, include lookup failure, or empty-unit error caused by that same malformed token. A distinct missing-final-newline warning is legitimate; duplicate copies are not.

### pp-unterminated-opening-parenthesis-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-unterminated-opening-parenthesis-in-preprocessor-expression.stderr)):

```text
error: unclosed `(` in `#if` expression
 --> pp-unterminated-opening-parenthesis-in-preprocessor-expression.c:2:1
  |
2 | #endif
  | ^ expected `)`

1 error generated.
```

- **P2: expression diagnostic highlights the following #endif.** Suggested fix: Preserve source vectors for expression operators/operands in the reducer. Place the primary caret on the offending operator, divisor, shift count, or unmatched opening delimiter on the #if line. For an absent operand, use the insertion point at the end of the #if condition. The current zero-length source created after consuming the newline lands on #endif.

### tokenizer-angle-header-eof.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-angle-header-eof.stderr)):

```text
warning: no newline at end of file
 --> tokenizer-angle-header-eof.c:1:14
  |
1 | #include <abc
  |              ^ the file ends without a newline
  |
  = note: C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character
  = help: add a newline at the end of the file

warning: no newline at end of file
 --> tokenizer-angle-header-eof.c:1:14
  |
1 | #include <abc
  |              ^ the file ends without a newline
  |
  = note: C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character
  = help: add a newline at the end of the file

error: unexpected end of file while parsing include directive
 --> tokenizer-angle-header-eof.c:1:2
  |
1 | #include <abc
  |  ^^^^^^^ the file ends here

error: cannot find header `abc`
 --> tokenizer-angle-header-eof.c:1:10
  |
1 | #include <abc
  |          ^^^^ not found in any search directory
  |
  = note: the path is absolute; no directory was searched
  = help: add the directory containing it with `--isystem <dir>`

error: translation unit is empty
 --> tokenizer-angle-header-eof.c:1:14
  |
1 | #include <abc
  |              ^ expected a declaration or function definition
  |
  = note: C99 §6.9: a translation unit contains at least one external declaration

3 errors and 2 warnings generated.
```

- **P2: unclosed angle header produces misleading include errors and duplicate newline warnings.** Suggested fix: Emit one unterminated-header diagnostic, retain at most one missing-final-newline warning, and stop include resolution for an invalid header token. Avoid a follow-on empty-translation-unit error caused by this already diagnosed input. The EOF diagnostic currently points at `include`, rather than the end of input.
- **P2: multiple errors from one malformed construct.** Suggested fix: Recover the already diagnosed malformed literal/header without emitting an additional missing declaration terminator, include lookup failure, or empty-unit error caused by that same malformed token. A distinct missing-final-newline warning is legitimate; duplicate copies are not.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### tokenizer-angle-header-newline.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-angle-header-newline.stderr)):

```text
error: cannot find header `abc`
 --> tokenizer-angle-header-newline.c:1:10
  |
1 | #include <abc
  |          ^^^^ not found in any search directory
  |
  = note: the path is absolute; no directory was searched
  = help: add the directory containing it with `--isystem <dir>`

1 error generated.
```

- **P2: unclosed angle header is misreported as a missing file.** Suggested fix: Recognize the missing `>` before trying header resolution. Diagnose an unterminated header name at `<abc` and recover to the directive newline. Do not claim the path is absolute.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### tokenizer-quoted-header-eof.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-quoted-header-eof.stderr)):

```text
warning: no newline at end of file
 --> tokenizer-quoted-header-eof.c:1:14
  |
1 | #include "abc
  |              ^ the file ends without a newline
  |
  = note: C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character
  = help: add a newline at the end of the file

error: unterminated header name
 --> tokenizer-quoted-header-eof.c:1:10
  |
1 | #include "abc
  |          ^^^^ the line ends before the closing delimiter
  |
  = note: C99 §6.4.7: a header name cannot span lines

error: unexpected end of file while parsing include directive
 --> tokenizer-quoted-header-eof.c:1:14
  |
1 | #include "abc
  |              ^ the file ends here

error: cannot find header `abc`
 --> tokenizer-quoted-header-eof.c:1:10
  |
1 | #include "abc
  |          ^^^^ not found in any search directory
  |
  = note: searched these directories:
            . (the working directory)
  = help: add the directory containing it with `--iquote <dir>` or `--isystem <dir>`

error: translation unit is empty
 --> tokenizer-quoted-header-eof.c:1:14
  |
1 | #include "abc
  |              ^ expected a declaration or function definition
  |
  = note: C99 §6.9: a translation unit contains at least one external declaration

4 errors and 1 warning generated.
```

- **P2: unterminated quoted header triggers file lookup and an empty-unit cascade.** Suggested fix: After diagnosing the missing quote, recover the include directive without attempting to find `abc`. Suppress the subsequent include-EOF and empty-unit errors that arise from that failed token. Prefer an EOF label when the physical file ends, rather than the synthetic line-end label.
- **P2: multiple errors from one malformed construct.** Suggested fix: Recover the already diagnosed malformed literal/header without emitting an additional missing declaration terminator, include lookup failure, or empty-unit error caused by that same malformed token. A distinct missing-final-newline warning is legitimate; duplicate copies are not.

### tokenizer-unterminated-character.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-unterminated-character.stderr)):

```text
warning: no newline at end of file
 --> tokenizer-unterminated-character.c:1:16
  |
1 | int value = 'a\
  |                ^ the file ends without a newline
  |
  = note: C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character
  = help: add a newline at the end of the file

error: unterminated character constant
 --> tokenizer-unterminated-character.c:1:13
  |
1 | int value = 'a\
  |             ^^^ the file ends before the closing `'`

error: expected `,`, `=`, `;`, or a function body after the declarator, found end of file
 --> tokenizer-unterminated-character.c:1:16
  |
1 | int value = 'a\
  |                ^ expected one of `,`, `=`, `;`, or `{`

2 errors and 1 warning generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Recover the already diagnosed malformed literal/header without emitting an additional missing declaration terminator, include lookup failure, or empty-unit error caused by that same malformed token. A distinct missing-final-newline warning is legitimate; duplicate copies are not.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.4.4 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### tokenizer-unterminated-string.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-unterminated-string.stderr)):

```text
warning: no newline at end of file
 --> tokenizer-unterminated-string.c:1:20
  |
1 | char *value = "abc\
  |                    ^ the file ends without a newline
  |
  = note: C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character
  = help: add a newline at the end of the file

error: unterminated string literal
 --> tokenizer-unterminated-string.c:1:15
  |
1 | char *value = "abc\
  |               ^^^^^ the file ends before the closing `"`

error: expected `,`, `=`, `;`, or a function body after the declarator, found end of file
 --> tokenizer-unterminated-string.c:1:20
  |
1 | char *value = "abc\
  |                    ^ expected one of `,`, `=`, `;`, or `{`

2 errors and 1 warning generated.
```

- **P2: multiple errors from one malformed construct.** Suggested fix: Recover the already diagnosed malformed literal/header without emitting an additional missing declaration terminator, include lookup failure, or empty-unit error caused by that same malformed token. A distinct missing-final-newline warning is legitimate; duplicate copies are not.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.stderr)):

```text
error: expected an identifier or `(` in the declarator, found integer constant `123`
 --> parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c:1:5
  |
1 | int 123;
  |     ^^^ expected a name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-enum-specifier-without-name-and-body.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-enum-specifier-without-name-and-body.stderr)):

```text
error: expected a tag name or `{` after `enum`, found `;`
 --> parser-enum-specifier-without-name-and-body.c:1:6
  |
1 | enum ;
  |      ^ expected a tag name or `{`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.2.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-curly-brace-in-compound-statement.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-curly-brace-in-compound-statement.stderr)):

```text
error: expected `}`, found end of file
 --> parser-expected-closing-curly-brace-in-compound-statement.c:2:1
  |
2 | 
  | ^ expected `}`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.
- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected `}`. Keep informative related labels and insertion help.

### parser-expected-closing-curly-brace-in-initializer-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-curly-brace-in-initializer-list.stderr)):

```text
error: expected `,` or `}` in the initializer list, found `;`
 --> parser-expected-closing-curly-brace-in-initializer-list.c:1:18
  |
1 | int array[] = { 1;
  |                  ^ expected `,` or `}`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.8p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-curly-brace-in-struct-declaration-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-curly-brace-in-struct-declaration-list.stderr)):

```text
error: expected `}` to close the member list, found end of file
 --> parser-expected-closing-curly-brace-in-struct-declaration-list.c:2:1
  |
2 | 
  | ^ expected `}`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.2.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-parenthesis-after-parenthesized-declarator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-parenthesis-after-parenthesized-declarator.stderr)):

```text
error: expected `)` to close the declarator, found `;`
 --> parser-expected-closing-parenthesis-after-parenthesized-declarator.c:1:11
  |
1 | int (value;
  |           ^ expected `)`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-parenthesis-in-statement.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-parenthesis-in-statement.stderr)):

```text
error: expected `)` in `if` statement, found end of file
 --> parser-expected-closing-parenthesis-in-statement.c:2:1
  |
2 | 
  | ^ expected `)`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8.4.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-square-bracket-in-array-designator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-square-bracket-in-array-designator.stderr)):

```text
error: expected `]` to close the array designator, found `=`
 --> parser-expected-closing-square-bracket-in-array-designator.c:1:20
  |
1 | int array[] = { [1 = 2 };
  |                    ^ expected `]`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.8p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-square-bracket-in-array-direct-declarator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-square-bracket-in-array-direct-declarator.stderr)):

```text
error: expected `]` to close the array declarator, found end of file
 --> parser-expected-closing-square-bracket-in-array-direct-declarator.c:2:1
  |
2 | 
  | ^ expected `]`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-closing-square-bracket-in-subscript.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-closing-square-bracket-in-subscript.stderr)):

```text
error: expected `]` to close the subscript, found end of file
 --> parser-expected-closing-square-bracket-in-subscript.c:2:1
  |
2 | 
  | ^ expected `]`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-colon-in-label.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-colon-in-label.stderr)):

```text
error: expected `:` after default label, found `;`
 --> parser-expected-colon-in-label.c:1:36
  |
1 | int f(void) { switch (1) { default ; } }
  |                                    ^ expected `:`

1 error generated.
```

- **P3: default keyword is not quoted.** Suggested fix: Use “after `default` label” in the message. statement_position currently quotes case and other statement keywords but omits default. Add C99 §6.8.1p1.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-comma-or-closing-curly-in-enumerator-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-comma-or-closing-curly-in-enumerator-list.stderr)):

```text
error: expected `,` or `}` after the enumerator, found identifier `B`
 --> parser-expected-comma-or-closing-curly-in-enumerator-list.c:1:12
  |
1 | enum E { A B };
  |            ^ expected `,` or `}`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.2.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-comma-or-closing-parenthesis-in-function-call.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-comma-or-closing-parenthesis-in-function-call.stderr)):

```text
error: expected an operator, found `;`
 --> parser-expected-comma-or-closing-parenthesis-in-function-call.c:1:18
  |
1 | int f(void) { f(1; }
  |                  ^ expected an operator or the end of the expression

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.stderr)):

```text
error: expected `,` or `)` after the parameter, found integer constant `1`
 --> parser-expected-comma-or-closing-parenthesis-in-function-declarator-parameter-list.c:1:13
  |
1 | int f(int a 1);
  |             ^ expected `,` or `)`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.stderr)):

```text
error: expected `,` or `)` after the parameter name, found identifier `b`
 --> parser-expected-comma-or-closing-parenthesis-in-k-and-r-function-declarator-parameter-list.c:1:9
  |
1 | int f(a b);
  |         ^ expected `,` or `)`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-comma-or-semicolon-in-struct-declarator-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-comma-or-semicolon-in-struct-declarator-list.stderr)):

```text
error: expected `;`, found keyword `int`
 --> parser-expected-comma-or-semicolon-in-struct-declarator-list.c:2:1
  |
1 | struct S { int a
  |                 - help: add `;` here
2 | int b; };
  | ^^^ expected `,` or `;`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.2.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declaration-continuation-after-declarator-long-double.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator-long-double.stderr)):

```text
error: expected `,`, `=`, `;`, or a function body after the declarator, found floating constant `1.5L`
 --> parser-expected-declaration-continuation-after-declarator-long-double.c:1:11
  |
1 | int value 1.5L;
  |           ^^^^ expected one of `,`, `=`, `;`, or `{`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declaration-continuation-after-declarator-tab.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator-tab.stderr)):

```text
error: expected `,`, `=`, `;`, or a function body after the declarator, found string literal `"abc"`
 --> parser-expected-declaration-continuation-after-declarator-tab.c:1:12
  |
1 |     int value "abc";
  |               ^^^^^ expected one of `,`, `=`, `;`, or `{`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declaration-continuation-after-declarator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declaration-continuation-after-declarator.stderr)):

```text
error: expected `,`, `=`, `;`, or a function body after the declarator, found identifier `extra`
 --> parser-expected-declaration-continuation-after-declarator.c:1:7
  |
1 | int a extra;
  |       ^^^^^ expected one of `,`, `=`, `;`, or `{`
  |
  = help: if this starts a new declaration, add `;` before it

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.stderr)):

```text
error: expected an identifier or `(` in the declarator, found end of file
 --> parser-expected-declarator-after-opening-parenthesis-in-direct-declarator-eof.c:2:1
  |
2 | 
  | ^ expected a name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.stderr)):

```text
error: expected an identifier or `(` in the declarator, found `;`
 --> parser-expected-declarator-after-opening-parenthesis-in-direct-declarator.c:1:6
  |
1 | int (;
  |      ^ expected a name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declarator-in-declaration-include.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declarator-in-declaration-include.stderr)):

```text
error: expected an identifier or `(` in the declarator, found `;`
 --> included-error.h:1:19
  |
1 | int header_value, ;
  |                   ^ expected a name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declarator-in-declaration.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declarator-in-declaration.stderr)):

```text
error: expected an identifier or `(` in the declarator, found `;`
 --> parser-expected-declarator-in-declaration.c:1:8
  |
1 | int a, ;
  |        ^ expected a name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-declarator-in-typedef.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-declarator-in-typedef.stderr)):

```text
error: expected a name for the typedef, found `;`
 --> parser-expected-declarator-in-typedef.c:1:12
  |
1 | typedef int;
  |            ^ expected an identifier
  |
  = note: a `typedef` declaration must name the type it defines

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.7p3 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.stderr)):

```text
error: expected an enumerator name or `}`, found integer constant `1`
 --> parser-expected-enumeration-constant-or-closing-curly-in-enumerator-list.c:1:10
  |
1 | enum E { 1 };
  |          ^ expected an identifier or `}`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.2.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-function-body.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-function-body.stderr)):

```text
error: expected a function body, found end of file
 --> parser-expected-function-body.c:2:1
  |
2 | 
  | ^ expected `{`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.9.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-goto-label.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-goto-label.stderr)):

```text
error: expected a label name after `goto`, found integer constant `123`
 --> parser-expected-goto-label.c:1:20
  |
1 | int f(void) { goto 123; }
  |                    ^^^ expected an identifier

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8.6.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-member-identifier.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-member-identifier.stderr)):

```text
error: expected a member name after `.` or `->`, found `;`
 --> parser-expected-member-identifier.c:1:22
  |
1 | int f(void) { object.; }
  |                      ^ expected an identifier

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-opening-parenthesis-in-statement.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-opening-parenthesis-in-statement.stderr)):

```text
error: expected `(` in `if` statement, found integer constant `1`
 --> parser-expected-opening-parenthesis-in-statement.c:1:18
  |
1 | int f(void) { if 1) return 0; }
  |                  ^ expected `(`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8.4.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-parameter-declaration-after-comma-in-function-declarator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-parameter-declaration-after-comma-in-function-declarator.stderr)):

```text
error: expected a parameter declaration or `...` after `,`, found `)`
 --> parser-expected-parameter-declaration-after-comma-in-function-declarator.c:1:14
  |
1 | int f(int a, );
  |              ^ expected a parameter

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-semicolon-in-statement.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-semicolon-in-statement.stderr)):

```text
error: expected `;` after `return` statement, found `}`
 --> parser-expected-semicolon-in-statement.c:1:24
  |
1 | int f(void) { return 0 }
  |                        ^ expected `;`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8.6.4p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-statement-expression-macro.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-statement-expression-macro.stderr)):

```text
error: expected an expression, found `;`
 --> parser-expected-statement-expression-macro.c:2:16
  |
2 | int value = BAD;
  |                ^ expected an expression

1 error generated.
```

- **P3: missing macro operand has no expansion context.** Suggested fix: Keep the semicolon primary and add the macro-use and replacement-operator ranges so the user can connect the missing operand to `BAD` and its `+` replacement. This is a provenance improvement, not a claim that the current semicolon caret is wrong.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.
- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected an expression. Keep informative related labels and insertion help.

### parser-expected-statement-expression-operand.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-statement-expression-operand.stderr)):

```text
error: expected an expression, found `;`
 --> parser-expected-statement-expression-operand.c:1:19
  |
1 | int f(void) { 1 + ; }
  |                   ^ expected an expression

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.
- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected an expression. Keep informative related labels and insertion help.

### parser-expected-statement-expression-operator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-statement-expression-operator.stderr)):

```text
error: expected an operator, found identifier `b`
 --> parser-expected-statement-expression-operator.c:1:17
  |
1 | int f(void) { a b; }
  |                 ^ expected an operator or the end of the expression
  |
  = help: if this starts a new declaration, add `;` before it

1 error generated.
```

- **P3: declaration-only help is weak in an expression statement.** Suggested fix: Here `a b` is in a function body and both tokens are ordinary identifiers. Prefer `add an operator between these expressions, or end the previous statement with ;`, quoting code. Offer the new-declaration-specific hint only when the following token is a declaration starter.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-expected-statement-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-statement-expression.stderr)):

```text
error: expected an expression, found `;`
 --> parser-expected-statement-expression.c:1:13
  |
1 | int value = ;
  |             ^ expected an expression

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.
- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected an expression. Keep informative related labels and insertion help.

### parser-expected-statement.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-expected-statement.stderr)):

```text
error: expected a statement, found keyword `else`
 --> parser-expected-statement.c:1:22
  |
1 | int f(void) { if (1) else return 0; }
  |                      ^^^^ expected a statement

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.8p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.
- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected a statement. Keep informative related labels and insertion help.

### parser-inline-specified-twice.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-inline-specified-twice.stderr)):

```text
warning: duplicate `inline`
 --> parser-inline-specified-twice.c:1:8
  |
1 | inline inline int f(void);
  |        ^^^^^^ `inline` was already specified
  |
  = note: repeating `inline` has no effect
  = note: `repeated-specifiers` warnings are on by default; pass `--no-repeated-specifier-warnings` to silence them

1 warning generated.
```

- **P3: inline warning lacks a C99 citation.** Suggested fix: Cite C99 §6.7.4p5 for repeated `inline`; the corresponding qualifier warnings already carry their standard references.

### parser-macro-expanded-keyword.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-macro-expanded-keyword.stderr)):

```text
error: expected an identifier or `(` in the declarator, found keyword `return`
 --> parser-macro-expanded-keyword.c:1:13
  |
1 | #define BAD return
  |             ^^^^^^ expected a name
2 | int BAD;
  |        - parsing resumes here
  |
  = note: `return` is a keyword and cannot be used as a name

1 error generated.
```

- **P3: macro diagnostic has no invocation-site label.** Suggested fix: Retain the primary spelling and location at `return`, and add a secondary `expanded from BAD here` range under `BAD` on line 2. `parsing resumes here` at the semicolon explains recovery but not where the offending macro was used.
- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-storage-class-redefinition.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-storage-class-redefinition.stderr)):

```text
error: cannot combine storage classes `static` and `extern`
 --> parser-storage-class-redefinition.c:1:8
  |
1 | static extern int value;
  |        ^^^^^^ `static` was already specified
  |
  = note: C99 §6.7.1p2: a declaration has at most one storage-class specifier

1 error generated.
```

- **P3: prior storage-class location is missing.** Suggested fix: Label the newly encountered `extern` as the second storage-class specifier, and mark the earlier `static` with a secondary range. The current label says static was already specified while pointing solely at extern.

### parser-struct-or-union-specifier-without-name-and-body.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-struct-or-union-specifier-without-name-and-body.stderr)):

```text
error: expected a tag name or `{` after `struct` or `union`, found `;`
 --> parser-struct-or-union-specifier-without-name-and-body.c:1:8
  |
1 | struct ;
  |        ^ expected a tag name or `{`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.2.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-type-qualifiers-without-declarator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-type-qualifiers-without-declarator.stderr)):

```text
error: expected an identifier or `(` in the declarator, found `;`
 --> parser-type-qualifiers-without-declarator.c:1:11
  |
1 | int *const;
  |           ^ expected a name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-unexpected-end-before-type-specifier.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-unexpected-end-before-type-specifier.stderr)):

```text
error: expected a type specifier, found end of file
 --> parser-unexpected-end-before-type-specifier.c:2:1
  |
2 | 
  | ^ expected a type specifier

1 error generated.
```

- **P3: primary label repeats the headline expectation.** Suggested fix: Omit this primary label, or use it to explain the owning construct/opening delimiter. The headline already says expected a type specifier. Keep informative related labels and insertion help.

### parser-unexpected-end-of-function-declarator-parameter-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-unexpected-end-of-function-declarator-parameter-list.stderr)):

```text
error: expected `)` to close the parameter list, found end of file
 --> parser-unexpected-end-of-function-declarator-parameter-list.c:2:1
  |
2 | 
  | ^ expected `)`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### parser-unexpected-end-of-variadic-function-declarator-parameter-list.c

Actual output ([exact bytes](tests/fixtures/diagnostics/parser-unexpected-end-of-variadic-function-declarator-parameter-list.stderr)):

```text
error: expected `)` after `...`, found end of file
 --> parser-unexpected-end-of-variadic-function-declarator-parameter-list.c:2:1
  |
2 | 
  | ^ expected `)`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.7.5p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-elif-directive-without-if-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-elif-directive-without-if-directive.stderr)):

```text
error: `#elif` without `#if`
 --> pp-elif-directive-without-if-directive.c:1:2
  |
1 | #elif 1
  |  ^^^^ no conditional directive is open here

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-else-directive-without-if-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-else-directive-without-if-directive.stderr)):

```text
error: `#else` without `#if`
 --> pp-else-directive-without-if-directive.c:1:2
  |
1 | #else
  |  ^^^^ no conditional directive is open here

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-error-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-error-directive.stderr)):

```text
error: #error stop here
 --> pp-error-directive.c:1:2
  |
1 | #error stop here
  |  ^^^^^ `#error` directive

1 error generated.
```

- **P3: #error fixed prefix is not quoted.** Suggested fix: Write the fixed directive prefix as `#error` followed by the source message, preserving source case and punctuation. Add C99 §6.10.5p1. Do not lowercase or rewrite user-supplied #error text.

### pp-expected-identifier-in-ifdef-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-expected-identifier-in-ifdef-directive.stderr)):

```text
error: expected a macro name after `#ifdef`, found number `1`
 --> pp-expected-identifier-in-ifdef-directive.c:1:8
  |
1 | #ifdef 1
  |        ^ expected a macro name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-expected-identifier-in-ifndef-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-expected-identifier-in-ifndef-directive.stderr)):

```text
error: expected a macro name after `#ifndef`, found number `1`
 --> pp-expected-identifier-in-ifndef-directive.c:1:9
  |
1 | #ifndef 1
  |         ^ expected a macro name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-expected-identifier-in-undef-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-expected-identifier-in-undef-directive.stderr)):

```text
error: expected a macro name after `#undef`, found number `1`
 --> pp-expected-identifier-in-undef-directive.c:1:8
  |
1 | #undef 1
  |        ^ expected a macro name

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-extra-tokens-after-ifdef-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-extra-tokens-after-ifdef-directive.stderr)):

```text
warning: extra tokens at end of `#ifdef` directive
 --> pp-extra-tokens-after-ifdef-directive.c:1:17
  |
1 | #ifdef __STDC__ extra
  |                 ^^^^^ ignored

1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-extra-tokens-after-ifndef-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-extra-tokens-after-ifndef-directive.stderr)):

```text
warning: extra tokens at end of `#ifndef` directive
 --> pp-extra-tokens-after-ifndef-directive.c:1:18
  |
1 | #ifndef __STDC__ extra
  |                  ^^^^^ ignored

1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-extra-tokens-after-include-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-extra-tokens-after-include-directive.stderr)):

```text
warning: extra tokens at end of `#include` directive
 --> pp-extra-tokens-after-include-directive.c:1:20
  |
1 | #include "clean.h" extra
  |                    ^^^^^ ignored

1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.2 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-extra-tokens-after-pragma-operator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-extra-tokens-after-pragma-operator.stderr)):

```text
warning: extra tokens after the pragma in `_Pragma`
 --> <pragma string>:2:1
  |
2 | 
  | ^ not part of the pragma

1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.6p2 and §6.10.9p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-forced-signed-promotion.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-forced-signed-promotion.stderr)):

```text
warning: integer constant does not fit in `int`
 --> pp-forced-signed-promotion.c:1:13
  |
1 | int value = 2147483648;
  |             ^^^^^^^^^^ this constant has type `long`

1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.4.1p5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-forced-unsigned-promotion.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-forced-unsigned-promotion.stderr)):

```text
warning: integer constant does not fit in `unsigned int`
 --> pp-forced-unsigned-promotion.c:1:13
  |
1 | int value = 4294967296u;
  |             ^^^^^^^^^^^ this constant has type `unsigned long`

1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.4.1p5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-function-call-operator-not-supported-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-function-call-operator-not-supported-in-preprocessor-expression.stderr)):

```text
warning: `F` is not defined; it evaluates to 0
 --> pp-function-call-operator-not-supported-in-preprocessor-expression.c:1:5
  |
1 | #if F(1)
  |     ^ not a macro
  |
  = note: C99 §6.10.1p3: identifiers that are not macro names are replaced with `0` in `#if`
  = help: use `defined(F)` to test whether it is defined

error: function call in `#if` expression
 --> pp-function-call-operator-not-supported-in-preprocessor-expression.c:1:6
  |
1 | #if F(1)
  |      ^ functions cannot be called during preprocessing
  |
  = note: C99 §6.10.1p3: identifiers that are not macros evaluate to `0`, so this looks like a call of an undefined function-like macro
  = help: define the function-like macro before this directive

1 error and 1 warning generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.

### pp-header-not-found.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-header-not-found.stderr)):

```text
error: cannot find header `missing-diagnostics-header.h`
 --> pp-header-not-found.c:1:10
  |
1 | #include "missing-diagnostics-header.h"
  |          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ not found in any search directory
  |
  = note: searched these directories:
            . (the working directory)
  = help: add the directory containing it with `--iquote <dir>` or `--isystem <dir>`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.2p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-hex-escape-sequence-too-large.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-hex-escape-sequence-too-large.stderr)):

```text
error: hexadecimal escape sequence is out of range
 --> pp-hex-escape-sequence-too-large.c:1:13
  |
1 | int value = '\x100000000';
  |             ^^^^^^^^^^^^^ contains an oversized `\x` escape

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.4.4 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-integer-literal-overflow.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-integer-literal-overflow.stderr)):

```text
error: integer constant is too large
 --> pp-integer-literal-overflow.c:1:13
  |
1 | int value = 18446744073709551616;
  |             ^^^^^^^^^^^^^^^^^^^^ does not fit in `unsigned long long`
  |
  = note: the largest integer type, `unsigned long long`, has 64 bits

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.4p2 and §6.4.4.1p5 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-invalid-hex-escape-sequence.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-invalid-hex-escape-sequence.stderr)):

```text
error: hexadecimal escape sequence is not a valid character
 --> pp-invalid-hex-escape-sequence.c:1:13
  |
1 | int value = '\x110000';
  |             ^^^^^^^^^^ contains an invalid `\x` escape

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.4.4 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-large-unicode-escape-sequence-too-small.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-large-unicode-escape-sequence-too-small.stderr)):

```text
error: `\U` escape needs exactly eight hexadecimal digits
 --> pp-large-unicode-escape-sequence-too-small.c:1:13
  |
1 | int value = '\U1234';
  |             ^^^^^^^^ contains a short `\U` escape

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.3p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-missing-closing-parenthesis-in-pragma-operator.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-closing-parenthesis-in-pragma-operator.stderr)):

```text
warning: `#pragma once` in main file
 --> <pragma string>:1:1
  |
1 | once
  | ^^^^ only affects files that are included

error: expected `)` to close `_Pragma(`, found number `1`
 --> pp-missing-closing-parenthesis-in-pragma-operator.c:1:16
  |
1 | _Pragma("once" 1)
  |                ^- skipped to recover
  |                |
  |                expected `)`
2 | int value;
  | --- parsing resumes here

1 error and 1 warning generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.9p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-missing-number-in-line-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-missing-number-in-line-directive.stderr)):

```text
error: expected a line number after `#line`, found string literal `"file.c"`
 --> pp-missing-number-in-line-directive.c:1:7
  |
1 | #line "file.c"
  |       ^^^^^^^^ expected a line number

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.4p3 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-more-endif-directives-than-if-directives.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-more-endif-directives-than-if-directives.stderr)):

```text
error: `#endif` without `#if`
 --> pp-more-endif-directives-than-if-directives.c:1:2
  |
1 | #endif
  |  ^^^^^ no conditional directive is open here

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-more-if-directives-than-endif-directives.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-more-if-directives-than-endif-directives.stderr)):

```text
error: unterminated `#if`
 --> pp-more-if-directives-than-endif-directives.c:1:2
  |
1 | #if 1
  |  ^^ this conditional has no matching `#endif`
  |
  = help: add `#endif` where the conditional section should end

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-no-condition-in-elif-directive.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-no-condition-in-elif-directive.stderr)):

```text
error: `#elif` with no condition
 --> pp-no-condition-in-elif-directive.c:3:1
  |
3 | #endif
  | ^ expected an expression
  |
  = help: write the condition to test, or use `#else`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.1p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-s-t-d-c-pragma-directive-without-argument.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-s-t-d-c-pragma-directive-without-argument.stderr)):

```text
error: expected a pragma name after `#pragma STDC`
 --> pp-s-t-d-c-pragma-directive-without-argument.c:1:13
  |
1 | #pragma STDC
  |             ^ expected `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE`

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.10.6p2 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-small-unicode-escape-sequence-too-short.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-small-unicode-escape-sequence-too-short.stderr)):

```text
error: `\u` escape needs exactly four hexadecimal digits
 --> pp-small-unicode-escape-sequence-too-short.c:1:13
  |
1 | int value = '\u12';
  |             ^^^^^^ contains a short `\u` escape

1 error generated.
```

- **P3: grammar/range explanation lacks a C99 citation.** Suggested fix: Add a short note citing C99 §6.4.3p1 for the relevant production or range rule. This is a consistency improvement; absence of a citation alone does not make the diagnostic factually wrong. Cite the construct that actually failed, not a folded follow-on error.

### pp-undefined-identifier-in-preprocessor-expression.c

Actual output ([exact bytes](tests/fixtures/diagnostics/pp-undefined-identifier-in-preprocessor-expression.stderr)):

```text
warning: `X` is not defined; it evaluates to 0
 --> pp-undefined-identifier-in-preprocessor-expression.c:1:5
  |
1 | #if X
  |     ^ not a macro
  |
  = note: C99 §6.10.1p3: identifiers that are not macro names are replaced with `0` in `#if`
  = help: use `defined(X)` to test whether it is defined

1 warning generated.
```

- **P3: wrong N1256 paragraph for #if replacement/arithmetic.** Suggested fix: Change C99 §6.10.1p3 to §6.10.1p4 in the relevant note. In N1256, identifier-to-zero replacement and intmax_t/uintmax_t arithmetic are paragraph 4; paragraph 3 describes the #if/#elif directive forms.

### tokenizer-unknown-token-backtick.c

Actual output ([exact bytes](tests/fixtures/diagnostics/tokenizer-unknown-token-backtick.stderr)):

```text
error: unexpected character '`' in source
 --> tokenizer-unknown-token-backtick.c:1:12
  |
1 | int value; `
  |            ^ no C token starts with this character
  |
  = note: C99 §5.2.1: `@`, `$`, and `` ` `` are not part of the basic source character set; outside literals and comments they cannot appear

1 error generated.
```

- **P3: backtick source spelling uses a different quoting style.** Suggested fix: Use U+0060 for this character, or a quoting convention that represents a literal backtick consistently. The current headline uses single quotes while other source characters use backticks.

## Variants not covered independently

- `tokenizer::UnterminatedIncludeString` — **believed unreachable**. The initial processor synthesizes a newline at EOF. Quoted header scanning ignores escapes, so it takes NewlineInIncludeString before it can reach EOF. Angle headers use a separate fallback path. The quoted/angle EOF stress fixtures preserve those actual outputs.
- `pp::CommaOperatorInPreprocessorExpression` — **unavailable in default CLI**. CompilerConfiguration::default selects ExtensionPolicy::Allow and the CLI has no strict-policy option. The emitter explicitly gates this variant on Warn or Deny. It is reachable through configured library/unit tests, but not bcc-rust <fixture>.
- `pp::UnexpectedTokenAtPhase7` — **believed unreachable**. The emitter only handles Placeholder, AngleBracketString, IncludeString, and Whitespace. Include strings are consumed by #include, whitespace is filtered, and empty argument placeholders are consumed by macro expansion before phase 7. No C-only CLI reproducer was found.
- `pp::HeaderFileInaccessible` — **environment-dependent; not covered**. Requires a discovered header whose subsequent read fails, such as a permissions/sharing violation or filesystem race. A normal checked-in C/header pair cannot establish that condition portably; the OS error wording is also host-dependent.
- `pp::InvalidOctalEscapeSequence` — **unreachable by range analysis**. The decoder reads at most three octal digits: 0..511. Every resulting number is a valid Unicode scalar, so char::try_from cannot fail.
- `pp::OctalEscapeSequenceTooLarge` — **unreachable by range analysis**. At most three octal digits are accumulated in u16. A maximum of 511 cannot overflow u16.
- `parser::ResourceLimitExceeded` — **reachable; not covered**. The CLI uses the default memory/frame budgets and does not expose smaller budgets. Reaching them needs a deliberately large input, contrary to the small-fixture scope. Existing unit tests lower the budgets; no tiny C-only default-CLI fixture is claimed.
- `parser::ParserFrameConsumedAtEndOfInput` — **internal invariant; not covered**. Reports a parser-machine bug rather than a malformed C production. No reproducer was found, and it should not be manufactured by changing parser state.
- `parser::ExpectedOpeningCurlyBraceInCompoundStatement` — **believed unreachable from normal dispatch**. The function/statement owners check for { before pushing this frame; a missing function brace is handled by ExpectedFunctionBody. The fallback is defensive frame-entry validation.
- `parser::UnexpectedEndBeforeDeclarationSpecifier` — **believed unreachable from normal dispatch**. Owners only push declaration-specifier parsing after recognizing a declaration starter, or after a token has already been consumed. EOF without a starter is handled by the owner; EOF after consumed specifiers uses UnexpectedEndBeforeTypeSpecifier.
- `parser::ExpectedStructOrUnionKeyword` — **believed unreachable from normal dispatch**. The specifier owner selects this frame only for a verified struct or union token. The frame-entry check is defensive.
- `parser::ExpectedEnumKeyword` — **believed unreachable from normal dispatch**. The specifier owner selects this frame only for a verified enum token. The frame-entry check is defensive.
- `parser::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator` — **unreachable with current lookahead**. is_array_pointer_marker recognizes * only when the following token is ]. Therefore ArrayExpectClose begins with ] and cannot see an unexpected token.
- `parser::UnexpectedEndOfArrayDeclaratorAfterPointer` — **unreachable with current lookahead**. Entering ArrayExpectClose requires a buffered following ], so EOF cannot immediately follow the recognized marker.
- `parser::PointerSpecifiedTwice` — **unreachable with current lookahead**. A second * prevents the first * from being recognized as an array pointer marker. The input instead enters ordinary bound-expression parsing.
- `pp::RedefinitionOfBuiltInMacro` — the C input reaches the attempt, but the compiler panics before the queued explanation is rendered. It is a crash regression, not rendered enum coverage.

The four folded parser variants and nineteen folded preprocessor variants are listed with dispatch evidence in COVERAGE.md. Their fixtures exercise the reporting paths, but the reporter absorbs their primary message into an earlier diagnostic at the same source location. Unreachable entries are source-backed conclusions for this snapshot, not claims about the C grammar or future compiler versions.
