# Diagnostic golden corpus

Each top-level `.c` is run as `bcc-rust <file-name>` with this directory as the current directory. Its sibling `.stderr` stores the exact bytes, including the summary. `NO_COLOR=1` is set; `CLICOLOR_FORCE`, `CPATH`, and `C_INCLUDE_PATH` are removed. `RUST_BACKTRACE=0` keeps crash regressions from depending on an inherited backtrace setting. No paths, whitespace, source spellings, or panic output are normalized.

Run `cargo test --test diagnostics_golden`. Use `BLESS=1 cargo test --test diagnostics_golden` to rewrite every snapshot. The guard remains active while blessing and runs every C fixture again. Both tests accumulate failures so one mismatch does not hide later fixtures.

Snapshots record current behavior, including defects. The guard test rejects panics, internal representations, and raw NUL bytes in any output, so a crash or control-byte leak cannot be blessed into a passing snapshot. Fix the compiler instead of normalizing such output.

`clean.h` supports the include-directive test; `included-error.h` contains the included-file parser error. The header files are included by C fixtures, not invoked independently.

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
[N1256 reference](../../../c-spec.pdf): conditional replacement/arithmetic is
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
| [Extra pragma tokens](pp-extra-tokens-after-pragma-once.stderr) | The extra-token diagnostic names and labels `once` instead of `extra`. |
| [Pragma switch](pp-missing-on-off-switch-in-s-t-d-c-pragma.stderr) | Name `FP_CONTRACT` as the pragma requiring a switch, rather than saying the switch belongs after `MAYBE`. |
| [System header lookup](pp-header-not-found-system.stderr) | Distinguish an empty search-path list from an absolute header path. |
| [Invalid macro replacement list](pp-cannot-use-hash-hash-after-function-like-macro-call.stderr) | Diagnose `##` at the start of the definition's replacement list, rather than describing pasting onto an invocation. Check fresh definitions as well as redefinitions. |
| [Left shift](pp-left-shift-overflow.stderr), [right shift](pp-right-shift-overflow.stderr) | Describe an invalid shift count separately from an overflowing result. |
| [Preprocessor arithmetic](pp-binary-minus-overflow.stderr) | Correct the N1256 replacement/arithmetic citation from §6.10.1p3 to §6.10.1p4 across the evaluator's notes. |
| [Angle-header newline](tokenizer-angle-header-newline.stderr), [angle-header EOF](tokenizer-angle-header-eof.stderr), [quoted-header EOF](tokenizer-quoted-header-eof.stderr) | The missing closing `>` or `"` is diagnosed at the absent delimiter, but the lookup of the partial name still reports a missing header; review whether that lookup should run. Angle EOF also reports an unexpected end of file in the directive, which quoted EOF no longer does; review whether it is redundant beside the missing `>`. |
| [Unicode identifier recovery](parser-unicode-identifier.stderr) | Review the missing-type and continuation diagnostics for a single malformed declaration together. |
| [Macro keyword](parser-macro-expanded-keyword.stderr), [macro operand](parser-expected-statement-expression-macro.stderr) | Add useful macro invocation context alongside spelling or recovery locations. |
| [Conflicting storage classes](parser-storage-class-redefinition.stderr) | Label the earlier `static` as well as the new `extern`. |

The prior panic and raw-NUL findings remain protected by the guard test and
their existing fixtures. Keep those regressions even though the old review's
captured failing output is no longer the current snapshot.

### Dispatch inventory

There are **222 C inputs and 222 stderr snapshots**, plus two supporting headers. Dispatch targets cover **1/1 initial-processing**, **5/5 tokenizer**, **128/134 preprocessor**, and **64/73 parser** variants. The parser count includes four follow-on variants folded into an earlier diagnostic; the preprocessor count includes nineteen folded variants. Thus 60 distinct parser variants have a separately visible message in these fixtures, exceeding the requested minimum of 40.

The mapping below comes from checking the emitter/dispatch paths and their CLI output. It is not private-enum instrumentation. Variants sharing wording are distinguished by their source trigger; folded variants do not claim an independently rendered golden message. Supplementary EOF, literal, macro, tab, Unicode, and include cases may target the same variant more than once.

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
| `CommaOperatorInPreprocessorExpression` | unavailable in default CLI | CompilerConfiguration::default selects ExtensionPolicy::Allow and the CLI has no strict-policy option. The emitter explicitly gates this variant on Warn or Deny. It is reachable through configured library/unit tests, but not bcc-rust <fixture>. |
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
| `ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression` | rendered | [pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c](pp-expected-right-hand-side-of-binary-operator-in-preprocessor-expression.c) |
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
| `NoConditionInIfDirective` | rendered | [pp-no-condition-in-if-directive.c](pp-no-condition-in-if-directive.c) |
| `NoConditionInElifDirective` | rendered | [pp-no-condition-in-elif-directive.c](pp-no-condition-in-elif-directive.c) |
| `MoreIfDirectivesThanEndifDirectives` | rendered | [pp-more-if-directives-than-endif-directives.c](pp-more-if-directives-than-endif-directives.c) |
| `MoreEndifDirectivesThanIfDirectives` | rendered | [pp-more-endif-directives-than-if-directives.c](pp-more-endif-directives-than-if-directives.c) |
| `ElifDirectiveWithoutIfDirective` | rendered | [pp-elif-directive-without-if-directive.c](pp-elif-directive-without-if-directive.c) |
| `ElseDirectiveWithoutIfDirective` | rendered | [pp-else-directive-without-if-directive.c](pp-else-directive-without-if-directive.c) |
| `ExpectedIdentifierInIfdefDirective` | rendered | [pp-expected-identifier-in-ifdef-directive.c](pp-expected-identifier-in-ifdef-directive.c) |
| `ExpectedIdentifierInIfndefDirective` | rendered | [pp-expected-identifier-in-ifndef-directive.c](pp-expected-identifier-in-ifndef-directive.c) |
| `ExpectedIdentifierInDefineDirective` | rendered | [pp-expected-identifier-in-define-directive.c](pp-expected-identifier-in-define-directive.c) |
| `RedefinitionOfBuiltInMacro` | rendered | [pp-redefinition-of-built-in-macro.c](pp-redefinition-of-built-in-macro.c) |
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
| `CannotUseHashHashAfterFunctionLikeMacroCall` | rendered | [pp-cannot-use-hash-hash-after-function-like-macro-call.c](pp-cannot-use-hash-hash-after-function-like-macro-call.c) |
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
| `RedefinitionOfFunctionLikeMacroAsObjectLikeMacro` | rendered | [pp-redefinition-of-function-like-macro-as-object-like-macro.c](pp-redefinition-of-function-like-macro-as-object-like-macro.c) |
| `RedefinitionOfObjectLikeMacroAsFunctionLikeMacro` | rendered | [pp-redefinition-of-object-like-macro-as-function-like-macro.c](pp-redefinition-of-object-like-macro-as-function-like-macro.c) |
| `ExpectedIdentifierInMacroDefinition` | rendered | [pp-expected-identifier-in-macro-definition.c](pp-expected-identifier-in-macro-definition.c) |
| `VariadicMacroMustBeLastParameter` | rendered | [pp-variadic-macro-must-be-last-parameter.c](pp-variadic-macro-must-be-last-parameter.c) |
| `ExpectedCommaOrClosingParenthesisInMacroDefinition` | rendered | [pp-expected-comma-or-closing-parenthesis-in-macro-definition.c](pp-expected-comma-or-closing-parenthesis-in-macro-definition.c) |
| `MacroRedefinedWithDifferentDefinition` | rendered | [pp-macro-redefined-with-different-definition.c](pp-macro-redefined-with-different-definition.c) |
| `ExpectedIdentifierInUndefDirective` | rendered | [pp-expected-identifier-in-undef-directive.c](pp-expected-identifier-in-undef-directive.c) |
| `ExpectedNewlineAfterUndefDirective` | rendered | [pp-expected-newline-after-undef-directive.c](pp-expected-newline-after-undef-directive.c) |
| `HashOperatorMustBeFollowedByAMacroArgument` | rendered | [pp-hash-operator-must-be-followed-by-a-macro-argument.c](pp-hash-operator-must-be-followed-by-a-macro-argument.c) |
| `IdentifierNotMacroArgumentAfterHashOperator` | rendered | [pp-identifier-not-macro-argument-after-hash-operator.c](pp-identifier-not-macro-argument-after-hash-operator.c) |
| `MissingRightHandSideOfHashHashOperator` | rendered | [pp-missing-right-hand-side-of-hash-hash-operator.c](pp-missing-right-hand-side-of-hash-hash-operator.c) |
| `MissingLeftHandSideOfHashHashOperator` | rendered | [pp-missing-left-hand-side-of-hash-hash-operator.c](pp-missing-left-hand-side-of-hash-hash-operator.c) |
| `TokenMergingError` | rendered | [pp-token-merging-error.c](pp-token-merging-error.c) |
| `MissingNumberInLineDirective` | rendered | [pp-missing-number-in-line-directive.c](pp-missing-number-in-line-directive.c) |
| `MissingNewlineAfterLineDirective` | rendered | [pp-missing-newline-after-line-directive.c](pp-missing-newline-after-line-directive.c) |
| `LineDirectiveIsNotASimpleDigitSequence` | rendered | [pp-line-directive-is-not-a-simple-digit-sequence.c](pp-line-directive-is-not-a-simple-digit-sequence.c), [pp-line-directive-suffix-panic.c](pp-line-directive-suffix-panic.c) |
| `LineDirectiveNumberTooLarge` | rendered | [pp-line-directive-number-too-large.c](pp-line-directive-number-too-large.c) |
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
| `ErrorDirective` | rendered | [pp-error-directive.c](pp-error-directive.c) |

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
| `ExpectedStructDeclarationBeforeClosingCurlyBrace` | rendered | [parser-expected-struct-declaration-before-closing-curly-brace.c](parser-expected-struct-declaration-before-closing-curly-brace.c) |
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
| `EmptyStructDeclarator` | rendered | [parser-empty-struct-declarator.c](parser-empty-struct-declarator.c) |

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
