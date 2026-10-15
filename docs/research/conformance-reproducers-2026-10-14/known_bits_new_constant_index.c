/* bcc-rust known_bits_new_constant_index.c --passes known-bits --emit ir
   panics: index out of bounds at src/optimizer/known_bits.rs:143 (the
   remainder rewrite adds a constant past the fact table; the following
   `r == 1` condition looks through the new `and`). The default pipeline
   folds the remainder first and does not panic. */
int main(void) {
    unsigned a = 16;
    unsigned r = a % 4u;
    if (r == 1)
        return 1;
    return 0;
}
