/* A diagnostic about a token a macro's body wrote: GCC reports it in the
   body and notes the expansion, Clang reports it at the expansion and
   notes the macro. A token from an argument gets no note here. */
#define SHIFT_TOO_FAR (1 << 40)
#define UNDECLARED_VALUE some_undeclared_thing
#define DECLARE_POINTER int *declared = 7
#define CONTINUED \
  int *continued = \
    8
#define NESTED DECLARE_NESTED
#define DECLARE_NESTED int *nested = 9
#define ARGUMENT(x) x
int shifts(void) { return SHIFT_TOO_FAR; }
int undeclared(void) { return UNDECLARED_VALUE; }
DECLARE_POINTER;
CONTINUED;
NESTED;
int *argument = ARGUMENT(10);
