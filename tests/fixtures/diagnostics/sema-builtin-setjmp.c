void *buffer[5];
int words[5];
void f(int value) {
    __builtin_longjmp(buffer, value);
    __builtin_longjmp(buffer, 2);
    __builtin_longjmp(buffer, 0);
    __builtin_longjmp(buffer, 1.5);
    __builtin_setjmp(5);
    __builtin_setjmp(buffer, 1);
    __builtin_longjmp(buffer);
    __builtin_longjmp(words, 1);
    __builtin_setjmp(&words);
}
int following;
