# Strict-C99 parser compatibility manifest

This corpus is reviewed against WG14/N1256. Compiler output is evidence, not
the authority for bcc-rust acceptance.

Pinned observations:

- GCC 13.2.0 (MinGW-W64 x86_64-ucrt-posix-seh)
- Clang 19.1.5, target `x86_64-pc-windows-msvc`
- bcc-rust Rust toolchain 1.99.0

Normalized profiles:

- GCC: `-std=c99 -pedantic-errors -Wall -Wextra -Wno-unused-parameter -m64 -fmax-errors=20 -fsyntax-only`
- Clang: `-std=c99 -pedantic-errors -Wall -Wextra -Wno-unused-parameter --target=x86_64-pc-windows-msvc -ferror-limit=20 -fsyntax-only`
- bcc-rust: C99, extensions denied, repeated-specifier warnings enabled

| Fixture | C99 classification | bcc-rust | GCC | Clang | Reviewed discrepancy class |
| --- | --- | --- | --- | --- | --- |
| `repeated-specifiers.c` | valid; repetition behaves as one occurrence (§6.7.3p4, §6.7.4p5) | accept with suppressible quality warnings | accept | accept | agreement; warning defaults may differ |
| `imaginary-type.c` | invalid core type grammar; `_Imaginary` remains reserved | diagnose, recovered syntax | diagnose | diagnose | agreement |
| `gnu-statement-expression.c` | excluded GNU extension | pedantic extension diagnostic, complete GNU syntax node | diagnose | diagnose | extension |
| `constraint-invalid-lvalue.c` | grammatical, constraint-invalid | retain complete syntax for semantic analysis | diagnose | diagnose | intentional parser/semantic phase boundary |
| `typedef-parameter-preference.c` | valid; typedef-name interpretation is mandatory (§6.7.5.3p11) | accept | accept | accept | agreement |

`run.ps1` reproduces the compiler observations and refuses silent compiler
version drift. Review and update this manifest whenever a pinned version or
classification changes.
