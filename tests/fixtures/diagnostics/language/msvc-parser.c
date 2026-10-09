__declspec(dllexport align(16)) unsigned __int64 wide;
__w64 int small;
int * __ptr32 __sptr narrow;
__unaligned int * __ptr64 __uptr pointer;
int (__stdcall *callback)(int);
__forceinline int __cdecl f(void) {
    __try { __leave; } __except(1) { }
    __try { } __finally { }
    __asm { mov eax, [ebx + (4)] }
    _asm nop
    return 0;
}
struct Inner { int member; };
struct Outer { struct Inner; union Tagged { int field; }; };
