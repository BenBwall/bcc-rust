/* `__extension__` silences GCC's -pedantic reports about its operand, as
   glibc's assert() relies on: `__extension__ __PRETTY_FUNCTION__`. */
const char *pretty(void) { return __PRETTY_FUNCTION__; }
const char *quiet_pretty(void) { return __extension__ __PRETTY_FUNCTION__; }
const char *name(void) { return __FUNCTION__; }
const char *quiet_name(void) { return __extension__ __FUNCTION__; }
const char *nested(int c) { return __extension__ (c ? __FUNCTION__ : __PRETTY_FUNCTION__); }
const char *outside(int c) { return c ? __extension__ __FUNCTION__ : __FUNCTION__; }
