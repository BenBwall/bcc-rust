/* An error in a block does not keep GCC from reporting its unused variables
   on leaving it. */
int ends_with_error(int a, unsigned u) {
    int x;
    {
        int y;
        if (a < u)
            return 1;
    }
    int *z = a;
    struct s *p = 0;
    int n = p->member;
    return a < u;
}

/* After the unit's first error GCC reports no unused local typedef, in any
   function. */
int later(int a, unsigned u) {
    typedef int T;
    int w;
    return a < u;
}
