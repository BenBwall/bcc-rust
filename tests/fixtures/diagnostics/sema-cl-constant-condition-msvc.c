/* cl's C4127 at its level 4: a constant condition other than a literal
   0 or 1, which `while (1)` and `do ... while (0)` write. */
#define TRUE 1
#define DO_ONCE(s) do { s; } while (0)
enum { ONE = 1 };
int g(int);

void conditions(int x)
{
    if (1) g(1);
    while (1) { if (g(2)) break; }
    for (;;) { if (g(3)) break; }
    for (; 1;) { if (g(4)) break; }
    do { g(5); } while (0);
    DO_ONCE(g(6));
    if (TRUE) g(7);
    if ((1)) g(8);
    if (!0) g(9);
    if ('\0') g(10);
    if (1u) g(11);
    if ((int)1) g(12);
    if (sizeof(int) ? 1 : 0) g(13);
    if (2) g(14);
    if (-1) g(15);
    if (ONE) g(16);
    if (sizeof(int) == 4) g(17);
    if (0 && x) g(18);
    if (x && 0) g(19);
    if ((char)1) g(20);
    if (1.0) g(21);
    if ("s") g(22);
    if (&x) g(23);
    if (x
        || 1) g(24);
    if (1
        || x) g(25);
    while (x == x) { if (g(26)) break; }
    x = 2 ? x : 3;
}
