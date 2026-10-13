/* GNU Statement Exprs; C99 6.8.1p4: labels preserve the tail expression. */
int main(void) {
    int a = ({ int x = 3; goto out; x = 4; out: x; });
    int b = ({ int x = 5; goto second; first: second: (x); });
    int c = ({ int x = 0; switch (1) { case 1: x = 7; goto case_out;
               default: x = 99; } case_out: x; });
    int d = ({ int x = 0; switch (2) { case 1: x = 99; break;
               default: x = 9; goto default_out; } default_out: x; });
    int array[2] = { 11, 13 };
    int *p = ({ goto pointer_out; pointer_out: array; });
    return a != 3 || b != 5 || c != 7 || d != 9 || p != array || p[1] != 13;
}
