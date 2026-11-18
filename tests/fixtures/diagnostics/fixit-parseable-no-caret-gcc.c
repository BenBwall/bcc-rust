/* GCC writes each hint again after the diagnostic that suggests it,
   quoted source or not, with byte columns; a hint at a token a macro
   expansion produced is dropped. */
#define ID(x) x
int f(_Bool b) {
	int r = ~b;
  r += ID(~b);
#line 40 "renamed.c"
  return r + ~b;
}
