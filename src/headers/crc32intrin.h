/* crc32intrin.h - SSE4.2 CRC32 intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. The CRC32
 * instruction's CRC-32C (Castagnoli) step in plain C: reflected, with the
 * polynomial 0x82F63B78, and no initial or final inversion. */

#ifndef __CRC32INTRIN_H
#define __CRC32INTRIN_H

static __inline__ __attribute__((__always_inline__, __nodebug__)) unsigned int
__bcc_crc32c(unsigned int __crc, unsigned long long __data, int __bytes) {
  int __bit;
  for (__bit = 0; __bit < 8 * __bytes; ++__bit) {
    __crc ^= (unsigned int)(__data >> __bit) & 1;
    __crc = (__crc >> 1) ^ (0x82F63B78u & (0u - (__crc & 1)));
  }
  return __crc;
}

static __inline__ __attribute__((__always_inline__, __nodebug__)) unsigned int
_mm_crc32_u8(unsigned int __c, unsigned char __d) {
  return __bcc_crc32c(__c, __d, 1);
}
static __inline__ __attribute__((__always_inline__, __nodebug__)) unsigned int
_mm_crc32_u16(unsigned int __c, unsigned short __d) {
  return __bcc_crc32c(__c, __d, 2);
}
static __inline__ __attribute__((__always_inline__, __nodebug__)) unsigned int
_mm_crc32_u32(unsigned int __c, unsigned int __d) {
  return __bcc_crc32c(__c, __d, 4);
}
#ifdef __x86_64__
static __inline__ __attribute__((__always_inline__, __nodebug__)) unsigned long long
_mm_crc32_u64(unsigned long long __c, unsigned long long __d) {
  return __bcc_crc32c((unsigned int)__c, __d, 8);
}
#endif

#endif /* __CRC32INTRIN_H */
