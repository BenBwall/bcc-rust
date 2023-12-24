#include "./float_parsing.h"
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

size_t long_double_to_string_get_size(long_double_t value, int *error) {
  long double ld_value;
  memcpy(&ld_value, value.bytes, sizeof(value));
  int err = snprintf(NULL, 0, "%Lf", ld_value);
  if (err < 0) {
    *error = errno;
    return 0;
  } else {
    *error = 0;
    return err;
  }
}

size_t long_double_to_string(long_double_t value, char *buffer,
                             size_t buffer_size, int *error) {
  long double ld_value;
  memcpy(&ld_value, value.bytes, sizeof(value));
  int err = snprintf(buffer, buffer_size, "%Lf", ld_value);
  if (err < 0) {
    *error = errno;
    return 0;
  } else {
    *error = 0;
    return err;
  }
}

long_double_t string_to_long_double(char const *const s, char **const endptr,
                                    int *const error) {
  long double value = strtold(s, endptr);
  *error = errno;
  long_double_t ret;
  memcpy(ret.bytes, &value, sizeof(value));
  return ret;
}

double string_to_double(char const *const s, char **const endptr,
                        int *const error) {
  double value = strtod(s, endptr);
  *error = errno;
  return value;
}

float string_to_float(char const *const s, char **const endptr,
                      int *const error) {
  float value = strtof(s, endptr);
  *error = errno;
  return value;
}
