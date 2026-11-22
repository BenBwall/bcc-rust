/* cl folds a ?: whose condition is a constant to the operand it selects:
   C4244 measures only that operand and reports there, a parameter named
   only in the other is unreferenced (C4100), last listed first, and C4127
   takes the selected operand, but no condition with a comma. */
typedef unsigned char Byte;
Byte g;
void take(Byte b);

Byte folded(unsigned long long wide, signed char narrow, long long ll)
{
    Byte b = sizeof(long) == 8 ? ll : narrow;
    g = (0 ? wide : narrow);
    g += 1.5 ? narrow : wide;
    take((void *)0 ? ll : narrow);
    (void)b;
    return -(0) ? ll : narrow;
}

Byte selected(unsigned long long wide, int i, long long ll)
{
    Byte b = 1 ? i : ll;
    g = (0 ? wide : ll);
    g /= -0.0 ? i : wide;
    take(sizeof(int) == 4 ? ll : i);
    (void)b;
    return (1, 0) ? i : (signed char)i;
}

int unreferenced(int a, int b, int c)
{
    return (0 && b) + (1 ? a : c) + (int)sizeof(0 ? c : a);
}

void conditions(int i)
{
    while ((1, 0) ? i : 0) {}
    while (1.5 ? 0 : i) {}
    if (0.5 ? 2 : 0) {}
    if (1 ? (0, 2) : 3) {}
    if (0 ? i : (1 ? 4 : i)) {}
}
