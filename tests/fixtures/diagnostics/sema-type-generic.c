int variable;
int choose(void) { return __builtin_choose_expr(variable, 1, 2); }
int duplicate(void) { return _Generic(1, int: 1, int: 2); }
int missing(void) { return _Generic(1, double: 1); }
int incomplete(void) { return _Generic(1, int[]: 1, default: 2); }
int nonarithmetic(void) { return __real__ &variable; }
int arity(void) { return __builtin_classify_type(); }
int after;
