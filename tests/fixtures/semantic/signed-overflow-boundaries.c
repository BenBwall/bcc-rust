/* Signed destination boundaries for all three GCC overflow operations. */
static int add8(long long a,long long b,int overflow,long long wrapped) { signed char r; return __builtin_add_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int sub8(long long a,long long b,int overflow,long long wrapped) { signed char r; return __builtin_sub_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int mul8(long long a,long long b,int overflow,long long wrapped) { signed char r; return __builtin_mul_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int add16(long long a,long long b,int overflow,long long wrapped) { short r; return __builtin_add_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int sub16(long long a,long long b,int overflow,long long wrapped) { short r; return __builtin_sub_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int mul16(long long a,long long b,int overflow,long long wrapped) { short r; return __builtin_mul_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int add32(long long a,long long b,int overflow,long long wrapped) { int r; return __builtin_add_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int sub32(long long a,long long b,int overflow,long long wrapped) { int r; return __builtin_sub_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
static int mul32(long long a,long long b,int overflow,long long wrapped) { int r; return __builtin_mul_overflow(a,b,&r)!=overflow
        || r!=wrapped; }
int main(void) {
    return add8(127LL,1LL,1,-128LL)
        || add8(126LL,1LL,0,127LL)
        || add8(-128LL,-1LL,1,127LL)
        || add8(-127LL,-1LL,0,-128LL)
        || sub8(-128LL,1LL,1,127LL)
        || sub8(-127LL,1LL,0,-128LL)
        || sub8(127LL,-1LL,1,-128LL)
        || sub8(126LL,-1LL,0,127LL)
        || mul8(16LL,8LL,1,-128LL)
        || mul8(15LL,8LL,0,120LL)
        || mul8(-128LL,-1LL,1,-128LL)
        || mul8(-128LL,1LL,0,-128LL)
        || add16(32767LL,1LL,1,-32768LL)
        || add16(32766LL,1LL,0,32767LL)
        || add16(-32768LL,-1LL,1,32767LL)
        || add16(-32767LL,-1LL,0,-32768LL)
        || sub16(-32768LL,1LL,1,32767LL)
        || sub16(-32767LL,1LL,0,-32768LL)
        || sub16(32767LL,-1LL,1,-32768LL)
        || sub16(32766LL,-1LL,0,32767LL)
        || mul16(256LL,128LL,1,-32768LL)
        || mul16(255LL,128LL,0,32640LL)
        || mul16(-32768LL,-1LL,1,-32768LL)
        || mul16(-32768LL,1LL,0,-32768LL)
        || add32(2147483647LL,1LL,1,-2147483648LL)
        || add32(2147483646LL,1LL,0,2147483647LL)
        || add32(-2147483648LL,-1LL,1,2147483647LL)
        || add32(-2147483647LL,-1LL,0,-2147483648LL)
        || sub32(-2147483648LL,1LL,1,2147483647LL)
        || sub32(-2147483647LL,1LL,0,-2147483648LL)
        || sub32(2147483647LL,-1LL,1,-2147483648LL)
        || sub32(2147483646LL,-1LL,0,2147483647LL)
        || mul32(65536LL,32768LL,1,-2147483648LL)
        || mul32(65535LL,32768LL,0,2147450880LL)
        || mul32(-2147483648LL,-1LL,1,-2147483648LL)
        || mul32(-2147483648LL,1LL,0,-2147483648LL); }
