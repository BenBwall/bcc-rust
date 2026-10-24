//! A case label's expression is analyzed in its own scope and position,
//! even in a switch whose condition code generation folds: a type it
//! declares is visible only from there on (#113 review, finding 1).
//! C99: §6.2.1 paragraphs 4 and 7, pp. 29-30; PDF pp. 41-42;
//! §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
use super::*;

#[test]
fn case_label_declarations_keep_their_scope_in_folded_switches() {
    for source in [
        // The earlier `r = K` names the outer enumerator.
        "enum { K = 2 }; int main(void) { int r = 0; switch (2) { case 2: r = K; break; case \
         sizeof(enum { K = 1 }): break; } return r != 2; }",
        "enum { K = 2 }; int main(void) { int r = 0; switch (2) { { case sizeof(enum { K = 1 }): \
         r = 4; break; } { case K: r = 2; break; } } return r != 2; }",
        "int main(void) { switch (1) { { case sizeof(enum { K = 1 }): break; } { case sizeof(enum \
         { K = 2 }) + 1: break; } } return 0; }",
        "struct T { char v[2]; }; int main(void) { int r = 0; switch (2) { { case sizeof(struct T \
         { char v[4]; }): r = 4; break; } { case sizeof(struct T): r = 2; break; } } return r != \
         2; }",
    ] {
        for (optimize, profile) in STAGES {
            with_module(
                source,
                Target::LinuxGnu,
                optimize,
                profile,
                |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                },
            );
        }
    }
}
