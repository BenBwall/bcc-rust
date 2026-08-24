#include "./float_parsing.h"
#include <inttypes.h>
#include <float.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

size_t long_double_to_string_get_size(long_double_t value)
{
  long double ld_value;
  char probe[1];
  memcpy(&ld_value, value.bytes, sizeof(value));
#if defined(__MINGW32__) && LDBL_MANT_DIG > DBL_MANT_DIG
  int err = __mingw_snprintf(probe, sizeof(probe), "%Lf", ld_value);
#elif LDBL_MANT_DIG == DBL_MANT_DIG
  int err = snprintf(probe, sizeof(probe), "%f", (double)ld_value);
#else
  int err = snprintf(probe, sizeof(probe), "%Lf", ld_value);
#endif
  return (size_t) err + 1;
}

size_t long_double_to_string(long_double_t value, char *buffer,
                             size_t buffer_size)
{
  long double ld_value;
  memcpy(&ld_value, value.bytes, sizeof(value));
#if defined(__MINGW32__) && LDBL_MANT_DIG > DBL_MANT_DIG
  int err = __mingw_snprintf(buffer, buffer_size, "%Lf", ld_value);
#elif LDBL_MANT_DIG == DBL_MANT_DIG
  int err = snprintf(buffer, buffer_size, "%f", (double)ld_value);
#else
  int err = snprintf(buffer, buffer_size, "%Lf", ld_value);
#endif
  return (size_t) err;
}

long_double_t string_to_long_double(char const *const s, char **const endptr)
{
  long_double_t ret;
  ret.value = strtold(s, endptr);
  return ret;
}

static bool is_integer(long double ld_value)
{
  return ld_value == ceill(ld_value);
}

operand_t long_double_to_operand(long_double_t value, int *error)
{
  operand_t ret;
  long double ld_value;
  memcpy(&ld_value, value.bytes, sizeof(value));
  if (!is_integer(ld_value) || ld_value < INT64_MIN || ld_value > UINT64_MAX)
  {
    *error = 1;
  }
  else
  {
    *error = 0;
  }
  if (ld_value < 0)
  {
    ret.is_unsigned = false;
    ret.value.signed_value = (int64_t)ld_value;
  }
  else
  {
    ret.is_unsigned = true;
    ret.value.unsigned_value = (uint64_t)ld_value;
  }
  return ret;
}
