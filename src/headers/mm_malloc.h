/* Not ISO C: GCC and Clang provide this resource header, and MinGW-w64's
   <malloc.h> includes it for _mm_malloc and _mm_free, aligned allocation for
   x86 vector code. The logic follows Clang's header: Windows runtimes
   allocate through their aligned allocators and other libraries through
   posix_memalign. */
#ifndef __BCC_MM_MALLOC_H
#define __BCC_MM_MALLOC_H
#include <stdlib.h>
#ifdef _WIN32
#include <malloc.h>
#else
extern int posix_memalign(void **__memptr, size_t __alignment, size_t __size);
#endif
/* The MSVC runtime's <malloc.h> may define both names as macros. */
#if !(defined(_WIN32) && defined(_mm_malloc))
static __inline__ void *_mm_malloc(size_t __size, size_t __align) {
  void *__memory;
  if (__align == 1)
    return malloc(__size);
  /* A power-of-two alignment is at least that of a pointer. */
  if (!(__align & (__align - 1)) && __align < sizeof(void *))
    __align = sizeof(void *);
#if defined(__MINGW32__)
  __memory = __mingw_aligned_malloc(__size, __align);
#elif defined(_WIN32)
  __memory = _aligned_malloc(__size, __align);
#else
  if (posix_memalign(&__memory, __align, __size))
    return 0;
#endif
  return __memory;
}

static __inline__ void _mm_free(void *__p) {
#if defined(__MINGW32__)
  __mingw_aligned_free(__p);
#elif defined(_WIN32)
  _aligned_free(__p);
#else
  free(__p);
#endif
}
#endif
#endif
