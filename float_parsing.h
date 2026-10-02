#include <float.h>
#include <stddef.h>
#include <stdint.h>

/* x87 extended precision stores 10 value bytes in a larger object; the
   remaining bytes are padding whose contents a store leaves unspecified. */
#if LDBL_MANT_DIG == 64 && (defined(__i386__) || defined(__x86_64__))
#define LONG_DOUBLE_VALUE_BYTES_ 10
#else
#define LONG_DOUBLE_VALUE_BYTES_ sizeof(long double)
#endif

enum
{
  LONG_DOUBLE_BYTES = sizeof(long double),
  LONG_DOUBLE_VALUE_BYTES = LONG_DOUBLE_VALUE_BYTES_,
  /* Enough for a sign, `0x1.`, one hex digit per four mantissa bits, and a
     binary exponent, with room to spare. */
  LONG_DOUBLE_HEX_CAPACITY = 64,
};

/* Value classes reported by `long_double_classify`. */
enum
{
  FLOAT_CLASS_NONZERO = 0,
  FLOAT_CLASS_ZERO = 1,
  FLOAT_CLASS_INFINITE = 2,
  FLOAT_CLASS_NAN = 3,
};

typedef union long_double_t
{
  uint8_t bytes[sizeof(long double)];
  long double value;
} long_double_t;

long_double_t string_to_long_double(char const *s, char **endptr);
double string_to_double(char const *s, char **endptr);
float string_to_float(char const *s, char **endptr);

int long_double_classify(long_double_t value);

/* Writes the exact value as a C99 hexadecimal floating constant without
   consulting the C library's printf, so the text is identical on every
   host. Returns the number of bytes written, excluding the terminator. */
size_t long_double_to_hex(long_double_t value, char *buffer,
                          size_t buffer_size);
