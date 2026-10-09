#include <emmintrin.h>
void f(__m128i v,int i) { v=_mm_shuffle_epi32(v,i); v=_mm_shuffle_epi32(v,256); }
int following;
