/* TCC knows __QI__, __HI__, __SI__, __DI__ and __word__ as __mode__, warns
   of any other mode in words that end in a newline, and ignores `mode`. */
typedef int bad __attribute__((__mode__(__foo__)));
typedef float vector __attribute__((__mode__(__V4SF__)));
typedef int wide __attribute__((__mode__(__TI__)));
typedef int plain __attribute__((__mode__(QI)));
typedef int unknown __attribute__((mode(__foo__)));
typedef int __attribute__((__mode__(__QI__))) byte;
typedef int after __attribute__((__mode__(__QI__)));
typedef float real __attribute__((__mode__(__QI__)));
typedef unsigned __attribute__((__mode__(__word__))) word;
_Static_assert(sizeof(bad) == 4 && sizeof(wide) == 4 && sizeof(plain) == 4, "kept");
_Static_assert(sizeof(byte) == 1 && sizeof(after) == 4 && sizeof(real) == 4, "placed");
_Static_assert(sizeof(word) == 4 && (word)-1 > 0, "word");
