/* TCC compiles in one pass, so under -Werror the first warning in the
   unit is the error it stops at, whichever of bcc's phases reports it. */
int f(void) { }
void g(void) { { l: } }
