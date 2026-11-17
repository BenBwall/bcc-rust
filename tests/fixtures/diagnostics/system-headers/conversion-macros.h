/* Macros whose bodies write the context of an implicit conversion: the
   call, the assignment operator, the `return`, the brace, the designator
   or the comma Clang takes as its context location, or the `##` it steps
   out to from a pasted token. */
int immediate(int value, char selector);
int printf(const char *format, ...);
#define SELECT(value, selector) immediate((value), (selector))
#define IMMEDIATE immediate
#define STORE(place, value) (*(place) = (value))
#define RETURN(value) return (value)
#define PAIR(first) { (first), 2 }
#define WIDE 300
#define INNER .pair.low = 301
#define PRINT_EXTRA(format) printf(format, 1, 2)
#define EXTRA_ARGUMENT 3
#define SYSTEM_COMMA(value) (0, value)
#define HIGH .pair.high = 321
#define DEEP_LAST .holder.last = 324
#define SYSTEM_NAME CAT(immedi, ate)
#define SYSTEM_CAT(a, b) a##b
