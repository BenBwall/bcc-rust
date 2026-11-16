/* Macros whose bodies write the context of an implicit conversion: the
   call, the assignment operator, the `return`, the brace or the designator
   Clang takes as its context location. */
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
