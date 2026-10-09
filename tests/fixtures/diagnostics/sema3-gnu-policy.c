int f(void) {__label__ L; void *p=&&L; int nested(void) {return 0;} goto *p; L: switch(1) {case 1 ... 3:;} return ({int x=1; x;});}
