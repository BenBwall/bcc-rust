#define __COUNTER__ 77
int counter = __COUNTER__;
#undef __has_attribute
#define __has_attribute(x) 1
int available = __has_attribute(unused);
