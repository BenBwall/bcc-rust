/* Any translation unit that includes the MinGW-w64 <windows.h> is refused on the Windows targets:
   winnt.h includes <x86intrin.h>/<emmintrin.h>, and bcc lowers every static inline function of its built-in
   mmintrin.h even though none is used: "not yet supported by lowering: vector types".
   bcc --target=x86_64-w64-windows-gnu -idirafter <mingw>/x86_64-w64-mingw32/include -D__CRT__NO_INLINE */
#include <windows.h>
int main(void) { return 0; }
