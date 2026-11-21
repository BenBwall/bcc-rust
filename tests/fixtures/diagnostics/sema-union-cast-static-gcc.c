/* GCC 14 takes a cast to a union type in a static initializer as constant
   only when the operand it parsed is. */
union U { int i; double d; float f; _Complex double z; long l; };
union U conditional = (union U)(1 ? 1.0 : 2.0);
union U mixed = (union U)(1 ? 1 : 2.0);
union U floating_condition = (union U)(1.0 ? 1 : 2);
union U comparison = (union U)(1.0 == 1.0);
union U complex_comparison = (union U)(1.0 != (_Complex double)1);
union U logical = (union U)(1.0 && 1);
union U converted_comparison = (union U)((long)(1.0 == 1.0));
union U negated = (union U)(-(1 ? 1.0 : 2.0));
union U braced[] = { (union U)(1 ? 1.0 : 2.0) };
void local(void) { static union U s = (union U)(1.0 < 2.0); (void)s; }
/* These are constant: integer operands, and conversions that fold. */
union U integers = (union U)(1 ? 1 : 2);
union U integer_comparison = (union U)(1 == 1);
union U not_floating = (union U)(!1.0);
union U sum = (union U)(1.0 + 1.0);
union U converted = (union U)((int)(1 ? 1.0 : 2.0));
union U widened = (union U)((1 ? 1.0f : 2.0f) + 1.0);
void automatic(void) { union U a = (union U)(1 ? 1.0 : 2.0); (void)a; }
