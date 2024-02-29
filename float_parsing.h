#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>

enum
{
  LONG_DOUBLE_BYTES = sizeof(long double),
};

typedef struct long_double_t
{
  uint8_t bytes[sizeof(long double)];
} long_double_t;

typedef union operand_value_t
{
  int64_t signed_value;
  uint64_t unsigned_value;
} operand_value_t;

typedef struct operand_t
{
  bool is_unsigned;
  operand_value_t value;
} operand_t;

size_t
long_double_to_string_get_size(long_double_t value, int *error);

size_t long_double_to_string(long_double_t value, char *buffer,
                             size_t buffer_size, int *error);
long_double_t string_to_long_double(char const *s, char **endptr, int *error);

operand_t long_double_to_operand(long_double_t value, int *error);

double string_to_double(char const *s, char **endptr, int *error);
float string_to_float(char const *s, char **endptr, int *error);
