int *ip; unsigned *up; const int *cip; void *vp; int (*f1)(int); int (*f2)(double);
void sign(int c) { (void)(c ? ip : up); }
void functions(int c) { (void)(c ? f1 : f2); }
void function_void(int c) { (void)(c ? f1 : vp); }
void integer(int c, long l) { int *r = c ? ip : l; (void)r; }
void integer_first(int c) { (void)(c ? 1 : ip); }
void merged(int c) { int *r = c ? ip : cip; (void)r; }
void null_arm(int c) { (void)(c ? ip : 0); }
int (*pa)[3]; const int (*cpa)[3];
void array_qualifiers(int c) { (void)(c ? pa : cpa); }
int following_valid;
