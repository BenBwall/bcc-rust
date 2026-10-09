struct Bits { unsigned __int128 too_wide:129; __int128 named_zero:0; };
int enormous[(__uint128_t)-1];
enum OutOfRange { huge = (__uint128_t)-1 };
void cases(__uint128_t u) {
    switch (u) {
        case ((__uint128_t)1 << 127) ... (__uint128_t)-1: break;
        case (__uint128_t)-1: break;
    }
}
int following;
