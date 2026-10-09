/* bcc declarations for MSVC compiler-provided intrinsics.
 * Signatures independently queried and checked against Clang 23.1.1. */
#ifndef __BCC_MSVC_INTRINSICS_H
#define __BCC_MSVC_INTRINSICS_H
#ifdef _MSC_VER
long _InterlockedAnd(volatile long *, long);
short _InterlockedAnd16(volatile short *, short);
char _InterlockedAnd8(volatile char *, char);
long _InterlockedCompareExchange(volatile long *, long, long);
short _InterlockedCompareExchange16(volatile short *, short, short);
long long _InterlockedCompareExchange64(volatile long long *, long long, long long);
char _InterlockedCompareExchange8(volatile char *, char, char);
long _InterlockedExchange(volatile long *, long);
short _InterlockedExchange16(volatile short *, short);
char _InterlockedExchange8(volatile char *, char);
long _InterlockedExchangeAdd(volatile long *, long);
short _InterlockedExchangeAdd16(volatile short *, short);
char _InterlockedExchangeAdd8(volatile char *, char);
long _InterlockedIncrement(volatile long *);
long _InterlockedOr(volatile long *, long);
short _InterlockedOr16(volatile short *, short);
char _InterlockedOr8(volatile char *, char);
long _InterlockedXor(volatile long *, long);
short _InterlockedXor16(volatile short *, short);
char _InterlockedXor8(volatile char *, char);
short __iso_volatile_load16(const volatile short *);
int __iso_volatile_load32(const volatile int *);
long long __iso_volatile_load64(const volatile long long *);
char __iso_volatile_load8(const volatile char *);
void __iso_volatile_store16(volatile short *, short);
void __iso_volatile_store32(volatile int *, int);
void __iso_volatile_store64(volatile long long *, long long);
void __iso_volatile_store8(volatile char *, char);
#endif
#endif
