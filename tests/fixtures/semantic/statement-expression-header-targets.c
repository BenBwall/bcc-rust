/* Header jumps use enclosing bodies: GCC Statement Exprs.
 * GCC verifies all 21 cases. Clang 23.1.1 crashes or selects an inner target
 * for the 15 cases guarded by CLANG_HEADER_REFERENCE; it verifies six cases.
 * bcc always executes every case against literal results and the GCC oracle.
 */
#ifndef CLANG_HEADER_REFERENCE
static int loop_break_while(void) { int i=0,n=0; while(i++<2) { while (({ ++n; break; 1; })) {} n+=100; } return i!=1 || n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int loop_break_do(void) { int i=0,n=0; while(i++<2) { do {} while (({ ++n; break; 0; })); n+=100; } return i!=1 || n!=1; }
#endif
static int loop_break_init(void) { int i=0,n=0; while(i++<2) { for (({ ++n; break; 0; }); 0; ) {} n+=100; } return i!=1 || n!=1; }
static int loop_break_decl(void) { int i=0,n=0; while(i++<2) { for (int unused=({ ++n; break; 0; }); 0; ) {} n+=100; } return i!=1 || n!=1; }
#ifndef CLANG_HEADER_REFERENCE
static int loop_break_condition(void) { int i=0,n=0; while(i++<2) { for (; ({ ++n; break; 1; }); ) {} n+=100; } return i!=1 || n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int loop_break_increment(void) { int i=0,n=0; while(i++<2) { for (; 1; ({ ++n; break; 0; })) {} n+=100; } return i!=1 || n!=1; }
#endif
static int loop_break_switch(void) { int i=0,n=0; while(i++<2) { switch (({ ++n; break; 0; })) {} n+=100; } return i!=1 || n!=1; }
#ifndef CLANG_HEADER_REFERENCE
static int loop_continue_while(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: while (({ ++n; continue; 1; })) {} n+=100;} n+=100; } return i!=3 || n!=2; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int loop_continue_do(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: do {} while (({ ++n; continue; 0; })); n+=100;} n+=100; } return i!=3 || n!=2; }
#endif
static int loop_continue_init(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: for (({ ++n; continue; 0; }); 0; ) {} n+=100;} n+=100; } return i!=3 || n!=2; }
static int loop_continue_decl(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: for (int unused=({ ++n; continue; 0; }); 0; ) {} n+=100;} n+=100; } return i!=3 || n!=2; }
#ifndef CLANG_HEADER_REFERENCE
static int loop_continue_condition(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: for (; ({ ++n; continue; 1; }); ) {} n+=100;} n+=100; } return i!=3 || n!=2; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int loop_continue_increment(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: for (; 1; ({ ++n; continue; 0; })) {} n+=100;} n+=100; } return i!=3 || n!=2; }
#endif
static int loop_continue_switch(void) { int i=0,n=0; while(i++<2) { switch(0) {case 0: switch (({ ++n; continue; 0; })) {} n+=100;} n+=100; } return i!=3 || n!=2; }
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_while(void) { int n=0; switch(0) {case 0: while (({ ++n; break; 1; })) {} n+=100;} return n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_do(void) { int n=0; switch(0) {case 0: do {} while (({ ++n; break; 0; })); n+=100;} return n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_init(void) { int n=0; switch(0) {case 0: for (({ ++n; break; 0; }); 0; ) {} n+=100;} return n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_decl(void) { int n=0; switch(0) {case 0: for (int unused=({ ++n; break; 0; }); 0; ) {} n+=100;} return n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_condition(void) { int n=0; switch(0) {case 0: for (; ({ ++n; break; 1; }); ) {} n+=100;} return n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_increment(void) { int n=0; switch(0) {case 0: for (; 1; ({ ++n; break; 0; })) {} n+=100;} return n!=1; }
#endif
#ifndef CLANG_HEADER_REFERENCE
static int switch_break_switch(void) { int n=0; switch(0) {case 0: switch (({ ++n; break; 0; })) {} n+=100;} return n!=1; }
#endif
int main(void) {
    int bad = 0;
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_break_while();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_break_do();
#endif
    bad |= loop_break_init();
    bad |= loop_break_decl();
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_break_condition();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_break_increment();
#endif
    bad |= loop_break_switch();
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_continue_while();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_continue_do();
#endif
    bad |= loop_continue_init();
    bad |= loop_continue_decl();
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_continue_condition();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= loop_continue_increment();
#endif
    bad |= loop_continue_switch();
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_while();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_do();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_init();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_decl();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_condition();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_increment();
#endif
#ifndef CLANG_HEADER_REFERENCE
    bad |= switch_break_switch();
#endif
    return bad;
}
