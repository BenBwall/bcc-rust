# Pinned strict-C99 compiler observations

Generated on 2026-08-28 with `run.ps1`. The exit code is the normalized
`-fsyntax-only` result under the exact profiles recorded in `manifest.md`.

| Fixture | GCC 13.2.0 | Clang 19.1.5 |
| --- | ---: | ---: |
| `constraint-invalid-lvalue.c` | 1 | 1 |
| `gnu-statement-expression.c` | 1 | 1 |
| `imaginary-type.c` | 1 | 1 |
| `repeated-specifiers.c` | 0 | 0 |
| `typedef-parameter-preference.c` | 0 | 0 |

An exit code of zero means the compiler accepted the translation unit; one
means it emitted at least one strict-C99 diagnostic. The reviewed reason for
each result and the bcc-rust phase boundary are in `manifest.md`.
