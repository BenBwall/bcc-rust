# Diagnostic golden corpus

Each top-level `.c` is run as `bcc-rust <file-name>` with this directory as the current directory. An optional sibling `.args` supplies whitespace-separated CLI flags before the filename. Its sibling `.stderr` stores the exact bytes, including the summary. `NO_COLOR=1` is set; `CLICOLOR_FORCE`, `CPATH`, and `C_INCLUDE_PATH` are removed. `RUST_BACKTRACE=0` keeps crash regressions from depending on an inherited backtrace setting. No paths, whitespace, source spellings, or panic output are normalized.

Run `cargo test --test diagnostics_golden`. Use `BLESS=1 cargo test --test diagnostics_golden` to rewrite every snapshot. The guard remains active while blessing and runs every C fixture again. Both tests accumulate failures so one mismatch does not hide later fixtures.

Snapshots record current behavior, including defects. The guard test rejects panics, internal representations, and raw NUL bytes in any output, so a crash or control-byte leak cannot be blessed into a passing snapshot. Fix the compiler instead of normalizing such output.

`clean.h` supports the include-directive test; `included-error.h` contains the included-file parser error; `include-next-relative.h` is found beside its includer, the primary source file, for the `#include_next` search-start warning; `pragma-system-header.h` and the `system-headers/` directory (`noisy.h`, `beside-noisy.h`) supply the user header with `#pragma GCC system_header` and the system headers of `pp-system-header-diagnostics.c`. The header files are included by C fixtures, not invoked independently.

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
| [Left shift](pp-left-shift-overflow.stderr), [right shift](pp-right-shift-overflow.stderr) | Describe an invalid shift count separately from an overflowing result. |
| [Angle-header newline](tokenizer-angle-header-newline.stderr), [angle-header EOF](tokenizer-angle-header-eof.stderr), [quoted-header EOF](tokenizer-quoted-header-eof.stderr) | The missing closing `>` or `"` is diagnosed at the absent delimiter, but the lookup of the partial name still reports a missing header; review whether that lookup should run. Angle EOF also reports an unexpected end of file in the directive, which quoted EOF no longer does; review whether it is redundant beside the missing `>`. |
| [Unicode identifier recovery](parser-unicode-identifier.stderr) | Review the missing-type and continuation diagnostics for a single malformed declaration together. |
| [Macro keyword](parser-macro-expanded-keyword.stderr), [macro operand](parser-expected-statement-expression-macro.stderr) | Add useful macro invocation context alongside spelling or recovery locations. |
| [Conflicting storage classes](parser-storage-class-redefinition.stderr) | Label the earlier `static` as well as the new `extern`. |

The prior panic and raw-NUL findings remain protected by the guard test and
their existing fixtures. Keep those regressions even though the old review's
captured failing output is no longer the current snapshot.

### Dispatch inventory

There are **380 top-level C inputs and 380 stderr snapshots**, plus seven supporting headers and 138 mode/policy `.args` sidecars. One hundred eighteen of the inputs cover semantic analysis (twenty-eight for declarations, twenty-eight for expressions and initializers, forty-one for statements and functions, three for 128-bit integers, four for resource intrinsics, six for atomics and generic selection, four for binary128 and type selections, four for vector constraints) and are listed in their own tables below. The [machine inventory](coverage.tsv) records **1 initial-processing**, **5 tokenizer**, **142 preprocessor** and **64 parser** variants. These include nineteen folded preprocessor variants and four folded parser variants. The other 123 preprocessor and 60 parser variants have separately rendered messages in that inventory; the parser total exceeds the requested minimum of 40. The per-variant tables below also describe untested and unreachable paths. Shared extension-origin diagnostics are tracked separately below.

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
| `WarningDirective` | rendered | [GNU89 warning message and extension policy](lexpp-gnu89-extensions.c); structured tests cover native C23 warnings and earlier strict rejection |
| `InvalidHexadecimalFloatLiteral` | rendered | [pp-invalid-hexadecimal-float-literal.c](pp-invalid-hexadecimal-float-literal.c) |
| `InvalidDecimalFloatLiteral` | rendered | [pp-invalid-decimal-float-literal.c](pp-invalid-decimal-float-literal.c) |
| `FloatConstantOutOfRange` | rendered | [pp-float-constant-out-of-range-underflow.c](pp-float-constant-out-of-range-underflow.c), [pp-float-constant-out-of-range.c](pp-float-constant-out-of-range.c) |
| `InvalidHexadecimalIntegerLiteral` | rendered | [pp-invalid-hexadecimal-integer-literal.c](pp-invalid-hexadecimal-integer-literal.c) |
| `InvalidBinaryIntegerLiteral` | rendered | [pp-invalid-binary-integer-literal.c](pp-invalid-binary-integer-literal.c) |
| `InvalidOctalIntegerLiteral` | rendered | [pp-invalid-octal-integer-literal.c](pp-invalid-octal-integer-literal.c) |
| `InvalidDecimalIntegerLiteral` | rendered | [pp-invalid-decimal-integer-literal.c](pp-invalid-decimal-integer-literal.c) |
| `IntegerLiteralOverflow` | rendered | [pp-integer-literal-overflow.c](pp-integer-literal-overflow.c) |
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
| `MoreEndifDirectivesThanIfDirectives` | rendered | [pp-more-endif-directives-than-if-directives.c](pp-more-endif-directives-than-if-directives.c) |
| `ElifDirectiveWithoutIfDirective` | rendered | [pp-elif-directive-without-if-directive.c](pp-elif-directive-without-if-directive.c) |
| `ElseDirectiveWithoutIfDirective` | rendered | [pp-else-directive-without-if-directive.c](pp-else-directive-without-if-directive.c) |
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
| `InvalidEscapeSequence` | rendered | [pp-invalid-escape-sequence.c](pp-invalid-escape-sequence.c) |
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
| `ExpectedDeclaratorInDeclaration` | reached; folded | [parser-expected-declarator-in-declaration-include.c](parser-expected-declarator-in-declaration-include.c), [parser-expected-declarator-in-declaration.c](parser-expected-declarator-in-declaration.c) |
| `ExpectedDeclarationContinuationAfterDeclarator` | rendered | [parser-expected-declaration-continuation-after-declarator-long-double.c](parser-expected-declaration-continuation-after-declarator-long-double.c), [parser-expected-declaration-continuation-after-declarator-missing-semicolon.c](parser-expected-declaration-continuation-after-declarator-missing-semicolon.c), [parser-expected-declaration-continuation-after-declarator-tab.c](parser-expected-declaration-continuation-after-declarator-tab.c), [parser-expected-declaration-continuation-after-declarator.c](parser-expected-declaration-continuation-after-declarator.c) |
| `UnexpectedEndBeforeDeclarationSpecifier` | believed unreachable from normal dispatch | Owners only push declaration-specifier parsing after recognizing a declaration starter, or after a token has already been consumed. EOF without a starter is handled by the owner; EOF after consumed specifiers uses UnexpectedEndBeforeTypeSpecifier. |
| `UnexpectedEndBeforeTypeSpecifier` | rendered | [parser-unexpected-end-before-type-specifier.c](parser-unexpected-end-before-type-specifier.c) |
| `DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis` | rendered | [parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c](parser-direct-declarator-must-start-with-identifier-or-opening-parenthesis.c), [parser-macro-expanded-keyword.c](parser-macro-expanded-keyword.c) |
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

[`tests/language_cli.rs`](../../language_cli.rs) checks every standard alias
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
| `IncompleteEnum` | [enum declaration](sema-enum-incomplete.c) | Strict C99 enum tag without a prior completion. |
| `InvalidConstant` | [constant](sema-constant.c) | Runtime enumerator operand, including unselected conditional/logical arms. |
| `ConstantOverflow` | [overflow](sema-overflow.c) | Exceptional ICE evaluation; no dependent enumerator cascade. |
| `EnumeratorRange` | [enum range](sema-enum-range.c) | Members that no 64-bit integer type holds; under `-pedantic-errors` the C23 extension diagnostic for values outside int precedes it. |
| `InvalidMember` | [member](sema-member.c) | Incomplete/function member and a flexible array that is not the last member. |
| `Extension(FlexibleArrayExtensions)` | [flexible extension](sema-flexible-extension.c) | GNU flexible arrays in unions or otherwise empty structures, nested flexible structures and arrays of them (§6.7.2.1p2, p16). |
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
below; implicit functions use the existing shared extension diagnostic.

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
| `InvalidMemberAccess` | [member-access](sema2-member-access.c) | Complete records and existing member names. |
| `InvalidCall` | [call](sema2-call.c) | Pointer to function designators. |
| `InvalidArgumentCount` | [argument-count](sema2-argument-count.c) | Prototype fixed and variadic arity. |
| `InvalidArgumentType` | [argument-type](sema2-argument-type.c) | Prototype assignment compatibility. |
| `InvalidCompoundLiteral` | [compound-literal](sema2-compound-literal.c) | Object type and no variably modified literal type. |
| `InvalidCondition` | [condition](sema2-condition.c) | Scalar selection/iteration conditions. |
| `InvalidSwitchExpression` | [switch](sema2-switch.c) | Integer switch operand. |
| `FailedAssertion` | [assertion](sema2-assertion.c) | Evaluated supported static assertion and its message (C11 mode). |
| `InvalidInitializer` | [initializer](sema2-initializer.c) | Scalar assignment conversion and valid object initialization. |
| `ExcessInitializer` | [excess](sema2-excess.c) | Scalar/aggregate/array current-object exhaustion. |
| `InvalidDesignator` | [designator](sema2-designator.c) | Existing members and in-range nonnegative ICE array indices. |
| `NonConstantInitializer` | [static-initializer](sema2-static-initializer.c) | Static arithmetic/address constant eligibility. |
| `ConstantOverflow` | [static-overflow](sema2-static-overflow.c) | Shared exceptional constant evaluation in initializers. |
| `Extension(ImplicitFunctionDeclaration)` | [implicit function](sema2-implicit-function.c) | Removed C89 function declaration policy in GNU99 pedantic mode. |

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
| `UnnamedDefinitionParameter` | [parameter-name](sema3-parameter-name.c) |
| `IncompleteDefinitionParameter` | [parameter-complete](sema3-parameter-complete.c) |
| `DefinitionStarArray` | [parameter-star](sema3-parameter-star.c) |
| `MainSignature` | [main](sema3-main.c) |
| `IncompleteInternalTentative` | [internal-tentative](sema3-internal-tentative.c) |
| `DuplicateDefinition` | [definition](sema3-definition.c) |
| `TentativeArrayAssumedOne` | [array-completion](sema3-array-completion.c) |
| `IncompleteTentativeDefinition` | [incomplete-tentative](sema3-incomplete-tentative.c) |
| `UndefinedInternal` | [internal-undefined](sema3-internal-undefined.c) |
| `UnusedStaticFunction` | [static-unused](sema3-static-unused.c) |
| `InlineInternalReference` | [inline-reference](sema3-inline-reference.c) |
| `InlineStaticObject` | [inline-object](sema3-inline-object.c) |
| `VoidReturnValue` | [void-return](sema3-void-return.c) |
| `MissingReturnValue` | [missing-value](sema3-missing-value.c) |
| `MissingReturnValueWarning` | [missing-value-warning](sema3-missing-value-warning.c) |
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
undeclared `ap` operand. Other extension diagnostics are unchanged.

### System headers

[pp-system-header-diagnostics.c](pp-system-header-diagnostics.c) runs with
`-std=c89 -pedantic-errors -isystem system-headers`. In `system-headers/noisy.h`
and the `beside-noisy.h` it includes, an undefined `#if` identifier, a
redefinition of the predefined `__INT64_C` and `long long` are withheld, while
`#error` is still reported. In `pragma-system-header.h` the warning before
`#pragma GCC system_header` remains and the ones after it are withheld. The main
file keeps its own warning and errors, including the redefinition of a system
header's macro. The `long long` that the system macro `NOISY_WIDE` supplies is
withheld even where the main file uses it, because it is spelled in the system
header. A `defined` that the system macros `NOISY_PROLOG` and `NOISY_DEFINED`
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

Six fixtures cover atomic diagnostics and preserve following valid input:

| Fixture | Coverage |
| --- | --- |
| `sema-atomic-type` | `InvalidAtomicType`: arrays, functions, qualified/already-atomic specifier operands, incomplete/void operands and invalid qualifier applications. |
| `sema-atomic-builtin` | `InvalidAtomicOperand` and shared arity errors: non-atomic C11 pointers, const writes, incompatible expected/value operands, floating bitwise fetches and invalid GNU/sync pointees. |
| `sema-atomic-order` | `InvalidAtomicOrder`, `InvalidAtomicFailureOrder` and `AtomicBufferQualifiers` warnings; valid runtime orders, stronger failure orders and unrestricted fences remain accepted. |
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
