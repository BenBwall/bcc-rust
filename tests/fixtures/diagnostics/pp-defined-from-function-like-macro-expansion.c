/* MinGW-w64's intrinsic guard: `##` forms the operand before `defined`
   reads it. Clang reports this only under -pedantic. */
#define __INTRINSIC_DEFINED__foo
#define PROLOG(name) (!defined(__INTRINSIC_DEFINED_ ## name))
#if !PROLOG(_foo) && PROLOG(_bar)
int foo_without_bar;
#endif
