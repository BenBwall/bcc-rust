void f(void) {
    for (enum { A = 1 }; ; ) break;
    for (struct S { int x; }; ; ) break;
}
