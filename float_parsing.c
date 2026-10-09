#include "./float_parsing.h"
#include <float.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static long double load_long_double(long_double_t const value)
{
  long double ld_value;
  memcpy(&ld_value, value.bytes, sizeof(ld_value));
  return ld_value;
}

long_double_t string_to_long_double(char const *const s, char **const endptr)
{
  /* Copy only the value bytes so padding is deterministically zero instead
     of stack contents left unspecified by the long double store. */
  long_double_t ret;
  long double const value = strtold(s, endptr);
  memset(ret.bytes, 0, sizeof(ret.bytes));
  memcpy(ret.bytes, &value, LONG_DOUBLE_VALUE_BYTES);
  return ret;
}

double string_to_double(char const *const s, char **const endptr)
{
#if defined(__MINGW32__)
  /* The legacy Windows CRT strtod does not implement C99 hex floats. */
  return __mingw_strtod(s, endptr);
#else
  return strtod(s, endptr);
#endif
}

float string_to_float(char const *const s, char **const endptr)
{
#if defined(__MINGW32__)
  return __mingw_strtof(s, endptr);
#else
  return strtof(s, endptr);
#endif
}

int long_double_classify(long_double_t const value)
{
  long double const ld_value = load_long_double(value);
  if (isnan(ld_value))
  {
    return FLOAT_CLASS_NAN;
  }
  if (isinf(ld_value))
  {
    return FLOAT_CLASS_INFINITE;
  }
  return ld_value == 0 ? FLOAT_CLASS_ZERO : FLOAT_CLASS_NONZERO;
}

/* C99 §6.6p4 and §6.3.1.8: preserve native x87 long-double precision and
   round each semantic operation to the selected target component type. */
static long_double_t store_long_double(long double value)
{
  long_double_t result;
  memset(result.bytes, 0, sizeof(result.bytes));
  memcpy(result.bytes, &value, LONG_DOUBLE_VALUE_BYTES);
  return result;
}

long_double_t long_double_from_double(double value)
{
  return store_long_double((long double)value);
}

long_double_t long_double_arithmetic(long_double_t left, long_double_t right,
                                    int operation, int precision)
{
  long double a = load_long_double(left), b = load_long_double(right);
  long double result;
  switch (operation) {
  case 0: result = a + b; break;
  case 1: result = a - b; break;
  case 2: result = a * b; break;
  case 3: result = a / b; break;
  case 5: result = -a; break;
  default: result = a; break;
  }
  if (precision == 1) { volatile float rounded = (float)result; result = rounded; }
  if (precision == 2) { volatile double rounded = (double)result; result = rounded; }
  return store_long_double(result);
}

int long_double_compare(long_double_t left, long_double_t right)
{
  long double a = load_long_double(left), b = load_long_double(right);
  if (isnan(a) || isnan(b))
  {
    return 2;
  }
  return a < b ? -1 : a > b ? 1 : 0;
}

size_t long_double_to_hex(long_double_t const value, char *const buffer,
                          size_t const buffer_size)
{
  static char const hex_digits[] = "0123456789abcdef";
  /* Every step below is exact: scaling by 2 or 16 and subtracting an
     integer part never rounds a finite binary value. */
  enum
  {
    MAX_FRACTION_DIGITS = (LDBL_MANT_DIG + 3) / 4
  };
  char text[LONG_DOUBLE_HEX_CAPACITY];
  size_t length = 0;
  long double ld_value = load_long_double(value);

  if (signbit(ld_value))
  {
    text[length++] = '-';
    ld_value = -ld_value;
  }
  if (isnan(ld_value))
  {
    memcpy(text + length, "nan", 3);
    length += 3;
  }
  else if (isinf(ld_value))
  {
    memcpy(text + length, "inf", 3);
    length += 3;
  }
  else if (ld_value == 0)
  {
    memcpy(text + length, "0x0p+0", 6);
    length += 6;
  }
  else
  {
    int exponent;
    /* frexpl normalizes subnormals too, so every finite nonzero value
       prints as 0x1.<fraction>p<exponent>. */
    long double fraction = frexpl(ld_value, &exponent) * 2 - 1;
    exponent -= 1;
    memcpy(text + length, "0x1", 3);
    length += 3;
    if (fraction != 0)
    {
      text[length++] = '.';
      for (int digits = 0; fraction != 0 && digits < MAX_FRACTION_DIGITS;
           ++digits)
      {
        int digit;
        fraction *= 16;
        digit = (int)fraction;
        text[length++] = hex_digits[digit];
        fraction -= digit;
      }
    }
    length += (size_t)snprintf(text + length, sizeof(text) - length, "p%+d",
                               exponent);
  }

  if (buffer_size == 0)
  {
    return 0;
  }
  if (length >= buffer_size)
  {
    length = buffer_size - 1;
  }
  memcpy(buffer, text, length);
  buffer[length] = '\0';
  return length;
}
