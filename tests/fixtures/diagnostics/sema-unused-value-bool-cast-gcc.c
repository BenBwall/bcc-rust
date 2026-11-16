/* GCC's build_c_cast leaves a cast to the operand's own type out, `_Bool`
   too, so the cast hides nothing from warn_if_unused_value: an effect
   inside is enough. gnulib's ckd_add is `(bool) __builtin_add_overflow`. */
int r, g(void);
_Bool h(void);
void f(int a, _Bool b, int *p) {
  ((_Bool) __builtin_add_overflow(a, 1, &r));
  ((_Bool) h());
  ((int) g());
  ((int) a++);
  ((_Bool) (b = 1));
  ((const int) a++);
  ((int *) p++);
  ((_Bool) a++);
  ((long) a++);
  ((_Bool) g());
}
