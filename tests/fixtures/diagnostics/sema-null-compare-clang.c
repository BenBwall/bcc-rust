/* Clang's addresses that are never null, compared with a null pointer or
   converted to _Bool: arrays, functions and the operand of &. */
int array[3];
void function(void);
int counter(void);
int *pointer(void);
_Bool truth(void);
int with_parameter(int);
struct S { int member[2]; int *p; } s, *sp;
__attribute__((weak)) int weak_array[2];

int compared(void) {
  if (array == 0)
    return 1;
  if (0 != function)
    return 2;
  if (&s == (void *)0)
    return 3;
  if (s.member != 0)
    return 4;
  if (sp->member == 0)
    return 5;
  if (pointer == 0)
    return 6;
  if (counter == 0)
    return 7;
  if (counter == (void *)0)
    return 8;
  if (with_parameter == 0)
    return 9;
  return 0;
}

int silent(void) {
  return &array[1] == 0 || &function == 0 || (array) == 0 || &(array) == 0
      || array[0] == 0 || &sp->p == 0 || weak_array == 0 || "text" == 0;
}

_Bool converted(void) {
  _Bool b = array;
  b = function;
  if (&s)
    return b;
  if (!sp->member)
    return 0;
  return truth;
}

#define ARRAY array
#define IS_NULL(x) ((x) == 0)
int macros(void) { return ARRAY == 0 || IS_NULL(array) || array == ((void *)0); }

/* A conversion to _Bool looks through parentheses. */
int parenthesized(void) {
  if ((array))
    return 1;
  if (!(&s))
    return 2;
  return ((sp->member)) ? 3 : 4;
}
