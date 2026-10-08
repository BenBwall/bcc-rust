__declspec int broken;
int f(void) {
    __try { }
    __leave
    __asm mov eax, [ebx
    return 0;
}
int following;
