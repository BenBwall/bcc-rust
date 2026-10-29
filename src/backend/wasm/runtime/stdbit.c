/* <stdbit.h> for wasm32-wasip1: the functions behind its macros, for a call
   that suppresses a macro or takes a function's address. Each computes
   with the type-generic builtin the macro expands to.
   C23: 7.18.3-7.18.16, pp. 305-311 (N3220). */
#include <stdbit.h>

#define FUNCTIONS(name, result)                                                \
    FUNCTION(name, _uc, unsigned char, result)                                 \
    FUNCTION(name, _us, unsigned short, result)                                \
    FUNCTION(name, _ui, unsigned int, result)                                  \
    FUNCTION(name, _ul, unsigned long, result)                                 \
    FUNCTION(name, _ull, unsigned long long, result)
#define FUNCTION(name, suffix, type, result)                                   \
    RESULT_##result(type)(stdc_##name##suffix)(type value) {                   \
        return stdc_##name(value);                                             \
    }
#define RESULT_count(type) unsigned int
#define RESULT_bool(type) _Bool
#define RESULT_value(type) type

FUNCTIONS(leading_zeros, count)
FUNCTIONS(leading_ones, count)
FUNCTIONS(trailing_zeros, count)
FUNCTIONS(trailing_ones, count)
FUNCTIONS(first_leading_zero, count)
FUNCTIONS(first_leading_one, count)
FUNCTIONS(first_trailing_zero, count)
FUNCTIONS(first_trailing_one, count)
FUNCTIONS(count_zeros, count)
FUNCTIONS(count_ones, count)
FUNCTIONS(has_single_bit, bool)
FUNCTIONS(bit_width, count)
FUNCTIONS(bit_floor, value)
FUNCTIONS(bit_ceil, value)
