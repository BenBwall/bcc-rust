#define MESSAGE "from a macro"
#pragma GCC warning MESSAGE
#pragma GCC warning ("joined" " message")
#pragma GCC warning 42
#pragma GCC error "stop here"
int value;
