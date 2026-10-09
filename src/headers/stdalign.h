/* C11: §7.15, p. 268; PDF p. 286. */
#ifndef __BCC_STDALIGN_H
#define __BCC_STDALIGN_H
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ < 202311L
#define alignas _Alignas
#define alignof _Alignof
#define __alignas_is_defined 1
#define __alignof_is_defined 1
#endif
#endif
#endif
