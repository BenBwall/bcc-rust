/* C99: §7.16, p. 253; PDF p. 265. Target definitions come from src/target.rs. */
#ifndef __BCC_STDBOOL_H
#define __BCC_STDBOOL_H
#define __bool_true_false_are_defined 1
#ifndef __STDC_VERSION__
#define bool _Bool
#define true 1
#define false 0
#else
#if __STDC_VERSION__ < 202311L
#define bool _Bool
#define true 1
#define false 0
#endif
#endif
#endif
